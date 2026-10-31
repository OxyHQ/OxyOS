#include "input.h"

#include "overscroll.h"

#include "report.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/scrollview/ScrollViewEventEmitter.h>
#include <react/renderer/components/scrollview/ScrollViewProps.h>
#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/components/view/TouchEventEmitter.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteState.h>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <cmath>
#include <numbers>
#include <optional>
#include <string>

using namespace facebook::react;

namespace oxyos {

namespace {

// Linux input code of the primary (left) button.
constexpr uint32_t kPrimaryButton = 272;

// A Linux button code as W3C's `button` (0 primary, 1 auxiliary, 2 secondary, 3 back, 4 forward)
// and its bit in `buttons`; -1 for one W3C does not name.
int w3cButton(uint32_t code) {
  switch (code) {
    case 272: return 0;  // BTN_LEFT
    case 274: return 1;  // BTN_MIDDLE
    case 273: return 2;  // BTN_RIGHT
    case 275: return 3;  // BTN_SIDE
    case 276: return 4;  // BTN_EXTRA
    default: return -1;
  }
}
int w3cButtonBit(int button) {
  static const int bits[] = {1, 4, 2, 8, 16};
  return button >= 0 && button < 5 ? bits[button] : 0;
}

// The pointer's shape over the views of `path` (root first), as Android resolves a pointer icon:
// from the view under the pointer up, the first that sets one. An editable text input shows the
// text shape, as Android's EditText; a view's `cursor` style its own (Android knows `pointer`,
// the hand; the other CSS cursors React Native parses take the shape that is theirs).
uint8_t pointerShape(const ViewTree& tree, const std::vector<Tag>& path) {
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    const auto& view = tree.views().at(*at).view;
    std::string name = view.componentName ? view.componentName : "";
    if (name == "AndroidTextInput") {
      auto input = std::dynamic_pointer_cast<const AndroidTextInputProps>(view.props);
      if (input && input->editable) return OXYOS_POINTER_TEXT;
    }
    auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
    if (!props) continue;
    switch (props->cursor) {
      case Cursor::Auto:
        continue;
      case Cursor::Pointer:
        return OXYOS_POINTER_HAND;
      case Cursor::Text:
        return OXYOS_POINTER_TEXT;
      case Cursor::NotAllowed:
      case Cursor::NoDrop:
        return OXYOS_POINTER_NOT_ALLOWED;
      case Cursor::Wait:
      case Cursor::Progress:
        return OXYOS_POINTER_WAIT;
      case Cursor::Help:
        return OXYOS_POINTER_HELP;
      case Cursor::Crosshair:
      case Cursor::Cell:
        return OXYOS_POINTER_CROSSHAIR;
      case Cursor::Move:
      case Cursor::AllScroll:
        return OXYOS_POINTER_MOVE;
      case Cursor::Grab:
        return OXYOS_POINTER_GRAB;
      case Cursor::Grabbing:
        return OXYOS_POINTER_GRABBING;
      case Cursor::EResize:
      case Cursor::WResize:
      case Cursor::EWResize:
      case Cursor::ColResize:
        return OXYOS_POINTER_RESIZE_HORIZONTAL;
      case Cursor::NResize:
      case Cursor::SResize:
      case Cursor::NSResize:
      case Cursor::RowResize:
        return OXYOS_POINTER_RESIZE_VERTICAL;
      case Cursor::NWResize:
      case Cursor::SEResize:
      case Cursor::NWSEResize:
        return OXYOS_POINTER_RESIZE_TOP_LEFT;
      case Cursor::NEResize:
      case Cursor::SWResize:
      case Cursor::NESWResize:
        return OXYOS_POINTER_RESIZE_TOP_RIGHT;
      case Cursor::ZoomIn:
        return OXYOS_POINTER_ZOOM_IN;
      case Cursor::ZoomOut:
        return OXYOS_POINTER_ZOOM_OUT;
      default:
        return OXYOS_POINTER_ARROW;
    }
  }
  return OXYOS_POINTER_ARROW;
}

// For gesture handlers' traversal: views all but transparent are passed over, and `reaches`
// counts a view as under the point.
struct GestureHit {
  const std::function<bool(Tag)>& reaches;
};

Tag hit(const ViewTree& tree, Tag tag, float x, float y, const GestureHit* gestures = nullptr) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return -1;
  const auto& view = it->second.view;
  const auto& frame = view.layoutMetrics.frame;
  // Where it is drawn: its transform moves, scales and turns it, and what is in it, as Android maps
  // a touch through a view's inverse matrix. A view scaled to nothing is under no point.
  Affine map = viewTransform(view);
  if (std::abs(map.determinant()) < 1e-6f) return -1;
  Point local = map.inverse().apply(Point{x, y});
  bool inside = local.x >= 0 && local.y >= 0 && local.x < frame.size.width && local.y < frame.size.height;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  auto mode = props ? props->pointerEvents : PointerEventsMode::Auto;
  if (mode == PointerEventsMode::None) return -1;
  // RNGH's root view's minimum alpha for touches.
  if (gestures && props && props->opacity < 0.1f) return -1;
  std::string name = view.componentName ? view.componentName : "";
  // A popup's or a shell region's subtree is in its own surface, not under the window's pointer;
  // a modal's is above the window's content (hitIn).
  if (ownSurface(name) || isModal(name)) return -1;
  // react-native-screens' header configurations describe a header the platform draws from them
  // (Android's toolbar, never the configuration's own view), and are laid out over their whole
  // screen (`absoluteFill`): no view of their own, they are under no point. OxyOS draws no native
  // stack header, so a screen showing one shows none (expo-router's stacks with `headerShown:
  // false` show none anyway).
  if (name == "RNSStackHeaderConfigAndroid" || name == "RNSScreenStackHeaderConfig") return -1;
  bool clips = name == "ScrollView" || (props && props->getClipsContentToBounds());
  if (clips && !inside) return -1;
  if (mode != PointerEventsMode::BoxOnly) {
    float cx = local.x, cy = local.y;
    if (name == "ScrollView" && view.state) {
      const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
      cx += state.contentOffset.x;
      cy += state.contentOffset.y;
    }
    // Later children are drawn above earlier ones, so they are hit first.
    const auto& children = it->second.children;
    for (auto child = children.rbegin(); child != children.rend(); ++child) {
      Tag found = hit(tree, *child, cx, cy, gestures);
      if (found >= 0) return found;
    }
  }
  bool reached = inside || (gestures && gestures->reaches(tag));
  return reached && mode != PointerEventsMode::BoxNone ? tag : -1;
}

Tag hitIn(const ViewTree& tree, const SurfaceContent& content, float x, float y, const GestureHit* gestures) {
  if (!content.popup && content.view == tree.root()) {
    // A modal shown takes the window's input, as its own window above the activity's does: its
    // content is at the window's origin, and nothing under it is reached.
    auto shown = modals(tree);
    if (!shown.empty()) {
      const auto& children = tree.views().at(shown.back()).children;
      for (auto child = children.rbegin(); child != children.rend(); ++child) {
        Tag found = hit(tree, *child, x, y, gestures);
        if (found >= 0) return found;
      }
      return -1;
    }
  }
  if (!content.popup) return content.view < 0 ? -1 : hit(tree, content.view, x, y, gestures);
  // The view's children are laid out relative to it: a popup's is its surface's origin, a shell
  // region is where it is in its surface (the surface's origin is `content.origin`).
  auto it = tree.views().find(content.view);
  if (it == tree.views().end()) return -1;
  auto frame = windowFrame(tree, content.view).value_or(Rect{content.origin, {}});
  float lx = x + content.origin.x - frame.origin.x, ly = y + content.origin.y - frame.origin.y;
  const auto& children = it->second.children;
  for (auto child = children.rbegin(); child != children.rend(); ++child) {
    Tag found = hit(tree, *child, lx, ly, gestures);
    if (found >= 0) return found;
  }
  return -1;
}

}  // namespace

Tag hitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y) {
  return hitIn(tree, content, x, y, nullptr);
}

Tag gestureHitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y,
                   const std::function<bool(Tag)>& reaches) {
  GestureHit gestures{reaches};
  return hitIn(tree, content, x, y, &gestures);
}

void PointerInput::cancelResponder() {
  if ((!down_ && !pinching_) || responderCancelled_) return;
  if (pinching_) {
    dispatchTouches(Phase::Cancel, pinchFingers(pinchSpread_, pinchDegrees_), std::nullopt);
  } else {
    dispatch(Phase::Cancel, x_, y_);
  }
  responderCancelled_ = true;
}

std::vector<PointerInput::Finger> PointerInput::pinchFingers(float spread, float degrees) const {
  constexpr float kInitialSeparation = 200;  // GestureConverter's INITIAL_PINCH_SEPARATION_PX
  float half = kInitialSeparation / scale_ * spread / 2;
  float radians = degrees * std::numbers::pi_v<float> / 180;
  float dx = half * std::cos(radians), dy = half * std::sin(radians);
  return {Finger{identifier_ - 1, pinchCentre_.x - dx, pinchCentre_.y - dy},
          Finger{identifier_, pinchCentre_.x + dx, pinchCentre_.y + dy}};
}

void PointerInput::pinch(const SurfaceContent& content, const OxyOSEvent& event) {
  auto points = [](const std::vector<Finger>& fingers) {
    std::vector<Point> out;
    for (const auto& finger : fingers) out.push_back(Point{finger.x, finger.y});
    return out;
  };
  if (event.detail == OXYOS_PINCH_BEGIN) {
    // Not while the button holds a touch.
    if (down_ || pinching_) return;
    content_ = content;
    pinchCentre_ = Point{event.x / 256.0f, event.y / 256.0f};
    pinchSpread_ = 1;
    pinchDegrees_ = 0;
    identifier_ += 2;
    auto fingers = pinchFingers(1, 0);
    // As a touch, the gesture's target is under the first finger down.
    target_ = hitTest(tree_, content_, fingers[0].x, fingers[0].y);
    if (target_ < 0) return;
    pinching_ = true;
    responderCancelled_ = false;
    dispatchTouches(Phase::Start, {fingers[0]}, 0);
    if (fingers_) fingers_(OXYOS_GESTURE_DOWN, 0, content_, points({fingers[0]}));
    if (!responderCancelled_) dispatchTouches(Phase::Start, fingers, 1);
    if (fingers_) fingers_(OXYOS_GESTURE_POINTER_DOWN, 1, content_, points(fingers));
    return;
  }
  if (!pinching_) return;
  pinchSpread_ = event.scale_65536 / 65536.0f;
  pinchDegrees_ = event.rotation_256 / 256.0f;
  auto fingers = pinchFingers(pinchSpread_, pinchDegrees_);
  switch (event.detail) {
    case OXYOS_PINCH_UPDATE:
      if (!responderCancelled_) dispatchTouches(Phase::Move, fingers, std::nullopt);
      if (fingers_) fingers_(OXYOS_GESTURE_MOVE, 0, content_, points(fingers));
      break;
    case OXYOS_PINCH_END:
      if (!responderCancelled_) dispatchTouches(Phase::End, fingers, 1);
      if (fingers_) fingers_(OXYOS_GESTURE_POINTER_UP, 1, content_, points(fingers));
      if (!responderCancelled_) dispatchTouches(Phase::End, {fingers[0]}, 0);
      if (fingers_) fingers_(OXYOS_GESTURE_UP, 0, content_, points({fingers[0]}));
      pinching_ = false;
      break;
    case OXYOS_PINCH_CANCEL:
      if (!responderCancelled_) dispatchTouches(Phase::Cancel, fingers, std::nullopt);
      if (fingers_) fingers_(OXYOS_GESTURE_CANCEL, 0, content_, points(fingers));
      pinching_ = false;
      break;
    default:
      break;
  }
}

HighResTimeStamp eventTime(const OxyOSEvent& event) {
  if (event.time_ns == 0) return HighResTimeStamp::now();
  return HighResTimeStamp::fromChronoSteadyClockTimePoint(
      std::chrono::steady_clock::time_point(std::chrono::nanoseconds(event.time_ns)));
}

void PointerInput::handle(const OxyOSEvent& event) {
  eventTime_ = eventTime(event);
  if (event.kind == OXYOS_EVENT_KEY) {
    // Linux codes of the modifiers, left and right.
    bool down = event.flag != 0;
    switch (event.code) {
      case 29: case 97: ctrl_ = down; break;
      case 42: case 54: shift_ = down; break;
      case 56: case 100: alt_ = down; break;
      case 125: case 126: meta_ = down; break;
      default: break;
    }
    return;
  }
  // Protocol positions and distances are in 1/256 logical pixels.
  // A touchpad's fingers coming to rest stop the glide their last flick left on that surface, as
  // macOS stops a scroll's momentum under resting fingers: its momentum ends where it is, once,
  // with no scroll after. Resting fingers are no drag: no scrollBeginDrag (they may lift unmoved);
  // moving on, they are a scroll, which begins one. A stretch springing back on that surface
  // (overscroll) is caught where it is, as a finger landing on Android's catches it, and let go
  // again when they lift or move on (a scroll that follows catches it again as its drag begins).
  if (event.kind == OXYOS_EVENT_HOLD) {
    if (event.detail == OXYOS_PINCH_BEGIN && glide_ && glide_->surface == event.surface) stopGlide();
    double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now().time_since_epoch()).count();
    for (auto& [tag, overscroll] : overscroll_) {
      if (overscroll.surface != event.surface) continue;
      for (auto& edge : overscroll.axis) {
        if (event.detail == OXYOS_PINCH_BEGIN) {
          edge.pull(0);
        } else if (!drag_) {
          edge.release(ms);
        }
      }
    }
    return;
  }
  if (event.kind != OXYOS_EVENT_SCROLL && event.kind != OXYOS_EVENT_POINTER && event.kind != OXYOS_EVENT_PINCH) return;
  if (event.kind == OXYOS_EVENT_POINTER && pointerModifiers_) {
    ctrl_ = (event.modifiers & OXYOS_MODIFIER_CONTROL) != 0;
    shift_ = (event.modifiers & OXYOS_MODIFIER_SHIFT) != 0;
    alt_ = (event.modifiers & OXYOS_MODIFIER_ALT) != 0;
    meta_ = (event.modifiers & OXYOS_MODIFIER_SUPER) != 0;
  }
  auto content = surfaces_(event.surface);
  if (!content) return;
  if (event.kind == OXYOS_EVENT_PINCH) return pinch(*content, event);
  if (event.kind == OXYOS_EVENT_SCROLL) {
    // A touchpad's drag stays with the surface it began on (the compositor sends it there).
    if (!drag_) content_ = *content;
    if (event.detail == OXYOS_SCROLL_FINGER) return fingerScroll(event.x / 256.0f, event.y / 256.0f, event.flag);
    if (event.flag) return;  // a continuous device's stop moves nothing
    stopGlide();
    return scroll(event.x / 256.0f, event.y / 256.0f);
  }
  float x = event.x / 256.0f, y = event.y / 256.0f;
  // A touch stays with the surface it started on.
  if (!down_) content_ = *content;
  x_ = x;
  y_ = y;
  if (event.detail == OXYOS_POINTER_ENTER || event.detail == OXYOS_POINTER_MOTION) hover(*content, x, y);
  if (event.detail == OXYOS_POINTER_LEAVE && event.surface == hoveredSurface_) hover(std::nullopt, x, y);
  if (event.detail == OXYOS_POINTER_PRESS || event.detail == OXYOS_POINTER_RELEASE) {
    pointerButton(*content, event.code, event.detail == OXYOS_POINTER_PRESS, x, y);
  }
  switch (event.detail) {
    case OXYOS_POINTER_PRESS:
      stopGlide();
      if (event.code == kPrimaryButton && !down_ && !pinching_) {
        target_ = hitTest(tree_, content_, x, y);
        if (target_ < 0) return;
        down_ = true;
        responderCancelled_ = false;
        ++identifier_;
        dispatch(Phase::Start, x, y);
        if (gestures_) gestures_(0, content_, x, y);
      }
      break;
    case OXYOS_POINTER_MOTION:
      if (down_) {
        if (!responderCancelled_) dispatch(Phase::Move, x, y);
        if (gestures_) gestures_(1, content_, x, y);
      }
      break;
    case OXYOS_POINTER_RELEASE:
      if (event.code == kPrimaryButton && down_) {
        if (!responderCancelled_) dispatch(Phase::End, x, y);
        if (gestures_) gestures_(2, content_, x, y);
        down_ = false;
      }
      break;
    case OXYOS_POINTER_LEAVE:
    case OXYOS_POINTER_CANCEL:
      // The buttons' releases go elsewhere now: none is held here.
      buttons_ = 0;
      if (down_) {
        if (!responderCancelled_) dispatch(Phase::Cancel, x, y);
        if (gestures_) gestures_(3, content_, x, y);
        down_ = false;
      }
      break;
    default:
      break;
  }
}

void PointerInput::hover(const std::optional<SurfaceContent>& content, float x, float y) {
  std::vector<Tag> now;
  if (content) {
    Tag target = hitTest(tree_, *content, x, y);
    if (target >= 0 && admits_ && !admits_(content->surface, target)) target = -1;
    if (target >= 0) now = ancestry(tree_, target);
  }
  uint32_t surface = content ? content->surface : hoveredSurface_;
  if (content && tellShape_) {
    uint8_t shape = pointerShape(tree_, now);
    auto told = toldShapes_.find(surface);
    if (told == toldShapes_.end() ? shape != OXYOS_POINTER_ARROW : told->second != shape) {
      toldShapes_[surface] = shape;
      tellShape_(surface, shape);
    }
  }
  Point page = content ? Point{content->origin.x + x, content->origin.y + y} : Point{x, y};
  // React Native's own pointer events processor (UIManagerBinding, PointerEventsProcessor) turns
  // a C++ platform's pointer events into W3C's: from the target of each one, as the pointer
  // moves, it sends pointerover and pointerout, pointerleave (innermost first) and pointerenter
  // (outermost first), each only where a view listens for it or under one that captures it, and
  // drops a pointermove no view on its path listens for. What the platform tells it is what iOS's
  // Fabric tells it of a mouse: a pointermove to the view under the pointer as it moves, and a
  // pointerleave when it leaves the window. Telling it more (an enter or a leave for each view the
  // pointer crossed) made it track the pointer as on each of those views in turn, so a listener
  // heard every crossing twice, and every view on the path cost JavaScript an event.
  auto send = [&](Tag tag, bool leaving) {
    auto it = tree_.views().find(tag);
    if (it == tree_.views().end()) return;
    auto emitter = std::dynamic_pointer_cast<const TouchEventEmitter>(it->second.view.eventEmitter);
    if (!emitter) return;
    Point origin = windowFrame(tree_, tag).value_or(Rect{}).origin;
    PointerEvent event{};
    event.pointerId = 1;
    event.pointerType = "mouse";
    event.isPrimary = true;
    event.clientPoint = page;
    event.screenPoint = page;
    event.offsetPoint = {page.x - origin.x, page.y - origin.y};
    event.width = 1;
    event.height = 1;
    event.button = -1;  // no button changed (W3C)
    event.buttons = buttons_;
    event.timeStamp = eventTime_;
    event.ctrlKey = ctrl_;
    event.shiftKey = shift_;
    event.altKey = alt_;
    event.metaKey = meta_;
    if (leaving) {
      emitter->onPointerLeave(event);
      Report::get().deliver("pointerLeave", surface, it->second.view);
    } else {
      emitter->onPointerMove(event);
    }
  };
  if (!now.empty()) {
    // A motion of less than a tenth of a pixel over the same view is none (Android's
    // ONMOVE_EPSILON).
    bool moved = !hoveredAt_ || hovered_.empty() || hovered_.back() != now.back() ||
                 std::abs(hoveredAt_->x - page.x) > 0.1f || std::abs(hoveredAt_->y - page.y) > 0.1f;
    if (moved) send(now.back(), false);
  } else if (!hovered_.empty()) {
    // Out of the window (or over none of its views): the pointer left the root.
    send(hovered_.back(), true);
  }
  hoveredAt_ = now.empty() ? std::nullopt : std::optional<Point>(page);
  hovered_ = std::move(now);
  hoveredSurface_ = surface;
}

void PointerInput::pointerButton(const SurfaceContent& content, uint32_t code, bool down, float x, float y) {
  int button = w3cButton(code);
  if (button < 0) return;
  int bit = w3cButtonBit(button);
  // A release of a button not held (pressed before the window was entered) is no one's.
  if (!down && !(buttons_ & bit)) return;
  buttons_ = down ? buttons_ | bit : buttons_ & ~bit;
  Tag target = hitTest(tree_, content, x, y);
  if (target < 0) return;
  auto it = tree_.views().find(target);
  if (it == tree_.views().end()) return;
  auto emitter = std::dynamic_pointer_cast<const TouchEventEmitter>(it->second.view.eventEmitter);
  if (!emitter) return;
  Point page{content.origin.x + x, content.origin.y + y};
  Point origin = windowFrame(tree_, target).value_or(Rect{}).origin;
  PointerEvent event{};
  event.pointerId = 1;
  event.pointerType = "mouse";
  event.isPrimary = true;
  event.clientPoint = page;
  event.screenPoint = page;
  event.offsetPoint = {page.x - origin.x, page.y - origin.y};
  event.width = 1;
  event.height = 1;
  event.pressure = down && button == 0 ? 0.5 : 0;
  event.button = button;
  event.buttons = buttons_;
  event.timeStamp = eventTime_;
  event.ctrlKey = ctrl_;
  event.shiftKey = shift_;
  event.altKey = alt_;
  event.metaKey = meta_;
  if (Report::get().enabled()) {
    std::string within;
    auto path = ancestry(tree_, target);
    for (auto at = path.rbegin(); at != path.rend() && within.empty(); ++at) {
      auto props = std::dynamic_pointer_cast<const ViewProps>(tree_.views().at(*at).view.props);
      if (props && !props->testId.empty()) within = props->testId;
    }
    Report::get().deliver(down ? "pointerDown" : "pointerUp", content.surface, it->second.view,
                          folly::dynamic::object("x", page.x)("y", page.y)("within", within)("button", button));
  }
  if (down) {
    emitter->onPointerDown(event);
  } else {
    emitter->onPointerUp(event);
  }
}

void PointerInput::scroll(float dx, float dy) {
  Tag target = hitTest(tree_, content_, x_, y_);
  if (target < 0) return;
  auto path = ancestry(tree_, target);
  // The innermost scroll view, or text input, that can still move in the scroll's direction
  // takes it, as nested scrolling views do on Android.
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    const auto& view = tree_.views().at(*at).view;
    std::string name = view.componentName ? view.componentName : "";
    if (name == "AndroidTextInput" && scrollText_ && scrollText_(*at, dx, dy)) return;
    if (scrollView(*at, dx, dy, content_.surface)) return;
  }
}

namespace {

// Android's ViewConfiguration: the slowest fling (MINIMUM_FLING_VELOCITY) and the fastest
// (MAXIMUM_FLING_VELOCITY), in dp per second.
constexpr float kMinimumFlingVelocity = 50, kMaximumFlingVelocity = 8000;

// The friction a scroll view glides with: one minus its `decelerationRate`. The C++ props'
// default (0.998) is iOS's and stands for none set, where Android's ScrollView keeps its own,
// 0.985 (ViewConfiguration's scroll friction, 0.015); React Native's 'normal' and 'fast' arrive as
// Android's numbers (processDecelerationRate takes Android's branch).
float frictionOf(const ShadowView& view) {
  auto props = std::dynamic_pointer_cast<const ScrollViewProps>(view.props);
  float rate = props ? props->decelerationRate : 0.998f;
  if (rate == 0.998f) rate = 0.985f;
  return std::clamp(1.0f - rate, 0.001f, 1.0f);
}

}  // namespace

void PointerInput::fingerScroll(float dx, float dy, bool stop) {
  if (!drag_) {
    if (stop) return;
    stopGlide();
    drag_ = Drag{};
    drag_->surface = content_.surface;
    // The scroll view this drag moves: the innermost that can move its way, unless a text input
    // under the pointer takes it first (its text scrolls, with no momentum: Android's text view
    // drags its text, which a touchpad's fingers would select instead).
    Tag target = hitTest(tree_, content_, x_, y_);
    for (auto path = ancestry(tree_, target); target >= 0 && !path.empty(); path.pop_back()) {
      Tag at = path.back();
      const auto& view = tree_.views().at(at).view;
      if (std::string(view.componentName ? view.componentName : "") == "AndroidTextInput" && scrollText_) break;
      auto range = scrollRange(at);
      if (!range) continue;
      auto [current, end] = *range;
      bool moves = (dx > 0 && current.x < end.x) || (dx < 0 && current.x > 0) || (dy > 0 && current.y < end.y) ||
                   (dy < 0 && current.y > 0);
      if (!moves) continue;
      drag_->view = at;
      break;
    }
    // None can move its way: the innermost scroll view that scrolls along the move's axis takes
    // the drag all the same, as Android's ScrollView intercepts a drag whichever way it goes once
    // its content scrolls, and may stretch past its end.
    if (drag_->view < 0) {
      int axis = std::abs(dy) >= std::abs(dx) ? 1 : 0;
      for (auto path = ancestry(tree_, target); target >= 0 && !path.empty(); path.pop_back()) {
        Tag at = path.back();
        const auto& view = tree_.views().at(at).view;
        if (std::string(view.componentName ? view.componentName : "") == "AndroidTextInput" && scrollText_) break;
        auto range = scrollRange(at);
        if (!range || scrollAxis(at) != axis || (axis ? range->second.y : range->second.x) <= 0) continue;
        drag_->view = at;
        break;
      }
    }
    if (drag_->view >= 0) {
      // A stretch springing back is caught where it is, as a touch catches Android's.
      if (auto it = overscroll_.find(drag_->view); it != overscroll_.end()) {
        for (auto& edge : it->second.axis) edge.pull(0);
      }
      scrollEvent(drag_->view, drag_->surface, "scrollBeginDrag");
    }
  }
  // When its device registered the event (eventTime): the velocity is the fingers', however late
  // the host handles their events.
  double now = eventTime_.toDOMHighResTimeStamp();
  if (drag_->view < 0) {
    if (stop) {
      drag_.reset();
    } else {
      scroll(dx, dy);
    }
    return;
  }
  Tag view = drag_->view;
  uint32_t surface = drag_->surface;
  if (!stop) {
    drag_->moved.x += dx;
    drag_->moved.y += dy;
    drag_->velocity.add(now, drag_->moved.x, drag_->moved.y);
    // A stretch showing takes back a move the other way first; what is left scrolls, and what goes
    // past an end pulls the stretch there. A move clamped at an end forgets the velocity so far,
    // as Android's ScrollView clears its VelocityTracker when overScrollBy clamps.
    float move[2] = {releaseStretch(view, 0, dx), releaseStretch(view, 1, dy)};
    auto range = scrollRange(view);
    scrollView(view, move[0], move[1], surface);
    if (range) {
      const float before[2] = {range->first.x, range->first.y}, end[2] = {range->second.x, range->second.y};
      for (int axis = 0; axis < 2; ++axis) {
        float to = before[axis] + move[axis];
        if (move[axis] == 0 || (to >= 0 && to <= end[axis])) continue;
        pullStretch(view, axis, move[axis], before[axis], end[axis]);
        if (drag_) drag_->velocity.clear();
      }
    }
    return;
  }
  auto [vx, vy] = drag_->velocity.at(now);
  drag_.reset();
  // The fingers lifting let the stretch go: it springs back (onRelease).
  if (auto it = overscroll_.find(view); it != overscroll_.end()) {
    double released = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now().time_since_epoch()).count();
    for (auto& edge : it->second.axis) edge.release(released);
  }
  if (tree_.views().find(view) == tree_.views().end()) return;
  // Android's ScrollView takes the velocity at most at the fastest fling, and React Native tells
  // JavaScript the tracker's, the finger's: against the offset, in dp per millisecond.
  vx = std::clamp(vx, -kMaximumFlingVelocity, kMaximumFlingVelocity);
  vy = std::clamp(vy, -kMaximumFlingVelocity, kMaximumFlingVelocity);
  scrollEvent(view, surface, "scrollEndDrag", Point{-vx / 1000, -vy / 1000});
  // React Native's ScrollView then always goes through its momentum (handlePostTouchScrolling),
  // flung or not.
  scrollEvent(view, surface, "momentumScrollBegin");
  auto range = scrollRange(view);
  Glide glide;
  glide.view = view;
  glide.surface = surface;
  glide.from = range ? range->first : Point{};
  glide.began = std::chrono::steady_clock::now();
  float friction = frictionOf(tree_.views().at(view).view);
  if (std::abs(vx) >= kMinimumFlingVelocity) glide.x = SplineFling(vx, friction);
  if (std::abs(vy) >= kMinimumFlingVelocity) glide.y = SplineFling(vy, friction);
  glide_ = glide;
  if (std::getenv("OXYOS_HOST_TRACE")) {
    std::cerr << "[input] fling " << view << " at " << vx << "," << vy << " dp/s: " << glide.y.distance() << " in "
              << glide.y.durationMs() << " ms\n";
  }
  // What it was flung with (none too slow to fling: the momentum then ends on the next frame,
  // where it is).
  Report::get().fling(tree_.views().at(view).view, vx, vy, glide.x.distance(), glide.y.distance(),
                      std::max(glide.x.durationMs(), glide.y.durationMs()), friction);
}

bool PointerInput::tick(bool ready) {
  if (!ready) return false;
  auto now = std::chrono::steady_clock::now();
  auto period = refresh_ ? refresh_->period() : std::chrono::milliseconds(16);
  // A stretch springing back steps once a frame, as EdgeEffect's spring is stepped as it is drawn.
  bool receding = false;
  for (const auto& [tag, overscroll] : overscroll_) {
    for (const auto& edge : overscroll.axis) receding |= !edge.resting() && !edge.pulling();
  }
  if (receding && (!stretchStepped_ || now - *stretchStepped_ >= period)) {
    stretchStepped_ = now;
    double ms = std::chrono::duration<double, std::milli>(now.time_since_epoch()).count();
    for (auto it = overscroll_.begin(); it != overscroll_.end();) {
      auto found = tree_.views().find(it->first);
      bool moved = false;
      if (found != tree_.views().end()) {
        Size size = found->second.view.layoutMetrics.frame.size;
        moved |= it->second.axis[0].step(ms, size.width);
        moved |= it->second.axis[1].step(ms, size.height);
      }
      Tag tag = it->first;
      bool rests = found == tree_.views().end() || (it->second.axis[0].resting() && it->second.axis[1].resting());
      if (moved || rests) {
        stretchChanged_ = true;
        if (found != tree_.views().end()) reportStretch(tag);
      }
      it = rests ? overscroll_.erase(it) : std::next(it);
    }
  } else if (!receding) {
    stretchStepped_.reset();
  }
  if (!glide_) return false;
  if (glide_->stepped && now - *glide_->stepped < period) return false;
  glide_->stepped = now;
  auto range = scrollRange(glide_->view);
  if (!range) {
    glide_.reset();
    return false;
  }
  double ms = std::chrono::duration<double, std::milli>(now - glide_->began).count();
  auto [current, end] = *range;
  Point to{static_cast<float>(glide_->from.x + glide_->x.at(ms)), static_cast<float>(glide_->from.y + glide_->y.at(ms))};
  // A glide reaching an end strikes the stretch there with its velocity (onAbsorb), as Android's
  // ScrollView.computeScroll does when the scroller crosses its range.
  const float wanted[2] = {to.x, to.y}, ends[2] = {end.x, end.y};
  const SplineFling* flings[2] = {&glide_->x, &glide_->y};
  for (int axis = 0; axis < 2; ++axis) {
    if (flings[axis]->distance() == 0 || (wanted[axis] >= 0 && wanted[axis] <= ends[axis])) continue;
    if (!canOverscroll(glide_->view, axis)) continue;
    auto& overscroll = overscroll_[glide_->view];
    overscroll.surface = glide_->surface;
    overscroll.side[axis] = wanted[axis] < 0 ? -1 : 1;
    overscroll.axis[axis].absorb(flings[axis]->velocity(ms),
                                 std::chrono::duration<double, std::milli>(now.time_since_epoch()).count());
    stretchChanged_ = true;
    reportStretch(glide_->view);
  }
  to = {std::clamp(to.x, 0.0f, end.x), std::clamp(to.y, 0.0f, end.y)};
  bool moved = scrollView(glide_->view, to.x - current.x, to.y - current.y, glide_->surface);
  // It ends when its time is up, or at the end it was flung towards, which the stretch then takes
  // (as Android 12's ScrollView aborts its scroller once its edge absorbs it).
  bool done = ms >= std::max(glide_->x.durationMs(), glide_->y.durationMs()) ||
              ((glide_->x.distance() == 0 || to.x <= 0 || to.x >= end.x) &&
               (glide_->y.distance() == 0 || to.y <= 0 || to.y >= end.y));
  if (done) stopGlide();
  return moved;
}

std::optional<std::chrono::steady_clock::time_point> PointerInput::nextFrame(bool ready) const {
  if (!ready) return std::nullopt;
  auto period = refresh_ ? refresh_->period() : std::chrono::milliseconds(16);
  std::optional<std::chrono::steady_clock::time_point> next;
  if (glide_) next = glide_->stepped ? *glide_->stepped + period : std::chrono::steady_clock::now();
  for (const auto& [tag, overscroll] : overscroll_) {
    for (const auto& edge : overscroll.axis) {
      if (edge.resting() || edge.pulling()) continue;
      auto due = stretchStepped_ ? *stretchStepped_ + period : std::chrono::steady_clock::now();
      next = next ? std::min(*next, due) : due;
    }
  }
  return next;
}

std::map<Tag, Point> PointerInput::stretches() const {
  std::map<Tag, Point> shown;
  for (const auto& [tag, overscroll] : overscroll_) {
    Point amount{overscroll.side[0] * overscroll.axis[0].amount(), overscroll.side[1] * overscroll.axis[1].amount()};
    if (amount.x != 0 || amount.y != 0) shown[tag] = amount;
  }
  return shown;
}

int PointerInput::scrollAxis(Tag tag) const {
  // Android's ScrollView scrolls vertically and its HorizontalScrollView horizontally; both are the
  // host's ScrollView, told apart by what their content scrolls.
  auto range = scrollRange(tag);
  return range && range->second.x > 0 && range->second.y <= 0 ? 0 : 1;
}

bool PointerInput::canOverscroll(Tag tag, int axis) const {
  auto it = tree_.views().find(tag);
  auto range = scrollRange(tag);
  if (it == tree_.views().end() || !range || scrollAxis(tag) != axis) return false;
  switch (overScrollModeOf(*it->second.view.props)) {
    case OverScrollMode::Always: return true;
    case OverScrollMode::Never: return false;
    case OverScrollMode::IfContentScrolls: return (axis ? range->second.y : range->second.x) > 0;
  }
  return false;
}

float PointerInput::releaseStretch(Tag tag, int axis, float delta) {
  auto it = overscroll_.find(tag);
  if (it == overscroll_.end() || delta == 0) return delta;
  auto& edge = it->second.axis[axis];
  int side = it->second.side[axis];
  if (edge.distance() <= 0 || !((side < 0 && delta > 0) || (side > 0 && delta < 0))) return delta;
  Size size = tree_.views().at(tag).view.layoutMetrics.frame.size;
  float extent = axis ? size.height : size.width;
  if (extent <= 0) return delta;
  float consumed = -edge.pull(-std::abs(delta) / extent) * extent;
  stretchChanged_ = true;
  reportStretch(tag);
  return delta > 0 ? std::max(0.0f, delta - consumed) : std::min(0.0f, delta + consumed);
}

void PointerInput::pullStretch(Tag tag, int axis, float delta, float before, float end) {
  if (!canOverscroll(tag, axis)) return;
  Size size = tree_.views().at(tag).view.layoutMetrics.frame.size;
  float extent = axis ? size.height : size.width;
  if (extent <= 0) return;
  // As ScrollView.onTouchEvent pulls it: by the whole move, where it would have gone past the end.
  auto& overscroll = overscroll_[tag];
  overscroll.surface = drag_ ? drag_->surface : content_.surface;
  float to = before + delta;
  int side = to < 0 ? -1 : 1;
  if (overscroll.side[axis] != side && overscroll.axis[axis].distance() > 0) return;
  overscroll.side[axis] = side;
  overscroll.axis[axis].pull(std::abs(delta) / extent);
  (void)end;
  stretchChanged_ = true;
  reportStretch(tag);
}

void PointerInput::reportStretch(Tag tag) {
  auto view = tree_.views().find(tag);
  if (view == tree_.views().end()) return;
  auto it = overscroll_.find(tag);
  Point amount{}, distance{};
  if (it != overscroll_.end()) {
    amount = {it->second.side[0] * it->second.axis[0].amount(), it->second.side[1] * it->second.axis[1].amount()};
    distance = {it->second.side[0] * it->second.axis[0].distance(), it->second.side[1] * it->second.axis[1].distance()};
  }
  Report::get().stretch(view->second.view, amount.x, amount.y, distance.x, distance.y);
}

void PointerInput::stopGlide() {
  if (!glide_) return;
  Glide glide = *glide_;
  glide_.reset();
  if (tree_.views().find(glide.view) != tree_.views().end()) scrollEvent(glide.view, glide.surface, "momentumScrollEnd");
}

// When a scroll event is dispatched, in milliseconds of the monotonic clock (performance.now()'s):
// its `timestamp`, as React Native's Android ScrollEvent takes `SystemClock.uptimeMillis()` when it
// is obtained, for every kind (the scroll, the drag's begin and end, the momentum's). Not the
// device's time of the input behind it: a glide's scrolls have no input behind them, and Android
// stamps a wheel's or a drag's scroll when the view scrolls, not with its MotionEvent's time.
static double dispatchTime() { return HighResTimeStamp::now().toDOMHighResTimeStamp(); }

// A scroll event whose `timestamp` reaches JavaScript in whole precision. React Native's C++
// ScrollEvent holds it as a Float of seconds, which on this platform is a float: a monotonic clock's
// seconds since boot in 24 bits come to JavaScript in steps of 31 ms. Android's ScrollEvent sends
// its uptimeMillis as a double; so does this payload, the rest of it React Native's own.
template <typename Base>
struct ExactlyStamped : Base {
  ExactlyStamped(const Base& event, double ms) : Base(event), ms(ms) {}
  double ms;
  facebook::jsi::Value asJSIValue(facebook::jsi::Runtime& runtime) const override {
    auto value = Base::asJSIValue(runtime);
    value.asObject(runtime).setProperty(runtime, "timestamp", ms);
    return value;
  }
};

void PointerInput::scrollEvent(Tag tag, uint32_t surface, const char* kind, std::optional<Point> velocity) {
  auto it = tree_.views().find(tag);
  if (it == tree_.views().end()) return;
  const double dispatched = dispatchTime();
  const auto& view = it->second.view;
  auto range = scrollRange(tag);
  auto emitter = std::dynamic_pointer_cast<const ScrollViewEventEmitter>(view.eventEmitter);
  if (emitter && view.state) {
    const auto& state = std::static_pointer_cast<const ConcreteState<ScrollViewState>>(view.state);
    ScrollEvent event;
    event.contentSize = state->getData().getContentSize();
    event.contentOffset = range ? range->first : Point{};
    event.containerSize = view.layoutMetrics.frame.size;
    event.zoomScale = 1;
    event.timestamp = static_cast<Float>(dispatched / 1000);  // seconds
    std::string name = kind;
    // As ScrollViewEventEmitter dispatches each (onScrollBeginDrag and the rest), stamped exactly.
    if (name == "scrollEndDrag") {
      ScrollEndDragEvent end(event);
      end.velocity = velocity.value_or(Point{});
      emitter->dispatchEvent(name, std::make_shared<ExactlyStamped<ScrollEndDragEvent>>(end, dispatched));
    } else if (name == "scrollBeginDrag" || name == "momentumScrollBegin" || name == "momentumScrollEnd") {
      emitter->dispatchEvent(name, std::make_shared<ExactlyStamped<ScrollEvent>>(event, dispatched));
    }
  }
  folly::dynamic extra = folly::dynamic::object("timestamp", dispatched);
  if (range) extra["x"] = range->first.x, extra["y"] = range->first.y;
  if (velocity) extra["vx"] = velocity->x, extra["vy"] = velocity->y;
  Report::get().deliver(kind, surface, view, extra);
}

std::optional<std::pair<facebook::react::Point, facebook::react::Point>> PointerInput::scrollRange(Tag tag) const {
  auto it = tree_.views().find(tag);
  if (it == tree_.views().end()) return std::nullopt;
  const auto& view = it->second.view;
  if (std::string(view.componentName ? view.componentName : "") != "ScrollView" || !view.state) return std::nullopt;
  auto props = std::dynamic_pointer_cast<const ScrollViewProps>(view.props);
  if (props && !props->scrollEnabled) return std::nullopt;
  const auto& state = std::static_pointer_cast<const ConcreteState<ScrollViewState>>(view.state);
  Size viewport = view.layoutMetrics.frame.size;
  Size content = state->getData().getContentSize();
  auto known = offsets_.find(tag);
  Point current = known != offsets_.end() ? known->second : state->getData().contentOffset;
  return std::make_pair(current, Point{std::max(0.0f, content.width - viewport.width),
                                       std::max(0.0f, content.height - viewport.height)});
}

bool PointerInput::scrollView(Tag tag, float dx, float dy, uint32_t surface) {
  auto range = scrollRange(tag);
  if (!range) return false;
  auto [current, end] = *range;
  Point next{std::clamp(current.x + dx, 0.0f, end.x), std::clamp(current.y + dy, 0.0f, end.y)};
  if (next == current) return false;
  const auto& view = tree_.views().at(tag).view;
  const auto& state = std::static_pointer_cast<const ConcreteState<ScrollViewState>>(view.state);
  Size viewport = view.layoutMetrics.frame.size;
  Size content = state->getData().getContentSize();
  offsets_[tag] = next;
  const double dispatched = dispatchTime();
  auto data = state->getData();
  data.contentOffset = next;
  state->updateState(std::move(data));
  if (auto emitter = std::dynamic_pointer_cast<const ScrollViewEventEmitter>(view.eventEmitter)) {
    ScrollEvent event;
    event.contentSize = content;
    event.contentOffset = next;
    event.containerSize = viewport;
    event.zoomScale = 1;
    event.timestamp = static_cast<Float>(dispatched / 1000);  // seconds
    // As ScrollViewEventEmitter::onScroll dispatches it (a unique event: coalesced), stamped exactly.
    emitter->dispatchUniqueEvent("scroll", std::make_shared<ExactlyStamped<ScrollEvent>>(event, dispatched));
  }
  Report::get().deliver("scroll", surface, view,
                        folly::dynamic::object("x", next.x)("y", next.y)("timestamp", dispatched));
  if (std::getenv("OXYOS_HOST_TRACE")) {
    std::cerr << "[input] scroll view " << tag << " to " << next.x << "," << next.y << " of " << content.width << "x"
              << content.height << "\n";
  }
  return true;
}

void PointerInput::dispatch(Phase phase, float x, float y) {
  dispatchTouches(phase, {Finger{identifier_, x, y}}, std::nullopt);
}

void PointerInput::dispatchTouches(Phase phase, const std::vector<Finger>& fingers, std::optional<size_t> changed) {
  auto it = tree_.views().find(target_);
  if (it == tree_.views().end()) return;
  auto emitter = std::dynamic_pointer_cast<const TouchEventEmitter>(it->second.view.eventEmitter);
  if (!emitter) return;
  Point targetOrigin = windowFrame(tree_, target_).value_or(Rect{}).origin;
  bool lifted = phase == Phase::End || phase == Phase::Cancel;
  TouchEvent event{};
  for (size_t i = 0; i < fingers.size(); ++i) {
    // Touches are reported in root coordinates, where React Native measures views; a popup's
    // content sits at its popup view's place in the tree.
    Point page{content_.origin.x + fingers[i].x, content_.origin.y + fingers[i].y};
    Touch touch{};
    touch.pagePoint = page;
    touch.screenPoint = page;
    touch.offsetPoint = {page.x - targetOrigin.x, page.y - targetOrigin.y};
    touch.identifier = fingers[i].identifier;
    touch.target = target_;
    bool isChanged = !changed || *changed == i;
    touch.force = lifted && isChanged ? 0 : 1;
    // JavaScript's `timestamp`, and the event's: when the device registered it.
    touch.timeStamp = eventTime_;
    touch.timestamp = static_cast<Float>(eventTime_.toDOMHighResTimeStamp());
    if (isChanged) event.changedTouches.insert(touch);
    if (!(lifted && isChanged)) {
      event.touches.insert(touch);
      event.targetTouches.insert(touch);
    }
  }
  const Finger& actor = fingers[changed.value_or(0)];
  Point page{content_.origin.x + actor.x, content_.origin.y + actor.y};
  static const char* events[] = {"touchStart", "touchMove", "touchEnd", "touchCancel"};
  if (Report::get().enabled()) {
    // The nearest ancestor with a testID names what was touched (the target is often its text).
    std::string within;
    auto path = ancestry(tree_, target_);
    for (auto at = path.rbegin(); at != path.rend() && within.empty(); ++at) {
      auto props = std::dynamic_pointer_cast<const ViewProps>(tree_.views().at(*at).view.props);
      if (props && !props->testId.empty()) within = props->testId;
    }
    Report::get().deliver(events[static_cast<int>(phase)], content_.surface, it->second.view,
                          folly::dynamic::object("x", page.x)("y", page.y)("within", within)(
                              "touches", static_cast<int>(event.touches.size())));
  }
  if (std::getenv("OXYOS_HOST_TRACE")) {
    static const char* names[] = {"start", "move", "end", "cancel"};
    std::cerr << "[input] touch " << names[static_cast<int>(phase)] << " target " << target_ << " at "
              << actor.x << "," << actor.y << " (" << event.touches.size() << " down)\n";
  }
  switch (phase) {
    case Phase::Start:
      emitter->onTouchStart(event);
      break;
    case Phase::Move:
      emitter->onTouchMove(event);
      break;
    case Phase::End:
      emitter->onTouchEnd(event);
      break;
    case Phase::Cancel:
      emitter->onTouchCancel(event);
      break;
  }
}

}  // namespace oxyos
