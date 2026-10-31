// The physics of a touchpad's scroll, as Android's: the velocity the fingers leave with
// (VelocityTracker) and the fling that follows (OverScroller), for the host's scroll views
// (input.h). Logical pixels are Android's dp.
#pragma once

#include <algorithm>
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
  // Its velocity `ms` after it began, logical pixels per second, signed as its distance
  // (SplineOverScroller.update's mCurrVelocity: the spline's slope over its duration); none once
  // its duration passed.
  double velocity(double ms) const {
    if (durationMs_ <= 0 || ms >= durationMs_) return 0;
    const auto& spline = table();
    const int index = std::min(static_cast<int>(kSamples * ms / durationMs_), kSamples - 1);
    const double coefficient = (spline[index + 1] - spline[index]) * kSamples;
    return coefficient * distance_ / durationMs_ * 1000.0;
  }
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

// Android's stretch overscroll (EdgeEffect with TYPE_STRETCH, the default since Android 12), as
// a ScrollView drives it at one edge of an axis: how far past its end the content is pulled, as a
// share of the view's size along the axis (`distance`), which a drag pulls (onPullDistance) and
// takes back when it moves the other way, the fingers lifting release (onRelease) and a fling
// reaching the end strikes (onAbsorb); released or struck, it springs back to rest
// (updateSpring). What is drawn is `amount()`, the stretch EdgeEffect gives the view's render
// node (dampStretchVector). Constants are AOSP's EdgeEffect, from its source as recalled (no
// copy of it is at hand): the spring's natural frequency 24.657 and damping ratio 0.98, a strike's
// velocity multiplied by 13, rest under 0.001 of a pixel moving under 0.01 a second, and the
// stretch 0.016 of the distance linearly plus 0.016 of (1 - e^(-distance·e/0.33)).
class EdgeStretch {
 public:
  float distance() const { return distance_; }
  bool resting() const { return state_ == State::Idle; }
  bool pulling() const { return state_ == State::Pull; }
  // onPullDistance: pulled by `delta` (a share of the view's size; negative takes it back),
  // never below none; returns what it took.
  float pull(float delta) {
    float to = std::max(0.0f, distance_ + delta);
    float taken = to - distance_;
    if (taken == 0 && to == 0) return 0;
    state_ = State::Pull;
    distance_ = to;
    velocity_ = 0;
    return taken;
  }
  // onRelease: let go at `ms` (the host's clock, milliseconds): it springs back from where it is.
  void release(double ms) {
    if (state_ != State::Pull) return;
    state_ = distance_ > 0 ? State::Recede : State::Idle;
    velocity_ = 0;
    started_ = ms;
  }
  // onAbsorb: a fling reaching the end at `velocity` (logical pixels per second, its magnitude)
  // strikes it at `ms`.
  void absorb(double velocity, double ms) {
    state_ = State::Recede;
    velocity_ = std::abs(velocity) * kAbsorbVelocityAdjustment;
    started_ = ms;
  }
  // updateSpring, for a view `size` logical pixels along the axis: where the spring is at `ms`;
  // true if it moved.
  bool step(double ms, float size) {
    if (state_ != State::Recede || size <= 0) return false;
    const double dt = (ms - started_) / 1000.0;
    if (dt < 0.001) return false;
    started_ = ms;
    const double damped = kNaturalFrequency * std::sqrt(1 - kDampingRatio * kDampingRatio);
    const double cosCoefficient = distance_ * size;
    const double sinCoefficient = (kDampingRatio * kNaturalFrequency * cosCoefficient + velocity_) / damped;
    const double decay = std::exp(-kDampingRatio * kNaturalFrequency * dt);
    const double displacement =
        decay * (cosCoefficient * std::cos(damped * dt) + sinCoefficient * std::sin(damped * dt));
    const double velocity = displacement * -kNaturalFrequency * kDampingRatio +
                            decay * (-damped * cosCoefficient * std::sin(damped * dt) +
                                     damped * sinCoefficient * std::cos(damped * dt));
    float before = distance_;
    distance_ = static_cast<float>(displacement / size);
    velocity_ = velocity;
    if (distance_ > 1) {
      distance_ = 1;
      velocity_ = 0;
    }
    // isAtEquilibrium: past rest, or still and within a thousandth of a pixel of it.
    if (displacement < 0 || (std::abs(velocity) < kVelocityThreshold && displacement < kValueThreshold)) {
      state_ = State::Idle;
      distance_ = 0;
      velocity_ = 0;
    }
    return distance_ != before;
  }
  // dampStretchVector: the stretch drawn, a share of the view's size.
  float amount() const {
    const double scalar = M_E / kExpStretchDistance;
    return static_cast<float>(kLinearStretch * distance_ + kExpStretch * (1 - std::exp(-distance_ * scalar)));
  }

 private:
  enum class State { Idle, Pull, Recede };
  static constexpr double kNaturalFrequency = 24.657, kDampingRatio = 0.98, kAbsorbVelocityAdjustment = 13;
  static constexpr double kVelocityThreshold = 0.01, kValueThreshold = 0.001;
  static constexpr double kLinearStretch = 0.016, kExpStretch = 0.016, kExpStretchDistance = 0.33;
  State state_ = State::Idle;
  float distance_ = 0;
  double velocity_ = 0, started_ = 0;
};

}  // namespace oxyos
