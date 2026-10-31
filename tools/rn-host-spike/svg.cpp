#include "svg.h"

#include <include/core/SkCanvas.h>
#include <include/core/SkPaint.h>
#include <include/core/SkPath.h>
#include <include/core/SkPathBuilder.h>
#include <include/core/SkPictureRecorder.h>
#include <include/core/SkRRect.h>
#include <include/effects/SkDashPathEffect.h>
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

using namespace facebook::react;

namespace oxyos {

namespace {

// What an element inherits from its group unless its `propList` names it, as react-native-svg's
// native renderers resolve it; the root's are SVG's initial values.
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
  SkMatrix matrix;
};

// What one drawing needs: the viewport percentages resolve against, and the canvas it records onto.
struct Drawing {
  float viewportWidth = 0, viewportHeight = 0;
  SkCanvas* canvas = nullptr;
};

void unsupported(const std::string& what) {
  static std::set<std::string> reported;
  if (reported.insert(what).second) std::cerr << "[host] unsupported on OxyOS: react-native-svg " << what << "\n";
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

// A brush's colour (0xAARRGGBB), or none: no paint, or one it cannot draw yet.
std::optional<uint32_t> brushColor(const folly::dynamic& brush, uint32_t current) {
  if (!brush.isObject()) return std::nullopt;
  auto type = brush.getDefault("type", 0).asInt();
  if (type == 0) {
    const auto& payload = brush.getDefault("payload");
    if (payload.isNumber()) return static_cast<uint32_t>(payload.asInt());
    unsupported("platform colours");
    return std::nullopt;
  }
  if (type == 2) return current;
  unsupported(type == 1 ? "gradients and patterns (url(#…) paint)" : "context-fill and context-stroke");
  return std::nullopt;
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

// The element's own paint over what it inherits, and its transform and opacity on top.
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
  if (props.matrix.size() == 6) {
    const auto& m = props.matrix;
    // SVG's (a, b, c, d, e, f): x' = a·x + c·y + e, y' = b·x + d·y + f.
    own.matrix = SkMatrix::Concat(parent.matrix, SkMatrix::MakeAll(m[0], m[2], m[4], m[1], m[3], m[5], 0, 0, 1));
  }
  if (!props.clipPath.empty()) unsupported("clip paths (drawn unclipped)");
  if (!props.mask.empty()) unsupported("masks (drawn unmasked)");
  if (!props.markerStart.empty() || !props.markerMid.empty() || !props.markerEnd.empty()) unsupported("markers");
  if (!props.filter.empty()) unsupported("filters (drawn unfiltered)");
  return own;
}

// One shape with the element's paint, filled then stroked, as react-native-svg's Android
// RenderableView draws it through its matrix.
void drawShape(Drawing& drawing, SkPath path, const Inherited& paint) {
  SkCanvas& canvas = *drawing.canvas;
  canvas.save();
  canvas.concat(paint.matrix);
  if (auto color = brushColor(paint.fill, paint.color)) {
    path.setFillType(paint.fillRule == 0 ? SkPathFillType::kEvenOdd : SkPathFillType::kWinding);
    SkPaint fill(withAlpha(*color, paint.fillOpacity * paint.opacity));
    fill.setAntiAlias(true);
    canvas.drawPath(path, fill);
  }
  if (auto color = brushColor(paint.stroke, paint.color)) {
    SkPaint stroke(withAlpha(*color, paint.strokeOpacity * paint.opacity));
    stroke.setAntiAlias(true);
    stroke.setStyle(SkPaint::kStroke_Style);
    stroke.setStrokeWidth(length(paint.strokeWidth, drawing, Along::Diagonal, 1));
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
  canvas.restore();
}

void flatten(const ViewTree& tree, Tag tag, const Inherited& parent, Drawing& drawing);

void children(const ViewTree& tree, const View& view, const Inherited& paint, Drawing& drawing) {
  for (Tag child : view.children) flatten(tree, child, paint, drawing);
}

void flatten(const ViewTree& tree, Tag tag, const Inherited& parent, Drawing& drawing) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const auto& view = it->second.view;
  std::string name = view.componentName ? view.componentName : "";
  if (name == "RNSVGGroup") {
    const auto& props = static_cast<const RNSVGGroupProps&>(*view.props);
    if (props.display == "none") return;
    children(tree, it->second, resolve(props, parent), drawing);
  } else if (name == "RNSVGPath") {
    const auto& props = static_cast<const RNSVGPathProps&>(*view.props);
    if (props.display == "none") return;
    auto path = SkParsePath::FromSVGString(props.d.c_str());
    if (!path) {
      unsupported("path data SVG rejects (shape left out)");
      return;
    }
    drawShape(drawing, *path, resolve(props, parent));
  } else if (name == "RNSVGRect") {
    const auto& props = static_cast<const RNSVGRectProps&>(*view.props);
    if (props.display == "none") return;
    SkRect rect = SkRect::MakeXYWH(length(props.x, drawing, Along::Width), length(props.y, drawing, Along::Height),
                                   length(props.width, drawing, Along::Width),
                                   length(props.height, drawing, Along::Height));
    // SVG's corners: one radius given stands for both; each at most half the side.
    float rx = length(props.rx, drawing, Along::Width), ry = length(props.ry, drawing, Along::Height);
    if (rx <= 0) rx = ry;
    if (ry <= 0) ry = rx;
    rx = std::min(rx, rect.width() / 2), ry = std::min(ry, rect.height() / 2);
    drawShape(drawing, rx > 0 && ry > 0 ? SkPath::RRect(SkRRect::MakeRectXY(rect, rx, ry)) : SkPath::Rect(rect),
              resolve(props, parent));
  } else if (name == "RNSVGCircle") {
    const auto& props = static_cast<const RNSVGCircleProps&>(*view.props);
    if (props.display == "none") return;
    float r = length(props.r, drawing, Along::Diagonal);
    drawShape(drawing, SkPath::Circle(length(props.cx, drawing, Along::Width), length(props.cy, drawing, Along::Height), r),
              resolve(props, parent));
  } else if (name == "RNSVGEllipse") {
    const auto& props = static_cast<const RNSVGEllipseProps&>(*view.props);
    if (props.display == "none") return;
    float cx = length(props.cx, drawing, Along::Width), cy = length(props.cy, drawing, Along::Height);
    float rx = length(props.rx, drawing, Along::Width), ry = length(props.ry, drawing, Along::Height);
    drawShape(drawing, SkPath::Oval(SkRect::MakeLTRB(cx - rx, cy - ry, cx + rx, cy + ry)), resolve(props, parent));
  } else if (name == "RNSVGLine") {
    const auto& props = static_cast<const RNSVGLineProps&>(*view.props);
    if (props.display == "none") return;
    SkPathBuilder line;
    line.moveTo(length(props.x1, drawing, Along::Width), length(props.y1, drawing, Along::Height))
        .lineTo(length(props.x2, drawing, Along::Width), length(props.y2, drawing, Along::Height));
    drawShape(drawing, line.detach(), resolve(props, parent));
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

// From the view box to the content box: SVG's preserveAspectRatio (react-native-svg's align and
// meetOrSlice: 0 meet, 1 slice, 2 none).
template <typename P>
SkMatrix viewBox(const P& props, float width, float height) {
  if (props.vbWidth <= 0 || props.vbHeight <= 0) return SkMatrix::I();
  float sx = width / props.vbWidth, sy = height / props.vbHeight;
  if (props.align == "none" || props.meetOrSlice == 2) {
    return SkMatrix::MakeAll(sx, 0, -props.minX * sx, 0, sy, -props.minY * sy, 0, 0, 1);
  }
  float s = props.meetOrSlice == 1 ? std::max(sx, sy) : std::min(sx, sy);
  const std::string& align = props.align.empty() ? std::string("xMidYMid") : props.align;
  auto offset = [&](const char* min, const char* mid, float free) {
    if (align.find(min) != std::string::npos) return 0.0f;
    if (align.find(mid) != std::string::npos) return free / 2;
    return free;
  };
  float tx = offset("xMin", "xMid", width - props.vbWidth * s);
  float ty = offset("YMin", "YMid", height - props.vbHeight * s);
  return SkMatrix::MakeAll(s, 0, tx - props.minX * s, 0, s, ty - props.minY * s, 0, 0, 1);
}

// What the picture depends on: the props of every element in the subtree, in order.
void signature(const ViewTree& tree, Tag tag, std::vector<const void*>& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  out.push_back(it->second.view.props.get());
  out.push_back(reinterpret_cast<const void*>(static_cast<intptr_t>(it->second.children.size())));
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
  drawing.canvas = recorder.beginRecording(SkRect::MakeWH(content.size.width, content.size.height));
  drawing.viewportWidth = props.vbWidth > 0 ? props.vbWidth : content.size.width;
  drawing.viewportHeight = props.vbHeight > 0 ? props.vbHeight : content.size.height;
  Inherited root;
  if (props.color) root.color = argb(props.color);
  root.matrix = viewBox(props, content.size.width, content.size.height);
  for (Tag child : svg.children) flatten(tree, child, root, drawing);
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