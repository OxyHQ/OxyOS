#include "input.h"

#include "report.h"

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
#include <optional>
#include <string>

using namespace facebook::react;

namespace oxyos {

namespace {

// Linux input code of the primary (left) button.
constexpr uint32_t kPrimaryButton = 272;

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
  if (!down_ || responderCancelled_) return;
  dispatch(Phase::Cancel, x_, y_);
  responderCancelled_ = true;
}

void PointerInput::handle(const OxyOSEvent& event) {
  // Protocol positions and distances are in 1/256 logical pixels.
  if (event.kind != OXYOS_EVENT_SCROLL && event.kind != OXYOS_EVENT_POINTER) return;
  auto content = surfaces_(event.surface);
  if (!content) return;
  if (event.kind == OXYOS_EVENT_SCROLL) {
    content_ = *content;
    return scroll(event.x / 256.0f, event.y / 256.0f);
  }
  float x = event.x / 256.0f, y = event.y / 256.0f;
  // A touch stays with the surface it started on.
  if (!down_) content_ = *content;
  x_ = x;
  y_ = y;
  if (event.detail == OXYOS_POINTER_ENTER || event.detail == OXYOS_POINTER_MOTION) hover(*content, x, y);
  if (event.detail == OXYOS_POINTER_LEAVE && event.surface == hoveredSurface_) hover(std::nullopt, x, y);
  switch (event.detail) {
    case OXYOS_POINTER_PRESS:
      if (event.code == kPrimaryButton && !down_) {
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
    if (target >= 0) now = ancestry(tree_, target);
  }
  uint32_t surface = content ? content->surface : hoveredSurface_;
  size_t common = 0;
  while (common < now.size() && common < hovered_.size() && now[common] == hovered_[common]) ++common;
  Point page = content ? Point{content->origin.x + x, content->origin.y + y} : Point{x, y};
  auto send = [&](Tag tag, bool enter) {
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
    event.buttons = down_ ? 1 : 0;
    if (enter) {
      emitter->onPointerEnter(event);
    } else {
      emitter->onPointerLeave(event);
    }
    Report::get().deliver(enter ? "pointerEnter" : "pointerLeave", surface, it->second.view);
  };
  for (size_t i = hovered_.size(); i > common; --i) send(hovered_[i - 1], false);
  for (size_t i = common; i < now.size(); ++i) send(now[i], true);
  hovered_ = std::move(now);
  hoveredSurface_ = surface;
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
    if (name != "ScrollView" || !view.state) continue;
    auto props = std::dynamic_pointer_cast<const ScrollViewProps>(view.props);
    if (props && !props->scrollEnabled) continue;
    const auto& state = std::static_pointer_cast<const ConcreteState<ScrollViewState>>(view.state);
    Size viewport = view.layoutMetrics.frame.size;
    Size content = state->getData().getContentSize();
    Point current = offsets_.count(*at) ? offsets_[*at] : state->getData().contentOffset;
    Point next{std::clamp(current.x + dx, 0.0f, std::max(0.0f, content.width - viewport.width)),
               std::clamp(current.y + dy, 0.0f, std::max(0.0f, content.height - viewport.height))};
    if (next == current) continue;
    offsets_[*at] = next;
    auto data = state->getData();
    data.contentOffset = next;
    state->updateState(std::move(data));
    if (auto emitter = std::dynamic_pointer_cast<const ScrollViewEventEmitter>(view.eventEmitter)) {
      ScrollEvent event;
      event.contentSize = content;
      event.contentOffset = next;
      event.containerSize = viewport;
      event.zoomScale = 1;
      emitter->onScroll(event);
    }
    Report::get().deliver("scroll", content_.surface, view, folly::dynamic::object("x", next.x)("y", next.y));
    if (std::getenv("OXYOS_HOST_TRACE")) {
      std::cerr << "[input] scroll view " << *at << " to " << next.x << "," << next.y << " of " << content.width
                << "x" << content.height << "\n";
    }
    return;
  }
}

void PointerInput::dispatch(Phase phase, float x, float y) {
  auto it = tree_.views().find(target_);
  if (it == tree_.views().end()) return;
  auto emitter = std::dynamic_pointer_cast<const TouchEventEmitter>(it->second.view.eventEmitter);
  if (!emitter) return;
  // Touches are reported in root coordinates, where React Native measures views; a popup's
  // content sits at its popup view's place in the tree.
  Point page{content_.origin.x + x, content_.origin.y + y};
  Point targetOrigin = windowFrame(tree_, target_).value_or(Rect{}).origin;
  Touch touch{};
  touch.pagePoint = page;
  touch.screenPoint = page;
  touch.offsetPoint = {page.x - targetOrigin.x, page.y - targetOrigin.y};
  touch.identifier = identifier_;
  touch.target = target_;
  touch.force = phase == Phase::End || phase == Phase::Cancel ? 0 : 1;
  touch.timestamp =
      std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - epoch_).count();
  TouchEvent event{};
  event.changedTouches.insert(touch);
  if (phase == Phase::Start || phase == Phase::Move) {
    event.touches.insert(touch);
    event.targetTouches.insert(touch);
  }
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
                          folly::dynamic::object("x", page.x)("y", page.y)("within", within));
  }
  if (std::getenv("OXYOS_HOST_TRACE")) {
    static const char* names[] = {"start", "move", "end", "cancel"};
    std::cerr << "[input] touch " << names[static_cast<int>(phase)] << " target " << target_ << " at " << x << ","
              << y << "\n";
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
