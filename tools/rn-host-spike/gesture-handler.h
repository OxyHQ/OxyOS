// react-native-gesture-handler's native side, after its Android module: `RNGestureHandlerModule`
// for JavaScript, the pointer stream fed to the recognizers (crates/oxyos-gestures, Android's
// orchestrator and handlers ported), and what they recognize delivered as Android delivers it:
// device events for handlers with JavaScript callbacks, Reanimated events for worklet ones.
// What OxyOS does not do (pinch, rotation, hover, Animated's native driver, the newer API's native
// detectors, mouse buttons but the primary) is reported once, never pretended.
#pragma once

#include "input.h"
#include "renderer.h"

#include <oxyos_client.h>

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <folly/dynamic.h>
#include <jsi/jsi.h>

#include <chrono>
#include <optional>

#include <functional>
#include <map>
#include <memory>
#include <set>
#include <string>

namespace oxyos {

class GestureHandlerHost {
 public:
  struct Delivery {
    // To RCTDeviceEventEmitter on the JavaScript thread.
    std::function<void(const std::string& name, folly::dynamic payload)> deviceEvent;
    // To Reanimated's event handlers for `view` (worklets), as RNGH's ReanimatedProxy does.
    std::function<void(const std::string& name, int view, const folly::dynamic& payload)> reanimatedEvent;
  };

  GestureHandlerHost(const ViewTree& tree, Delivery delivery);
  ~GestureHandlerHost();
  GestureHandlerHost(const GestureHandlerHost&) = delete;
  GestureHandlerHost& operator=(const GestureHandlerHost&) = delete;

  std::shared_ptr<facebook::react::TurboModule> module(facebook::jsi::Runtime& runtime,
                                                       const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // The pointer stream, from PointerInput: the primary button's press, motion while pressed,
  // release and cancel, at (x, y) of `content`'s surface.
  enum class Phase { Down, Move, Up, Cancel };
  void pointer(Phase phase, const SurfaceContent& content, float x, float y);
  // A touchpad pinch's fingers, from PointerInput: an OXYOS_GESTURE_* action about finger `index`,
  // each finger at its point of `content`'s surface. They are fingers (RNGH's pointer type 0), as
  // Android's touchpad pinch reports its own.
  void fingers(uint8_t action, size_t index, const SurfaceContent& content,
               const std::vector<facebook::react::Point>& points);
  // Timers due fire. Once per loop; true if anything was delivered.
  bool tick();
  // When a handler's timer next falls due (a long press, a tap's wait), if one is set.
  std::optional<std::chrono::steady_clock::time_point> nextDeadline() const;

  // A handler activated: React Native's touch is to be cancelled (cancelsJSResponder), and the
  // host's own touch handling (a text input's drag selection) is to let go.
  std::function<void()> onCancelResponder;
  std::function<void()> onIntercept;

 private:
  bool deliver();
  // A touch event (onTouchesDown and the rest), for a handler that needs pointer data.
  void deliverTouches(const OxyOSGestureOutput& out, const std::string& handler);
  void setState(int tag, int state);
  void configure(facebook::jsi::Runtime& rt, int tag, const facebook::jsi::Value& config, bool create,
                 const std::string& name, bool reset);
  bool reaches(facebook::react::Tag view, float x, float y) const;
  // The frame of a view in the root, for the recognizers (OxyOSGestureFrameFn).
  static uint8_t frameOf(void* context, int32_t view, OxyOSGestureFrame* out);
  static uint64_t now();

  const ViewTree& tree_;
  Delivery delivery_;
  OxyOSGestures* gestures_;
  std::map<int, int> actionTypes_;           // by handler tag, as attached
  std::map<int, std::string> names_;          // by handler tag, as created
  std::map<int, facebook::react::Tag> views_;  // by handler tag, its view
};

}  // namespace oxyos
