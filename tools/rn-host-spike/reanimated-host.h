// OxyOS platform glue for react-native-reanimated, the counterpart of Reanimated's Android
// NativeProxy: the shared C++ core runs unchanged on the worklets UI runtime and commits animated
// props through Fabric.
#pragma once

#include "display-refresh.h"

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <jsi/jsi.h>

#include <folly/dynamic.h>

#include <chrono>
#include <functional>
#include <memory>
#include <optional>
#include <string>
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
  // Its frames are the display's (`refresh`).
  ReanimatedHost(WorkletsHost& worklets, std::function<facebook::react::Scheduler*()> scheduler,
                 const DisplayRefresh& refresh);
  ~ReanimatedHost();

  std::shared_ptr<facebook::react::TurboModule> module(
      facebook::jsi::Runtime& runtime,
      const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // Once per frame, when wanted: runs requested render callbacks, then lets Reanimated commit
  // its updates (and the React commits it paused). A frame is wanted when a render callback was
  // requested, or when the host loop says something happened (wantFrame) that may have given
  // Reanimated updates: a UI-runtime frame, an event, JavaScript.
  bool tick();
  void wantFrame() { wanted_ = true; }
  // When the next wanted frame is due, if one is.
  std::optional<std::chrono::steady_clock::time_point> nextFrame() const;
  // An event for the worklet event handlers of view `tag` (useEvent), as Reanimated's Android
  // NativeProxy receives react-native-gesture-handler's: `name` is the handler's ("onX").
  void event(const std::string& name, int tag, const folly::dynamic& payload);

 private:
  WorkletsHost& worklets_;
  std::function<facebook::react::Scheduler*()> scheduler_;
  const DisplayRefresh& refresh_;
  std::shared_ptr<reanimated::ReanimatedModuleProxy> proxy_;
  std::shared_ptr<std::vector<std::function<void(double)>>> renders_;
  double lastFrame_ = 0;
  bool wanted_ = true;
};

}  // namespace oxyos
