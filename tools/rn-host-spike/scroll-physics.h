// The physics of a touchpad's scroll, as Android's: the velocity the fingers leave with
// (VelocityTracker) and the fling that follows (OverScroller), for the host's scroll views
// (input.h). Logical pixels are Android's dp.
#pragma once

#include <array>
#include <cmath>
#include <deque>

namespace oxyos {

// The velocity a scroll leaves with, as Android's default VelocityTracker estimates a pointer's
// (LeastSquaresVelocityTrackerStrategy, degree 2): a second-degree least-squares fit of the
// position over the samples of the last 100 ms (at most 20), unweighted, whose slope at the
// newest sample is the velocity; none once the pointer has stood still for 40 ms
// (ASSUME_POINTER_STOPPED_TIME).
class ScrollVelocity {
 public:
  void clear() { samples_.clear(); }
  // The scroll's accumulated position (logical pixels) at `ms`.
  void add(double ms, float x, float y) {
    samples_.push_back({ms, x, y});
    while (samples_.size() > kMaxSamples) samples_.pop_front();
  }
  // Logical pixels per second on each axis at `now` (the fingers' lift).
  std::array<float, 2> at(double now) const {
    if (samples_.empty() || now - samples_.back().ms > kStoppedMs) return {0, 0};
    return {fit(&Sample::x), fit(&Sample::y)};
  }

 private:
  struct Sample {
    double ms;
    float x, y;
  };
  static constexpr size_t kMaxSamples = 20;
  static constexpr double kHorizonMs = 100, kStoppedMs = 40;

  float fit(float Sample::*axis) const {
    const double newest = samples_.back().ms;
    // Ages in seconds (zero or negative) and positions, newest first, within the horizon.
    double t[kMaxSamples], p[kMaxSamples];
    size_t m = 0;
    for (auto it = samples_.rbegin(); it != samples_.rend() && m < kMaxSamples; ++it) {
      if (newest - it->ms > kHorizonMs) break;
      t[m] = (it->ms - newest) / 1000.0;
      p[m] = (*it).*axis;
      ++m;
    }
    if (m < 2) return 0;
    if (m == 2) {
      // Android's degree falls to what the samples allow: a line through the two.
      return t[0] == t[1] ? 0 : static_cast<float>((p[0] - p[1]) / (t[0] - t[1]));
    }
    // Normal equations of p = b0 + b1 t + b2 t², solved by Cramer's rule; the slope at t = 0 is b1.
    double s[5] = {0, 0, 0, 0, 0}, r[3] = {0, 0, 0};
    for (size_t i = 0; i < m; ++i) {
      double power = 1;
      for (int k = 0; k < 5; ++k) {
        s[k] += power;
        if (k < 3) r[k] += power * p[i];
        power *= t[i];
      }
    }
    const double a[3][3] = {{s[0], s[1], s[2]}, {s[1], s[2], s[3]}, {s[2], s[3], s[4]}};
    auto det = [](const double (&m)[3][3]) {
      return m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) +
             m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    };
    const double d = det(a);
    if (std::abs(d) < 1e-18) {
      return t[0] == t[m - 1] ? 0 : static_cast<float>((p[0] - p[m - 1]) / (t[0] - t[m - 1]));
    }
    const double b1[3][3] = {{s[0], r[0], s[2]}, {s[1], r[1], s[3]}, {s[2], r[2], s[4]}};
    return static_cast<float>(det(b1) / d);
  }

  std::deque<Sample> samples_;
};

// A scroll view's fling, as Android's OverScroller flings a ScrollView (SplineOverScroller): the
// distance and duration from the velocity the fingers left with, through the scroll friction
// (ViewConfiguration.getScrollFriction, 0.015, which React Native's `decelerationRate` 0.985 is
// one minus) and the deceleration of a device of 160 dots per inch under Earth's gravity (logical
// pixels are Android's dp), and the position over time along Android's spline.
class SplineFling {
 public:
  // Starts a fling at `velocity` logical pixels per second, with `friction` (1 - decelerationRate).
  SplineFling(float velocity, float friction = 0.015f) {
    if (velocity == 0) return;
    // SplineOverScroller.getSplineDeceleration, getSplineFlingDuration, getSplineFlingDistance.
    const double deceleration = std::log(kInflexion * std::abs(velocity) / (friction * kPhysicalCoeff));
    durationMs_ = 1000.0 * std::exp(deceleration / (kDecelerationRate - 1.0));
    distance_ = friction * kPhysicalCoeff * std::exp(kDecelerationRate / (kDecelerationRate - 1.0) * deceleration);
    if (velocity < 0) distance_ = -distance_;
  }
  double durationMs() const { return durationMs_; }
  double distance() const { return distance_; }
  // How far it has gone `ms` after it began (SplineOverScroller.update's spline step); the whole
  // distance once its duration passed.
  double at(double ms) const {
    if (durationMs_ <= 0 || ms >= durationMs_) return distance_;
    const auto& spline = table();
    const double t = ms / durationMs_;
    const int index = static_cast<int>(kSamples * t);
    if (index >= kSamples) return distance_;
    const double low = static_cast<double>(index) / kSamples, high = static_cast<double>(index + 1) / kSamples;
    const double coefficient = (spline[index + 1] - spline[index]) / (high - low);
    return (spline[index] + (t - low) * coefficient) * distance_;
  }

 private:
  static constexpr double kInflexion = 0.35;
  // log(0.78) / log(0.9).
  static constexpr double kDecelerationRate = 2.3582018154259448;
  // SensorManager.GRAVITY_EARTH (m/s²) × 39.37 (inches per metre) × 160 (dots per inch) × 0.84,
  // OverScroller's physical coefficient for a device of density 1.
  static constexpr double kPhysicalCoeff = 9.80665 * 39.37 * 160.0 * 0.84;
  static constexpr int kSamples = 100;
  // SplineOverScroller's SPLINE_POSITION, computed as its static initializer computes it.
  static const std::array<double, kSamples + 1>& table() {
    static const std::array<double, kSamples + 1> spline = [] {
      std::array<double, kSamples + 1> position{};
      constexpr double startTension = 0.5, endTension = 1.0;
      constexpr double p1 = startTension * kInflexion, p2 = 1.0 - endTension * (1.0 - kInflexion);
      double xMin = 0.0;
      for (int i = 0; i < kSamples; ++i) {
        const double alpha = static_cast<double>(i) / kSamples;
        double xMax = 1.0, x, tx, coefficient;
        while (true) {
          x = xMin + (xMax - xMin) / 2.0;
          coefficient = 3.0 * x * (1.0 - x);
          tx = coefficient * ((1.0 - x) * p1 + x * p2) + x * x * x;
          if (std::abs(tx - alpha) < 1e-5) break;
          if (tx > alpha) {
            xMax = x;
          } else {
            xMin = x;
          }
        }
        position[i] = coefficient * ((1.0 - x) * startTension + x) + x * x * x;
      }
      position[kSamples] = 1.0;
      return position;
    }();
    return spline;
  }

  double durationMs_ = 0, distance_ = 0;
};

}  // namespace oxyos
