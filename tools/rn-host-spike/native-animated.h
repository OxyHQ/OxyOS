// React Native's Animated with its native driver (`useNativeDriver: true`) on OxyOS: React Native's
// own C++ implementation (react/renderer/animated: `AnimatedModule`, `NativeAnimatedNodesManager`),
// as Android runs it with `cxxNativeAnimatedEnabled`. JavaScript builds the animated graph through
// the `NativeAnimatedModule` TurboModule; the host steps it once a frame on its loop (Android's
// Choreographer drives it through `driveCxxAnimations`), and the manager sets the props it computes
// on the mounted views between React's commits (`updateViewProps`, Android's mounting layer's
// `synchronouslyUpdateViewOnUIThread`). Layout props, and the last values of every finished
// animation, are committed to Fabric's shadow tree, and the props it manages are kept on the views
// React commits again (React Native's `AnimatedMountingOverrideDelegate`).
#pragma once

#include "renderer.h"

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <folly/dynamic.h>

#include <chrono>
#include <functional>
#include <memory>
#include <optional>

namespace facebook::react {
class NativeAnimatedNodesManagerProvider;
class Scheduler;
}  // namespace facebook::react

namespace oxyos {

class NativeAnimatedHost {
 public:
  NativeAnimatedHost();

  // The `NativeAnimatedModule` TurboModule. JavaScript asks for it once Fabric's UIManager is
  // installed in the runtime, which the manager it creates attaches to.
  std::shared_ptr<facebook::react::TurboModule> module(const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // Once per frame (at most 60 a second, as worklets' frames), while the manager asks for frames:
  // runs the operations JavaScript queued, steps the animations and commits their props. True if
  // it ran.
  bool tick();
  // When the next frame is due, while the manager asks for frames: the loop sleeps until then.
  std::optional<std::chrono::steady_clock::time_point> nextFrame() const;

 private:
  struct Frames {
    // The manager's frame (`onRender`), from when it asks for frames until it stops.
    std::function<void()> render;
    double last = 0;
  };
  std::shared_ptr<Frames> frames_;
  std::shared_ptr<facebook::react::NativeAnimatedNodesManagerProvider> provider_;
};

// Sets `props` on mounted view `tag` between commits, as Android's mounting layer updates a native
// view's props synchronously: they are parsed onto the view's props by its component descriptor,
// and the view is drawn, hit and damaged with them. False if the view is not mounted (its next
// mount carries them). Each update is reported (report.h's `animated`).
bool updateViewProps(ViewTree& tree, const facebook::react::Scheduler& scheduler, facebook::react::Tag tag,
                     const folly::dynamic& props);

}  // namespace oxyos
