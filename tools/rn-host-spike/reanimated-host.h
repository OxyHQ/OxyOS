// OxyOS platform glue for react-native-reanimated, the counterpart of Reanimated's Android
// NativeProxy: the shared C++ core runs unchanged on the worklets UI runtime and commits animated
// props through Fabric.
#pragma once

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <jsi/jsi.h>

#include <folly/dynamic.h>

#include <functional>
#include <string>
#include <memory>
#include <vector>

namespace facebook::react {
class Scheduler;
}
namespace reanimated {
class ReanimatedModuleProxy;
}

namespace oxyos {

class WorkletsHost;

class ReanimatedHost {
 public:
  // `scheduler` returns the Fabric scheduler, which exists before JavaScript installs the module.
  ReanimatedHost(WorkletsHost& worklets, std::function<facebook::react::Scheduler*()> scheduler);
  ~ReanimatedHost();

  std::shared_ptr<facebook::react::TurboModule> module(
      facebook::jsi::Runtime& runtime,
      const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // Once per frame: runs requested render callbacks, then lets Reanimated commit its updates.
  bool tick();
  // An event for the worklet event handlers of view `tag` (useEvent), as Reanimated's Android
  // NativeProxy receives react-native-gesture-handler's: `name` is the handler's ("onX").
  void event(const std::string& name, int tag, const folly::dynamic& payload);

 private:
  WorkletsHost& worklets_;
  std::function<facebook::react::Scheduler*()> scheduler_;
  std::shared_ptr<reanimated::ReanimatedModuleProxy> proxy_;
  std::shared_ptr<std::vector<std::function<void(double)>>> renders_;
  double lastFrame_ = 0;
};

}  // namespace oxyos
