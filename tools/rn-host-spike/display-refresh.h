// The refresh of the display the window is on, as the compositor tells it (the protocol's REFRESH,
// docs/adr/0018-several-displays.mdx): what everything the host paces by the display is paced by,
// as Android's Choreographer is by its display's vsync. requestAnimationFrame (animation-frames.h),
// Animated's native driver (native-animated.h), worklets' and Reanimated's frames, and how long a
// resized window waits for JavaScript (bloom-host.cpp) each take one frame of it: 16.7 ms at 60 Hz,
// 8.3 at 120, 6.9 at 144. The compositor tells it before the window's first configure, and again
// when the window moves to another display or its display is driven at another rate.
#pragma once

#include <chrono>
#include <cstdint>

namespace oxyos {

class DisplayRefresh {
 public:
  using Clock = std::chrono::steady_clock;

  // In millihertz (119998 is 120 Hz); 0 is ignored.
  void set(uint32_t mhz) {
    if (mhz != 0) mhz_ = mhz;
  }
  uint32_t mhz() const { return mhz_; }
  // The time between the display's frames.
  Clock::duration period() const {
    return std::chrono::duration_cast<Clock::duration>(std::chrono::duration<double>(1000.0 / mhz_));
  }
  double periodMs() const { return 1e6 / mhz_; }

 private:
  // Until the compositor tells it, and for a render to a file (which has no display), 60 Hz: no
  // frame reaches a display before it is told.
  uint32_t mhz_ = 60000;
};

}  // namespace oxyos
