// OxyOS platform glue for react-native-reanimated, the counterpart of Reanimated's Android
// NativeProxy: the shared C++ core runs unchanged on the worklets UI runtime and commits animated
// props through Fabric.
#pragma once

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <jsi/jsi.h>

#include <functional>
#include <memory>
#include <vector>

namespace facebook::react {
class UIManager;
}
namespace reanimated {
class ReanimatedModuleProxy;
}

namespace oxy {

class WorkletsHost;

class ReanimatedHost {
 public:
  ReanimatedHost(WorkletsHost& worklets, std::function<std::shared_ptr<facebook::react::UIManager>()> uiManager);
  ~ReanimatedHost();

  std::shared_ptr<facebook::react::TurboModule> module(
      facebook::jsi::Runtime& runtime,
      const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // Once per frame: runs requested render callbacks, then lets Reanimated commit its updates.
  bool tick();

 private:
  WorkletsHost& worklets_;
  std::function<std::shared_ptr<facebook::react::UIManager>()> uiManager_;
  std::shared_ptr<reanimated::ReanimatedModuleProxy> proxy_;
  std::shared_ptr<std::vector<std::function<void(double)>>> renders_;
  double lastFrame_ = 0;
};

}  // namespace oxy
