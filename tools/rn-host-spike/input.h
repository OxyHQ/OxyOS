// Pointer input for the host's window: compositor pointer events become React Native touch
// events on the view under the pointer, as Android's touch handling delivers them, so React
// Native's responder system and Pressability work unchanged. Scrolls move the nearest scroll
// view that can scroll, which owns its offset as a native scroll view does.
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <react/renderer/core/ReactPrimitives.h>

#include <chrono>
#include <functional>
#include <map>
#include <optional>
#include <unordered_map>
#include <vector>

namespace oxyos {

// What a protocol surface shows: the window shows the root (popup subtrees and shell regions
// excluded); a popup shows its popup view's children, a shell surface its region's (`popup` for
// both). `origin` is where the surface's own origin sits in root coordinates, the space React
// Native measures in.
struct SurfaceContent {
  uint32_t surface = 0;
  facebook::react::Tag view = -1;
  bool popup = false;
  facebook::react::Point origin{};
};

// The deepest view under a surface-local logical point that takes touches, honouring
// `pointerEvents`, clipping and scroll offsets as they are drawn. -1 if none does.
facebook::react::Tag hitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y);
// The deepest view under the point for react-native-gesture-handler, as its Android orchestrator
// traverses: as hitTest, but a view all but transparent (opacity under 0.1) is passed over, and a
// view `reaches` counts is under the point though outside its bounds (a handler's hit slop).
facebook::react::Tag gestureHitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y,
                                    const std::function<bool(facebook::react::Tag)>& reaches);

class PointerInput {
 public:
  using Surfaces = std::function<std::optional<SurfaceContent>(uint32_t surface)>;
  PointerInput(const ViewTree& tree, Surfaces surfaces) : tree_(tree), surfaces_(std::move(surfaces)) {}

  // Handles a compositor pointer or scroll event. The primary button starts, moves and ends one
  // touch; a cancel or the pointer leaving cancels it.
  void handle(const OxyOSEvent& event);
  // Scrolls a text input's text by (dx, dy) if it can still move that way (textinput.h); true
  // if it moved. A text input under the pointer takes the wheel before the scroll views around it.
  void scrollsText(std::function<bool(facebook::react::Tag, float dx, float dy)> scrollText) {
    scrollText_ = std::move(scrollText);
  }
  // The touch's start, moves, end and cancel (0 to 3) also go to gesture handlers
  // (gesture-handler.h), after React Native, as Android's root view passes them on.
  void feedsGestures(std::function<void(int phase, const SurfaceContent& content, float x, float y)> feed) {
    gestures_ = std::move(feed);
  }
  // The pointer's shape over a surface changed (OXYOS_POINTER_*): the compositor is to be told.
  void showsShapes(std::function<void(uint32_t surface, uint8_t shape)> tell) { tellShape_ = std::move(tell); }
  // A touchpad pinch's two fingers also go to gesture handlers: `action` an OXYOS_GESTURE_*
  // action, `index` the finger it is about, `fingers` where each finger is.
  void feedsFingers(std::function<void(uint8_t action, size_t index, const SurfaceContent& content,
                                       const std::vector<facebook::react::Point>& fingers)>
                        feed) {
    fingers_ = std::move(feed);
  }
  // The window's scale: a pinch's fingers start a set number of its pixels apart.
  void setScale(float scale) { scale_ = scale; }
  // A gesture handler activated and cancels React Native's touch: it is sent touchcancel and
  // nothing more of this touch, as Android's JSTouchDispatcher does when a native gesture starts.
  void cancelResponder();

 private:
  void scroll(float dx, float dy);
  // Moves the hover to the views under (x, y) of `content` (none for nullopt): pointer leave
  // events for the views left, innermost first, then enter events for the views entered,
  // outermost first, as W3C pointer events do. Pressability's hover is built on them.
  void hover(const std::optional<SurfaceContent>& content, float x, float y);

  enum class Phase { Start, Move, End, Cancel };
  void dispatch(Phase phase, float x, float y);
  struct Finger {
    int identifier;
    float x, y;  // surface-local logical
  };
  // Sends React Native a touch event for `fingers`: `changed` is the one that started, ended or
  // was cancelled, or all of them (nullopt, and always for a move). A finger that ended or was
  // cancelled is no longer among the touches.
  void dispatchTouches(Phase phase, const std::vector<Finger>& fingers, std::optional<size_t> changed);
  // A touchpad pinch, as Android 14's touchpad stack gives it to applications
  // (inputflinger's GestureConverter): two fingers around the pointer, starting
  // INITIAL_PINCH_SEPARATION_PX (200 of the display's pixels) apart side by side, finger 0 down
  // then finger 1, both moving as the spread changes, then finger 1 up and finger 0 up. Android's
  // fingers never turn; here they turn as libinput reports the user's fingers turning, so rotation
  // handlers see it. A pinch libinput cancels cancels both fingers.
  void pinch(const SurfaceContent& content, const OxyOSEvent& event);
  std::vector<Finger> pinchFingers(float spread, float degrees) const;

  const ViewTree& tree_;
  Surfaces surfaces_;
  std::function<bool(facebook::react::Tag, float, float)> scrollText_;
  std::function<void(int, const SurfaceContent&, float, float)> gestures_;
  std::function<void(uint8_t, size_t, const SurfaceContent&, const std::vector<facebook::react::Point>&)> fingers_;
  float scale_ = 1;
  // The pinch going on: where it is, and its spread and turn as last told.
  bool pinching_ = false;
  facebook::react::Point pinchCentre_{};
  float pinchSpread_ = 1, pinchDegrees_ = 0;
  bool responderCancelled_ = false;  // React Native has let go of the current touch
  std::function<void(uint32_t, uint8_t)> tellShape_;
  std::map<uint32_t, uint8_t> toldShapes_;  // the shape each surface last asked for
  SurfaceContent content_{};  // the surface the pointer is over
  std::vector<facebook::react::Tag> hovered_;  // views under the pointer, root first
  uint32_t hoveredSurface_ = 0;
  bool down_ = false;
  facebook::react::Tag target_ = -1;
  int identifier_ = 0;
  float x_ = 0, y_ = 0;  // last pointer position, surface-local logical
  // Offsets this host gave each scroll view; the state commit that carries one may still be
  // pending when the next scroll arrives.
  std::unordered_map<facebook::react::Tag, facebook::react::Point> offsets_;
  std::chrono::steady_clock::time_point epoch_ = std::chrono::steady_clock::now();
};

}  // namespace oxyos
