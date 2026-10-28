#include "input.h"

#include "report.h"

#include <react/renderer/components/scrollview/ScrollViewEventEmitter.h>
#include <react/renderer/components/scrollview/ScrollViewProps.h>
#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/components/view/TouchEventEmitter.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteState.h>

#include <algorithm>
#include <cstdlib>
#include <iostream>
#include <optional>
#include <string>

using namespace facebook::react;

namespace oxy {

namespace {

// Linux input code of the primary (left) button.
constexpr uint32_t kPrimaryButton = 272;

Tag hit(const ViewTree& tree, Tag tag, float x, float y) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return -1;
  const auto& view = it->second.view;
  const auto& frame = view.layoutMetrics.frame;
  float left = frame.origin.x, top = frame.origin.y;
  bool inside = x >= left && y >= top && x < left + frame.size.width && y < top + frame.size.height;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  auto mode = props ? props->pointerEvents : PointerEventsMode::Auto;
  if (mode == PointerEventsMode::None) return -1;
  std::string name = view.componentName ? view.componentName : "";
  // A popup's subtree is in its own surface, not under the window's pointer.
  if (name == kPopupComponent) return -1;
  bool clips = name == "ScrollView" || (props && props->getClipsContentToBounds());
  if (clips && !inside) return -1;
  if (mode != PointerEventsMode::BoxOnly) {
    float cx = x - left, cy = y - top;
    if (name == "ScrollView" && view.state) {
      const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
      cx += state.contentOffset.x;
      cy += state.contentOffset.y;
    }
    // Later children are drawn above earlier ones, so they are hit first.
    const auto& children = it->second.children;
    for (auto child = children.rbegin(); child != children.rend(); ++child) {
      Tag found = hit(tree, *child, cx, cy);
      if (found >= 0) return found;
    }
  }
  return inside && mode != PointerEventsMode::BoxNone ? tag : -1;
}

}  // namespace

Tag hitTest(const ViewTree& tree, const SurfaceContent& content, float x, float y) {
  if (!content.popup) return content.view < 0 ? -1 : hit(tree, content.view, x, y);
  // A popup's children are laid out relative to the popup, which is its surface's origin.
  auto it = tree.views().find(content.view);
  if (it == tree.views().end()) return -1;
  const auto& children = it->second.children;
  for (auto child = children.rbegin(); child != children.rend(); ++child) {
    Tag found = hit(tree, *child, x, y);
    if (found >= 0) return found;
  }
  return -1;
}

void PointerInput::handle(const OxyEvent& event) {
  // Protocol positions and distances are in 1/256 logical pixels.
  if (event.kind != OXY_EVENT_SCROLL && event.kind != OXY_EVENT_POINTER) return;
  auto content = surfaces_(event.surface);
  if (!content) return;
  if (event.kind == OXY_EVENT_SCROLL) {
    content_ = *content;
    return scroll(event.x / 256.0f, event.y / 256.0f);
  }
  float x = event.x / 256.0f, y = event.y / 256.0f;
  // A touch stays with the surface it started on.
  if (!down_) content_ = *content;
  x_ = x;
  y_ = y;
  if (event.detail == OXY_POINTER_ENTER || event.detail == OXY_POINTER_MOTION) hover(*content, x, y);
  if (event.detail == OXY_POINTER_LEAVE && event.surface == hoveredSurface_) hover(std::nullopt, x, y);
  switch (event.detail) {
    case OXY_POINTER_PRESS:
      if (event.code == kPrimaryButton && !down_) {
        target_ = hitTest(tree_, content_, x, y);
        if (target_ < 0) return;
        down_ = true;
        ++identifier_;
        dispatch(Phase::Start, x, y);
      }
      break;
    case OXY_POINTER_MOTION:
      if (down_) dispatch(Phase::Move, x, y);
      break;
    case OXY_POINTER_RELEASE:
      if (event.code == kPrimaryButton && down_) {
        dispatch(Phase::End, x, y);
        down_ = false;
      }
      break;
    case OXY_POINTER_LEAVE:
    case OXY_POINTER_CANCEL:
      if (down_) {
        dispatch(Phase::Cancel, x, y);
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
  // The innermost scroll view that can still move in the scroll's direction takes it, as nested
  // scroll views do on Android.
  for (auto at = path.rbegin(); at != path.rend(); ++at) {
    const auto& view = tree_.views().at(*at).view;
    if (std::string(view.componentName ? view.componentName : "") != "ScrollView" || !view.state) continue;
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
    if (std::getenv("OXY_HOST_TRACE")) {
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
  if (std::getenv("OXY_HOST_TRACE")) {
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

}  // namespace oxy
