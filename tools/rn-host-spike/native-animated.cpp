#include "native-animated.h"

#include "report.h"
#include "ui-thread.h"

#include <react/renderer/animated/AnimatedModule.h>
#include <react/renderer/animated/NativeAnimatedNodesManagerProvider.h>
#include <react/renderer/core/PropsParserContext.h>
#include <react/renderer/core/RawProps.h>
#include <react/renderer/scheduler/Scheduler.h>

#include <iostream>

using namespace facebook;
using namespace facebook::react;

namespace oxyos {

namespace {

// React Native's module, unchanged but for the one method its C++ implementation leaves empty
// (`updateAnimatedNodeConfig`, a platform colour's AnimatedColor given a new value): reported, as
// the host reports every call it has no counterpart for, instead of doing nothing silently.
class Module : public AnimatedModule {
 public:
  using AnimatedModule::AnimatedModule;

  jsi::Value create(jsi::Runtime& rt, const jsi::PropNameID& prop) override {
    if (prop.utf8(rt) == "updateAnimatedNodeConfig") {
      return jsi::Function::createFromHostFunction(rt, prop, 2, [](jsi::Runtime&, const jsi::Value&, const jsi::Value*, size_t) {
        static bool reported = false;
        if (!reported) {
          reported = true;
          std::cerr << "[host] unsupported on OxyOS: NativeAnimatedModule.updateAnimatedNodeConfig "
                       "(React Native's C++ Animated does not implement it)\n";
        }
        return jsi::Value::undefined();
      });
    }
    return AnimatedModule::create(rt, prop);
  }
};

}  // namespace

NativeAnimatedHost::NativeAnimatedHost(const DisplayRefresh& refresh)
    : refresh_(refresh), frames_(std::make_shared<Frames>()) {
  // The manager asks for frames when JavaScript sends it operations (on its thread) or an
  // animation starts, and stops when a frame finds nothing to animate (on the UI thread), as iOS's
  // animated module provider drives it: asked from JavaScript's thread, the loop is woken for them.
  // State the callbacks share with the host outlives either: the manager stops its frames as it
  // is destroyed, with the runtime.
  std::weak_ptr<Frames> frames = frames_;
  provider_ = std::make_shared<NativeAnimatedNodesManagerProvider>(
      [frames](std::function<void()>&& render, bool /*isAsync*/) {
        if (auto shared = frames.lock()) {
          std::lock_guard lock(shared->mutex);
          shared->render = std::make_shared<std::function<void()>>(std::move(render));
        }
        if (!UiThread::isCurrent()) UiThread::wake();
      },
      [frames](bool /*isAsync*/) {
        if (auto shared = frames.lock()) {
          std::lock_guard lock(shared->mutex);
          shared->render = nullptr;
        }
      });
}

std::shared_ptr<TurboModule> NativeAnimatedHost::module(const std::shared_ptr<CallInvoker>& invoker) {
  return std::make_shared<Module>(invoker, provider_);
}

namespace {
// When the native driver's frame being rendered was sampled (NativeAnimatedHost::tick), for the
// report's `animated` lines (updateViewProps).
thread_local std::optional<int64_t> tFrameSampledNs;
}  // namespace

bool NativeAnimatedHost::tick() {
  double now = nowMs();
  std::shared_ptr<std::function<void()>> render;
  {
    std::lock_guard lock(frames_->mutex);
    if (!frames_->render || now - frames_->last < refresh_.periodMs()) return false;
    frames_->last = now;
    // The frame may stop the frames (the last animation ended), which drops `render`: held here.
    render = frames_->render;
  }
  tFrameSampledNs = static_cast<int64_t>(now * 1e6);
  (*render)();
  tFrameSampledNs.reset();
  return true;
}

std::optional<std::chrono::steady_clock::time_point> NativeAnimatedHost::nextFrame() const {
  std::lock_guard lock(frames_->mutex);
  if (!frames_->render) return std::nullopt;
  double wait = std::max(0.0, frames_->last + refresh_.periodMs() - nowMs());
  return std::chrono::steady_clock::now() +
         std::chrono::duration_cast<std::chrono::steady_clock::duration>(std::chrono::duration<double, std::milli>(wait));
}

bool updateViewProps(ViewTree& tree, const Scheduler& scheduler, Tag tag, const folly::dynamic& props) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return false;
  const auto& view = it->second.view;
  const auto* descriptor = scheduler.findComponentDescriptorByHandle_DO_NOT_USE_THIS_IS_BROKEN(view.componentHandle);
  if (!descriptor) {
    std::cerr << "[host] no component descriptor for " << Report::name(view) << ": animated props not set\n";
    return false;
  }
  PropsParserContext context{view.surfaceId, *scheduler.getContextContainer()};
  auto updated = descriptor->cloneProps(context, view.props, RawProps(props));
  Report::get().animated(view, props, tFrameSampledNs);
  tree.setProps(tag, std::move(updated));
  return true;
}

}  // namespace oxyos
