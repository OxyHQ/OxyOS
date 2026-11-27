// OxyOS platform glue for react-native-worklets: the shared C++ core runs unchanged; this file
// supplies what each platform must (UI scheduling, animation frames, logging, networking) and the
// `WorkletsModule` TurboModule that installs the core's proxy into the React Native runtime.
#pragma once

#include "display-refresh.h"

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>
#include <jsi/jsi.h>

#include <atomic>
#include <functional>
#include <mutex>
#include <chrono>
#include <memory>
#include <optional>
#include <vector>

namespace worklets {
class UIScheduler;
class WorkletsModuleProxy;
class WorkletsModuleProxyInitializer;
}  // namespace worklets

namespace oxyos {

class WorkletsHost {
 public:
  // Its frames are the display's (`refresh`). `compiled` is the runtime's framework's worklets
  // compiled ahead (`framework/worklets.hbc`, sample/worklets.mjs), or "" for none. `onJsThread`
  // says whether the caller is React Native's JavaScript thread (js-thread.h): only there may a
  // worklet call into its runtime at once (worklets' JSScheduler).
  WorkletsHost(const DisplayRefresh& refresh, std::string compiled, std::function<bool()> onJsThread);
  ~WorkletsHost();

  std::shared_ptr<facebook::react::TurboModule> module(
      facebook::jsi::Runtime& runtime,
      const std::shared_ptr<facebook::react::CallInvoker>& invoker);

  // Runs pending UI-runtime jobs and, once per frame interval, requested animation frames, on the
  // UI thread (ui-thread.h), the host's loop: the UI runtime is its, as on Android's and iOS's.
  //
  // Worklets' frame loop asks for its next frame in every frame, and Reanimated runs its mappers
  // at the end of every frame, whether or not anything changed: run as asked, an application
  // doing nothing would wake at every refresh. A frame runs only when it is wanted: a frame
  // callback was asked for since the last frame (the UI runtime's requestAnimationFrame, which
  // worklets' timers use too, is watched), or something happened that may give the frame's
  // finalizers work (a UI job, or the host's own work: JavaScript, an event, a mount; the host
  // loop says so with wantFrame). Otherwise worklets' request waits, and the loop sleeps.
  // Runs queued UI jobs and the frame callbacks due (at most once a frame of the display); true if
  // it ran any.
  bool tick();
  // The next frame is wanted: what the host did this turn may change what it acts on.
  void wantFrame() { *wanted_ = true; }
  // When frame callbacks waiting for a wanted frame are next due, if any: the loop sleeps until then.
  std::optional<std::chrono::steady_clock::time_point> nextFrame() const;

  std::shared_ptr<worklets::WorkletsModuleProxy> proxy() const { return proxy_; }
  // Worklets' UI runtime, once made (null before).
  facebook::jsi::Runtime* uiRuntime() const;
  std::shared_ptr<worklets::UIScheduler> uiScheduler() const { return ui_; }

 private:
  const DisplayRefresh& refresh_;
  std::shared_ptr<worklets::UIScheduler> ui_;
  // The frame callbacks asked for: on the UI thread, or by a worklet JavaScript's thread runs at once.
  struct Frames {
    std::mutex mutex;
    std::vector<std::function<void(double)>> callbacks;
  };
  std::shared_ptr<Frames> frames_;
  // Whether the next frame is wanted (tick); shared with the UI runtime's requestAnimationFrame.
  std::shared_ptr<std::atomic<bool>> wanted_;
  // Whether the UI runtime's requests are watched (watchRequests): until they are, or if they
  // cannot be, every frame is wanted.
  std::atomic<bool> watching_ = false, watched_ = false;
  void watchRequests();
  // The UI runtime makes each worklet from what the framework compiled ahead, its text compiled
  // only for one it does not have (an application's own).
  void makeWorkletsCompiled(facebook::jsi::Runtime& ui);
  std::string compiled_;
  std::function<bool()> onJsThread_;
  std::shared_ptr<worklets::WorkletsModuleProxyInitializer> initializer_;
  std::shared_ptr<worklets::WorkletsModuleProxy> proxy_;
  double lastFrame_ = 0;
};

}  // namespace oxyos
