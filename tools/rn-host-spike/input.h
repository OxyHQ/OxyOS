// Pointer input for the host's window: compositor pointer events become React Native touch
// events on the view under the pointer, as Android's touch handling delivers them, so React
// Native's responder system and Pressability work unchanged. Scrolls move the nearest scroll
// view that can scroll, which owns its offset as a native scroll view does: a wheel's notches at
// once, a touchpad's fingers as Android 14 gives them to a ScrollView (a drag, flung when they
// lift).
#pragma once

#include "display-refresh.h"
#include "renderer.h"
#include "scroll-physics.h"

#include <oxyos_client.h>

#include <react/renderer/core/ReactPrimitives.h>
#include <react/timing/primitives.h>

#include <algorithm>
#include <chrono>
#include <functional>
#include <map>
#include <optional>
#include <unordered_map>
#include <utility>
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
  // touch; a cancel or the pointer leaving cancels it. Every mouse button's press and release is
  // also a W3C pointer event (pointerdown, pointerup) to the view under the pointer, bubbling, with
  // the button, the buttons held and the modifiers, as Android's JSPointerDispatcher sends a
  // mouse's: a secondary press reaches nothing else (no touch), so it opens a context menu and
  // presses nothing. Key events only update the modifiers held.
  void handle(const OxyOSEvent& event);
  // A compositor of version 8 or newer tells each pointer event the keyboard's modifiers held,
  // whichever surface has the keyboard (docs/adr/0034-the-dock.mdx): a Control-click on a surface
  // that does not have it. Before, they are only what this client's own keys said.
  void takesPointerModifiers(bool told) { pointerModifiers_ = told; }
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
  // Whether the pointer over `view` of `surface` reaches it. While a popup holds the compositor's
  // grab the pointer's movement over the surface the popup hangs from comes too (a menu bar goes on
  // tracking it), but only the views the popup tracks take it (popup.h, `PopupHost::tracks`); over
  // any other the pointer is over none.
  void admitsHover(std::function<bool(uint32_t surface, facebook::react::Tag view)> admits) {
    admits_ = std::move(admits);
  }
  // A touchpad pinch's two fingers also go to gesture handlers: `action` an OXYOS_GESTURE_*
  // action, `index` the finger it is about, `fingers` where each finger is.
  void feedsFingers(std::function<void(uint8_t action, size_t index, const SurfaceContent& content,
                                       const std::vector<facebook::react::Point>& fingers)>
                        feed) {
    fingers_ = std::move(feed);
  }
  // When the device registered the event being handled, in milliseconds of the monotonic clock
  // (the protocol's time_ns; when it carries none, when the host got it): what gesture handlers
  // measure velocities by, as Android's VelocityTracker reads a MotionEvent's eventTime.
  uint64_t eventTimeMs() const {
    return static_cast<uint64_t>(std::max(0.0, eventTime_.toDOMHighResTimeStamp()));
  }
  // The window's scale: a pinch's fingers start a set number of its pixels apart.
  void setScale(float scale) { scale_ = scale; }
  // A gesture handler activated and cancels React Native's touch: it is sent touchcancel and
  // nothing more of this touch, as Android's JSTouchDispatcher does when a native gesture starts.
  void cancelResponder();
  // A scroll view's offset and the furthest it goes (its content past its viewport), as the
  // wheel last left it; nullopt for a view that is not a scroll view that scrolls.
  std::optional<std::pair<facebook::react::Point, facebook::react::Point>> scrollRange(facebook::react::Tag tag) const;
  // Scrolls the scroll view `tag` (on `surface`) by (dx, dy), as far as it goes, and sends its
  // scroll event, as the wheel does; true if it moved.
  bool scrollView(facebook::react::Tag tag, float dx, float dy, uint32_t surface);

  // A scroll view flung by a touchpad's fingers glides once a frame of `refresh` (the window's
  // display, display-refresh.h), as Android's OverScroller is stepped on each vsync: `tick`, once
  // a turn of the loop, steps it when a frame is due and the window can take one (`ready`); true
  // if it moved. `nextFrame` is when the next step is due, while one glides.
  void pacedBy(const DisplayRefresh& refresh) { refresh_ = &refresh; }
  bool tick(bool ready);
  std::optional<std::chrono::steady_clock::time_point> nextFrame(bool ready) const;

  // The scroll views stretched past their ends (overscroll): each one's stretch along x and y, a
  // share of its size, negative past its start, positive past its end (ViewTree::setStretches).
  std::map<facebook::react::Tag, facebook::react::Point> stretches() const;
  // Whether a stretch changed since this was last asked: the window draws again.
  bool takeStretchChanged() { return std::exchange(stretchChanged_, false); }

 private:
  void scroll(float dx, float dy);
  // A touchpad's scroll (OXYOS_SCROLL_FINGER), as Android 14 gives two fingers' swipe to a
  // ScrollView (inputflinger's GestureConverter: a finger dragged): its first event begins a drag
  // of the innermost scroll view that can move its way (onScrollBeginDrag), which every event
  // then moves; its stop is the finger lifted: onScrollEndDrag with the velocity it left with,
  // then the momentum events around the fling it starts, if it is fast enough.
  void fingerScroll(float dx, float dy, bool stop);
  // Ends the glide going on, if any, where it is (onMomentumScrollEnd): a press, a wheel or new
  // fingers stop it, as a touch stops Android's.
  void stopGlide();
  // Overscroll, as Android 12's ScrollView stretches (EdgeEffect, TYPE_STRETCH): the axis a scroll
  // view scrolls along (0 for x, 1 for y), whether it may stretch along it (its overScrollMode:
  // always, never, or only when its content scrolls, React Native's "auto"), the stretch showing
  // taking back a move the other way (`ScrollView.releaseGlow`; returns what is left of the move),
  // and a move from `before` that went past an end pulling it (`onPullDistance`).
  int scrollAxis(facebook::react::Tag tag) const;
  bool canOverscroll(facebook::react::Tag tag, int axis) const;
  float releaseStretch(facebook::react::Tag tag, int axis, float delta);
  void pullStretch(facebook::react::Tag tag, int axis, float delta, float before, float end);
  void reportStretch(facebook::react::Tag tag);
  // The scroll view's scroll event of `kind` (scrollBeginDrag, scrollEndDrag, momentumScrollBegin,
  // momentumScrollEnd) at its offset; `velocity` for the end of a drag.
  void scrollEvent(facebook::react::Tag tag, uint32_t surface, const char* kind,
                   std::optional<facebook::react::Point> velocity = std::nullopt);
  // The pointer over (x, y) of `content` (none for nullopt: it left): a W3C pointermove to the view
  // under it, or a pointerleave once it left, from which React Native's pointer events processor
  // makes pointerover, pointerout, pointerenter and pointerleave for the views that listen, as
  // it does for iOS's Fabric. Pressability's hover is built on them. The pointer's shape follows
  // the views under it.
  void hover(const std::optional<SurfaceContent>& content, float x, float y);

  // A mouse button's W3C pointer event (pointerdown or pointerup) to the view under (x, y).
  void pointerButton(const SurfaceContent& content, uint32_t code, bool down, float x, float y);

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
  std::function<bool(uint32_t, facebook::react::Tag)> admits_;
  std::map<uint32_t, uint8_t> toldShapes_;  // the shape each surface last asked for
  SurfaceContent content_{};  // the surface the pointer is over
  std::vector<facebook::react::Tag> hovered_;  // views under the pointer, root first
  uint32_t hoveredSurface_ = 0;
  std::optional<facebook::react::Point> hoveredAt_;  // where the last pointermove was, on the page
  bool down_ = false;
  bool pointerModifiers_ = false;
  // The mouse buttons held (W3C `buttons` bits) and the keyboard's modifiers.
  int buttons_ = 0;
  bool ctrl_ = false, shift_ = false, alt_ = false, meta_ = false;
  facebook::react::Tag target_ = -1;
  int identifier_ = 0;
  float x_ = 0, y_ = 0;  // last pointer position, surface-local logical
  // Offsets this host gave each scroll view; the state commit that carries one may still be
  // pending when the next scroll arrives.
  std::unordered_map<facebook::react::Tag, facebook::react::Point> offsets_;
  // When the event being handled happened (eventTime): what every event it makes carries.
  facebook::react::HighResTimeStamp eventTime_ = facebook::react::HighResTimeStamp::now();
  const DisplayRefresh* refresh_ = nullptr;
  // The touchpad's drag going on: the scroll view it moves (-1 when its scroll went to a text
  // input or no scroll view could move), on which surface, how far it has gone and its samples.
  struct Drag {
    facebook::react::Tag view = -1;
    uint32_t surface = 0;
    facebook::react::Point moved{};
    ScrollVelocity velocity;
  };
  std::optional<Drag> drag_;
  // The glide after it: the scroll view, where it began and when, its fling on each axis, and
  // when it last stepped.
  struct Glide {
    facebook::react::Tag view = -1;
    uint32_t surface = 0;
    facebook::react::Point from{};
    std::chrono::steady_clock::time_point began;
    SplineFling x{0}, y{0};
    std::optional<std::chrono::steady_clock::time_point> stepped;
  };
  std::optional<Glide> glide_;
  // Each scroll view's stretch along each axis, and which end it is past (-1 its start, 1 its end),
  // while one shows; when the receding ones last stepped.
  struct Overscroll {
    EdgeStretch axis[2];
    int side[2] = {0, 0};
    uint32_t surface = 0;
  };
  std::map<facebook::react::Tag, Overscroll> overscroll_;
  std::optional<std::chrono::steady_clock::time_point> stretchStepped_;
  bool stretchChanged_ = false;
};

// When `event` happened, as its device registered it: the protocol's `time_ns`, CLOCK_MONOTONIC,
// which is steady_clock here and so the clock of React Native's timestamps and of JavaScript's
// `performance.now()`. React Native's events carry it as Android's carry a MotionEvent's
// `eventTime`: a W3C pointer event's `timeStamp`, a touch's `timestamp`. An event the compositor
// knows no time for happened now.
facebook::react::HighResTimeStamp eventTime(const OxyOSEvent& event);

}  // namespace oxyos
