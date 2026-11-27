#include "progress-bar.h"

#include <react/renderer/core/LayoutConstraints.h>
#include <react/renderer/graphics/Color.h>

#include <include/core/SkCanvas.h>
#include <include/core/SkPaint.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <cstring>

namespace oxyos {

using namespace facebook::react;

// NOLINTNEXTLINE(modernize-avoid-c-arrays)
const char kAndroidProgressBar[] = "AndroidProgressBar";

namespace {

// AOSP's progress_indeterminate_material: the trims' cycle, and the ring's turn.
constexpr double kTrimCycleMs = 1333;
constexpr double kTurnMs = 4444;
constexpr double kTurns = 2;  // 720 degrees a turn of the animation
constexpr double kTrimTo = 0.75, kOffsetTo = 0.25;
// The horizontal bar's indeterminate sweep (a segment a third of the track wide crossing it).
constexpr double kSweepMs = 2000;

std::atomic<bool> gReducedMotion = false;

// A cubic Bézier's y at its x (x increasing along it), as Android's PathInterpolator reads its path.
double bezierAt(double x, double x0, double y0, double x1, double y1, double x2, double y2, double x3, double y3) {
  auto at = [](double t, double a, double b, double c, double d) {
    double u = 1 - t;
    return u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d;
  };
  double lo = 0, hi = 1;
  for (int i = 0; i < 40; ++i) {
    double mid = (lo + hi) / 2;
    (at(mid, x0, x1, x2, x3) < x ? lo : hi) = mid;
  }
  return at((lo + hi) / 2, y0, y1, y2, y3);
}

// trim_start_interpolator: "L0.5,0 C 0.7,0 0.6,1 1,1".
double trimStart(double x) {
  if (x <= 0.5) return 0;
  return bezierAt(x, 0.5, 0, 0.7, 0, 0.6, 1, 1, 1);
}

// trim_end_interpolator: "C0.2,0 0.1,1 0.5,1 L 1,1".
double trimEnd(double x) {
  if (x >= 0.5) return 1;
  return bezierAt(x, 0, 0, 0.2, 0, 0.1, 1, 0.5, 1);
}

double cycle(double ms, double period) {
  double f = std::fmod(ms, period) / period;
  return f < 0 ? f + 1 : f;
}

}  // namespace

Size ProgressBarShadowNode::measureContent(const LayoutContext& /*layoutContext*/,
                                           const LayoutConstraints& layoutConstraints) const {
  const auto& props = getConcreteProps();
  float side = progressBarSize(props.styleAttr);
  if (props.styleAttr == "Horizontal") {
    // As wide as it may be (else a Normal one's side), as tall as Android's horizontal style.
    float width = std::isfinite(layoutConstraints.maximumSize.width) ? layoutConstraints.maximumSize.width : 48;
    return layoutConstraints.clamp(Size{width, side});
  }
  return layoutConstraints.clamp(Size{side, side});
}

float progressBarSize(const std::string& style) {
  if (style == "Small" || style == "SmallInverse") return 16;
  if (style == "Large" || style == "LargeInverse") return 76;
  if (style == "Horizontal") return 16;
  return 48;  // Normal, Inverse
}

bool progressBarShown(const AndroidProgressBarProps& props) { return props.animating; }

bool progressBarSpins(const AndroidProgressBarProps& props) { return props.animating && props.indeterminate; }

bool spins(const ShadowView& view) {
  if (!view.componentName || std::strcmp(view.componentName, kAndroidProgressBar) != 0 || !view.props) return false;
  return progressBarSpins(static_cast<const AndroidProgressBarProps&>(*view.props));
}

ProgressArc indeterminateArc(double ms, bool reducedMotion) {
  double f = cycle(ms, kTrimCycleMs);
  double turn = kTurns * cycle(ms, kTurnMs) + kOffsetTo * f;
  if (reducedMotion) return ProgressArc{turn, turn + kTrimTo};
  double start = kTrimTo * trimStart(f), end = kTrimTo * trimEnd(f);
  return ProgressArc{turn + start, turn + std::max(start, end)};
}

ProgressArc determinateArc(double progress) { return ProgressArc{0, std::clamp(progress, 0.0, 1.0)}; }

void drawProgressBar(SkCanvas& canvas, const AndroidProgressBarProps& props, float x, float y, float w, float h,
                     uint32_t accent, float opacity, double ms, bool reducedMotion) {
  if (!progressBarShown(props) || w <= 0 || h <= 0) return;
  SkColor4f color;
  if (props.color) {
    auto c = colorComponentsFromColor(props.color);
    color = SkColor4f{c.red, c.green, c.blue, c.alpha * opacity};
  } else {
    color = SkColor4f::FromColor(0xff000000u | accent);
    color.fA *= opacity;
  }
  SkPaint paint(color);
  paint.setAntiAlias(true);
  if (props.styleAttr == "Horizontal") {
    // A track 4 dp tall centred in the box, its progress (or a third of it sweeping across) filled.
    float track = std::min(4.0f, h), top = y + (h - track) / 2;
    SkPaint under(SkColor4f{color.fR, color.fG, color.fB, color.fA * 0.3f});
    under.setAntiAlias(true);
    canvas.drawRect(SkRect::MakeXYWH(x, top, w, track), under);
    double from = 0, to = std::clamp(props.progress, 0.0, 1.0);
    if (props.indeterminate) {
      double f = reducedMotion ? 0.5 : cycle(ms, kSweepMs);
      from = std::clamp(f * 4 / 3 - 1.0 / 3, 0.0, 1.0);
      to = std::clamp(f * 4 / 3, 0.0, 1.0);
    }
    if (to > from) canvas.drawRect(SkRect::MakeXYWH(x + w * from, top, w * (to - from), track), paint);
    return;
  }
  // The ring, AOSP's vector scaled into the box's square: Small's 12-unit viewport, else the
  // 48-unit one.
  bool small = props.styleAttr == "Small" || props.styleAttr == "SmallInverse";
  double viewport = small ? 12 : 48, radius = small ? 5 : 19, stroke = small ? 1 : 4;
  float side = std::min(w, h);
  float unit = static_cast<float>(side / viewport);
  float cx = x + w / 2, cy = y + h / 2;
  float r = static_cast<float>(radius) * unit;
  ProgressArc arc = props.indeterminate ? indeterminateArc(ms, reducedMotion) : determinateArc(props.progress);
  double sweep = arc.to - arc.from;
  if (sweep <= 0) return;
  paint.setStyle(SkPaint::kStroke_Style);
  paint.setStrokeWidth(static_cast<float>(stroke) * unit);
  paint.setStrokeCap(SkPaint::kSquare_Cap);
  // From 12 o'clock (Skia's -90 degrees), clockwise.
  canvas.drawArc(SkRect::MakeLTRB(cx - r, cy - r, cx + r, cy + r), static_cast<float>(-90 + 360 * arc.from),
                 static_cast<float>(360 * std::min(sweep, 1.0)), false, paint);
}

double progressClockMs() {
  static const double held = [] {
    const char* phase = std::getenv("OXYOS_HOST_PROGRESS_PHASE_MS");
    return phase && *phase ? std::atof(phase) : -1.0;
  }();
  if (held >= 0) return held;
  static const auto start = std::chrono::steady_clock::now();
  return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start).count();
}

void setProgressReducedMotion(bool reduced) { gReducedMotion = reduced; }

bool progressReducedMotion() { return gReducedMotion; }

}  // namespace oxyos
