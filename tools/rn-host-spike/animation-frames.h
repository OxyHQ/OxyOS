// requestAnimationFrame on OxyOS, run once a frame as Android's Choreographer runs it. Bridgeless
// React Native installs it as a zero-delay timer (TimerManager: "the same as setTimeout(0)"), which
// Android's timer registry still fires on the next Choreographer frame; the host's fires timers as
// soon as they are due, so a JavaScript-driven animation committed as often as the loop turned (a
// 600 ms Animated.timing: about 600 commits, a core busy throughout). The host replaces the two
// globals with its own after React Native installs them:
// - a frame runs the callbacks requested before it began, each with the frame's time (on
//   performance.now()'s clock) and each as its own task, so microtasks run between them; callbacks
//   requested during a frame wait for the next, and cancelAnimationFrame removes a callback until
//   its own turn comes, in the frame running too;
// - a frame begins only when the window's previous frame was answered by the compositor (presented
//   or discarded) and a display frame (1/60 s: the protocol does not tell an output's refresh) has
//   passed since the last one: never faster than the compositor takes frames, nor than the display.
//   Without a window, the display's frames alone.
#pragma once

#include <ReactCommon/RuntimeExecutor.h>
#include <jsi/jsi.h>

#include <chrono>
#include <cstdint>
#include <map>
#include <memory>
#include <optional>

namespace oxyos {

class AnimationFrames {
 public:
  using Clock = std::chrono::steady_clock;

  explicit AnimationFrames(facebook::react::RuntimeExecutor executor);
  // Drops the callbacks still waiting, while the runtime they belong to is alive.
  ~AnimationFrames();

  // Replaces React Native's requestAnimationFrame and cancelAnimationFrame in `runtime`.
  void install(facebook::jsi::Runtime& runtime);

  // Once a turn of the loop: begins a frame if callbacks wait for one and it is due. `ready` is
  // whether the window can take a frame (its previous one answered). True if a frame began.
  bool tick(bool ready);
  // When the next frame is due, if callbacks wait and the window can take it: the loop sleeps until
  // then (the compositor's answer wakes it otherwise).
  std::optional<Clock::time_point> nextFrame(bool ready) const;

 private:
  struct State {
    uint32_t next = 1;
    // Callbacks waiting for the next frame, and those of the frame running, by their handle.
    std::map<uint32_t, std::shared_ptr<facebook::jsi::Function>> waiting, running;
    std::optional<Clock::time_point> last;
  };
  facebook::react::RuntimeExecutor executor_;
  std::shared_ptr<State> state_;
};

}  // namespace oxyos
