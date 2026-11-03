#include "svg.h"

#include <include/core/SkCanvas.h>
#include <include/core/SkColorFilter.h>
#include <include/core/SkPaint.h>
#include <include/core/SkPath.h>
#include <include/core/SkPathBuilder.h>
#include <include/core/SkPictureRecorder.h>
#include <include/core/SkRRect.h>
#include <include/core/SkSamplingOptions.h>
#include <include/core/SkShader.h>
#include <include/effects/SkDashPathEffect.h>
#include <include/effects/SkGradient.h>
#include <include/effects/SkLumaColorFilter.h>
#include <include/pathops/SkPathOps.h>
#include <include/utils/SkParsePath.h>

#include <react/renderer/components/rnsvg/ComponentDescriptors.h>
#include <react/renderer/components/rnsvg/Props.h>
#include <react/renderer/components/rnsvg/RNSVGComponentDescriptors.h>
#include <react/renderer/components/rnsvg/RNSVGImageComponentDescriptor.h>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <deque>
#include <iostream>
#include <map>
#include <set>
#include <type_traits>

using namespace facebook::react;

namespace oxyos {

namespace {

// What an element inherits from its group unless its `propList` names it, as react-native-svg's
// native renderers resolve it; the root's are SVG's initial values. Transforms are the canvas's:
// each element concatenates its own around what it draws.
struct Inherited {
  folly::dynamic fill = folly::dynamic::object("type", 0)("payload", static_cast<int64_t>(0xff000000));
  Float fillOpacity = 1;
  int fillRule = 1;  // react-native-svg: 0 evenodd, 1 nonzero
  folly::dynamic stroke = nullptr;
  Float strokeOpacity = 1;
  folly::dynamic strokeWidth = 1;
  int strokeLinecap = 0, strokeLinejoin = 0;
  folly::dynamic strokeDasharray = nullptr;
  Float strokeDashoffset = 0, strokeMiterlimit = 4;
  uint32_t color = 0xff000000;  // currentColor
  float opacity = 1;
};

// What one drawing needs: the viewport percentages resolve against, the canvas it records onto,
// and the SVG view's definitions (gradients, patterns, clip paths, masks) by their `id`, wherever
// in its tree they are, as Android's SvgView collects them before it draws.
struct Drawing {
  const ViewTree* tree = nullptr;
  float viewportWidth = 0, viewportHeight = 0;
  SkCanvas* canvas = nullptr;
  const std::map<std::string, const View*>* definitions = nullptr;
  int depth = 0;  // patterns and masks drawn inside others, so a definition using itself ends
};

constexpr int kMaxDepth = 8;

void unsupported(const std::string& what) {
  static std::set<std::string> reported;
  if (reported.insert(what).second) std::cerr << "[host] unsupported on OxyOS: react-native-svg " << what << "\n";
}

std::string nameOf(const View& view) { return view.view.componentName ? view.view.componentName : ""; }

const View* find(const Drawing& drawing, Tag tag) {
  auto it = drawing.tree->views().find(tag);
  return it == drawing.tree->views().end() ? nullptr : &it->second;
}

// The definition `id` refers to, if it is one of `kind`.
const View* definition(const Drawing& drawing, const std::string& id, const char* kind) {
  auto it = drawing.definitions->find(id);
  if (it == drawing.definitions->end() || nameOf(*it->second) != kind) return nullptr;
  return it->second;
}

// A length: a number, or a string with a number, a percentage of the viewport (`along` its width,
// its height, or its normalized diagonal) or one of the units SVG converts to user units.
enum class Along { Width, Height, Diagonal };
float length(const folly::dynamic& value, const Drawing& drawing, Along along, float fallback = 0) {
  if (value.isNumber()) return static_cast<float>(value.asDouble());
  if (!value.isString()) return fallback;
  const std::string& text = value.getString();
  char* end = nullptr;
  float number = std::strtof(text.c_str(), &end);
  if (end == text.c_str()) return fallback;
  std::string unit(end);
  if (unit == "%") {
    float base = along == Along::Width    ? drawing.viewportWidth
                 : along == Along::Height ? drawing.viewportHeight
                                          : std::sqrt((drawing.viewportWidth * drawing.viewportWidth +
                                                       drawing.viewportHeight * drawing.viewportHeight) /
                                                      2);
    return number / 100 * base;
  }
  if (unit.empty() || unit == "px") return number;
  if (unit == "pt") return number * 1.25f;
  if (unit == "pc") return number * 15;
  if (unit == "mm") return number * 3.543307f;
  if (unit == "cm") return number * 35.43307f;
  if (unit == "in") return number * 90;
  unsupported("length unit " + unit);
  return fallback;
}

// A coordinate in object bounding box units: a fraction of the box, given as a number or a
// percentage (react-native-svg's PropHelper.fromRelative with the box as the base).
float fraction(const folly::dynamic& value, float fallback) {
  if (value.isNumber()) return static_cast<float>(value.asDouble());
  if (!value.isString()) return fallback;
  const std::string& text = value.getString();
  char* end = nullptr;
  float number = std::strtof(text.c_str(), &end);
  if (end == text.c_str()) return fallback;
  return std::string(end) == "%" ? number / 100 : number;
}

// A React Native colour as 0xAARRGGBB.
uint32_t argb(const SharedColor& color) {
  auto c = colorComponentsFromColor(color);
  auto byte = [](float v) { return static_cast<uint32_t>(std::lround(std::clamp(v, 0.0f, 1.0f) * 255)); };
  return (byte(c.alpha) << 24) | (byte(c.red) << 16) | (byte(c.green) << 8) | byte(c.blue);
}

SkColor4f withAlpha(uint32_t color, float opacity) {
  SkColor4f c = SkColor4f::FromColor(color);
  c.fA *= std::clamp(opacity, 0.0f, 1.0f);
  return c;
}

bool listed(const std::vector<std::string>& propList, const char* name) {
  return std::find(propList.begin(), propList.end(), name) != propList.end();
}

// SVG's (a, b, c, d, e, f) as react-native-svg gives a transform: x' = a·x + c·y + e,
// y' = b·x + d·y + f. The identity for anything else.
SkMatrix matrixOf(const std::vector<Float>& m) {
  if (m.size() != 6) return SkMatrix::I();
  return SkMatrix::MakeAll(m[0], m[2], m[4], m[1], m[3], m[5], 0, 0, 1);
}

// The element's own paint over what it inherits, and its opacity on top.
template <typename P>
Inherited resolve(const P& props, const Inherited& parent) {
  Inherited own = parent;
  const auto& list = props.propList;
  if (listed(list, "fill")) own.fill = props.fill;
  if (listed(list, "fillOpacity")) own.fillOpacity = props.fillOpacity;
  if (listed(list, "fillRule")) own.fillRule = props.fillRule;
  if (listed(list, "stroke")) own.stroke = props.stroke;
  if (listed(list, "strokeOpacity")) own.strokeOpacity = props.strokeOpacity;
  if (listed(list, "strokeWidth")) own.strokeWidth = props.strokeWidth;
  if (listed(list, "strokeLinecap")) own.strokeLinecap = props.strokeLinecap;
  if (listed(list, "strokeLinejoin")) own.strokeLinejoin = props.strokeLinejoin;
  if (listed(list, "strokeDasharray")) own.strokeDasharray = props.strokeDasharray;
  if (listed(list, "strokeDashoffset")) own.strokeDashoffset = props.strokeDashoffset;
  if (listed(list, "strokeMiterlimit")) own.strokeMiterlimit = props.strokeMiterlimit;
  if (props.color) own.color = argb(props.color);
  own.opacity *= props.opacity;
  if (!props.markerStart.empty() || !props.markerMid.empty() || !props.markerEnd.empty()) unsupported("markers");
  if (!props.filter.empty()) unsupported("filters (drawn unfiltered)");
  return own;
}

// From a box (`from`) to a viewport (`to`): SVG's preserveAspectRatio (react-native-svg's align and
// meetOrSlice: 0 meet, 1 slice, 2 none), as its ViewBox.getTransform.
SkMatrix fit(const SkRect& from, const SkRect& to, const std::string& alignment, int meetOrSlice) {
  if (from.width() <= 0 || from.height() <= 0) return SkMatrix::I();
  float sx = to.width() / from.width(), sy = to.height() / from.height();
  if (alignment == "none" || meetOrSlice == 2) {
    return SkMatrix::MakeAll(sx, 0, to.x() - from.x() * sx, 0, sy, to.y() - from.y() * sy, 0, 0, 1);
  }
  float s = meetOrSlice == 1 ? std::max(sx, sy) : std::min(sx, sy);
  const std::string& align = alignment.empty() ? std::string("xMidYMid") : alignment;
  auto offset = [&](const char* min, const char* mid, float free) {
    if (align.find(min) != std::string::npos) return 0.0f;
    if (align.find(mid) != std::string::npos) return free / 2;
    return free;
  };
  float tx = to.x() + offset("xMin", "xMid", to.width() - from.width() * s);
  float ty = to.y() + offset("YMin", "YMid", to.height() - from.height() * s);
  return SkMatrix::MakeAll(s, 0, tx - from.x() * s, 0, s, ty - from.y() * s, 0, 0, 1);
}

// From a view box to a viewport of `width` × `height` at the origin.
template <typename P>
SkMatrix viewBox(const P& props, float width, float height) {
  if (props.vbWidth <= 0 || props.vbHeight <= 0) return SkMatrix::I();
  return fit(SkRect::MakeXYWH(props.minX, props.minY, props.vbWidth, props.vbHeight), SkRect::MakeWH(width, height),
             props.align, props.meetOrSlice);
}

// ---- Geometry --------------------------------------------------------------------------------

bool isShape(const std::string& name) {
  return name == "RNSVGPath" || name == "RNSVGRect" || name == "RNSVGCircle" || name == "RNSVGEllipse" ||
         name == "RNSVGLine";
}

// Elements whose children are drawn (or, in a clip path, outline it) in their coordinates.
bool isContainer(const std::string& name) {
  return name == "RNSVGGroup" || name == "RNSVGClipPath" || name == "RNSVGMask" || name == "RNSVGPattern" ||
         name == "RNSVGDefs";
}

// A shape's outline in its own user space (its transform not applied), as react-native-svg's
// shape views make it.
std::optional<SkPath> shapePath(const Drawing& drawing, const View& view, const std::string& name) {
  const auto& props = *view.view.props;
  if (name == "RNSVGPath") {
    auto path = SkParsePath::FromSVGString(static_cast<const RNSVGPathProps&>(props).d.c_str());
    if (!path) unsupported("path data SVG rejects (shape left out)");
    return path;
  }
  if (name == "RNSVGRect") {
    const auto& p = static_cast<const RNSVGRectProps&>(props);
    SkRect rect = SkRect::MakeXYWH(length(p.x, drawing, Along::Width), length(p.y, drawing, Along::Height),
                                   length(p.width, drawing, Along::Width), length(p.height, drawing, Along::Height));
    // SVG's corners: one radius given stands for both; each at most half the side.
    float rx = length(p.rx, drawing, Along::Width), ry = length(p.ry, drawing, Along::Height);
    if (rx <= 0) rx = ry;
    if (ry <= 0) ry = rx;
    rx = std::min(rx, rect.width() / 2), ry = std::min(ry, rect.height() / 2);
    return rx > 0 && ry > 0 ? SkPath::RRect(SkRRect::MakeRectXY(rect, rx, ry)) : SkPath::Rect(rect);
  }
  if (name == "RNSVGCircle") {
    const auto& p = static_cast<const RNSVGCircleProps&>(props);
    return SkPath::Circle(length(p.cx, drawing, Along::Width), length(p.cy, drawing, Along::Height),
                          length(p.r, drawing, Along::Diagonal));
  }
  if (name == "RNSVGEllipse") {
    const auto& p = static_cast<const RNSVGEllipseProps&>(props);
    float cx = length(p.cx, drawing, Along::Width), cy = length(p.cy, drawing, Along::Height);
    float rx = length(p.rx, drawing, Along::Width), ry = length(p.ry, drawing, Along::Height);
    return SkPath::Oval(SkRect::MakeLTRB(cx - rx, cy - ry, cx + rx, cy + ry));
  }
  if (name == "RNSVGLine") {
    const auto& p = static_cast<const RNSVGLineProps&>(props);
    SkPathBuilder line;
    line.moveTo(length(p.x1, drawing, Along::Width), length(p.y1, drawing, Along::Height))
        .lineTo(length(p.x2, drawing, Along::Width), length(p.y2, drawing, Along::Height));
    return line.detach();
  }
  return std::nullopt;
}

// An image element's viewport: x, y, width and height in user units.
SkRect imageRect(const Drawing& drawing, const View& view) {
  const auto& p = static_cast<const RNSVGImageProps&>(*view.view.props);
  return SkRect::MakeXYWH(length(p.x, drawing, Along::Width), length(p.y, drawing, Along::Height),
                          length(p.width, drawing, Along::Width), length(p.height, drawing, Along::Height));
}

// The props every react-native-svg element has (codegen gives each its own class, with the same
// members).
struct Common {
  const std::string* display;
  const std::vector<Float>* matrix;
  const std::string* clipPath;
  int clipRule;
  const std::string* mask;
};
template <typename P>
Common common(const P& p) {
  return {&p.display, &p.matrix, &p.clipPath, p.clipRule, &p.mask};
}
std::optional<Common> commonOf(const View& view) {
  const std::string name = nameOf(view);
  const auto& props = *view.view.props;
  if (name == "RNSVGGroup") return common(static_cast<const RNSVGGroupProps&>(props));
  if (name == "RNSVGPath") return common(static_cast<const RNSVGPathProps&>(props));
  if (name == "RNSVGRect") return common(static_cast<const RNSVGRectProps&>(props));
  if (name == "RNSVGCircle") return common(static_cast<const RNSVGCircleProps&>(props));
  if (name == "RNSVGEllipse") return common(static_cast<const RNSVGEllipseProps&>(props));
  if (name == "RNSVGLine") return common(static_cast<const RNSVGLineProps&>(props));
  if (name == "RNSVGClipPath") return common(static_cast<const RNSVGClipPathProps&>(props));
  if (name == "RNSVGMask") return common(static_cast<const RNSVGMaskProps&>(props));
  if (name == "RNSVGPattern") return common(static_cast<const RNSVGPatternProps&>(props));
  if (name == "RNSVGImage") return common(static_cast<const RNSVGImageProps&>(props));
  return std::nullopt;
}

// The element's outline in its own user space, as react-native-svg's getPath gives it: a shape's
// path; a container's, its children's, each through its transform, united (Region.Op.UNION) or
// one after the other (a clip path with clipRule evenodd, whose path is then filled even-odd).
std::optional<SkPath> outline(const Drawing& drawing, const View& view, bool unite) {
  const std::string name = nameOf(view);
  auto props = commonOf(view);
  if (props && *props->display == "none") return std::nullopt;
  if (isShape(name)) return shapePath(drawing, view, name);
  if (name == "RNSVGImage") return SkPath::Rect(imageRect(drawing, view));
  if (!isContainer(name)) return std::nullopt;
  std::optional<SkPath> united;
  SkPathBuilder joined;
  bool any = false;
  for (Tag tag : view.children) {
    const View* child = find(drawing, tag);
    if (!child) continue;
    auto path = outline(drawing, *child, unite);
    if (!path) continue;
    auto childProps = commonOf(*child);
    SkPath placed = path->makeTransform(childProps ? matrixOf(*childProps->matrix) : SkMatrix::I());
    any = true;
    if (!unite) {
      joined.addPath(placed);
    } else if (!united) {
      united = placed;
    } else if (auto both = Op(*united, placed, kUnion_SkPathOp)) {
      united = *both;
    }
  }
  if (!any) return std::nullopt;
  return unite ? united : std::optional<SkPath>(joined.detach());
}

// The element's bounding box in its own user space (the object bounding box gradients, patterns
// and masks in objectBoundingBox units are relative to).
SkRect boundingBox(const Drawing& drawing, const View& view) {
  auto path = outline(drawing, view, false);
  return path ? path->computeTightBounds() : SkRect::MakeEmpty();
}

// ---- Paint servers ---------------------------------------------------------------------------

void flatten(Drawing& drawing, Tag tag, const Inherited& parent);

// A gradient's colours and offsets (react-native-svg's `gradient`: offset, colour, ...), sorted
// by JavaScript, each colour's alpha its stop-opacity's.
bool stops(const std::vector<Float>& gradient, std::vector<SkColor4f>& colors, std::vector<float>& offsets) {
  for (size_t i = 0; i + 1 < gradient.size(); i += 2) {
    offsets.push_back(std::clamp(static_cast<float>(gradient[i]), offsets.empty() ? 0.0f : offsets.back(), 1.0f));
    colors.push_back(SkColor4f::FromColor(static_cast<uint32_t>(static_cast<int64_t>(gradient[i + 1]))));
  }
  return !colors.empty();
}

// A linear or radial gradient's shader for an element whose bounding box is `box`, as
// react-native-svg's Android Brush sets it up: objectBoundingBox coordinates (the default) are
// fractions of the box, userSpaceOnUse lengths of the viewport; gradientTransform in the space of
// those coordinates (SVG's, which Chromium follows: in the box's space for objectBoundingBox);
// pad spreading; one stop a solid colour; no stops, or an empty box for objectBoundingBox, no paint.
template <typename P>
sk_sp<SkShader> gradientShader(const Drawing& drawing, const P& props, const SkRect& box) {
  std::vector<SkColor4f> colors;
  std::vector<float> offsets;
  if (!stops(props.gradient, colors, offsets)) return nullptr;
  bool boxUnits = props.gradientUnits == 0;
  if (boxUnits && (box.width() <= 0 || box.height() <= 0)) return nullptr;
  SkMatrix local = boxUnits ? SkMatrix::Translate(box.x(), box.y()) * SkMatrix::Scale(box.width(), box.height())
                            : SkMatrix::I();
  local.preConcat(matrixOf(props.gradientTransform));
  auto x = [&](const folly::dynamic& v, float fallback) {
    return boxUnits ? fraction(v, fallback) : length(v, drawing, Along::Width, fallback * drawing.viewportWidth);
  };
  auto y = [&](const folly::dynamic& v, float fallback) {
    return boxUnits ? fraction(v, fallback) : length(v, drawing, Along::Height, fallback * drawing.viewportHeight);
  };
  if (colors.size() == 1) return SkShaders::Color(colors[0], nullptr);
  SkGradient gradient(SkGradient::Colors(colors, offsets, SkTileMode::kClamp), SkGradient::Interpolation{});
  if constexpr (std::is_same_v<P, RNSVGLinearGradientProps>) {
    SkPoint points[2] = {{x(props.x1, 0), y(props.y1, 0)}, {x(props.x2, 1), y(props.y2, 0)}};
    return SkShaders::LinearGradient(points, gradient, &local);
  } else {
    // react-native-svg's rx and ry (its `r` given as both): an ellipse is the circle of radius rx
    // scaled vertically by ry / rx, as its Android Brush scales it.
    float rx = x(props.rx, 0.5f), ry = y(props.ry, 0.5f);
    float cx = x(props.cx, 0.5f), cy = y(props.cy, 0.5f);
    float fx = props.fx.isNull() ? cx : x(props.fx, 0.5f), fy = props.fy.isNull() ? cy : y(props.fy, 0.5f);
    if (rx <= 0 || ry <= 0) return SkShaders::Color(colors.back(), nullptr);
    float ratio = ry / rx;
    local.preScale(1, ratio);
    SkPoint center{cx, cy / ratio}, focus{fx, fy / ratio};
    if (focus == center) return SkShaders::RadialGradient(center, rx, gradient, &local);
    // A focal point (Chromium draws it; Android's Brush leaves it out): the gradient from the
    // focus to the circle, the focus moved inside the circle as SVG asks.
    SkVector offset = focus - center;
    if (offset.length() > rx * 0.999f) focus = center + offset * (rx * 0.999f / offset.length());
    return SkShaders::TwoPointConicalGradient(focus, 0, center, rx, gradient, &local);
  }
}

// A pattern's shader, as SVG and Chromium tile it (react-native-svg's Android Brush draws the same
// tile into a bitmap; here it is a picture Skia draws at the output's resolution, never a bitmap
// scaled): the tile at x, y of width × height (fractions of the box in objectBoundingBox units, the
// default; lengths in userSpaceOnUse), its content from the tile's origin, fitted by its viewBox,
// or in the box's units with patternContentUnits objectBoundingBox; then patternTransform.
sk_sp<SkShader> patternShader(Drawing& drawing, const View& pattern, const SkRect& box) {
  const auto& props = static_cast<const RNSVGPatternProps&>(*pattern.view.props);
  if (drawing.depth >= kMaxDepth) return nullptr;
  SkRect tile;
  if (props.patternUnits == 0) {
    if (box.width() <= 0 || box.height() <= 0) return nullptr;
    tile = SkRect::MakeXYWH(box.x() + fraction(props.x, 0) * box.width(), box.y() + fraction(props.y, 0) * box.height(),
                            fraction(props.width, 0) * box.width(), fraction(props.height, 0) * box.height());
  } else {
    tile = SkRect::MakeXYWH(length(props.x, drawing, Along::Width), length(props.y, drawing, Along::Height),
                            length(props.width, drawing, Along::Width), length(props.height, drawing, Along::Height));
  }
  if (tile.width() <= 0 || tile.height() <= 0) return nullptr;
  SkPictureRecorder recorder;
  Drawing content = drawing;
  content.canvas = recorder.beginRecording(SkRect::MakeWH(tile.width(), tile.height()));
  content.depth += 1;
  if (props.vbWidth > 0 && props.vbHeight > 0) {
    content.canvas->concat(viewBox(props, tile.width(), tile.height()));
  } else if (props.patternContentUnits == 0) {
    content.canvas->scale(box.width(), box.height());
  }
  Inherited base = resolve(props, Inherited{});
  base.opacity = 1;
  for (Tag child : pattern.children) flatten(content, child, base);
  SkMatrix local = matrixOf(props.patternTransform) * SkMatrix::Translate(tile.x(), tile.y());
  SkRect tileRect = SkRect::MakeWH(tile.width(), tile.height());
  return recorder.finishRecordingAsPicture()->makeShader(SkTileMode::kRepeat, SkTileMode::kRepeat,
                                                         SkFilterMode::kLinear, &local, &tileRect);
}

// Sets up `paint` for a brush (react-native-svg's: 0 a colour, 1 a reference to a paint server by
// id, 2 currentColor, 3 and 4 the context's fill and stroke) at `opacity`; false for none: no
// brush, a reference to nothing (none, as SVG and Chromium take it), or a paint it cannot draw.
bool setUp(SkPaint& paint, Drawing& drawing, const folly::dynamic& brush, uint32_t current, float opacity,
           const SkRect& box) {
  if (!brush.isObject()) return false;
  auto type = brush.getDefault("type", 0).asInt();
  if (type == 0) {
    const auto& payload = brush.getDefault("payload");
    if (!payload.isNumber()) {
      unsupported("platform colours");
      return false;
    }
    paint.setColor(withAlpha(static_cast<uint32_t>(payload.asInt()), opacity));
    return true;
  }
  if (type == 2) {
    paint.setColor(withAlpha(current, opacity));
    return true;
  }
  if (type != 1) {
    unsupported("context-fill and context-stroke");
    return false;
  }
  const auto& reference = brush.getDefault("brushRef");
  if (!reference.isString()) return false;
  const std::string& id = reference.getString();
  sk_sp<SkShader> shader;
  if (const View* linear = definition(drawing, id, "RNSVGLinearGradient")) {
    shader = gradientShader(drawing, static_cast<const RNSVGLinearGradientProps&>(*linear->view.props), box);
  } else if (const View* radial = definition(drawing, id, "RNSVGRadialGradient")) {
    shader = gradientShader(drawing, static_cast<const RNSVGRadialGradientProps&>(*radial->view.props), box);
  } else if (const View* pattern = definition(drawing, id, "RNSVGPattern")) {
    shader = patternShader(drawing, *pattern, box);
  }
  if (!shader) return false;
  paint.setColor(SkColors::kBlack);
  paint.setAlphaf(std::clamp(opacity, 0.0f, 1.0f));
  paint.setShader(std::move(shader));
  return true;
}

// ---- Drawing ---------------------------------------------------------------------------------

// One shape with the element's paint, filled then stroked, as react-native-svg's Android
// RenderableView draws it (on the canvas, already in the element's coordinates).
void drawShape(Drawing& drawing, SkPath path, const Inherited& paint) {
  SkCanvas& canvas = *drawing.canvas;
  path.setFillType(paint.fillRule == 0 ? SkPathFillType::kEvenOdd : SkPathFillType::kWinding);
  SkRect box = path.computeTightBounds();
  SkPaint fill;
  fill.setAntiAlias(true);
  if (setUp(fill, drawing, paint.fill, paint.color, paint.fillOpacity * paint.opacity, box)) {
    canvas.drawPath(path, fill);
  }
  SkPaint stroke;
  stroke.setAntiAlias(true);
  float width = length(paint.strokeWidth, drawing, Along::Diagonal, 1);
  if (width > 0 && setUp(stroke, drawing, paint.stroke, paint.color, paint.strokeOpacity * paint.opacity, box)) {
    stroke.setStyle(SkPaint::kStroke_Style);
    stroke.setStrokeWidth(width);
    static const SkPaint::Cap kCaps[] = {SkPaint::kButt_Cap, SkPaint::kRound_Cap, SkPaint::kSquare_Cap};
    static const SkPaint::Join kJoins[] = {SkPaint::kMiter_Join, SkPaint::kRound_Join, SkPaint::kBevel_Join};
    stroke.setStrokeCap(kCaps[std::clamp(paint.strokeLinecap, 0, 2)]);
    stroke.setStrokeJoin(kJoins[std::clamp(paint.strokeLinejoin, 0, 2)]);
    stroke.setStrokeMiter(paint.strokeMiterlimit);
    if (paint.strokeDasharray.isArray() && !paint.strokeDasharray.empty()) {
      std::vector<SkScalar> intervals;
      for (const auto& value : paint.strokeDasharray) intervals.push_back(length(value, drawing, Along::Diagonal));
      // An odd list is repeated, as SVG repeats it.
      if (intervals.size() % 2 == 1) intervals.insert(intervals.end(), intervals.begin(), intervals.end());
      stroke.setPathEffect(SkDashPathEffect::Make(intervals, paint.strokeDashoffset));
    }
    canvas.drawPath(path, stroke);
  }
}

// Clips the canvas (in the element's coordinates) to the clip path `id`, as react-native-svg's
// VirtualView.clip: its children's outlines through their transforms and the clip path's own,
// united for clipRule nonzero, one after the other and filled even-odd for evenodd (its default).
// A reference to nothing leaves the element unclipped, as Android does.
void clip(Drawing& drawing, const std::string& id, int clipRule) {
  const View* clipPath = definition(drawing, id, "RNSVGClipPath");
  if (!clipPath) {
    unsupported("a clipPath referring to no ClipPath (drawn unclipped)");
    return;
  }
  bool evenOdd = clipRule == 0;
  auto path = outline(drawing, *clipPath, !evenOdd);
  // A clip path with nothing in it hides the element.
  SkPath area = path ? *path : SkPath();
  if (evenOdd) area.setFillType(SkPathFillType::kEvenOdd);
  drawing.canvas->clipPath(area.makeTransform(matrixOf(*commonOf(*clipPath)->matrix)), true);
}

// Masks what was drawn since the layer the caller saved, as react-native-svg's Android
// RenderableView.render masks it: the mask's content (in the element's user space, or its
// bounding box's with maskContentUnits objectBoundingBox) within the mask's rectangle (x, y,
// width, height: fractions of the box in objectBoundingBox units, the default; lengths in
// userSpaceOnUse), its luminance (mask-type luminance, the default) or alpha (alpha) kept of what
// is under it (DST_IN). Luminance is Skia's luma of the premultiplied colour, SVG's
// luminance × alpha (Android multiplies by alpha once more).
void applyMask(Drawing& drawing, const View& mask, const SkRect& box) {
  const auto& props = static_cast<const RNSVGMaskProps&>(*mask.view.props);
  SkCanvas& canvas = *drawing.canvas;
  SkRect area;
  if (props.maskUnits == 0) {
    area = SkRect::MakeXYWH(box.x() + fraction(props.x, 0) * box.width(), box.y() + fraction(props.y, 0) * box.height(),
                            fraction(props.width, 1) * box.width(), fraction(props.height, 1) * box.height());
  } else {
    area = SkRect::MakeXYWH(length(props.x, drawing, Along::Width), length(props.y, drawing, Along::Height),
                            length(props.width, drawing, Along::Width, drawing.viewportWidth),
                            length(props.height, drawing, Along::Height, drawing.viewportHeight));
  }
  SkPaint keep;
  keep.setBlendMode(SkBlendMode::kDstIn);
  canvas.saveLayer(nullptr, &keep);
  SkPaint luminance;
  if (props.maskType == 0) luminance.setColorFilter(SkLumaColorFilter::Make());
  canvas.saveLayer(nullptr, &luminance);
  canvas.clipRect(area, true);
  if (props.maskContentUnits == 0) {
    canvas.translate(box.x(), box.y());
    canvas.scale(box.width(), box.height());
  }
  Drawing content = drawing;
  content.depth += 1;
  Inherited base = resolve(props, Inherited{});
  base.opacity = 1;
  if (content.depth < kMaxDepth) {
    for (Tag child : mask.children) flatten(content, child, base);
  }
  canvas.restore();
  canvas.restore();
}

// An element: its transform, its clip path and its mask around what `body` draws.
template <typename F>
void element(Drawing& drawing, const View& view, const Common& props, F&& body) {
  SkCanvas& canvas = *drawing.canvas;
  canvas.save();
  canvas.concat(matrixOf(*props.matrix));
  if (!props.clipPath->empty()) clip(drawing, *props.clipPath, props.clipRule);
  const View* mask = props.mask->empty() ? nullptr : definition(drawing, *props.mask, "RNSVGMask");
  if (!props.mask->empty() && !mask) unsupported("a mask referring to no Mask (drawn unmasked)");
  if (mask) canvas.saveLayer(nullptr, nullptr);
  body();
  if (mask) {
    applyMask(drawing, *mask, boundingBox(drawing, view));
    canvas.restore();
  }
  canvas.restore();
}

void flatten(Drawing& drawing, Tag tag, const Inherited& parent) {
  const View* found = find(drawing, tag);
  if (!found) return;
  const View& view = *found;
  const std::string name = nameOf(view);
  if (name == "RNSVGGroup") {
    const auto& props = static_cast<const RNSVGGroupProps&>(*view.view.props);
    if (props.display == "none") return;
    Inherited own = resolve(props, parent);
    element(drawing, view, common(props), [&] {
      for (Tag child : view.children) flatten(drawing, child, own);
    });
  } else if (isShape(name)) {
    auto props = *commonOf(view);
    if (*props.display == "none") return;
    auto path = shapePath(drawing, view, name);
    if (!path) return;
    Inherited own;
    const auto& base = *view.view.props;
    if (name == "RNSVGPath") own = resolve(static_cast<const RNSVGPathProps&>(base), parent);
    if (name == "RNSVGRect") own = resolve(static_cast<const RNSVGRectProps&>(base), parent);
    if (name == "RNSVGCircle") own = resolve(static_cast<const RNSVGCircleProps&>(base), parent);
    if (name == "RNSVGEllipse") own = resolve(static_cast<const RNSVGEllipseProps&>(base), parent);
    if (name == "RNSVGLine") own = resolve(static_cast<const RNSVGLineProps&>(base), parent);
    element(drawing, view, props, [&] { drawShape(drawing, *path, own); });
  } else if (name == "RNSVGImage") {
    // react-native-svg's ImageView: its picture (loaded by images.h at the size it is drawn) placed
    // in its viewport by preserveAspectRatio and clipped to it; nothing until it has loaded.
    const auto& props = static_cast<const RNSVGImageProps&>(*view.view.props);
    if (props.display == "none") return;
    float opacity = parent.opacity * props.opacity;
    element(drawing, view, common(props), [&] {
      if (!view.picture || !view.picture->image) return;
      const auto& image = view.picture->image;
      SkRect viewport = imageRect(drawing, view);
      if (viewport.isEmpty()) return;
      SkCanvas& canvas = *drawing.canvas;
      canvas.save();
      canvas.clipRect(viewport, true);
      canvas.concat(fit(SkRect::MakeWH(image->width(), image->height()), viewport, props.align, props.meetOrSlice));
      SkPaint paint;
      paint.setAntiAlias(true);
      paint.setAlphaf(std::clamp(opacity, 0.0f, 1.0f));
      canvas.drawImage(image, 0, 0, SkSamplingOptions(SkFilterMode::kLinear), &paint);
      canvas.restore();
    });
  } else if (name == "RNSVGDefs" || name == "RNSVGClipPath" || name == "RNSVGMask" || name == "RNSVGMarker" ||
             name == "RNSVGPattern" || name == "RNSVGSymbol" || name == "RNSVGLinearGradient" ||
             name == "RNSVGRadialGradient" || name == "RNSVGFilter" || name.rfind("RNSVGFe", 0) == 0) {
    // Definitions are drawn only where something refers to them.
  } else if (name == "RNSVGText" || name == "RNSVGTSpan" || name == "RNSVGTextPath") {
    unsupported("text");
  } else {
    unsupported(name);
  }
}

// Every definition in the SVG view's tree by its name (react-native-svg's `name`, the element's
// id); a later one of the same name replaces an earlier, as Android's SvgView defines them.
void define(const Drawing& drawing, Tag tag, std::map<std::string, const View*>& definitions) {
  const View* view = find(drawing, tag);
  if (!view) return;
  const std::string name = nameOf(*view);
  const auto& props = *view->view.props;
  const std::string* id = nullptr;
  if (name == "RNSVGLinearGradient") id = &static_cast<const RNSVGLinearGradientProps&>(props).name;
  if (name == "RNSVGRadialGradient") id = &static_cast<const RNSVGRadialGradientProps&>(props).name;
  if (name == "RNSVGPattern") id = &static_cast<const RNSVGPatternProps&>(props).name;
  if (name == "RNSVGClipPath") id = &static_cast<const RNSVGClipPathProps&>(props).name;
  if (name == "RNSVGMask") id = &static_cast<const RNSVGMaskProps&>(props).name;
  if (id && !id->empty()) definitions[*id] = view;
  for (Tag child : view->children) define(drawing, child, definitions);
}

// What the picture depends on: the props of every element in the subtree, in order.
void signature(const ViewTree& tree, Tag tag, std::vector<const void*>& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  out.push_back(it->second.view.props.get());
  out.push_back(reinterpret_cast<const void*>(static_cast<intptr_t>(it->second.children.size())));
  // An image element's picture, which arrives after its props.
  out.push_back(it->second.picture.get());
  for (Tag child : it->second.children) signature(tree, child, out);
}

struct Cached {
  std::vector<const void*> signature;
  Size size;
  sk_sp<SkPicture> picture;
};

template <typename P>
sk_sp<SkPicture> record(const ViewTree& tree, const View& svg, const P& props) {
  auto content = svg.view.layoutMetrics.getContentFrame();
  if (content.size.width <= 0 || content.size.height <= 0) return nullptr;
  SkPictureRecorder recorder;
  Drawing drawing;
  drawing.tree = &tree;
  drawing.canvas = recorder.beginRecording(SkRect::MakeWH(content.size.width, content.size.height));
  drawing.viewportWidth = props.vbWidth > 0 ? props.vbWidth : content.size.width;
  drawing.viewportHeight = props.vbHeight > 0 ? props.vbHeight : content.size.height;
  std::map<std::string, const View*> definitions;
  for (Tag child : svg.children) define(drawing, child, definitions);
  drawing.definitions = &definitions;
  Inherited root;
  if (props.color) root.color = argb(props.color);
  drawing.canvas->concat(viewBox(props, content.size.width, content.size.height));
  for (Tag child : svg.children) flatten(drawing, child, root);
  return recorder.finishRecordingAsPicture();
}

}  // namespace

void addSvgComponents(ComponentDescriptorProviderRegistry& providers) {
  providers.add(concreteComponentDescriptorProvider<RNSVGSvgViewComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGSvgViewAndroidComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGCircleComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGClipPathComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGDefsComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGEllipseComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeBlendComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeColorMatrixComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeCompositeComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeFloodComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeGaussianBlurComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeMergeComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFeOffsetComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGFilterComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGForeignObjectComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGGroupComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGImageComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGLinearGradientComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGLineComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGMarkerComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGMaskComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGPathComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGPatternComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGRadialGradientComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGRectComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGSymbolComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGTextComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGTextPathComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGTSpanComponentDescriptor>());
  providers.add(concreteComponentDescriptorProvider<RNSVGUseComponentDescriptor>());
}

bool isSvgView(const std::string& name) { return name == "RNSVGSvgView" || name == "RNSVGSvgViewAndroid"; }

sk_sp<SkPicture> svgPicture(const ViewTree& tree, Tag tag) {
  static std::map<Tag, Cached> cache;
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) {
    cache.erase(tag);
    return nullptr;
  }
  const auto& view = it->second.view;
  Cached now;
  now.size = view.layoutMetrics.getContentFrame().size;
  signature(tree, tag, now.signature);
  auto& kept = cache[tag];
  if (kept.picture && kept.signature == now.signature && kept.size == now.size) return kept.picture;
  std::string name = view.componentName ? view.componentName : "";
  now.picture = name == "RNSVGSvgView" ? record(tree, it->second, static_cast<const RNSVGSvgViewProps&>(*view.props))
                                       : record(tree, it->second, static_cast<const RNSVGSvgViewAndroidProps&>(*view.props));
  kept = std::move(now);
  // Views gone since are forgotten.
  for (auto entry = cache.begin(); entry != cache.end();) {
    entry = tree.views().count(entry->first) ? std::next(entry) : cache.erase(entry);
  }
  return kept.picture;
}

}  // namespace oxyos