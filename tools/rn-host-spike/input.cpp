#include "input.h"

#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/components/view/TouchEventEmitter.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteState.h>

#include <cstdlib>
#include <iostream>
#include <optional>
#include <string>

using namespace facebook::react;

namespace oxy {

namespace {

// Linux input code of the primary (left) button.
constexpr uint32_t kPrimaryButton = 272;

// Window-local origin of every view, as the renderer places it.
std::optional<Point> origin(const ViewTree& tree, Tag tag, Tag at, Point offset) {
  auto it = tree.views().find(at);
  if (it == tree.views().end()) return std::nullopt;
  const auto& view = it->second.view;
  Point position{offset.x + view.layoutMetrics.frame.origin.x, offset.y + view.layoutMetrics.frame.origin.y};
  if (at == tag) return position;
  Point children = position;
  if (std::string(view.componentName ? view.componentName : "") == "ScrollView" && view.state) {
    const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
    children.x -= state.contentOffset.x;
    children.y -= state.contentOffset.y;
  }
  for (Tag child : it->second.children) {
    if (auto found = origin(tree, tag, child, children)) return found;
  }
  return std::nullopt;
}

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

Tag hitTest(const ViewTree& tree, float x, float y) {
  return tree.root() < 0 ? -1 : hit(tree, tree.root(), x, y);
}

void PointerInput::handle(const OxyEvent& event) {
  if (event.kind != OXY_EVENT_POINTER) return;
  // Protocol positions are in 1/256 logical pixels.
  float x = event.x / 256.0f, y = event.y / 256.0f;
  switch (event.detail) {
    case OXY_POINTER_PRESS:
      if (event.code == kPrimaryButton && !down_) {
        target_ = hitTest(tree_, x, y);
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

void PointerInput::dispatch(Phase phase, float x, float y) {
  auto it = tree_.views().find(target_);
  if (it == tree_.views().end()) return;
  auto emitter = std::dynamic_pointer_cast<const TouchEventEmitter>(it->second.view.eventEmitter);
  if (!emitter) return;
  Point targetOrigin = origin(tree_, target_, tree_.root(), {0, 0}).value_or(Point{0, 0});
  Touch touch{};
  touch.pagePoint = {x, y};
  touch.screenPoint = {x, y};
  touch.offsetPoint = {x - targetOrigin.x, y - targetOrigin.y};
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
