// OxyOS platform glue for react-native-worklets: the shared C++ core runs unchanged; this file
// supplies what each platform must (UI scheduling, animation frames, logging, networking) and the
// `WorkletsModule` TurboModule that installs the core's proxy into the React Native runtime.
#pragma once

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <jsi/jsi.h>

#include <functional>
#include <memory>
#include <vector>

namespace worklets {
class UIScheduler;
class WorkletsModuleProxy;
class WorkletsModuleProxyInitializer;
}  // namespace worklets

namespace oxy {

class WorkletsHost {
 public:
  WorkletsHost();
  ~WorkletsHost();

  std::shared_ptr<facebook::react::TurboModule> module(
      facebook::jsi::Runtime& runtime,
      const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // Runs pending UI-runtime jobs and, once per frame interval, requested animation frames.
  // The host has one thread: the UI runtime runs on it between React Native's tasks. Worklets
  // requests a frame every frame, so a frame is not work by itself: what an animation changes
  // reaches the host as a mount.
  void tick();

  std::shared_ptr<worklets::WorkletsModuleProxy> proxy() const { return proxy_; }
  std::shared_ptr<worklets::UIScheduler> uiScheduler() const { return ui_; }

 private:
  std::shared_ptr<worklets::UIScheduler> ui_;
  std::shared_ptr<std::vector<std::function<void(double)>>> frames_;
  std::shared_ptr<worklets::WorkletsModuleProxyInitializer> initializer_;
  std::shared_ptr<worklets::WorkletsModuleProxy> proxy_;
  double lastFrame_ = 0;
};

}  // namespace oxy
