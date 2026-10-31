#include "background.h"

#include <react/renderer/graphics/Color.h>

#include <include/core/SkBlurTypes.h>
#include <include/core/SkColorFilter.h>
#include <include/core/SkMaskFilter.h>
#include <include/core/SkPaint.h>
#include <include/core/SkPathBuilder.h>
#include <include/core/SkPathEffect.h>
#include <include/core/SkShader.h>
#include <include/effects/SkDashPathEffect.h>
#include <include/effects/SkGradient.h>
#include <include/effects/SkImageFilters.h>

#include <algorithm>
#include <cmath>
#include <optional>
#include <vector>

using namespace facebook::react;

namespace oxyos {

namespace {

constexpr uint32_t kBlack = 0xff000000u;
// What a colour React Native's C++ props hold no value for is drawn as. They cannot tell a colour
// left out from `transparent`: both are its UndefinedColor, 0. Android, whose drawables read the
// JavaScript value itself, draws a border or outline left without a colour black; here it is drawn
// transparent, as `transparent` must be (Bloom gives every border it draws a colour).
constexpr uint32_t kUnset = 0;

// A colour as Android's ARGB int; `unset` for a colour the props hold no value for.
uint32_t argb(const SharedColor& color, uint32_t unset) {
  if (!color) return unset;
  auto c = colorComponentsFromColor(color);
  auto byte = [](float v) { return static_cast<uint32_t>(std::lround(std::clamp(v, 0.0f, 1.0f) * 255)); };
  return byte(c.alpha) << 24 | byte(c.red) << 16 | byte(c.green) << 8 | byte(c.blue);
}

uint32_t alphaOf(uint32_t color) { return color >> 24; }

// A colour with the view's alpha multiplied in (an alpha filter canvas: a view without overlapping
// rendering drawn at its alpha).
SkColor4f faded(uint32_t color, float opacity) {
  SkColor4f c = SkColor4f::FromColor(color);
  c.fA *= opacity;
  return c;
}

SkPaint paintOf(uint32_t color, float opacity, bool antiAlias = true) {
  SkPaint paint(faded(color, opacity));
  paint.setAntiAlias(antiAlias);
  return paint;
}

bool rounded(const BorderRadii& r) {
  for (const auto& corner : {r.topLeft, r.topRight, r.bottomLeft, r.bottomRight}) {
    if (corner.horizontal > 0 || corner.vertical > 0) return true;
  }
  return false;
}

bool uniform(const BorderRadii& r) {
  return r.topLeft == r.topRight && r.topLeft == r.bottomLeft && r.topLeft == r.bottomRight;
}

// Path.addRoundRect's radii, in Skia's corner order (upper left, upper right, lower right, lower left).
struct Radii {
  SkVector corners[4];
};

Radii radiiOf(const BorderRadii& r) {
  return Radii{{{r.topLeft.horizontal, r.topLeft.vertical},
                {r.topRight.horizontal, r.topRight.vertical},
                {r.bottomRight.horizontal, r.bottomRight.vertical},
                {r.bottomLeft.horizontal, r.bottomLeft.vertical}}};
}

SkRRect rrectOf(const SkRect& rect, const Radii& radii) {
  Radii kept = radii;
  for (auto& corner : kept.corners) corner = {std::max(0.0f, corner.fX), std::max(0.0f, corner.fY)};
  SkRRect rrect;
  rrect.setRectRadii(rect, kept.corners);
  return rrect;
}

// Android's DashPathEffect for a border or outline style at a width.
sk_sp<SkPathEffect> dashes(bool dashed, bool dotted, float width) {
  if (dashed) {
    const SkScalar intervals[] = {width * 3, width * 3, width * 3, width * 3};
    return SkDashPathEffect::Make(intervals, 0);
  }
  if (dotted) {
    const SkScalar intervals[] = {width, width, width, width};
    return SkDashPathEffect::Make(intervals, 0);
  }
  return nullptr;
}

// FilterHelper.sigmaToRadius and back through HWUI's Blur::convertRadiusToSigma: a blur asked as a
// standard deviation in dp is drawn with that deviation in pixels, and none at half a pixel or less.
std::optional<float> blurSigma(float sigma, float scale) {
  if (sigma * scale <= 0.5f) return std::nullopt;
  return sigma;
}

// BoxShadowBorderRadius' adjustRadiusForSpread: a corner grown (or shrunk) with the spread, as CSS
// grows it.
float adjustRadiusForSpread(float radius, float spread) {
  float multiplier = 1;
  if (radius < std::abs(spread)) multiplier = 1 + std::pow(radius / std::abs(spread) - 1, 3.0f);
  return std::max(0.0f, radius + spread * multiplier);
}

// OutsetBoxShadowDrawable.draw.
void drawOutsetShadow(SkCanvas& canvas, const BoxShadow& shadow, const BorderRadii& radii, const SkRect& bounds,
                      float opacity, float scale) {
  SkPaint paint = paintOf(argb(shadow.color, kUnset), opacity, false);
  // BLUR_RADIUS_SIGMA_SCALE: CSS's blur radius is twice the deviation.
  if (auto sigma = blurSigma(shadow.blurRadius * 0.5f, scale)) {
    paint.setMaskFilter(SkMaskFilter::MakeBlur(kNormal_SkBlurStyle, *sigma));
  }
  float spread = shadow.spreadDistance;
  SkRect shadowRect = bounds.makeOutset(spread, spread).makeOffset(shadow.offsetX, shadow.offsetY);
  canvas.save();
  if (rounded(radii)) {
    // Never drawn under the box: the box less a subpixel inset (0.4 px) clipped out.
    float inset = 0.4f / scale;
    canvas.clipRRect(rrectOf(bounds.makeInset(inset, inset), radiiOf(radii)), SkClipOp::kDifference, true);
    Radii grown = radiiOf(radii);
    for (auto& corner : grown.corners) {
      corner = {adjustRadiusForSpread(corner.fX, spread), adjustRadiusForSpread(corner.fY, spread)};
    }
    paint.setAntiAlias(true);
    canvas.drawRRect(rrectOf(shadowRect, grown), paint);
  } else {
    canvas.clipRect(bounds, SkClipOp::kDifference, false);
    canvas.drawRect(shadowRect, paint);
  }
  canvas.restore();
}

// InsetBoxShadowDrawable.draw.
void drawInsetShadow(SkCanvas& canvas, const BoxShadow& shadow, const BorderMetrics& border, const SkRect& bounds,
                     float opacity, float scale) {
  SkPaint paint = paintOf(argb(shadow.color, kUnset), opacity);
  if (auto sigma = blurSigma(shadow.blurRadius * 0.5f, scale)) {
    paint.setMaskFilter(SkMaskFilter::MakeBlur(kNormal_SkBlurStyle, *sigma));
  }
  const auto& widths = border.borderWidths;
  SkRect padding = SkRect::MakeLTRB(bounds.fLeft + widths.left, bounds.fTop + widths.top,
                                    bounds.fRight - widths.right, bounds.fBottom - widths.bottom);
  float spread = shadow.spreadDistance;
  SkRect inner = padding;
  if (2 * spread > padding.width()) {
    inner.setEmpty();
  } else {
    inner.inset(spread, spread);
  }
  inner.offset(shadow.offsetX, shadow.offsetY);
  // The blur's extent: the whole blur radius taken as a deviation (sigmaToRadius(blurRadius)).
  float pxSigma = shadow.blurRadius * scale;
  float extent = pxSigma > 0.5f ? (pxSigma - 0.5f) / 0.57735f / scale : 0;
  SkRect outer = padding.makeOutset(extent, extent);
  outer.join(inner);
  canvas.save();
  if (rounded(border.borderRadii)) {
    const auto& r = border.borderRadii;
    Radii paddingRadii{{{std::max(0.0f, r.topLeft.horizontal - widths.left), std::max(0.0f, r.topLeft.vertical - widths.top)},
                        {std::max(0.0f, r.topRight.horizontal - widths.right), std::max(0.0f, r.topRight.vertical - widths.top)},
                        {std::max(0.0f, r.bottomRight.horizontal - widths.right),
                         std::max(0.0f, r.bottomRight.vertical - widths.bottom)},
                        {std::max(0.0f, r.bottomLeft.horizontal - widths.left),
                         std::max(0.0f, r.bottomLeft.vertical - widths.bottom)}}};
    canvas.clipRRect(rrectOf(padding, paddingRadii), SkClipOp::kIntersect, true);
    Radii innerRadii = paddingRadii;
    for (auto& corner : innerRadii.corners) {
      corner = {adjustRadiusForSpread(corner.fX, -spread), adjustRadiusForSpread(corner.fY, -spread)};
    }
    canvas.drawDRRect(SkRRect::MakeRect(outer), rrectOf(inner, innerRadii), paint);
  } else {
    canvas.clipRect(padding, SkClipOp::kIntersect, false);
    canvas.drawDRRect(SkRRect::MakeRect(outer), SkRRect::MakeRect(inner), paint);
  }
  canvas.restore();
}

// BackgroundDrawable.draw.
void drawBackgroundColor(SkCanvas& canvas, const ViewProps& props, const BorderMetrics& border, const SkRect& bounds,
                         float opacity, float scale) {
  uint32_t color = argb(props.backgroundColor, 0);
  if (alphaOf(color) == 0 || opacity <= 0) return;
  const auto& w = border.borderWidths;
  bool hasBorder = w.left != 0 || w.top != 0 || w.right != 0 || w.bottom != 0;
  SkRect rect = bounds;
  SkPaint paint = paintOf(color, opacity);
  if (rounded(border.borderRadii)) {
    // pathAdjustment: under a border, the rounded background is kept 0.8 px inside the box, so it
    // does not show at the border's anti-aliased outer edge.
    if (hasBorder) rect.inset(0.8f / scale, 0.8f / scale);
    canvas.drawRRect(rrectOf(rect, radiiOf(border.borderRadii)), paint);
  } else {
    canvas.drawRect(rect, paint);
  }
}

// BorderDrawable.getEllipseIntersectionWithLine: where the line from the outer corner to the inner
// rectangle's corner meets the inner corner's ellipse; `result` is kept when they do not meet.
void ellipseIntersection(double left, double top, double right, double bottom, double x0, double y0, double x1,
                         double y1, SkPoint& result) {
  double cx = (left + right) / 2, cy = (top + bottom) / 2;
  x0 -= cx, y0 -= cy, x1 -= cx, y1 -= cy;
  double a = std::abs(right - left) / 2, b = std::abs(bottom - top) / 2;
  double m = (y1 - y0) / (x1 - x0);
  double c = y0 - m * x0;
  double A = b * b + a * a * m * m;
  double B = 2 * a * a * c * m;
  double C = a * a * (c * c - b * b);
  double D = std::sqrt(-C / A + std::pow(B / (2 * A), 2.0));
  double x = -B / (2 * A) - D;
  double y = m * x + c;
  if (!std::isnan(x + cx) && !std::isnan(y + cy)) result = {static_cast<float>(x + cx), static_cast<float>(y + cy)};
}

void drawQuadrilateral(SkCanvas& canvas, uint32_t color, float opacity, bool antiAlias, SkPoint p1, SkPoint p2,
                       SkPoint p3, SkPoint p4) {
  if (color == 0) return;
  SkPathBuilder path;
  path.moveTo(p1).lineTo(p2).lineTo(p3).lineTo(p4).lineTo(p1);
  canvas.drawPath(path.detach(), paintOf(color, opacity, antiAlias));
}

// BorderDrawable.drawRoundedBorders.
void drawRoundedBorder(SkCanvas& canvas, const ViewProps& props, const BorderMetrics& border, const SkRect& bounds,
                       uint32_t colors[4], float opacity, float scale) {
  const auto& w = border.borderWidths;
  const auto& r = border.borderRadii;
  SkRect outer = bounds, inner = bounds;
  if (alphaOf(colors[0]) != 0 || alphaOf(colors[1]) != 0 || alphaOf(colors[2]) != 0 || alphaOf(colors[3]) != 0) {
    inner = SkRect::MakeLTRB(bounds.fLeft + w.left, bounds.fTop + w.top, bounds.fRight - w.right,
                             bounds.fBottom - w.bottom);
  }
  SkRect center = SkRect::MakeLTRB(bounds.fLeft + w.left * 0.5f, bounds.fTop + w.top * 0.5f,
                                   bounds.fRight - w.right * 0.5f, bounds.fBottom - w.bottom * 0.5f);
  float innerTL[2] = {std::max(0.0f, r.topLeft.horizontal - w.left), std::max(0.0f, r.topLeft.vertical - w.top)};
  float innerTR[2] = {std::max(0.0f, r.topRight.horizontal - w.right), std::max(0.0f, r.topRight.vertical - w.top)};
  float innerBR[2] = {std::max(0.0f, r.bottomRight.horizontal - w.right),
                      std::max(0.0f, r.bottomRight.vertical - w.bottom)};
  float innerBL[2] = {std::max(0.0f, r.bottomLeft.horizontal - w.left),
                      std::max(0.0f, r.bottomLeft.vertical - w.bottom)};
  canvas.save();
  canvas.clipRRect(rrectOf(outer, radiiOf(r)), SkClipOp::kIntersect, true);
  if (w.top > 0 || w.bottom > 0 || w.left > 0 || w.right > 0) {
    // The `borderWidth` shorthand and the `borderColor` one (getBorderColor(ALL)).
    auto all = props.yogaStyle.border(facebook::yoga::Edge::All).value();
    float full = all.isDefined() ? all.unwrap() : 0;
    uint32_t allColor = argb(props.borderColors.all.value_or(SharedColor{}), kUnset);
    bool sameWidths = w.top == full && w.bottom == full && w.left == full && w.right == full;
    bool sameColors = colors[0] == allColor && colors[1] == allColor && colors[2] == allColor && colors[3] == allColor;
    if (sameWidths && sameColors) {
      if (full > 0) {
        SkPaint paint = paintOf(allColor, opacity);
        paint.setStyle(SkPaint::kStroke_Style);
        paint.setStrokeWidth(full);
        auto style = border.borderStyles.top;
        paint.setPathEffect(dashes(style == BorderStyle::Dashed, style == BorderStyle::Dotted, full));
        Radii centerRadii{{{r.topLeft.horizontal - w.left * 0.5f, r.topLeft.vertical - w.top * 0.5f},
                           {r.topRight.horizontal - w.right * 0.5f, r.topRight.vertical - w.top * 0.5f},
                           {r.bottomRight.horizontal - w.right * 0.5f, r.bottomRight.vertical - w.bottom * 0.5f},
                           {r.bottomLeft.horizontal - w.left * 0.5f, r.bottomLeft.vertical - w.bottom * 0.5f}}};
        if (uniform(r)) {
          for (auto& corner : centerRadii.corners) corner = centerRadii.corners[0];
        }
        canvas.drawRRect(rrectOf(center, centerRadii), paint);
      }
    } else {
      // Each side a quadrilateral in its own colour, from the outer corners to where the inner
      // corners' ellipses meet the diagonals, within the ring.
      Radii innerRadii{{{innerTL[0], innerTL[1]}, {innerTR[0], innerTR[1]}, {innerBR[0], innerBR[1]}, {innerBL[0], innerBL[1]}}};
      canvas.clipRRect(rrectOf(inner, innerRadii), SkClipOp::kDifference, true);
      SkPoint tl{inner.fLeft, inner.fTop}, tr{inner.fRight, inner.fTop}, bl{inner.fLeft, inner.fBottom},
          br{inner.fRight, inner.fBottom};
      ellipseIntersection(inner.fLeft, inner.fTop, inner.fLeft + 2 * innerTL[0], inner.fTop + 2 * innerTL[1],
                          outer.fLeft, outer.fTop, inner.fLeft, inner.fTop, tl);
      ellipseIntersection(inner.fLeft, inner.fBottom - 2 * innerBL[1], inner.fLeft + 2 * innerBL[0], inner.fBottom,
                          outer.fLeft, outer.fBottom, inner.fLeft, inner.fBottom, bl);
      ellipseIntersection(inner.fRight - 2 * innerTR[0], inner.fTop, inner.fRight, inner.fTop + 2 * innerTR[1],
                          outer.fRight, outer.fTop, inner.fRight, inner.fTop, tr);
      ellipseIntersection(inner.fRight - 2 * innerBR[0], inner.fBottom - 2 * innerBR[1], inner.fRight, inner.fBottom,
                          outer.fRight, outer.fBottom, inner.fRight, inner.fBottom, br);
      // gapBetweenPaths: the sides overlap by 0.8 px, so no seam shows between them.
      float gap = 0.8f / scale;
      float left = outer.fLeft, right = outer.fRight, top = outer.fTop, bottom = outer.fBottom;
      if (w.left > 0) {
        drawQuadrilateral(canvas, colors[0], opacity, true, {left, top - gap}, {tl.fX, tl.fY - gap},
                          {bl.fX, bl.fY + gap}, {left, bottom + gap});
      }
      if (w.top > 0) {
        drawQuadrilateral(canvas, colors[1], opacity, true, {left - gap, top}, {tl.fX - gap, tl.fY},
                          {tr.fX + gap, tr.fY}, {right + gap, top});
      }
      if (w.right > 0) {
        drawQuadrilateral(canvas, colors[2], opacity, true, {right, top - gap}, {tr.fX, tr.fY - gap},
                          {br.fX, br.fY + gap}, {right, bottom + gap});
      }
      if (w.bottom > 0) {
        drawQuadrilateral(canvas, colors[3], opacity, true, {left - gap, bottom}, {bl.fX - gap, bl.fY},
                          {br.fX + gap, br.fY}, {right + gap, bottom});
      }
    }
  }
  canvas.restore();
}

// BorderDrawable.drawRectangularBorders: on Android's whole pixels.
void drawRectangularBorder(SkCanvas& canvas, const BorderMetrics& border, const SkRect& bounds, uint32_t colors[4],
                           float opacity, float scale) {
  const auto& w = border.borderWidths;
  int borderLeft = static_cast<int>(std::lround(w.left * scale));
  int borderTop = static_cast<int>(std::lround(w.top * scale));
  int borderRight = static_cast<int>(std::lround(w.right * scale));
  int borderBottom = static_cast<int>(std::lround(w.bottom * scale));
  if (borderLeft <= 0 && borderTop <= 0 && borderRight <= 0 && borderBottom <= 0) return;
  // The view's bounds in device pixels, which Android's are.
  int left = static_cast<int>(std::lround(bounds.fLeft * scale)), top = static_cast<int>(std::lround(bounds.fTop * scale));
  int right = static_cast<int>(std::lround(bounds.fRight * scale));
  int bottom = static_cast<int>(std::lround(bounds.fBottom * scale));
  auto at = [scale](float pixels) { return pixels / scale; };
  // fastBorderCompatibleColorOrZero: every side drawn is one opaque colour.
  uint32_t fast = 0;
  if (alphaOf(colors[0]) == 255 && alphaOf(colors[1]) == 255 && alphaOf(colors[2]) == 255 && alphaOf(colors[3]) == 255) {
    uint32_t andSmear = (borderLeft > 0 ? colors[0] : ~0u) & (borderTop > 0 ? colors[1] : ~0u) &
                        (borderRight > 0 ? colors[2] : ~0u) & (borderBottom > 0 ? colors[3] : ~0u);
    uint32_t orSmear = (borderLeft > 0 ? colors[0] : 0u) | (borderTop > 0 ? colors[1] : 0u) |
                       (borderRight > 0 ? colors[2] : 0u) | (borderBottom > 0 ? colors[3] : 0u);
    fast = andSmear == orSmear ? andSmear : 0;
  }
  auto style = border.borderStyles.top;
  if (fast != 0) {
    SkPaint paint = paintOf(fast, opacity);
    paint.setStyle(SkPaint::kStroke_Style);
    auto line = [&](int width, float x0, float y0, float x1, float y1) {
      paint.setStrokeWidth(at(static_cast<float>(width)));
      paint.setPathEffect(dashes(style == BorderStyle::Dashed, style == BorderStyle::Dotted, at(static_cast<float>(width))));
      SkPathBuilder path;
      path.moveTo(at(x0), at(y0)).lineTo(at(x1), at(y1));
      canvas.drawPath(path.detach(), paint);
    };
    // Each side a line along its middle (Android's whole-pixel halves: width / 2 in integers).
    if (borderLeft > 0) line(borderLeft, left + borderLeft / 2, top, left + borderLeft / 2, bottom);
    if (borderTop > 0) line(borderTop, left, top + borderTop / 2, right, top + borderTop / 2);
    if (borderRight > 0) line(borderRight, right - borderRight / 2, top, right - borderRight / 2, bottom);
    if (borderBottom > 0) line(borderBottom, left, bottom - borderBottom / 2, right, bottom - borderBottom / 2);
    return;
  }
  // Sides of several colours: quadrilaterals mitred at the corners, not anti-aliased.
  float width = static_cast<float>(right - left), height = static_cast<float>(bottom - top);
  float l = static_cast<float>(left), t = static_cast<float>(top);
  auto p = [&](float x, float y) { return SkPoint{at(x), at(y)}; };
  if (borderLeft > 0) {
    drawQuadrilateral(canvas, colors[0], opacity, false, p(l, t), p(l + borderLeft, t + borderTop),
                      p(l + borderLeft, t + height - borderBottom), p(l, t + height));
  }
  if (borderTop > 0) {
    drawQuadrilateral(canvas, colors[1], opacity, false, p(l, t), p(l + borderLeft, t + borderTop),
                      p(l + width - borderRight, t + borderTop), p(l + width, t));
  }
  if (borderRight > 0) {
    drawQuadrilateral(canvas, colors[2], opacity, false, p(l + width, t), p(l + width, t + height),
                      p(l + width - borderRight, t + height - borderBottom), p(l + width - borderRight, t + borderTop));
  }
  if (borderBottom > 0) {
    drawQuadrilateral(canvas, colors[3], opacity, false, p(l, t + height), p(l + width, t + height),
                      p(l + width - borderRight, t + height - borderBottom), p(l + borderLeft, t + height - borderBottom));
  }
}

void drawBorder(SkCanvas& canvas, const ViewProps& props, const BorderMetrics& border, const SkRect& bounds,
                float opacity, float scale) {
  // BorderColors.resolve, each side its own colour, the cascade (left, start, all) resolved by the props.
  uint32_t colors[4] = {argb(border.borderColors.left, kUnset), argb(border.borderColors.top, kUnset),
                        argb(border.borderColors.right, kUnset), argb(border.borderColors.bottom, kUnset)};
  if (rounded(border.borderRadii)) {
    drawRoundedBorder(canvas, props, border, bounds, colors, opacity, scale);
  } else {
    drawRectangularBorder(canvas, border, bounds, colors, opacity, scale);
  }
}

// OutlineDrawable.draw.
void drawOutline(SkCanvas& canvas, const ViewProps& props, const BorderMetrics& border, const SkRect& bounds,
                 float opacity, float scale) {
  float width = props.outlineWidth;
  if (width == 0) return;
  float offset = props.outlineOffset;
  float grow = width * 0.5f + offset - 0.8f / scale;
  SkRect rect = bounds.makeOutset(grow, grow);
  SkPaint paint = paintOf(argb(props.outlineColor, kUnset), opacity);
  paint.setStyle(SkPaint::kStroke_Style);
  paint.setStrokeWidth(width);
  paint.setPathEffect(
      dashes(props.outlineStyle == OutlineStyle::Dashed, props.outlineStyle == OutlineStyle::Dotted, width));
  if (rounded(border.borderRadii)) {
    Radii radii = radiiOf(border.borderRadii);
    for (auto& corner : radii.corners) {
      corner = {corner.fX != 0 ? corner.fX + width * 0.5f + offset : 0, corner.fY != 0 ? corner.fY + width * 0.5f + offset : 0};
    }
    canvas.drawRRect(rrectOf(rect, radii), paint);
  } else {
    canvas.drawRect(rect, paint);
  }
}

// ColorStopUtils.getFixedColorStops: positions resolved along a gradient line of `length`, missing
// ones spread evenly, none before the one before, and transition hints turned into stops.
struct Stop {
  std::optional<uint32_t> color;
  std::optional<float> position;
};

std::vector<Stop> fixedStops(const std::vector<ColorStop>& stops, float length) {
  auto resolve = [length](const ValueUnit& position) -> std::optional<float> {
    switch (position.unit) {
      case UnitType::Point:
        return position.value / length;
      case UnitType::Percent:
        return position.value / 100;
      default:
        return std::nullopt;
    }
  };
  std::vector<Stop> fixed(stops.size());
  if (stops.empty()) return fixed;
  bool missing = false;
  float maxSoFar = resolve(stops[0].position).value_or(0);
  for (size_t i = 0; i < stops.size(); ++i) {
    auto position = resolve(stops[i].position);
    if (!position) {
      if (i == 0) position = 0.0f;
      else if (i == stops.size() - 1) position = 1.0f;
    }
    std::optional<uint32_t> color;
    if (stops[i].color) color = argb(stops[i].color, 0);
    if (position) {
      float kept = std::max(*position, maxSoFar);
      fixed[i] = Stop{color, kept};
      maxSoFar = kept;
    } else {
      fixed[i] = Stop{color, std::nullopt};
      missing = true;
    }
  }
  if (missing) {
    size_t lastDefined = 0;
    for (size_t i = 1; i < fixed.size(); ++i) {
      auto end = fixed[i].position, start = fixed[lastDefined].position;
      size_t unpositioned = i - lastDefined - 1;
      if (end && start && unpositioned > 0) {
        float increment = (*end - *start) / static_cast<float>(unpositioned + 1);
        for (size_t j = 1; j <= unpositioned; ++j) fixed[lastDefined + j].position = *start + increment * j;
        lastDefined = i;
      } else if (end) {
        lastDefined = i;
      }
    }
  }
  // processColorTransitionHints.
  std::vector<Stop> out(fixed.begin(), fixed.end());
  long indexOffset = 0;
  auto blend = [](uint32_t a, uint32_t b, float t) {
    auto channel = [&](int shift) {
      float ca = (a >> shift) & 0xff, cb = (b >> shift) & 0xff;
      return static_cast<uint32_t>(std::lround(ca + (cb - ca) * t)) << shift;
    };
    return channel(24) | channel(16) | channel(8) | channel(0);
  };
  for (size_t i = 1; i + 1 < fixed.size(); ++i) {
    if (fixed[i].color) continue;
    long x = static_cast<long>(i) + indexOffset;
    if (x < 1) continue;
    auto offsetLeft = out[x - 1].position, offsetRight = out[x + 1].position, offset = out[x].position;
    if (!offsetLeft || !offsetRight || !offset) continue;
    float leftDist = *offset - *offsetLeft, rightDist = *offsetRight - *offset, total = *offsetRight - *offsetLeft;
    auto leftColor = out[x - 1].color, rightColor = out[x + 1].color;
    auto equal = [](float a, float b) { return std::abs(a - b) < 0.00001f; };
    if (equal(leftDist, rightDist)) {
      out.erase(out.begin() + x);
      --indexOffset;
      continue;
    }
    if (equal(leftDist, 0)) {
      out[x].color = rightColor;
      continue;
    }
    if (equal(rightDist, 0)) {
      out[x].color = leftColor;
      continue;
    }
    std::vector<Stop> added;
    if (leftDist > rightDist) {
      for (int y = 0; y <= 6; ++y) added.push_back({std::nullopt, *offsetLeft + leftDist * ((7.0f + y) / 13.0f)});
      added.push_back({std::nullopt, *offset + rightDist * (1.0f / 3.0f)});
      added.push_back({std::nullopt, *offset + rightDist * (2.0f / 3.0f)});
    } else {
      added.push_back({std::nullopt, *offsetLeft + leftDist * (1.0f / 3.0f)});
      added.push_back({std::nullopt, *offsetLeft + leftDist * (2.0f / 3.0f)});
      for (int y = 0; y <= 6; ++y) added.push_back({std::nullopt, *offset + rightDist * (y / 13.0f)});
    }
    double logRatio = std::log(0.5) / std::log(leftDist / total);
    for (auto& stop : added) {
      float relative = (*stop.position - *offsetLeft) / total;
      auto weighting = static_cast<float>(std::pow(static_cast<double>(relative), logRatio));
      if (!std::isfinite(weighting)) continue;
      if (leftColor && rightColor) stop.color = blend(*leftColor, *rightColor, weighting);
    }
    out.erase(out.begin() + x);
    out.insert(out.begin() + x, added.begin(), added.end());
    indexOffset += 8;
  }
  return out;
}

// An android.graphics gradient's colours and positions, CLAMP, interpolated as Android's are
// (straight, not premultiplied, colours).
sk_sp<SkShader> gradient(SkPoint start, SkPoint end, const std::vector<Stop>& stops, bool radial, float radius) {
  std::vector<SkColor4f> colors;
  std::vector<float> positions;
  for (const auto& stop : stops) {
    // A stop Android leaves unset keeps its array's zeros: transparent black at 0.
    colors.push_back(stop.color && stop.position ? SkColor4f::FromColor(*stop.color) : SkColor4f{0, 0, 0, 0});
    positions.push_back(stop.color && stop.position ? *stop.position : 0);
  }
  if (colors.empty()) return nullptr;
  SkGradient description(SkGradient::Colors(colors, positions, SkTileMode::kClamp), SkGradient::Interpolation{});
  if (radial) return SkShaders::RadialGradient(start, radius, description);
  const SkPoint points[2] = {start, end};
  return SkShaders::LinearGradient(points, description);
}

// LinearGradient.getShader, for a tile of `width` × `height`.
sk_sp<SkShader> linearShader(const LinearGradient& linear, float width, float height) {
  double angle = 180;
  if (std::holds_alternative<Float>(linear.direction)) {
    angle = std::get<Float>(linear.direction);
  } else {
    double w = width, h = height;
    switch (std::get<GradientKeyword>(linear.direction)) {
      case GradientKeyword::ToTopRight:
        angle = 90 - std::atan(w / h) * 180 / M_PI;
        break;
      case GradientKeyword::ToBottomRight:
        angle = std::atan(w / h) * 180 / M_PI + 90;
        break;
      case GradientKeyword::ToTopLeft:
        angle = std::atan(w / h) * 180 / M_PI + 270;
        break;
      case GradientKeyword::ToBottomLeft:
        angle = std::atan(h / w) * 180 / M_PI + 180;
        break;
    }
  }
  // endPointsFromAngle.
  double adjusted = std::fmod(angle, 360.0);
  if (adjusted < 0) adjusted += 360;
  SkPoint start, end;
  if (adjusted == 0) {
    start = {0, height}, end = {0, 0};
  } else if (adjusted == 90) {
    start = {0, 0}, end = {width, 0};
  } else if (adjusted == 180) {
    start = {0, 0}, end = {0, height};
  } else if (adjusted == 270) {
    start = {width, 0}, end = {0, 0};
  } else {
    auto slope = static_cast<float>(std::tan((90 - adjusted) * M_PI / 180));
    float perpendicular = -1 / slope;
    float hw = width / 2, hh = height / 2;
    SkPoint corner = adjusted < 90 ? SkPoint{hw, hh} : adjusted < 180 ? SkPoint{hw, -hh}
                     : adjusted < 270 ? SkPoint{-hw, -hh} : SkPoint{-hw, hh};
    float c = corner.fY - perpendicular * corner.fX;
    float endX = c / (slope - perpendicular);
    float endY = perpendicular * endX + c;
    end = {hw + endX, hh - endY};
    start = {hw - endX, hh + endY};
  }
  float length = (end - start).length();
  return gradient(start, end, fixedStops(linear.colorStops, length), false, 0);
}

// RadialGradient.getShader.
sk_sp<SkShader> radialShader(const RadialGradient& radial, float width, float height) {
  float cx = width / 2, cy = height / 2;
  const auto& position = radial.position;
  if (position.top) {
    cy = position.top->resolve(height);
  } else if (position.bottom) {
    cy = height - position.bottom->resolve(height);
  }
  if (position.left) {
    cx = position.left->resolve(width);
  } else if (position.right) {
    cx = width - position.right->resolve(width);
  }
  bool circle = radial.shape == RadialGradientShape::Circle;
  using Keyword = RadialGradientSize::SizeKeyword;
  auto toSide = [&](Keyword keyword) {
    float rx, ry;
    if (keyword == Keyword::ClosestSide) {
      rx = std::min(cx, width - cx), ry = std::min(cy, height - cy);
    } else {
      rx = std::max(cx, width - cx), ry = std::max(cy, height - cy);
    }
    if (circle) {
      float r = keyword == Keyword::ClosestSide ? std::min(rx, ry) : std::max(rx, ry);
      return std::pair{r, r};
    }
    return std::pair{rx, ry};
  };
  auto toCorner = [&](Keyword keyword) {
    const SkPoint corners[4] = {{0, 0}, {width, 0}, {width, height}, {0, height}};
    bool closest = keyword == Keyword::ClosestCorner;
    size_t index = 0;
    float distance = SkPoint::Distance({cx, cy}, corners[0]);
    for (size_t i = 1; i < 4; ++i) {
      float d = SkPoint::Distance({cx, cy}, corners[i]);
      if (closest ? d < distance : d > distance) distance = d, index = i;
    }
    if (circle) return std::pair{distance, distance};
    auto side = toSide(closest ? Keyword::ClosestSide : Keyword::FarthestSide);
    float aspect = side.first / side.second;
    if (aspect == 0 || !std::isfinite(aspect)) return std::pair{0.0f, 0.0f};
    float ox = corners[index].fX - cx, oy = corners[index].fY - cy;
    float a = std::sqrt(ox * ox + oy * oy * aspect * aspect);
    return std::pair{a, a / aspect};
  };
  std::pair<float, float> radius;
  if (std::holds_alternative<Keyword>(radial.size.value)) {
    auto keyword = std::get<Keyword>(radial.size.value);
    radius = keyword == Keyword::ClosestSide || keyword == Keyword::FarthestSide ? toSide(keyword) : toCorner(keyword);
  } else {
    const auto& d = std::get<RadialGradientSize::Dimensions>(radial.size.value);
    float rx = d.x.resolve(width), ry = d.y.resolve(height);
    radius = circle ? std::pair{std::max(rx, ry), std::max(rx, ry)} : std::pair{rx, ry};
  }
  auto stops = fixedStops(radial.colorStops, std::max(radius.first, radius.second));
  auto shader = gradient({cx, cy}, {cx, cy}, stops, true, std::max(radius.first, 0.00001f));
  if (shader && !circle && std::abs(radius.first - radius.second) > 0.00001f) {
    // An ellipse: the circle scaled about the centre, as Matrix.setScale(1, ry / rx, cx, cy).
    SkMatrix about;
    about.setScale(1, radius.second / radius.first, cx, cy);
    shader = shader->makeWithLocalMatrix(about);
  }
  return shader;
}

// BackgroundImageDrawable.draw: each layer, the last first, tiled over the border box from the
// padding box's origin, within the border box's rounded rectangle.
void drawBackgroundImage(SkCanvas& canvas, const ViewProps& props, const BorderMetrics& border, const SkRect& bounds,
                         float opacity) {
  if (props.backgroundImage.empty()) return;
  const auto& w = border.borderWidths;
  SkRect positioning = SkRect::MakeLTRB(bounds.fLeft + w.left, bounds.fTop + w.top, bounds.fRight - w.right,
                                        bounds.fBottom - w.bottom);
  SkRect painting = bounds;
  if (positioning.width() <= 0 || positioning.height() <= 0 || painting.width() <= 0 || painting.height() <= 0) return;
  canvas.save();
  if (rounded(border.borderRadii)) {
    canvas.clipRRect(rrectOf(painting, radiiOf(border.borderRadii)), SkClipOp::kIntersect, true);
  } else {
    canvas.clipRect(painting, SkClipOp::kIntersect, true);
  }
  const auto& layers = props.backgroundImage;
  for (size_t index = layers.size(); index-- > 0;) {
    auto pick = [index](const auto& list) -> const auto* { return list.empty() ? nullptr : &list[index % list.size()]; };
    const BackgroundSize* size = pick(props.backgroundSize);
    const BackgroundRepeat* repeat = pick(props.backgroundRepeat);
    const BackgroundPosition* position = pick(props.backgroundPosition);
    // calculateBackgroundImageSize: a gradient's natural size is the positioning area's.
    float tileWidth = positioning.width(), tileHeight = positioning.height();
    if (size && std::holds_alternative<BackgroundSizeLengthPercentage>(*size)) {
      const auto& lengths = std::get<BackgroundSizeLengthPercentage>(*size);
      if (!lengths.isXAuto() && !lengths.isYAuto()) {
        tileWidth = std::get<ValueUnit>(lengths.x).resolve(positioning.width());
        tileHeight = std::get<ValueUnit>(lengths.y).resolve(positioning.height());
      }
    }
    auto repeatX = repeat ? repeat->x : BackgroundRepeatStyle::Repeat;
    auto repeatY = repeat ? repeat->y : BackgroundRepeatStyle::Repeat;
    if (repeatX == BackgroundRepeatStyle::Round && tileWidth > 0 &&
        std::abs(std::fmod(positioning.width(), tileWidth)) > 0.00001f) {
      float count = std::round(positioning.width() / tileWidth);
      if (count > 0) tileWidth = positioning.width() / count;
    }
    if (repeatY == BackgroundRepeatStyle::Round && tileHeight > 0 &&
        std::abs(std::fmod(positioning.height(), tileHeight)) > 0.00001f) {
      float count = std::round(positioning.height() / tileHeight);
      if (count > 0) tileHeight = positioning.height() / count;
    }
    if (tileWidth <= 0 || tileHeight <= 0) continue;
    sk_sp<SkShader> shader = std::holds_alternative<LinearGradient>(layers[index])
                                 ? linearShader(std::get<LinearGradient>(layers[index]), tileWidth, tileHeight)
                                 : radialShader(std::get<RadialGradient>(layers[index]), tileWidth, tileHeight);
    if (!shader) continue;
    SkPaint paint;
    paint.setAntiAlias(true);
    paint.setShader(shader);
    paint.setAlphaf(opacity);
    // calculateBackgroundPosition.
    float availableX = positioning.width() - tileWidth, availableY = positioning.height() - tileHeight;
    float x = 0, y = 0;
    if (position && position->left) {
      x = position->left->resolve(availableX);
    } else if (position && position->right) {
      x = availableX - position->right->resolve(availableX);
    }
    if (position && position->top) {
      y = position->top->resolve(availableY);
    } else if (position && position->bottom) {
      y = availableY - position->bottom->resolve(availableY);
    }
    x += positioning.fLeft, y += positioning.fTop;
    auto tiles = [&](BackgroundRepeatStyle style, float tile, float extent, float origin, float& initial, float& spacing) {
      int count = 1;
      spacing = 0;
      float roundedTile = std::round(tile);
      if (style == BackgroundRepeatStyle::Space) {
        float available = extent - tile * 2;
        if (roundedTile > 0 && (available > 0 || std::abs(available) < 0.00001f)) {
          int center = static_cast<int>(std::floor(std::round(available) / roundedTile));
          float free = available - center * tile;
          count = center + 2;
          spacing = free / static_cast<float>(count - 1);
          initial = origin;
        }
      } else if (style == BackgroundRepeatStyle::Round || style == BackgroundRepeatStyle::Repeat) {
        if (roundedTile > 0) {
          int before = static_cast<int>(std::ceil(std::round(initial - origin) / roundedTile));
          int after = static_cast<int>(std::ceil(std::round(extent - (initial - origin)) / roundedTile));
          count = before + after;
          initial -= before * tile;
        }
      }
      return count;
    };
    float spacingX = 0, spacingY = 0;
    // Android measures the tiles from the painting area's origin, which is the view's.
    int columns = tiles(repeatX, tileWidth, painting.width(), painting.fLeft, x, spacingX);
    int rows = tiles(repeatY, tileHeight, painting.height(), painting.fTop, y, spacingY);
    float tx = x;
    for (int i = 0; i < columns; ++i, tx += tileWidth + spacingX) {
      float ty = y;
      for (int j = 0; j < rows; ++j, ty += tileHeight + spacingY) {
        canvas.save();
        canvas.translate(tx, ty);
        canvas.drawRect(SkRect::MakeWH(tileWidth, tileHeight), paint);
        canvas.restore();
      }
    }
  }
  canvas.restore();
}

}  // namespace

SkRRect borderBox(const BorderMetrics& border, float x, float y, float w, float h) {
  return rrectOf(SkRect::MakeXYWH(x, y, w, h), radiiOf(border.borderRadii));
}

bool drawsBackground(const ViewProps& props) {
  if (alphaOf(argb(props.backgroundColor, 0)) != 0 || !props.boxShadow.empty() || !props.backgroundImage.empty() ||
      props.outlineWidth != 0) {
    return true;
  }
  for (auto edge : {facebook::yoga::Edge::All, facebook::yoga::Edge::Left, facebook::yoga::Edge::Top, facebook::yoga::Edge::Right, facebook::yoga::Edge::Bottom,
                    facebook::yoga::Edge::Start, facebook::yoga::Edge::End, facebook::yoga::Edge::Horizontal, facebook::yoga::Edge::Vertical}) {
    auto width = props.yogaStyle.border(edge).value();
    if (width.isDefined() && width.unwrap() > 0) return true;
  }
  return false;
}

bool hasBackground(const ViewProps& props, const LayoutMetrics& layout) {
  if (drawsBackground(props) || props.backgroundColor) return true;
  auto border = props.resolveBorderMetrics(layout);
  const auto& colors = border.borderColors;
  return rounded(border.borderRadii) || colors.left || colors.top || colors.right || colors.bottom;
}

void drawBackground(SkCanvas& canvas, const ViewProps& props, const LayoutMetrics& layout, float x, float y, float w,
                    float h, float opacity, float scale) {
  if (opacity <= 0) return;
  auto border = props.resolveBorderMetrics(layout);
  SkRect bounds = SkRect::MakeXYWH(x, y, w, h);
  // CompositeBackgroundDrawable's layers: the shadows in reverse, so the first is on top.
  for (auto it = props.boxShadow.rbegin(); it != props.boxShadow.rend(); ++it) {
    if (!it->inset) drawOutsetShadow(canvas, *it, border.borderRadii, bounds, opacity, scale);
  }
  drawBackgroundColor(canvas, props, border, bounds, opacity, scale);
  drawBackgroundImage(canvas, props, border, bounds, opacity);
  drawBorder(canvas, props, border, bounds, opacity, scale);
  for (auto it = props.boxShadow.rbegin(); it != props.boxShadow.rend(); ++it) {
    if (it->inset) drawInsetShadow(canvas, *it, border, bounds, opacity, scale);
  }
  drawOutline(canvas, props, border, bounds, opacity, scale);
}

Overflow backgroundOverflow(const ViewProps& props) {
  Overflow out;
  for (const auto& shadow : props.boxShadow) {
    if (shadow.inset) continue;
    // A Gaussian's reach: three deviations (half the blur radius each) past the spread.
    float reach = std::max(0.0f, shadow.spreadDistance) + 1.5f * std::max(0.0f, shadow.blurRadius) + 1;
    out.left = std::max(out.left, reach - shadow.offsetX);
    out.right = std::max(out.right, reach + shadow.offsetX);
    out.top = std::max(out.top, reach - shadow.offsetY);
    out.bottom = std::max(out.bottom, reach + shadow.offsetY);
  }
  if (props.outlineWidth > 0) {
    float reach = props.outlineWidth + std::max(0.0f, props.outlineOffset) + 1;
    out.left = std::max(out.left, reach);
    out.right = std::max(out.right, reach);
    out.top = std::max(out.top, reach);
    out.bottom = std::max(out.bottom, reach);
  }
  return out;
}

void clipToPaddingBox(SkCanvas& canvas, const ViewProps& props, const LayoutMetrics& layout, float x, float y, float w,
                      float h) {
  auto border = props.resolveBorderMetrics(layout);
  const auto& widths = border.borderWidths;
  SkRect padding = SkRect::MakeLTRB(x + widths.left, y + widths.top, x + w - widths.right, y + h - widths.bottom);
  if (!rounded(border.borderRadii)) {
    canvas.clipRect(padding, SkClipOp::kIntersect, false);
    return;
  }
  const auto& r = border.borderRadii;
  Radii inner{{{std::max(0.0f, r.topLeft.horizontal - widths.left), std::max(0.0f, r.topLeft.vertical - widths.top)},
               {std::max(0.0f, r.topRight.horizontal - widths.right), std::max(0.0f, r.topRight.vertical - widths.top)},
               {std::max(0.0f, r.bottomRight.horizontal - widths.right),
                std::max(0.0f, r.bottomRight.vertical - widths.bottom)},
               {std::max(0.0f, r.bottomLeft.horizontal - widths.left), std::max(0.0f, r.bottomLeft.vertical - widths.bottom)}}};
  canvas.clipRRect(rrectOf(padding, inner), SkClipOp::kIntersect, true);
}

namespace {

// ColorMatrix rows as Skia takes them: its translations from Android's 0 to 255 to 0 to 1.
sk_sp<SkColorFilter> matrix(std::array<float, 20> m) {
  for (int row = 0; row < 4; ++row) m[row * 5 + 4] /= 255.0f;
  return SkColorFilters::Matrix(m.data());
}

sk_sp<SkColorFilter> colorMatrixFilter(FilterType type, float amount) {
  float inv = 1 - amount;
  switch (type) {
    case FilterType::Brightness:
      return matrix({amount, 0, 0, 0, 0, 0, amount, 0, 0, 0, 0, 0, amount, 0, 0, 0, 0, 0, 1, 0});
    case FilterType::Opacity:
      return matrix({1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, amount, 0});
    case FilterType::Contrast: {
      float intercept = 255 * (-(amount / 2.0f) + 0.5f);
      return matrix({amount, 0, 0, 0, intercept, 0, amount, 0, 0, intercept, 0, 0, amount, 0, intercept, 0, 0, 0, 1, 0});
    }
    case FilterType::Grayscale:
      return matrix({0.2126f + 0.7874f * inv, 0.7152f - 0.7152f * inv, 0.0722f - 0.0722f * inv, 0, 0,
                     0.2126f - 0.2126f * inv, 0.7152f + 0.2848f * inv, 0.0722f - 0.0722f * inv, 0, 0,
                     0.2126f - 0.2126f * inv, 0.7152f - 0.7152f * inv, 0.0722f + 0.9278f * inv, 0, 0,
                     0, 0, 0, 1, 0});
    case FilterType::Sepia:
      return matrix({0.393f + 0.607f * inv, 0.769f - 0.769f * inv, 0.189f - 0.189f * inv, 0, 0,
                     0.349f - 0.349f * inv, 0.686f + 0.314f * inv, 0.168f - 0.168f * inv, 0, 0,
                     0.272f - 0.272f * inv, 0.534f - 0.534f * inv, 0.131f + 0.869f * inv, 0, 0,
                     0, 0, 0, 1, 0});
    case FilterType::Saturate: {
      // ColorMatrix.setSaturation.
      float r = 0.213f * inv, g = 0.715f * inv, b = 0.072f * inv;
      return matrix({r + amount, g, b, 0, 0, r, g + amount, b, 0, 0, r, g, b + amount, 0, 0, 0, 0, 0, 1, 0});
    }
    case FilterType::HueRotate: {
      float c = std::cos(amount * static_cast<float>(M_PI) / 180), s = std::sin(amount * static_cast<float>(M_PI) / 180);
      return matrix({0.213f + c * 0.787f - s * 0.213f, 0.715f - c * 0.715f - s * 0.715f, 0.072f - c * 0.072f + s * 0.928f, 0, 0,
                     0.213f - c * 0.213f + s * 0.143f, 0.715f + c * 0.285f + s * 0.140f, 0.072f - c * 0.072f - s * 0.283f, 0, 0,
                     0.213f - c * 0.213f - s * 0.787f, 0.715f - c * 0.715f + s * 0.715f, 0.072f + c * 0.928f + s * 0.072f, 0, 0,
                     0, 0, 0, 1, 0});
    }
    case FilterType::Invert: {
      float slope = 1 - 2 * amount, intercept = amount * 255;
      return matrix({slope, 0, 0, 0, intercept, 0, slope, 0, 0, intercept, 0, 0, slope, 0, intercept, 0, 0, 0, 1, 0});
    }
    default:
      return nullptr;
  }
}

}  // namespace

sk_sp<SkImageFilter> viewFilter(const ViewProps& props) {
  sk_sp<SkImageFilter> chain;
  for (const auto& filter : props.filter) {
    if (filter.type == FilterType::Blur) {
      float sigma = std::get<Float>(filter.parameters);
      // createBlurEffect: nothing at half a pixel or less (the scale is the canvas's; a logical
      // half pixel is taken here, which is what Android's density 1 sees).
      if (sigma <= 0.5f) continue;
      chain = SkImageFilters::Blur(sigma, sigma, SkTileMode::kDecal, chain);
    } else if (filter.type == FilterType::DropShadow) {
      const auto& shadow = std::get<DropShadowParams>(filter.parameters);
      // createDropShadowEffect: the content in the shadow's colour (SRC_IN), moved and blurred,
      // under the content itself.
      auto identity = SkImageFilters::Offset(0, 0, chain);
      auto moved = SkImageFilters::Offset(shadow.offsetX, shadow.offsetY, chain);
      auto colored = SkImageFilters::ColorFilter(
          SkColorFilters::Blend(SkColor4f::FromColor(argb(shadow.color, kBlack)), nullptr, SkBlendMode::kSrcIn), moved);
      auto blurred = shadow.standardDeviation > 0.5f
                         ? SkImageFilters::Blur(shadow.standardDeviation, shadow.standardDeviation, SkTileMode::kDecal, colored)
                         : colored;
      chain = SkImageFilters::Blend(SkBlendMode::kSrcOver, blurred, identity);
    } else if (auto color = colorMatrixFilter(filter.type, std::get<Float>(filter.parameters))) {
      chain = SkImageFilters::ColorFilter(color, chain);
    }
  }
  return chain;
}

float filterOverflow(const ViewProps& props) {
  float reach = 0;
  for (const auto& filter : props.filter) {
    if (filter.type == FilterType::Blur) {
      reach += 3 * std::get<Float>(filter.parameters);
    } else if (filter.type == FilterType::DropShadow) {
      const auto& shadow = std::get<DropShadowParams>(filter.parameters);
      reach += 3 * shadow.standardDeviation + std::max(std::abs(shadow.offsetX), std::abs(shadow.offsetY));
    }
  }
  return reach > 0 ? reach + 1 : 0;
}

}  // namespace oxyos
