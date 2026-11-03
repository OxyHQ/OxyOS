#include "input.h"

#include "android-view-props.h"
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

// Linux input codes of the primary (left) and secondary (right) buttons.
constexpr uint32_t kPrimaryButton = 272;
constexpr uint32_t kSecondaryButton = 273;

// Whether the view is a text input (React Native's Android TextInput, textinput.h).
bool isTextInput(const ViewTree& tree, Tag tag) {
  auto it = tree.views().find(tag);
  return it != tree.views().end() && it->second.view.componentName &&
         std::string(it->second.view.componentName) == "AndroidTextInput";
}

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

bool acceptsFirstMouse(const ViewTree& tree, Tag target) {
  if (target < 0) return false;
  auto path = ancestry(tree, target);
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    const auto& view = tree.views().at(*at).view;
    if (!view.props) continue;
    auto own = androidViewPropsOf(*view.props);
    if (own.acceptsFirstMouse >= 0) return own.acceptsFirstMouse == 1;
    if (view.componentName && std::string(view.componentName) == "AndroidTextInput") {
      auto input = std::dynamic_pointer_cast<const AndroidTextInputProps>(view.props);
      return input && input->editable;
    }
    if (own.focusable) return false;
  }
  return false;
}

bool PointerInput::withheld(const OxyOSEvent& event) {
  if (event.kind != OXYOS_EVENT_POINTER) return false;
  switch (event.detail) {
    case OXYOS_POINTER_PRESS: {
      if (!event.flag || event.code != kPrimaryButton || down_ || pinching_ || withheld_) return false;
      auto content = surfaces_(event.surface);
      if (!content) return false;
      Tag target = hitTest(tree_, *content, event.x / 256.0f, event.y / 256.0f);
      withheld_ = !acceptsFirstMouse(tree_, target);
      Report::get().input(withheld_ ? "first_mouse.declined" : "first_mouse.accepted", event.surface);
      return withheld_;
    }
    case OXYOS_POINTER_RELEASE:
      if (!withheld_ || event.code != kPrimaryButton) return false;
      withheld_ = false;
      return true;
    case OXYOS_POINTER_LEAVE:
    case OXYOS_POINTER_CANCEL:
      // The press is over elsewhere: its release goes to no one here.
      withheld_ = false;
      return false;
    default:
      return false;
  }
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
  // A press on a scrollbar is the scrollbar's until it is let go (as a Mac's scroller tracks the
  // mouse): its motion drags the thumb, and nothing reaches the views under it.
  if (barPress_ && event.surface == barPress_->surface) {
    x_ = x;
    y_ = y;
    if (event.detail == OXYOS_POINTER_MOTION) return dragBar(x, y);
    if (event.detail == OXYOS_POINTER_RELEASE && event.code == kPrimaryButton) {
      releaseBar();
      hoverBar(barAt(*content, x, y), event.surface);
      return;
    }
    if (event.detail == OXYOS_POINTER_PRESS || event.detail == OXYOS_POINTER_RELEASE) return;
    if (event.detail == OXYOS_POINTER_LEAVE || event.detail == OXYOS_POINTER_CANCEL) releaseBar();
  }
  // A touch stays with the surface it started on.
  if (!down_) content_ = *content;
  x_ = x;
  y_ = y;
  if (event.detail == OXYOS_POINTER_ENTER || event.detail == OXYOS_POINTER_MOTION) {
    hover(*content, x, y);
    // Over a scrollbar's track, it shows wide (a hidden one too, as a Mac's overlay scroller
    // appears under the pointer), and while a button is held elsewhere it stays as it was.
    if (!down_) hoverBar(barAt(*content, x, y), event.surface);
  }
  if (event.detail == OXYOS_POINTER_LEAVE && event.surface == hoveredSurface_) hover(std::nullopt, x, y);
  if (event.detail == OXYOS_POINTER_LEAVE && overBar_) hoverBar(std::nullopt, event.surface);
  if (event.detail == OXYOS_POINTER_PRESS && event.code == kPrimaryButton && !down_ && !pinching_) {
    if (auto bar = barAt(*content, x, y)) {
      auto shown = barsShown_.find(bar->first);
      bool showing = (shown != barsShown_.end() && shown->second.alpha > 0) ||
                     persistentScrollbars(tree_.views().at(bar->first).view);
      if (showing) return pressBar(*content, bar->first, bar->second, x, y);
    }
  }
  // A Control-click on a text input is its secondary click, as a Mac takes one: its button is the
  // secondary one to JavaScript, it is no touch, and it opens the input's context menu. Elsewhere a
  // Control-click is the application's (the dock opens its menu, a list adds to its selection).
  if (event.detail == OXYOS_POINTER_PRESS && event.code == kPrimaryButton && !down_ && !controlClick_) {
    controlClick_ = ctrl_ && textMenu_ && isTextInput(tree_, hitTest(tree_, *content, x, y));
  }
  bool primaryAsSecondary = controlClick_ && event.code == kPrimaryButton;
  if (event.detail == OXYOS_POINTER_PRESS || event.detail == OXYOS_POINTER_RELEASE) {
    uint32_t button = primaryAsSecondary ? kSecondaryButton : event.code;
    pointerButton(*content, button, event.detail == OXYOS_POINTER_PRESS, x, y);
    if (event.detail == OXYOS_POINTER_PRESS && button == kSecondaryButton && textMenu_) {
      Tag target = hitTest(tree_, *content, x, y);
      if (isTextInput(tree_, target)) textMenu_(target, content->surface, x, y);
    }
  }
  if (event.detail == OXYOS_POINTER_LEAVE || event.detail == OXYOS_POINTER_CANCEL) controlClick_ = false;
  if (primaryAsSecondary) {
    if (event.detail == OXYOS_POINTER_RELEASE) controlClick_ = false;
    if (event.detail == OXYOS_POINTER_PRESS || event.detail == OXYOS_POINTER_RELEASE) return;
  }
  switch (event.detail) {
    case OXYOS_POINTER_PRESS: {
      stopGlide();
      // A press stops the animated scrolls of the scroll views under it, as a touch aborts
      // Android's ScrollView's scroller.
      Tag under = hitTest(tree_, *content, x, y);
      if (under >= 0) {
        for (Tag at : ancestry(tree_, under)) {
          if (animations_.count(at)) stopAnimation(at);
        }
      }
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
    }
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
    if (scrollView(*at, dx, dy, content_.surface)) {
      if (animations_.count(*at)) stopAnimation(*at);
      return;
    }
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
      if (animations_.count(drag_->view)) stopAnimation(drag_->view);
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
  bool moved = false;
  // Animated scrolls (smoothScrollBy, JavaScript's scrollTo), once a frame each.
  for (auto it = animations_.begin(); it != animations_.end();) {
    Tag tag = it->first;
    Animation& animation = it->second;
    if (animation.stepped && now - *animation.stepped < period) {
      ++it;
      continue;
    }
    animation.stepped = now;
    auto range = scrollRange(tag);
    if (!range) {
      it = animations_.erase(it);
      continue;
    }
    double ms = std::chrono::duration<double, std::milli>(now - animation.began).count();
    Point to{animation.x.at(ms), animation.y.at(ms)};
    moved |= scrollView(tag, to.x - range->first.x, to.y - range->first.y, animation.surface, animation.awaken);
    if (animation.x.done(ms)) {
      bool momentum = animation.momentum;
      uint32_t surface = animation.surface;
      it = animations_.erase(it);
      if (momentum) scrollEvent(tag, surface, "momentumScrollEnd");
      continue;
    }
    ++it;
  }
  // A press held in a scrollbar's track pages on.
  if (barPress_ && !barPress_->thumb && barPress_->repeat && now >= *barPress_->repeat) pageTowardsPointer();
  moved |= stepBars(now);
  if (!glide_) return moved;
  if (glide_->stepped && now - *glide_->stepped < period) return moved;
  glide_->stepped = now;
  auto range = scrollRange(glide_->view);
  if (!range) {
    glide_.reset();
    return moved;
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
  moved |= scrollView(glide_->view, to.x - current.x, to.y - current.y, glide_->surface);
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
  auto now = std::chrono::steady_clock::now();
  auto due = [&](std::chrono::steady_clock::time_point at) { next = next ? std::min(*next, at) : at; };
  if (glide_) next = glide_->stepped ? *glide_->stepped + period : now;
  for (const auto& [tag, animation] : animations_) due(animation.stepped ? *animation.stepped + period : now);
  if (barPress_ && barPress_->repeat) due(*barPress_->repeat);
  // Scrollbars fading or growing step once a frame; one held shown is due when its hold ends.
  for (const auto& [tag, bars] : bars_) {
    auto shown = barsShown_.find(tag);
    bool growing = bars.hovered >= 0 && bars.expansion[bars.hovered] < 1;
    if (growing) due(bars.grown ? *bars.grown + period : now);
    if (bars.awake && shown != barsShown_.end()) {
      auto hold = *bars.awake + std::chrono::microseconds(static_cast<long>(kScrollbarHoldMs * 1000));
      due(now < hold ? hold : now + period);
    }
  }
  for (const auto& [tag, overscroll] : overscroll_) {
    for (const auto& edge : overscroll.axis) {
      if (edge.resting() || edge.pulling()) continue;
      due(stretchStepped_ ? *stretchStepped_ + period : now);
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

bool PointerInput::scrollView(Tag tag, float dx, float dy, uint32_t surface, bool awaken) {
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
  scrolledOn_[tag] = surface;
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
  if (awaken) awakenBars(tag, surface);
  return true;
}

void PointerInput::clampScrolls() {
  std::vector<std::pair<Tag, Point>> back;
  for (auto it = offsets_.begin(); it != offsets_.end();) {
    if (!tree_.views().count(it->first)) {
      scrolledOn_.erase(it->first);
      it = offsets_.erase(it);
      continue;
    }
    if (auto range = scrollRange(it->first)) {
      auto [current, end] = *range;
      if (current.x > end.x || current.y > end.y) {
        back.emplace_back(it->first, Point{std::min(current.x, end.x) - current.x, std::min(current.y, end.y) - current.y});
      }
    }
    ++it;
  }
  // A layout's clamp is no scroll the person made: no scrollbar shows for it.
  for (const auto& [tag, by] : back) scrollView(tag, by.x, by.y, scrolledOn_[tag], false);
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


// Animated scrolls ---------------------------------------------------------------------------

void PointerInput::startAnimation(Tag tag, uint32_t surface, Point to, ScrollAnimation::Curve curve, bool momentum,
                                  bool awaken) {
  auto range = scrollRange(tag);
  if (!range) return;
  auto [current, end] = *range;
  to = {std::clamp(to.x, 0.0f, end.x), std::clamp(to.y, 0.0f, end.y)};
  if (glide_ && glide_->view == tag) stopGlide();
  // A scroll already animating there goes on from where it is (its momentum, if any, still framing
  // it); a new one starts.
  auto it = animations_.find(tag);
  if (to == current && it == animations_.end()) return;
  bool framed = it != animations_.end() && it->second.momentum;
  Animation animation;
  animation.surface = surface;
  animation.x = ScrollAnimation(current.x, to.x, curve);
  animation.y = ScrollAnimation(current.y, to.y, curve);
  animation.began = std::chrono::steady_clock::now();
  animation.momentum = momentum || framed;
  animation.awaken = awaken;
  animations_[tag] = animation;
  if (momentum && !framed) scrollEvent(tag, surface, "momentumScrollBegin");
}

void PointerInput::stopAnimation(Tag tag) {
  auto it = animations_.find(tag);
  if (it == animations_.end()) return;
  bool momentum = it->second.momentum;
  uint32_t surface = it->second.surface;
  animations_.erase(it);
  if (momentum) scrollEvent(tag, surface, "momentumScrollEnd");
}

bool PointerInput::smoothScrollBy(Tag tag, float dx, float dy, uint32_t surface) {
  auto range = scrollRange(tag);
  if (!range) return false;
  auto [current, end] = *range;
  // From where an animation going on was going, so that each key adds its step.
  Point from = current;
  if (auto it = animations_.find(tag); it != animations_.end()) from = {it->second.x.to(), it->second.y.to()};
  Point to{std::clamp(from.x + dx, 0.0f, end.x), std::clamp(from.y + dy, 0.0f, end.y)};
  // On the display's pixels, as Android's are whole.
  if (scale_ > 0) to = {std::round(to.x * scale_) / scale_, std::round(to.y * scale_) / scale_};
  to = {std::clamp(to.x, 0.0f, end.x), std::clamp(to.y, 0.0f, end.y)};
  if (to == from) return false;
  startAnimation(tag, surface, to, ScrollAnimation::Curve::ViscousFluid, false, true);
  return true;
}

void PointerInput::reveal(Tag tag, uint32_t surface, float margin) {
  auto found = tree_.views().find(tag);
  if (found == tree_.views().end()) return;
  if (scale_ > 0) margin = std::round(margin * scale_) / scale_;
  // The view's rectangle, with the margin, in the coordinates of each view up its path in turn.
  Size size = found->second.view.layoutMetrics.frame.size;
  Rect rect{{-margin, -margin}, {size.width + 2 * margin, size.height + 2 * margin}};
  auto path = ancestry(tree_, tag);
  for (size_t i = path.size(); i-- > 1;) {
    const auto& child = tree_.views().at(path[i]).view;
    // A modal's window, a popup's or a shell region's surface is drawn on its own, never within
    // the scroll views its view sits in in the tree: they do not hold it.
    std::string childName = child.componentName ? child.componentName : "";
    if (isModal(childName) || ownSurface(childName)) return;
    Point moved = transformOffset(child);
    rect.origin.x += child.layoutMetrics.frame.origin.x + moved.x;
    rect.origin.y += child.layoutMetrics.frame.origin.y + moved.y;
    Tag parent = path[i - 1];
    auto range = scrollRange(parent);
    if (!range) continue;
    // In the scroll view's content now: the least move that shows it, as
    // ScrollView.computeScrollDeltaToGetChildRectOnScreen works it out (no fading edge), along each
    // axis, never past the content's ends.
    const auto& view = tree_.views().at(parent).view;
    auto [offset, end] = *range;
    auto delta = [](float start, float length, float screen, float viewport, float limit) {
      float bottom = start + length, screenBottom = screen + viewport, d = 0;
      if (bottom > screenBottom && start > screen) {
        d = length > viewport ? start - screen : bottom - screenBottom;
        d = std::min(d, limit - screen);
      } else if (start < screen && bottom < screenBottom) {
        d = length > viewport ? -(screenBottom - bottom) : -(screen - start);
        d = std::max(d, -screen);
      }
      return d;
    };
    Size viewport = view.layoutMetrics.frame.size;
    float dx = delta(rect.origin.x, rect.size.width, offset.x, viewport.width, end.x);
    float dy = delta(rect.origin.y, rect.size.height, offset.y, viewport.height, end.y);
    if (dx != 0 || dy != 0) scrollView(parent, dx, dy, surface, false);
    // In the scroll view's own coordinates, kept within what it shows: what an outer one brings
    // into view, as AppKit's scrollRectToVisible passes the rectangle up its clip views.
    auto after = scrollRange(parent);
    Point now = after ? after->first : offset;
    rect.origin.x -= now.x;
    rect.origin.y -= now.y;
    rect = Rect::intersect(rect, Rect{{0, 0}, viewport});
    if (rect.size.width <= 0 || rect.size.height <= 0) return;
  }
}

void PointerInput::command(const ShadowView& view, const std::string& name, const folly::dynamic& args) {
  if (std::string(view.componentName ? view.componentName : "") != "ScrollView") return;
  Tag tag = view.tag;
  auto range = scrollRange(tag);
  uint32_t surface = surfaceOf_ ? surfaceOf_(tag) : 1;
  auto number = [&](size_t i) { return args.isArray() && args.size() > i && args[i].isNumber() ? args[i].asDouble() : 0.0; };
  auto flag = [&](size_t i) { return args.isArray() && args.size() > i && args[i].isBool() && args[i].getBool(); };
  if (name == "flashScrollIndicators") {
    awakenBars(tag, surface);
    return;
  }
  if (!range || (name != "scrollTo" && name != "scrollToEnd")) return;
  // As ReactScrollViewManager: abortAnimation, then the scroll, animated or at once.
  if (glide_ && glide_->view == tag) stopGlide();
  stopAnimation(tag);
  range = scrollRange(tag);
  auto [current, end] = *range;
  Point to = current;
  bool animated = false;
  if (name == "scrollTo") {
    to = {static_cast<float>(number(0)), static_cast<float>(number(1))};
    animated = flag(2);
  } else {
    // To the end of the axis its content scrolls along (a horizontal one's right end).
    if (scrollAxis(tag) == 0) to.x = end.x; else to.y = end.y;
    animated = flag(0);
  }
  to = {std::clamp(to.x, 0.0f, end.x), std::clamp(to.y, 0.0f, end.y)};
  Report::get().input("scroll.command", surface,
                      folly::dynamic::object("target", Report::name(view))("command", name)("animated", animated)(
                          "x", to.x)("y", to.y));
  if (animated) {
    startAnimation(tag, surface, to, ScrollAnimation::Curve::AccelerateDecelerate, true, false);
  } else {
    scrollView(tag, to.x - current.x, to.y - current.y, surface, false);
  }
}

// Overlay scrollbars --------------------------------------------------------------------------

void PointerInput::awakenBars(Tag tag, uint32_t surface) {
  auto it = tree_.views().find(tag);
  if (it == tree_.views().end()) return;
  if (!hasScrollbar(it->second.view, 0) && !hasScrollbar(it->second.view, 1)) return;
  auto& bars = bars_[tag];
  bars.surface = surface;
  bars.awake = std::chrono::steady_clock::now();
  stepBars(*bars.awake);
}

std::optional<std::pair<Tag, int>> PointerInput::barAt(const SurfaceContent& content, float x, float y) const {
  Tag target = hitTest(tree_, content, x, y);
  if (target < 0) return std::nullopt;
  auto path = ancestry(tree_, target);
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    const auto& view = tree_.views().at(*at).view;
    if (std::string(view.componentName ? view.componentName : "") != "ScrollView") continue;
    auto range = scrollRange(*at);
    auto frame = windowFrame(tree_, *at);
    if (!range || !frame) continue;
    Point local{content.origin.x + x - frame->origin.x, content.origin.y + y - frame->origin.y};
    for (int axis : {1, 0}) {
      auto geometry = scrollbarGeometry(view, range->first, axis, 1, scale_);
      if (geometry && geometry->track.containsPoint(local)) return std::make_pair(*at, axis);
    }
  }
  return std::nullopt;
}

void PointerInput::hoverBar(std::optional<std::pair<Tag, int>> bar, uint32_t surface) {
  if (bar == overBar_) return;
  auto now = std::chrono::steady_clock::now();
  if (overBar_) {
    // Left: it fades from now, as after a scroll.
    if (auto it = bars_.find(overBar_->first); it != bars_.end()) {
      it->second.hovered = -1;
      it->second.awake = now;
    }
  }
  overBar_ = bar;
  if (bar) {
    auto& bars = bars_[bar->first];
    bars.surface = surface;
    bars.hovered = bar->second;
    bars.awake.reset();
    if (!bars.grown) bars.grown = now;
  }
  stepBars(now);
}

bool PointerInput::stepBars(std::chrono::steady_clock::time_point now) {
  std::map<Tag, ScrollbarShown> shown;
  for (auto it = bars_.begin(); it != bars_.end();) {
    Tag tag = it->first;
    Bars& bars = it->second;
    if (tree_.views().find(tag) == tree_.views().end()) {
      it = bars_.erase(it);
      continue;
    }
    bool held = bars.hovered >= 0 || (barPress_ && barPress_->view == tag);
    float alpha = 0;
    if (held) {
      alpha = 1;
    } else if (bars.awake) {
      double ms = std::max(0.0, std::chrono::duration<double, std::milli>(now - *bars.awake).count());
      alpha = ms < kScrollbarHoldMs ? 1.0f
                                    : static_cast<float>(std::max(0.0, 1.0 - (ms - kScrollbarHoldMs) / kScrollbarFadeMs));
    }
    // The bar the pointer is over grows towards wide; once wide, a bar stays so until it has gone,
    // as a Mac's does.
    int growing = bars.hovered >= 0 ? bars.hovered : (barPress_ && barPress_->view == tag ? barPress_->axis : -1);
    // (A step may be asked with a time a little before the last, taken before a scroll in the same
    // turn stepped it: time never runs back here.)
    if (growing >= 0 && bars.grown) {
      double ms = std::max(0.0, std::chrono::duration<double, std::milli>(now - *bars.grown).count());
      bars.expansion[growing] = std::min(1.0f, bars.expansion[growing] + static_cast<float>(ms / kScrollbarExpandMs));
    }
    if (!bars.grown || now > *bars.grown) bars.grown = now;
    if (alpha <= 0) {
      it = bars_.erase(it);
      continue;
    }
    shown[tag] = ScrollbarShown{alpha, {bars.expansion[0], bars.expansion[1]}};
    ++it;
  }
  if (shown == barsShown_) return false;
  // What shows, reported as it changes: a view's scrollbars appearing, growing wide, going.
  if (Report::get().enabled()) {
    auto report = [&](Tag tag, const char* event, const ScrollbarShown& state) {
      auto view = tree_.views().find(tag);
      if (view == tree_.views().end()) return;
      folly::dynamic axes = folly::dynamic::array;
      folly::dynamic thumbs = folly::dynamic::object;
      auto frame = windowFrame(tree_, tag);
      auto range = scrollRange(tag);
      for (int axis = 0; axis < 2; ++axis) {
        if (!hasScrollbar(view->second.view, axis)) continue;
        axes.push_back(axis ? "y" : "x");
        if (auto geometry = scrollbarGeometry(view->second.view, range ? range->first : Point{}, axis,
                                              state.expansion[axis], scale_);
            geometry && frame) {
          const Rect& k = geometry->thumb;
          thumbs[axis ? "y" : "x"] = folly::dynamic::array(frame->origin.x + k.origin.x, frame->origin.y + k.origin.y,
                                                           k.size.width, k.size.height);
        }
      }
      Report::get().scrollbar(event, view->second.view, std::move(axes), std::move(thumbs), state.alpha);
    };
    for (const auto& [tag, state] : shown) {
      auto before = barsShown_.find(tag);
      if (before == barsShown_.end()) report(tag, "shown", state);
      for (int axis = 0; axis < 2; ++axis) {
        bool wide = state.expansion[axis] >= 1, was = before != barsShown_.end() && before->second.expansion[axis] >= 1;
        if (wide && !was) report(tag, axis ? "wide-y" : "wide-x", state);
      }
    }
    for (const auto& [tag, state] : barsShown_) {
      if (!shown.count(tag)) report(tag, "hidden", state);
    }
  }
  barsShown_ = std::move(shown);
  barsChanged_ = true;
  return true;
}

void PointerInput::pressBar(const SurfaceContent& content, Tag tag, int axis, float x, float y) {
  stopGlide();
  stopAnimation(tag);
  auto range = scrollRange(tag);
  auto frame = windowFrame(tree_, tag);
  if (!range || !frame) return;
  const auto& view = tree_.views().at(tag).view;
  auto geometry = scrollbarGeometry(view, range->first, axis, 1, scale_);
  if (!geometry) return;
  float along = axis ? content.origin.y + y - frame->origin.y : content.origin.x + x - frame->origin.x;
  const Rect& thumb = geometry->thumb;
  float start = axis ? thumb.origin.y : thumb.origin.x, length = axis ? thumb.size.height : thumb.size.width;
  BarPress press;
  press.view = tag;
  press.surface = content.surface;
  press.axis = axis;
  press.thumb = along >= start && along < start + length;
  press.pointer = along;
  press.from = axis ? range->first.y : range->first.x;
  barPress_ = press;
  if (auto it = bars_.find(tag); it != bars_.end()) it->second.awake.reset();
  Report::get().input("scrollbar.press", content.surface,
                      folly::dynamic::object("target", Report::name(view))("axis", axis ? "y" : "x")(
                          "thumb", press.thumb));
  if (press.thumb) {
    // As Android's ScrollView, intercepting a mouse's press on its thumb, begins a drag.
    scrollEvent(tag, content.surface, "scrollBeginDrag");
  } else {
    pageTowardsPointer();
    // NSScroller's periodic paging while the button is held: after a pause, then steadily.
    if (barPress_) barPress_->repeat = std::chrono::steady_clock::now() + std::chrono::milliseconds(400);
  }
  stepBars(std::chrono::steady_clock::now());
}

void PointerInput::dragBar(float x, float y) {
  if (!barPress_ || !barPress_->thumb) return;
  auto range = scrollRange(barPress_->view);
  auto frame = windowFrame(tree_, barPress_->view);
  auto content = surfaces_(barPress_->surface);
  if (!range || !frame || !content) return;
  const auto& view = tree_.views().at(barPress_->view).view;
  auto geometry = scrollbarGeometry(view, range->first, barPress_->axis, 1, scale_);
  if (!geometry) return;
  int axis = barPress_->axis;
  float along = axis ? content->origin.y + y - frame->origin.y : content->origin.x + x - frame->origin.x;
  // The thumb follows the pointer along its track; the content goes as far for each pixel of it as
  // the track's free length is to the content's (View#handleScrollBarDragging).
  float track = axis ? geometry->track.size.height : geometry->track.size.width;
  float thumb = axis ? geometry->thumb.size.height : geometry->thumb.size.width;
  float travel = track - 2 * kScrollbarInset - thumb;
  float end = axis ? range->second.y : range->second.x;
  if (travel <= 0 || end <= 0) return;
  float to = std::clamp(barPress_->from + (along - barPress_->pointer) * end / travel, 0.0f, end);
  float current = axis ? range->first.y : range->first.x;
  if (to == current) return;
  scrollView(barPress_->view, axis ? 0 : to - current, axis ? to - current : 0, barPress_->surface);
}

void PointerInput::releaseBar() {
  if (!barPress_) return;
  BarPress press = *barPress_;
  barPress_.reset();
  if (auto it = bars_.find(press.view); it != bars_.end() && it->second.hovered < 0) {
    it->second.awake = std::chrono::steady_clock::now();
  }
  // The drag it began ends where it is, still: Android's View#handleScrollBarDragging takes the
  // release before ReactScrollView.onTouchEvent would end it, so Android sends none; the host ends
  // the drag it began, so JavaScript's begin and end pair.
  if (press.thumb && tree_.views().find(press.view) != tree_.views().end()) {
    scrollEvent(press.view, press.surface, "scrollEndDrag", Point{0, 0});
  }
  stepBars(std::chrono::steady_clock::now());
}

void PointerInput::pageTowardsPointer() {
  if (!barPress_) return;
  auto range = scrollRange(barPress_->view);
  if (!range) return;
  const auto& view = tree_.views().at(barPress_->view).view;
  int axis = barPress_->axis;
  // Where the thumb will be once the paging going on ends: it pages until it reaches the pointer.
  Point target = range->first;
  if (auto it = animations_.find(barPress_->view); it != animations_.end()) target = {it->second.x.to(), it->second.y.to()};
  auto geometry = scrollbarGeometry(view, target, axis, 1, scale_);
  if (!geometry) return;
  float start = axis ? geometry->thumb.origin.y : geometry->thumb.origin.x;
  float length = axis ? geometry->thumb.size.height : geometry->thumb.size.width;
  int direction = barPress_->pointer < start ? -1 : barPress_->pointer >= start + length ? 1 : 0;
  if (direction == 0) {
    barPress_->repeat.reset();
    return;
  }
  Size viewport = view.layoutMetrics.frame.size;
  float page = direction * pageStep(axis ? viewport.height : viewport.width);
  if (!smoothScrollBy(barPress_->view, axis ? 0 : page, axis ? page : 0, barPress_->surface)) {
    barPress_->repeat.reset();
    return;
  }
  barPress_->repeat = std::chrono::steady_clock::now() + std::chrono::milliseconds(100);
}

}  // namespace oxyos
