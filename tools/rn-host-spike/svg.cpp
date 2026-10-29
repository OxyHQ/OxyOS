#include "svg.h"

#include <oxyos_client.h>

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

// SVG's matrix (a, b, c, d, e, f): x' = a·x + c·y + e, y' = b·x + d·y + f.
struct Matrix {
  float a = 1, b = 0, c = 0, d = 1, e = 0, f = 0;
  // This transform after `inner` (inner applied first).
  Matrix operator*(const Matrix& inner) const {
    return {a * inner.a + c * inner.b,     b * inner.a + d * inner.b,     a * inner.c + c * inner.d,
            b * inner.c + d * inner.d,     a * inner.e + c * inner.f + e, b * inner.e + d * inner.f + f};
  }
};

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
  Matrix matrix;
};

// Everything one drawing needs: the viewport percentages resolve against, and the shapes with what
// their pointers point to.
struct Drawing {
  float viewportWidth = 0, viewportHeight = 0;
  std::vector<OxyOSVectorShape> shapes;
  std::deque<std::vector<float>> dashes;
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

uint32_t withAlpha(uint32_t color, float opacity) {
  auto alpha = static_cast<uint32_t>(std::lround(((color >> 24) & 0xff) * std::clamp(opacity, 0.0f, 1.0f)));
  return (alpha << 24) | (color & 0xffffff);
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
    own.matrix = parent.matrix * Matrix{m[0], m[1], m[2], m[3], m[4], m[5]};
  }
  if (!props.clipPath.empty()) unsupported("clip paths (drawn unclipped)");
  if (!props.mask.empty()) unsupported("masks (drawn unmasked)");
  if (!props.markerStart.empty() || !props.markerMid.empty() || !props.markerEnd.empty()) unsupported("markers");
  if (!props.filter.empty()) unsupported("filters (drawn unfiltered)");
  return own;
}

// One shape with the element's paint.
template <typename P>
void addShape(Drawing& drawing, const P& props, const Inherited& paint, OxyOSVectorShape shape) {
  const auto& m = paint.matrix;
  std::copy_n(std::array<float, 6>{m.a, m.b, m.c, m.d, m.e, m.f}.begin(), 6, shape.transform);
  if (auto color = brushColor(paint.fill, paint.color)) {
    shape.fill = 1;
    shape.even_odd = paint.fillRule == 0;
    shape.fill_color = withAlpha(*color, paint.fillOpacity * paint.opacity);
  }
  if (auto color = brushColor(paint.stroke, paint.color)) {
    shape.stroke = 1;
    shape.stroke_color = withAlpha(*color, paint.strokeOpacity * paint.opacity);
    shape.stroke_width = length(paint.strokeWidth, drawing, Along::Diagonal, 1);
    shape.cap = static_cast<uint8_t>(std::clamp(paint.strokeLinecap, 0, 2));
    shape.join = static_cast<uint8_t>(std::clamp(paint.strokeLinejoin, 0, 2));
    shape.miter_limit = paint.strokeMiterlimit;
    if (paint.strokeDasharray.isArray() && !paint.strokeDasharray.empty()) {
      auto& dash = drawing.dashes.emplace_back();
      for (const auto& value : paint.strokeDasharray) dash.push_back(length(value, drawing, Along::Diagonal));
      shape.dash = dash.data();
      shape.dash_count = dash.size();
      shape.dash_offset = paint.strokeDashoffset;
    }
  }
  if (shape.fill || shape.stroke) drawing.shapes.push_back(shape);
  (void)props;
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
  OxyOSVectorShape shape{};
  if (name == "RNSVGGroup") {
    const auto& props = static_cast<const RNSVGGroupProps&>(*view.props);
    if (props.display == "none") return;
    children(tree, it->second, resolve(props, parent), drawing);
  } else if (name == "RNSVGPath") {
    const auto& props = static_cast<const RNSVGPathProps&>(*view.props);
    if (props.display == "none") return;
    shape.kind = OXYOS_SHAPE_PATH;
    shape.data = props.d.data();
    shape.data_length = props.d.size();
    addShape(drawing, props, resolve(props, parent), shape);
  } else if (name == "RNSVGRect") {
    const auto& props = static_cast<const RNSVGRectProps&>(*view.props);
    if (props.display == "none") return;
    shape.kind = OXYOS_SHAPE_RECT;
    float rx = length(props.rx, drawing, Along::Width), ry = length(props.ry, drawing, Along::Height);
    float geometry[6] = {length(props.x, drawing, Along::Width), length(props.y, drawing, Along::Height),
                         length(props.width, drawing, Along::Width), length(props.height, drawing, Along::Height),
                         rx, ry};
    std::copy_n(geometry, 6, shape.geometry);
    addShape(drawing, props, resolve(props, parent), shape);
  } else if (name == "RNSVGCircle") {
    const auto& props = static_cast<const RNSVGCircleProps&>(*view.props);
    if (props.display == "none") return;
    shape.kind = OXYOS_SHAPE_ELLIPSE;
    float r = length(props.r, drawing, Along::Diagonal);
    float geometry[6] = {length(props.cx, drawing, Along::Width), length(props.cy, drawing, Along::Height), r, r, 0,
                         0};
    std::copy_n(geometry, 6, shape.geometry);
    addShape(drawing, props, resolve(props, parent), shape);
  } else if (name == "RNSVGEllipse") {
    const auto& props = static_cast<const RNSVGEllipseProps&>(*view.props);
    if (props.display == "none") return;
    shape.kind = OXYOS_SHAPE_ELLIPSE;
    float geometry[6] = {length(props.cx, drawing, Along::Width), length(props.cy, drawing, Along::Height),
                         length(props.rx, drawing, Along::Width), length(props.ry, drawing, Along::Height), 0, 0};
    std::copy_n(geometry, 6, shape.geometry);
    addShape(drawing, props, resolve(props, parent), shape);
  } else if (name == "RNSVGLine") {
    const auto& props = static_cast<const RNSVGLineProps&>(*view.props);
    if (props.display == "none") return;
    shape.kind = OXYOS_SHAPE_LINE;
    float geometry[6] = {length(props.x1, drawing, Along::Width), length(props.y1, drawing, Along::Height),
                         length(props.x2, drawing, Along::Width), length(props.y2, drawing, Along::Height), 0, 0};
    std::copy_n(geometry, 6, shape.geometry);
    addShape(drawing, props, resolve(props, parent), shape);
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
Matrix viewBox(const P& props, float width, float height) {
  if (props.vbWidth <= 0 || props.vbHeight <= 0) return {};
  float sx = width / props.vbWidth, sy = height / props.vbHeight;
  if (props.align == "none" || props.meetOrSlice == 2) return {sx, 0, 0, sy, -props.minX * sx, -props.minY * sy};
  float s = props.meetOrSlice == 1 ? std::max(sx, sy) : std::min(sx, sy);
  const std::string& align = props.align.empty() ? std::string("xMidYMid") : props.align;
  auto offset = [&](const char* min, const char* mid, float free) {
    if (align.find(min) != std::string::npos) return 0.0f;
    if (align.find(mid) != std::string::npos) return free / 2;
    return free;
  };
  float tx = offset("xMin", "xMid", width - props.vbWidth * s);
  float ty = offset("YMin", "YMid", height - props.vbHeight * s);
  return {s, 0, 0, s, tx - props.minX * s, ty - props.minY * s};
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
  int width = 0, height = 0;
  std::shared_ptr<const Picture> picture;
};

template <typename P>
std::shared_ptr<const Picture> draw(const ViewTree& tree, const View& svg, const P& props, float scale) {
  auto content = svg.view.layoutMetrics.getContentFrame();
  int width = static_cast<int>(std::ceil(content.size.width * scale));
  int height = static_cast<int>(std::ceil(content.size.height * scale));
  if (width <= 0 || height <= 0) return nullptr;
  Drawing drawing;
  drawing.viewportWidth = props.vbWidth > 0 ? props.vbWidth : content.size.width;
  drawing.viewportHeight = props.vbHeight > 0 ? props.vbHeight : content.size.height;
  Inherited root;
  if (props.color) root.color = argb(props.color);
  // Device pixels: the picture is the content box at the display's scale.
  root.matrix = Matrix{scale, 0, 0, scale, 0, 0} * viewBox(props, content.size.width, content.size.height);
  for (Tag child : svg.children) flatten(tree, child, root, drawing);
  auto picture = std::make_shared<Picture>();
  picture->width = width;
  picture->height = height;
  picture->pixels.resize(static_cast<size_t>(width) * height * 4);
  int skipped = oxyos_vector_render(drawing.shapes.data(), drawing.shapes.size(), static_cast<uint32_t>(width),
                                    static_cast<uint32_t>(height), picture->pixels.data(), picture->pixels.size());
  if (skipped < 0) {
    std::cerr << "[host] svg: " << oxyos_last_error() << "\n";
    return nullptr;
  }
  if (skipped > 0) unsupported("path data SVG rejects (shape left out)");
  return picture;
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

std::shared_ptr<const Picture> svgPicture(const ViewTree& tree, Tag tag, float scale) {
  static std::map<Tag, Cached> cache;
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) {
    cache.erase(tag);
    return nullptr;
  }
  const auto& view = it->second.view;
  auto content = view.layoutMetrics.getContentFrame();
  int width = static_cast<int>(std::ceil(content.size.width * scale));
  int height = static_cast<int>(std::ceil(content.size.height * scale));
  Cached now;
  signature(tree, tag, now.signature);
  auto& kept = cache[tag];
  if (kept.picture && kept.signature == now.signature && kept.width == width && kept.height == height) {
    return kept.picture;
  }
  std::string name = view.componentName ? view.componentName : "";
  now.picture = name == "RNSVGSvgView"
                    ? draw(tree, it->second, static_cast<const RNSVGSvgViewProps&>(*view.props), scale)
                    : draw(tree, it->second, static_cast<const RNSVGSvgViewAndroidProps&>(*view.props), scale);
  now.width = width;
  now.height = height;
  kept = std::move(now);
  // Views gone since are forgotten.
  for (auto entry = cache.begin(); entry != cache.end();) {
    entry = tree.views().count(entry->first) ? std::next(entry) : cache.erase(entry);
  }
  return kept.picture;
}

}  // namespace oxyos
