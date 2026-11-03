#include "scrollbars.h"

#include <include/core/SkPaint.h>
#include <include/core/SkRRect.h>
#include <react/renderer/components/scrollview/ScrollViewProps.h>
#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/core/ConcreteState.h>

#include <algorithm>
#include <cmath>

using namespace facebook::react;

namespace oxyos {

namespace {

struct Box {
  float x0, y0, x1, y1;
};

// The view's padding box in its own coordinates: inside its borders, where its content and its
// scrollbars are clipped (renderer.cpp's clipToPaddingBox).
Box paddingBox(const ShadowView& view) {
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  Size size = view.layoutMetrics.frame.size;
  if (!props) return {0, 0, size.width, size.height};
  auto widths = props->resolveBorderMetrics(view.layoutMetrics).borderWidths;
  return {widths.left, widths.top, size.width - widths.right, size.height - widths.bottom};
}

// The content's size and the viewport's, as the scroll view's state and frame give them.
std::optional<std::pair<Size, Size>> extents(const ShadowView& view) {
  if (std::string(view.componentName ? view.componentName : "") != "ScrollView" || !view.state) return std::nullopt;
  const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
  return std::make_pair(state.getContentSize(), view.layoutMetrics.frame.size);
}

}  // namespace

bool hasScrollbar(const ShadowView& view, int axis) {
  auto sizes = extents(view);
  if (!sizes) return false;
  auto [content, viewport] = *sizes;
  bool scrolls = axis ? content.height > viewport.height : content.width > viewport.width;
  if (!scrolls) return false;
  auto props = std::dynamic_pointer_cast<const ScrollViewProps>(view.props);
  if (!props) return true;
  return axis ? props->showsVerticalScrollIndicator : props->showsHorizontalScrollIndicator;
}

bool persistentScrollbars(const ShadowView& view) {
  auto props = std::dynamic_pointer_cast<const ScrollViewProps>(view.props);
  return props && props->persistentScrollbar;
}

std::optional<ScrollbarGeometry> scrollbarGeometry(const ShadowView& view, Point offset, int axis, float expansion,
                                                   float scale) {
  if (!hasScrollbar(view, axis) || scale <= 0) return std::nullopt;
  auto [content, viewport] = *extents(view);
  Box box = paddingBox(view);
  // Everything on the display's pixels: lengths rounded to whole pixels, laid from the box's edges
  // (which layout puts on whole pixels).
  auto px = [&](float logical) { return std::round(logical * scale); };
  const float track = px(kScrollbarTrack), inset = px(kScrollbarInset);
  const float thickness = px(kScrollbarThin + (kScrollbarWide - kScrollbarThin) * std::clamp(expansion, 0.0f, 1.0f));
  // Both bars shown: each track stops short of the corner the other one's crosses.
  const bool corner = hasScrollbar(view, 1 - axis);
  const float x0 = box.x0 * scale, y0 = box.y0 * scale, x1 = box.x1 * scale, y1 = box.y1 * scale;
  // Along the bar: where its track begins and ends; across: its far edge.
  const float start = axis ? y0 : x0;
  const float end = (axis ? y1 : x1) - (corner ? track : 0);
  const float edge = axis ? x1 : y1;
  if (end - start < 2 * inset + 1) return std::nullopt;
  // The thumb is the viewport's share of the content along the track, at the offset's share of the
  // way its content goes, as NSScroller's knob proportion and value.
  const float length = end - start - 2 * inset;
  const float viewportLength = axis ? viewport.height : viewport.width;
  const float contentLength = axis ? content.height : content.width;
  const float range = std::max(1.0f, contentLength - viewportLength);
  const float thumbLength = std::min(length, std::max(px(kScrollbarMinThumb), std::round(length * viewportLength / contentLength)));
  const float progress = std::clamp((axis ? offset.y : offset.x) / range, 0.0f, 1.0f);
  const float thumbStart = start + inset + std::round((length - thumbLength) * progress);
  auto rect = [&](float a0, float a1, float b0, float b1) {
    // (a: along the bar, b: across it), back in logical pixels.
    return axis ? Rect{{b0 / scale, a0 / scale}, {(b1 - b0) / scale, (a1 - a0) / scale}}
                : Rect{{a0 / scale, b0 / scale}, {(a1 - a0) / scale, (b1 - b0) / scale}};
  };
  ScrollbarGeometry geometry;
  geometry.track = rect(start, end, edge - track, edge);
  geometry.thumb = rect(thumbStart, thumbStart + thumbLength, edge - inset - thickness, edge - inset);
  return geometry;
}

void drawScrollbars(SkCanvas& canvas, const ShadowView& view, float x, float y, const ScrollbarShown& shown, bool dark,
                    float scale) {
  const bool persistent = persistentScrollbars(view);
  const float alpha = persistent ? 1.0f : shown.alpha;
  if (alpha <= 0) return;
  auto props = std::dynamic_pointer_cast<const ScrollViewProps>(view.props);
  // NSScroller's knob styles: dark (a dark thumb with a light edge, for light backgrounds) and light.
  bool lightKnob = dark;
  if (props && props->indicatorStyle == ScrollViewIndicatorStyle::Black) lightKnob = false;
  if (props && props->indicatorStyle == ScrollViewIndicatorStyle::White) lightKnob = true;
  const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
  const float hairline = 1 / scale;
  for (int axis = 0; axis < 2; ++axis) {
    auto geometry = scrollbarGeometry(view, state.contentOffset, axis, shown.expansion[axis], scale);
    if (!geometry) continue;
    // The track, as the pointer brings it in: translucent, with a hairline on its inner edge.
    if (float track = alpha * std::clamp(shown.expansion[axis], 0.0f, 1.0f); track > 0) {
      const Rect& t = geometry->track;
      SkPaint fill(dark ? SkColor4f{43 / 255.0f, 43 / 255.0f, 43 / 255.0f, 0.8f * track}
                        : SkColor4f{250 / 255.0f, 250 / 255.0f, 250 / 255.0f, 0.8f * track});
      canvas.drawRect(SkRect::MakeXYWH(x + t.origin.x, y + t.origin.y, t.size.width, t.size.height), fill);
      SkPaint line(dark ? SkColor4f{1, 1, 1, 0.1f * track} : SkColor4f{0, 0, 0, 0.1f * track});
      SkRect separator = axis ? SkRect::MakeXYWH(x + t.origin.x, y + t.origin.y, hairline, t.size.height)
                              : SkRect::MakeXYWH(x + t.origin.x, y + t.origin.y, t.size.width, hairline);
      canvas.drawRect(separator, line);
    }
    // The thumb: fully rounded, its fill and a hairline edge of the other tone, so it shows over
    // content of either.
    const Rect& k = geometry->thumb;
    SkRect thumb = SkRect::MakeXYWH(x + k.origin.x, y + k.origin.y, k.size.width, k.size.height);
    float radius = std::min(k.size.width, k.size.height) / 2;
    SkPaint fill(lightKnob ? SkColor4f{1, 1, 1, 0.5f * alpha} : SkColor4f{0, 0, 0, 0.5f * alpha});
    fill.setAntiAlias(true);
    canvas.drawRRect(SkRRect::MakeRectXY(thumb, radius, radius), fill);
    SkPaint edge(lightKnob ? SkColor4f{0, 0, 0, 0.25f * alpha} : SkColor4f{1, 1, 1, 0.35f * alpha});
    edge.setAntiAlias(true);
    edge.setStyle(SkPaint::kStroke_Style);
    edge.setStrokeWidth(hairline);
    SkRect inner = thumb.makeInset(hairline / 2, hairline / 2);
    canvas.drawRRect(SkRRect::MakeRectXY(inner, radius - hairline / 2, radius - hairline / 2), edge);
  }
}

}  // namespace oxyos
