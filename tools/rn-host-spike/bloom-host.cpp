// OxyOS React Native host: Hermes + bridgeless ReactInstance + Fabric on Linux. JavaScript runs on a
// thread of its own (js-thread.h), the host's loop on the main thread.
// Loads a Metro bundle built for the `oxyos` platform, starts a surface and records Fabric's
// mounting transactions into a retained view tree that the renderer draws into a buffer.
#include "accessibility.h"
#include "trace.h"
#include "android-components.h"
#include "application.h"
#include "damage.h"
#include "expo-blur.h"
#include "expo-image.h"
#include "files.h"
#include "frame-diagnostics.h"
#include "keyboard-controller.h"
#include "masked-view.h"
#include "gesture-handler.h"
#include "host-modules.h"
#include "turbo-module-cache.h"
#include "key-chords.h"
#include "keyboard-focus.h"
#include "images.h"
#include "input.h"
#include "js-thread.h"
#include "ui-thread.h"
#include "android-view-props.h"
#include "overscroll.h"
#include "intl.h"
#include "modal.h"
#include "network.h"
#include "websocket.h"
#include "popup.h"
#include "drag.h"
#include "safe-area.h"
#include "notifications.h"
#include "system.h"
#include "shell-state.h"
#include "shell-surface.h"
#include "svg.h"
#include "memory.h"
#include "scheduling.h"
#include "crash-report.h"
#include "report.h"
#include "skia-gpu.h"
#include "renderer.h"
#include "text.h"
#include "textinput.h"
#include "window.h"
#include "animation-frames.h"
#include "native-animated.h"
#include "reanimated-host.h"
#include "worklets-host.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <limits>
#include <hermes/hermes.h>
#include <jsi/jsi.h>
#include <cxxreact/JSBigString.h>
#include <folly/json.h>
#include <cxxreact/MessageQueueThread.h>
#include <react/featureflags/ReactNativeFeatureFlags.h>
#include <react/featureflags/ReactNativeFeatureFlagsDefaults.h>
#include <react/nativemodule/core/ReactCommon/TurboModuleBinding.h>
#include <react/nativemodule/defaults/DefaultTurboModules.h>
#include <react/renderer/componentregistry/ComponentDescriptorProviderRegistry.h>
#include <react/renderer/components/image/ImageComponentDescriptor.h>
#include <react/renderer/components/root/RootComponentDescriptor.h>
#include <react/renderer/components/view/BaseViewEventEmitter.h>
#include <react/renderer/components/scrollview/ScrollViewComponentDescriptor.h>
#include <react/renderer/components/text/ParagraphComponentDescriptor.h>
#include <react/renderer/components/text/RawTextComponentDescriptor.h>
#include <react/renderer/components/text/TextComponentDescriptor.h>
#include <react/renderer/components/unimplementedview/UnimplementedViewComponentDescriptor.h>
#include <react/renderer/components/view/ViewComponentDescriptor.h>
#include <react/renderer/core/EventBeat.h>
#include <react/renderer/imagemanager/ImageManager.h>
#include <react/renderer/mounting/MountingCoordinator.h>
#include <react/renderer/scheduler/Scheduler.h>
#include <react/renderer/scheduler/SchedulerDelegate.h>
#include <react/renderer/scheduler/SchedulerToolbox.h>
#include <react/renderer/runtimescheduler/RuntimeScheduler.h>
#include <react/renderer/scheduler/SurfaceHandler.h>
#include <react/runtime/JSRuntimeFactory.h>
#include <react/runtime/ReactInstance.h>
#include <react/runtime/TimerManager.h>
#include <react/utils/ContextContainer.h>

#include <atomic>
#include <chrono>
#include <deque>
#include <filesystem>
#include <optional>
#include <poll.h>
#include <cmath>
#include <csignal>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <map>
#include <mutex>
#include <set>
#include <queue>
#include <sstream>
#include <thread>
#include <csignal>
#include <execinfo.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <ucontext.h>
#include <unistd.h>

using namespace facebook;
using namespace facebook::react;

namespace {

using Clock = std::chrono::steady_clock;

class Flags : public ReactNativeFeatureFlagsDefaults {
 public:
  bool enableBridgelessArchitecture() override { return true; }
  // A desktop pointer hovers: Pressability's hover (onHoverIn/Out) is built on W3C pointer
  // events when this is on, and absent otherwise on React Native's Android path.
  bool shouldPressibilityUseW3CPointerEventsForHover() override { return true; }
  // Animated's native driver is React Native's C++ implementation (native-animated.h), as Android
  // runs it with this flag: JavaScript then talks to `NativeAnimatedModule` with batches.
  bool cxxNativeAnimatedEnabled() override { return true; }
};

// Fabric's event beat: what the host dispatched to views (input, layout, a scroll) reaches JavaScript
// when a beat is induced, as Android's Choreographer induces them. The loop induces them once it
// handled the compositor's events, JavaScript's thread as it is about to sleep (what its commits
// dispatched: onLayout): either queues the beat's callback on JavaScript's thread.
class Beat : public EventBeat {
 public:
  Beat(std::shared_ptr<OwnerBox> owner, RuntimeScheduler& scheduler)
      : EventBeat(std::move(owner), scheduler) {
    // Called by induce() when the beat was asked for: something was dispatched.
    unstable_setInduceCallback([] { tInduced = true; });
    std::lock_guard lock(mutex());
    live().insert(this);
  }
  ~Beat() override {
    std::lock_guard lock(mutex());
    live().erase(this);
  }

  // Induces every beat asked for; whether any was, which queues its callback on JavaScript's thread.
  static bool tickAll() {
    std::lock_guard lock(mutex());
    tInduced = false;
    for (auto* beat : live()) beat->induce();
    return tInduced;
  }
  // Something was dispatched to a view (an event, a state update) since the last call.
  void request() const override {
    EventBeat::request();
    requested() = true;
  }
  static bool takeRequested() { return requested().exchange(false); }
  static bool pendingRequest() { return requested().load(); }

 private:
  static inline thread_local bool tInduced = false;
  static std::atomic<bool>& requested() {
    static std::atomic<bool> any = false;
    return any;
  }
  static std::mutex& mutex() {
    static std::mutex mutex;
    return mutex;
  }
  static std::set<Beat*>& live() {
    static std::set<Beat*> beats;
    return beats;
  }
};

// Fabric's mounting layer (docs/adr/0067-the-ui-thread.mdx), as Android's: a commit's transaction is
// pulled on the thread that made it, once it is to be rendered (on JavaScript's thread at the end of
// the task that committed, so a layout effect's commit is shown with the commit before it; on the UI
// thread at once, for a commit made there: a window laid out at a new size, Reanimated's frame, a
// scroll view's offset), and handed to the UI thread, which applies it to the host's view tree. An
// accessibility event (AccessibilityInfo.sendAccessibilityEvent) goes into the same queue, behind the
// commits made before it, as Android's mounting layer runs its SendAccessibilityEventMountItem behind
// the mount items queued before it: the scheduler hands an accessibility event over at once, while it
// hands a commit over only at the end of its task, so an effect React flushes with its commit would
// otherwise be handled against the tree before that commit (docs/adr/0039-keyboard-access.mdx,
// decision 26: the menu bar's title focused by the same commit that asks for the keyboard). A view
// command (a text input's focus or blur, a scroll view's scrollTo) runs before the transactions
// waiting, as Android's MountItemDispatcher runs view commands first: JavaScript queued it against
// the views as they are, which the transactions after may remove (a focused field blurred as its
// popover closes). One for a view not mounted yet runs again once they are applied, as Android
// retries it. Everything is applied by the loop, in order.
class Mounter : public SchedulerDelegate {
 public:
  explicit Mounter(oxyos::ViewTree& tree) : tree_(tree) {}
  void attach(const Scheduler* scheduler) { scheduler_ = scheduler; }
  int transactions = 0;
  void schedulerDidFinishTransaction(const std::shared_ptr<const MountingCoordinator>& coordinator) override {
    std::lock_guard lock(mutex_);
    coordinators_[coordinator->getSurfaceId()] = coordinator;
    unpulled_ = true;
  }
  void schedulerShouldRenderTransactions(
      const std::shared_ptr<const MountingCoordinator>& coordinator) override {
    {
      std::lock_guard lock(mutex_);
      coordinators_[coordinator->getSurfaceId()] = coordinator;
      pullLocked();
    }
    // Made on the UI thread: applied at once, unless a scenario applies everything at the frame.
    if (oxyos::UiThread::isCurrent()) {
      if (!mountAtFrame) apply();
      return;
    }
    oxyos::UiThread::wake();
  }
  // A scenario's forced order (OXYOS_HOST_TEST_ORDER=focus-first): a commit made on the UI thread is
  // applied only at the loop's frame too.
  bool mountAtFrame = false;
  // Whether anything waits to be applied, and since when (the oldest).
  bool pending() {
    std::lock_guard lock(mutex_);
    return !queue_.empty() || !commands_.empty();
  }
  std::optional<std::chrono::steady_clock::time_point> waitingSince() {
    std::lock_guard lock(mutex_);
    if (queue_.empty() && commands_.empty()) return std::nullopt;
    return queuedAt_;
  }
  // Applies what was handed over, in order: the UI thread, or JavaScript's as it calls into the host
  // (host-modules.h's HostCall), the UI thread's state held either way. True if it mounted or handled
  // anything.
  bool apply() {
    std::deque<Item> items, commands;
    {
      std::lock_guard lock(mutex_);
      items.swap(queue_);
      commands.swap(commands_);
    }
    bool did = false;
    std::set<SurfaceId> mounted;
    // View commands first; one for a view not mounted yet once the transactions are (Android's retry).
    std::vector<Item> retried;
    for (auto& command : commands) {
      did = true;
      if (!tree_.views().count(command.view.tag)) {
        retried.push_back(std::move(command));
      } else if (onCommand) {
        onCommand(command.view, command.name, command.args);
      }
    }
    for (auto& item : items) {
      did = true;
      switch (item.kind) {
        case Item::Kind::Transaction:
          mount(item.surface, *item.transaction);
          mounted.insert(item.surface);
          appliedBatch_ = std::max(appliedBatch_, item.batch);
          break;
        case Item::Kind::Command:  // in commands_, run above
          break;
        case Item::Kind::AccessibilityEvent:
          if (onAccessibilityEvent) onAccessibilityEvent(item.view, item.name);
          break;
        case Item::Kind::Announcement:
          if (onAnnouncement) onAnnouncement(item.name);
          break;
      }
    }
    for (auto& command : retried) {
      if (tree_.views().count(command.view.tag) && onCommand) onCommand(command.view, command.name, command.args);
    }
    // Pulled to be applied later, a surface's transactions count as pending (hasPendingTransactions,
    // which IntersectionObserver's first paint waits on) until the last handed over is applied.
    if (!mounted.empty()) {
      std::lock_guard lock(mutex_);
      for (auto surface : mounted) {
        bool more = std::any_of(queue_.begin(), queue_.end(), [&](const Item& item) {
          return item.kind == Item::Kind::Transaction && item.surface == surface;
        });
        auto coordinator = coordinators_.find(surface);
        if (!more && coordinator != coordinators_.end()) coordinator->second->didPerformAsyncTransactions();
      }
    }
    return did;
  }
  // Whether the tree changed since the last call: a mount, or Animated's native driver.
  bool takeChanged() { return std::exchange(changed_, false); }
  // The batch of the loop's work JavaScript was running (js-thread.h) when it made the commits applied
  // so far, the latest: the frame showing them answers the inputs of that batch.
  uint64_t appliedBatch() const { return appliedBatch_; }
  // What JavaScript's thread runs now, as it pulls a commit.
  std::function<uint64_t()> runningBatch;
  // Raised as something is handed over, which the loop applies once JavaScript is idle: JavaScript's
  // thread then wakes the loop as it settles, whenever that is.
  std::atomic<bool>* wakeWhenIdle = nullptr;
  void schedulerShouldMergeReactRevision(SurfaceId) override {}
  void schedulerDidRequestPreliminaryViewAllocation(const ShadowNode&) override {}
  // A transaction about to be applied, its views still in the tree (a modal's last frame).
  std::function<void(const ShadowViewMutationList&)> beforeApply;
  // View commands from JavaScript (a text input's focus, blur, ...), for the host's native side.
  std::function<void(const ShadowView&, const std::string&, const folly::dynamic&)> onCommand;
  void schedulerDidDispatchCommand(const ShadowView& view, const std::string& name, const folly::dynamic& args) override {
    {
      std::lock_guard lock(mutex_);
      queued();
      commands_.push_back(Item{.kind = Item::Kind::Command, .view = view, .name = name, .args = args});
    }
    if (!oxyos::UiThread::isCurrent()) oxyos::UiThread::wake();
  }
  // Accessibility events from JavaScript: moving accessibility focus, for the host's accessibility
  // state.
  std::function<void(const ShadowView&, const std::string&)> onAccessibilityEvent;
  void schedulerDidSendAccessibilityEvent(const ShadowView& view, const std::string& type) override {
    queue(Item{.kind = Item::Kind::AccessibilityEvent, .view = view, .name = type});
  }
  // An announcement JavaScript makes (AccessibilityInfo.announceForAccessibility) while what it did in
  // an earlier batch is held back (holdsEarlierThan): behind that work, said after the focus it moved.
  std::function<void(const std::string&)> onAnnouncement;
  void announce(const std::string& text) { queue(Item{.kind = Item::Kind::Announcement, .name = text}); }
  // Whether the queue holds what JavaScript did in a batch before `batch` (js-thread.h's): work the
  // loop has not applied yet, handing JavaScript what came for it first (decision 2 of ADR 0067).
  bool holdsEarlierThan(uint64_t batch) {
    std::lock_guard lock(mutex_);
    return std::any_of(queue_.begin(), queue_.end(),
                       [batch](const Item& item) { return item.batch != 0 && item.batch < batch; });
  }
  void schedulerDidSetIsJSResponder(const ShadowView&, bool, bool) override {}
  // Animated's native driver setting a view's props between commits (native-animated.h), on the UI
  // thread as Android's mounting layer updates a native view: the tree changed, as with a mount.
  void schedulerShouldSynchronouslyUpdateViewOnUIThread(Tag tag, const folly::dynamic& props) override {
    if (scheduler_ && oxyos::updateViewProps(tree_, *scheduler_, tag, props)) changed_ = true;
  }
  void schedulerDidUpdateShadowTree(const std::unordered_map<Tag, folly::dynamic>&) override {}
  void schedulerDidCaptureViewSnapshot(Tag, SurfaceId) override {}
  void schedulerDidSetViewSnapshot(Tag, Tag, SurfaceId) override {}
  void schedulerDidClearPendingSnapshots() override {}

 private:
  struct Item {
    enum class Kind { Transaction, Command, AccessibilityEvent, Announcement } kind;
    SurfaceId surface = 0;
    // Pulled or sent on JavaScript's thread: the batch it ran (0 for the UI thread's).
    uint64_t batch = 0;
    std::shared_ptr<const MountingTransaction> transaction;
    ShadowView view;
    std::string name;
    folly::dynamic args = nullptr;
  };
  void queued() {
    if (queue_.empty() && commands_.empty()) queuedAt_ = std::chrono::steady_clock::now();
    if (wakeWhenIdle) *wakeWhenIdle = true;
  }
  // Pulls every surface's transaction into the queue, then what waited for it.
  void pullLocked() {
    for (auto& [surface, coordinator] : coordinators_) {
      // Reanimated's layout-animation proxy overrides a pull off the UI thread, so an empty
      // transaction is returned even when nothing was committed.
      auto transaction = coordinator->pullTransaction(/* willPerformAsynchronously */ true);
      if (transaction && !transaction->getMutations().empty()) {
        queued();
        queue_.push_back(Item{.kind = Item::Kind::Transaction,
                              .surface = surface,
                              .batch = !oxyos::UiThread::isCurrent() && runningBatch ? runningBatch() : 0,
                              .transaction = std::make_shared<const MountingTransaction>(std::move(*transaction))});
      }
    }
    unpulled_ = false;
    if (!waiting_.empty()) queued();
    for (auto& item : waiting_) queue_.push_back(std::move(item));
    waiting_.clear();
  }
  void queue(Item item) {
    if (!oxyos::UiThread::isCurrent() && runningBatch) item.batch = runningBatch();
    {
      std::lock_guard lock(mutex_);
      // Behind a commit made before it: one not yet handed over, which it waits for, or one handed
      // over and not yet applied, which it follows in the queue.
      bool behind = unpulled_ || std::any_of(queue_.begin(), queue_.end(), [](const Item& queued) {
        return queued.kind == Item::Kind::Transaction;
      });
      if (behind && item.kind == Item::Kind::AccessibilityEvent) oxyos::Report::get().input("a11y.event.queued", 0);
      if (unpulled_) {
        waiting_.push_back(std::move(item));
      } else {
        queued();
        queue_.push_back(std::move(item));
      }
    }
    if (!oxyos::UiThread::isCurrent()) oxyos::UiThread::wake();
  }
  void mount(SurfaceId surface, const MountingTransaction& transaction) {
    oxyos::FrameDiagnostics::Scope mounting(oxyos::FrameDiagnostics::Part::Mount);
    // Where its commit's time went (Fabric's telemetry): the commit, its layout (Yoga) and the text
    // measured in that layout.
    const auto& telemetry = transaction.getTelemetry();
    auto since = [](auto from, auto to) {
      return std::max(0.0, std::chrono::duration<double, std::milli>(to - from).count());
    };
    oxyos::FrameDiagnostics::get().committed(
        since(telemetry.getCommitStartTime(), telemetry.getCommitEndTime()),
        since(telemetry.getLayoutStartTime(), telemetry.getLayoutEndTime()),
        std::chrono::duration<double, std::milli>(telemetry.getTextMeasureTime()).count());
    if (beforeApply) beforeApply(transaction.getMutations());
    oxyos::Report::get().committed(transaction.getMutations());
    tree_.apply(transaction.getMutations());
    ++transactions;
    // As each platform's mounting layer does: report the mount, which runs the UIManager's mount
    // hooks. Reanimated resumes its own commits there.
    if (scheduler_) scheduler_->reportMount(surface);
    changed_ = true;
  }
  oxyos::ViewTree& tree_;
  const Scheduler* scheduler_ = nullptr;
  // The UI thread's state's, as the tree is.
  bool changed_ = false;
  uint64_t appliedBatch_ = 0;
  // Shared with the threads that commit and send. A pull holds it, so the queue has transactions in
  // the order the coordinators gave them.
  std::mutex mutex_;
  bool unpulled_ = false;
  std::deque<Item> queue_;
  // View commands, run before the queue (apply).
  std::deque<Item> commands_;
  std::chrono::steady_clock::time_point queuedAt_{};  // when the queue last stopped being empty
  std::vector<Item> waiting_;
  std::map<SurfaceId, std::shared_ptr<const MountingCoordinator>> coordinators_;
};

// What the UI thread hands JavaScript (docs/adr/0067-the-ui-thread.mdx, decision 5): device events,
// a frame's callbacks, a key to answer, a module's answer, Fabric's beat. Each is a task of the loop's
// turn on JavaScript's thread (JsThread::runOnQueue), which reaches React Native's scheduler there:
// what came while JavaScript worked is handed over together once it is idle, and React batches what
// it changes into one render, as when the loop and JavaScript took turns. From any other thread (a
// worklet's runOnJS on JavaScript's own, a decoder's) it goes to React Native at once.
// Keys typed together answered back to back (docs/adr/0067-the-ui-thread.mdx, decision 6): an offer's
// task, once its key is answered, waits for the loop to offer the next key of the same burst (the
// events held behind it) or to find none, so that JavaScript renders what the keys changed once,
// after the last, as when the loop asked JavaScript for each key's answer in its turn. Bounded: a
// loop that does not decide in time leaves JavaScript to go on.
class KeyBurst {
 public:
  // On JavaScript's thread, once key `key` is answered: whether another key of the burst follows.
  bool awaitDecision(uint64_t key) {
    std::unique_lock lock(mutex_);
    if (!changed_.wait_for(lock, kAtMost, [&] { return decided_ >= key; })) return false;
    return decided_ == key && another_;
  }
  // On the UI thread: what follows key `key` is decided, `another` key offered or none.
  void decided(uint64_t key, bool another) {
    {
      std::lock_guard lock(mutex_);
      if (key >= decided_) {
        decided_ = key;
        another_ = another;
      }
    }
    changed_.notify_all();
  }

 private:
  static constexpr auto kAtMost = std::chrono::milliseconds(50);
  std::mutex mutex_;
  std::condition_variable changed_;
  uint64_t decided_ = 0;
  bool another_ = false;
};

class ToJavaScript {
 public:
  ToJavaScript(ReactInstance& instance, oxyos::JsThread& thread) : instance_(instance), thread_(thread) {}

  void callFunctionOnModule(const std::string& module, const std::string& method, folly::dynamic args) {
    run([&instance = instance_, module, method, args = std::move(args)]() mutable {
      instance.callFunctionOnModule(module, method, std::move(args));
    });
  }
  // A state JavaScript is told whole (`RCTDeviceEventEmitter`'s `event` with `payload`: the session's
  // preferences, the displays, the window's size, a "read again" signal): one not yet delivered is
  // replaced by the newer, keeping its place, as Fabric coalesces a unique event (a scroll's). A
  // state that changes several times while JavaScript works reaches it once, as it is then.
  void tellState(const std::string& event, folly::dynamic payload) {
    if (!oxyos::UiThread::isCurrent()) {
      instance_.callFunctionOnModule("RCTDeviceEventEmitter", "emit", told(event, std::move(payload)));
      return;
    }
    {
      std::lock_guard lock(statesMutex_);
      auto [it, added] = states_.insert_or_assign(event, std::move(payload));
      if (!added) return;
    }
    run([this, event] {
      folly::dynamic payload;
      {
        std::lock_guard lock(statesMutex_);
        auto it = states_.find(event);
        if (it == states_.end()) return;
        payload = std::move(it->second);
        states_.erase(it);
      }
      instance_.callFunctionOnModule("RCTDeviceEventEmitter", "emit", told(event, std::move(payload)));
    });
  }
  // A task of React Native's scheduler, at `priority`.
  void schedule(SchedulerPriority priority, std::function<void(jsi::Runtime&)> task) {
    run([scheduler = instance_.getRuntimeScheduler(), priority, task = std::move(task)]() mutable {
      scheduler->scheduleTask(priority, std::move(task));
    });
  }
  // A runtime executor (React Native's buffered one) whose work, queued by the loop, is the loop's.
  RuntimeExecutor executor(RuntimeExecutor inner) {
    return [this, inner](std::function<void(jsi::Runtime&)>&& callback) {
      run([inner, callback = std::move(callback)]() mutable { inner(std::move(callback)); });
    };
  }
  // A call invoker (the modules', worklets' and Reanimated's) whose async calls, made by the loop,
  // are the loop's.
  std::shared_ptr<CallInvoker> invoker(std::shared_ptr<CallInvoker> inner) {
    return std::make_shared<Invoker>(*this, std::move(inner));
  }
  // Fabric's beat asked for by what the loop dispatched to views: induced on JavaScript's thread (it
  // queues the beat's callback there), handed over with the rest. Whether any was asked for.
  bool beat();
  // A key offered (Bloom's PlatformKeyboard listener, its answer awaited by the loop's input behind
  // it): run as soon as JavaScript's thread is free, before anything queued there (React's render
  // of what the key before changed among it), as the loop asked JavaScript for each key's answer in
  // its turn (docs/adr/0067-the-ui-thread.mdx, decision 6).
  void offerKey(std::function<void()> task) {
    handed_ = true;
    thread_.runOnQueueFirst(std::move(task));
  }
  // The loop's turn ends (or the offscreen settle's): what it handed over runs, once JavaScript is
  // idle.
  void handOver() { thread_.handOver(); }
  // Whether the loop queued anything for JavaScript since the last call.
  bool handed() { return std::exchange(handed_, false); }

 private:
  class Invoker : public CallInvoker {
   public:
    Invoker(ToJavaScript& to, std::shared_ptr<CallInvoker> inner) : to_(to), inner_(std::move(inner)) {}
    using CallInvoker::invokeAsync;
    using CallInvoker::invokeSync;
    void invokeAsync(CallFunc&& func) noexcept override {
      to_.run([inner = inner_, func = std::move(func)]() mutable { inner->invokeAsync(std::move(func)); });
    }
    void invokeAsync(SchedulerPriority priority, CallFunc&& func) noexcept override {
      to_.run([inner = inner_, priority, func = std::move(func)]() mutable {
        inner->invokeAsync(priority, std::move(func));
      });
    }
    void invokeSync(CallFunc&& func) override { inner_->invokeSync(std::move(func)); }

   private:
    ToJavaScript& to_;
    std::shared_ptr<CallInvoker> inner_;
  };
  void run(std::function<void()> task) {
    if (oxyos::UiThread::isCurrent()) {
      handed_ = true;
      thread_.runOnQueue(std::move(task));
    } else {
      task();
    }
  }
  bool handed_ = false;
  static folly::dynamic told(const std::string& event, folly::dynamic payload) {
    auto args = folly::dynamic::array(event);
    if (!payload.isNull()) args.push_back(std::move(payload));
    return args;
  }
  // The states told and not yet delivered, by event.
  std::mutex statesMutex_;
  std::map<std::string, folly::dynamic> states_;
  ReactInstance& instance_;
  oxyos::JsThread& thread_;
};

bool ToJavaScript::beat() {
  if (!Beat::takeRequested()) return false;
  run([] { Beat::tickAll(); });
  return true;
}

// A fatal signal's report in the session's log, and in the report file the session gave
// (crash-report.h): the signal and, for a bad access, the address it touched; the thread it hit (by
// name: the host's loop is the main one, named after the executable, JavaScript's `oxyos-js`, Hermes'
// collector `hades`); the registers at the
// fault, which tell a bad pointer from a bad index; and the stack, its frames' offsets to symbolise
// against the build's unstripped objects. Only async-signal-safe calls, into descriptors already
// open.
void crashed(int signal, siginfo_t* info, void* context) {
  void* frames[64];
  int count = backtrace(frames, 64);
  char thread[17] = {};
  prctl(PR_GET_NAME, thread, 0, 0, 0);
  const auto& machine = static_cast<const ucontext_t*>(context)->uc_mcontext;
  for (int fd : {2, oxyos::crashReportFd()}) {
    if (fd < 0) continue;
    // A fault the kernel raised has an address (si_code > 0); a signal sent by a process, abort()'s
    // among them, has its sender in the same storage instead.
    if (info->si_code > 0 && signal != SIGABRT) {
      dprintf(fd, "[host] fatal signal %d (code %d) at address %p, on thread %d '%s'\n", signal, info->si_code,
              info->si_addr, static_cast<int>(gettid()), thread);
    } else {
      dprintf(fd, "[host] fatal signal %d (code %d) sent by process %d, on thread %d '%s'\n", signal,
              info->si_code, static_cast<int>(info->si_pid), static_cast<int>(gettid()), thread);
    }
#if defined(__x86_64__)
    const auto* r = machine.gregs;
    dprintf(fd,
            "[host] rip %llx rsp %llx rbp %llx rax %llx rbx %llx rcx %llx rdx %llx rsi %llx rdi %llx r8 %llx r9 %llx "
            "r10 %llx r11 %llx r12 %llx r13 %llx r14 %llx r15 %llx\n",
            r[REG_RIP], r[REG_RSP], r[REG_RBP], r[REG_RAX], r[REG_RBX], r[REG_RCX], r[REG_RDX], r[REG_RSI],
            r[REG_RDI], r[REG_R8], r[REG_R9], r[REG_R10], r[REG_R11], r[REG_R12], r[REG_R13], r[REG_R14],
            r[REG_R15]);
#elif defined(__aarch64__)
    dprintf(fd, "[host] pc %llx sp %llx", machine.pc, machine.sp);
    for (int i = 0; i < 31; ++i) dprintf(fd, " x%d %llx", i, machine.regs[i]);
    dprintf(fd, "\n");
#endif
    backtrace_symbols_fd(frames, count, fd);
  }
  _exit(128 + signal);
}

// A hang's report (docs/adr/0067-hangs-and-force-quit.mdx, decision 4): the session asks with SIGQUIT
// before it force-quits a host that stopped answering, as Java's SIGQUIT dumps its threads. What the
// JavaScript thread (`oxyos-js`, js-thread.h; the main thread before it started) is doing is what
// hangs: the signal sent to the process, the first thread to take it notes who asked and hands it to
// that thread (tgkill), whose stack is then written, as the fatal signal's, into the log and the report
// file, and the host goes on (hung) until the session kills it. Only async-signal-safe calls, into
// descriptors already open. Hermes' JavaScript stack is not read: walking its frames from a signal is
// not safe while it runs.
std::atomic<int> gHangAsker{0};

void hung(int signal, siginfo_t* info, void*) {
  const pid_t process = getpid();
  const pid_t javascript = oxyos::jsThreadId();
  const pid_t target = javascript != 0 ? javascript : process;
  if (info->si_pid != process) gHangAsker.store(info->si_pid, std::memory_order_relaxed);
  if (gettid() != target) {
    syscall(SYS_tgkill, process, target, signal);
    return;
  }
  void* frames[64];
  int count = backtrace(frames, 64);
  for (int fd : {2, oxyos::crashReportFd()}) {
    if (fd < 0) continue;
    dprintf(fd, "[host] hang: asked by process %d (SIGQUIT); the JavaScript thread's stack:\n",
            gHangAsker.load(std::memory_order_relaxed));
    backtrace_symbols_fd(frames, count, fd);
  }
}

// The report above, installed for the process, also for a crash that overflowed a thread's stack: the
// handler runs on a stack of its own, the main thread's set here and JavaScript's by its thread
// (js-thread.h).
void reportFatalSignals() {
  static std::vector<char> stack(std::max<size_t>(SIGSTKSZ, 64 * 1024));
  stack_t alternate{};
  alternate.ss_sp = stack.data();
  alternate.ss_size = stack.size();
  sigaltstack(&alternate, nullptr);
  struct sigaction action {};
  action.sa_sigaction = crashed;
  action.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_RESETHAND;
  sigemptyset(&action.sa_mask);
  for (int signal : {SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT}) sigaction(signal, &action, nullptr);
  // A hang's report, asked again for each hang: not reset, on the same stack of its own.
  struct sigaction quit {};
  quit.sa_sigaction = hung;
  quit.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_RESTART;
  sigemptyset(&quit.sa_mask);
  sigaction(SIGQUIT, &quit, nullptr);
  // backtrace() loads its unwinder the first time it is called, which allocates: once now, so the
  // handlers never do.
  void* warm[1];
  backtrace(warm, 1);
}

// The runtime this host is (docs/adr/0030-installing-applications.mdx, decisions 2 and 17): the
// directory its executable is in, its runtime.json's numbers, and its framework, which every
// application's JavaScript runtime evaluates before the application's own bundle.
struct RuntimeFacts {
  int abi = 0;
  int api = 0;
  std::string framework;
  std::string frameworkURL;
  // Its framework's worklets compiled ahead (sample/worklets.mjs), "" for a framework built before
  // them: worklets' UI runtime makes each from it (worklets-host.h).
  std::string worklets;
  // The keyboard's focus ring's colour for each of Bloom's accents, light and dark (0xRRGGBB): its
  // `ring` token, as the framework's build took it from Bloom (`framework/focus-rings.json`). Empty
  // for a framework built before it had them.
  std::map<std::string, std::pair<uint32_t, uint32_t>> focusRings;
};

RuntimeFacts runtimeFacts() {
  char exe[4096];
  ssize_t length = readlink("/proc/self/exe", exe, sizeof exe - 1);
  if (length <= 0) throw std::runtime_error("cannot read /proc/self/exe");
  std::string directory(exe, static_cast<size_t>(length));
  directory = directory.substr(0, directory.rfind('/'));
  std::ifstream description(directory + "/runtime.json");
  if (!description) throw std::runtime_error(directory + " is not a runtime: it has no runtime.json");
  std::stringstream text;
  text << description.rdbuf();
  auto parsed = folly::parseJson(text.str());
  RuntimeFacts facts;
  facts.abi = static_cast<int>(parsed["abi"].asInt());
  facts.api = static_cast<int>(parsed["api"].asInt());
  facts.framework = directory + "/framework/framework.hbc";
  // The framework's URL is its source's, which its assets are beside; only its bytecode ships.
  facts.frameworkURL = "file://" + directory + "/framework/framework.js";
  if (std::ifstream(directory + "/framework/worklets.hbc").good()) facts.worklets = directory + "/framework/worklets.hbc";
  if (!std::ifstream(facts.framework).good()) {
    throw std::runtime_error("runtime " + std::to_string(facts.abi) + " has no framework: " + facts.framework);
  }
  if (std::ifstream rings(directory + "/framework/focus-rings.json"); rings) {
    std::stringstream json;
    json << rings.rdbuf();
    auto hex = [](const folly::dynamic& value) -> std::optional<uint32_t> {
      if (!value.isString() || value.asString().size() != 7 || value.asString()[0] != '#') return std::nullopt;
      return static_cast<uint32_t>(std::stoul(value.asString().substr(1), nullptr, 16));
    };
    try {
      auto parsed = folly::parseJson(json.str());
      for (const auto& [name, schemes] : parsed.items()) {
        auto light = schemes.isObject() ? hex(schemes.getDefault("light")) : std::nullopt;
        auto dark = schemes.isObject() ? hex(schemes.getDefault("dark")) : std::nullopt;
        if (name.isString() && light && dark) facts.focusRings[name.asString()] = {*light, *dark};
      }
    } catch (const std::exception& error) {
      std::cerr << "[host] the framework's focus rings: " << error.what() << "\n";
    }
  }
  return facts;
}

void stage(const char* name) {
  if (oxyos::tracing()) std::cerr << "[host] " << name << "\n";
  oxyos::Report::get().stage(name);
}

}  // namespace

// What the application's storage, the connectivity and the Oxy session answered (application.h,
// account-session.h), to JavaScript.
bool deliverApplication(oxyos::HostServices& services) {
  bool busy = services.storage && services.storage->deliver();
  if (services.connectivity) busy |= services.connectivity->deliver();
  if (services.accountSession) busy |= services.accountSession->deliver();
  if (services.migration) busy |= services.migration->deliver();
  return busy;
}

// Their descriptors, which wake the loop when they have something.
std::vector<int> applicationFds(const oxyos::HostServices& services) {
  std::vector<int> fds;
  if (services.storage) fds.push_back(services.storage->fd());
  if (services.connectivity && services.connectivity->fd() >= 0) fds.push_back(services.connectivity->fd());
  if (services.accountSession) {
    fds.push_back(services.accountSession->fd());
    if (int subscription = services.accountSession->subscriptionFd(); subscription >= 0) fds.push_back(subscription);
  }
  if (services.migration) fds.push_back(services.migration->fd());
  return fds;
}

int main(int argc, char** argv) {
  // Before anything allocates much or a thread starts (memory.h).
  oxyos::configureProcessMemory();
  // The UI thread's short slice, before any thread starts, so none inherits it (scheduling.h).
  oxyos::requestInteractiveSlice();
  // A pipe or socket whose reader went away (a paste's receiver, a server) is an error on that
  // write, never a signal that ends the app.
  std::signal(SIGPIPE, SIG_IGN);
  if (argc < 4) {
    std::cerr << "usage: oxyos-bloom-host BUNDLE.js MODULE (OUT.png | --window [WIDTH HEIGHT] | --shell OUTPUT)\n"
                 "  OUT.png   run until the app settles, then draw it into OUT.png\n"
                 "  --window  present it in a window on the compositor at $OXYOS_COMPOSITOR_SOCKET\n"
                 "  --shell   run it as the session's shell on OUTPUT (needs the shell grant): its root is the\n"
                 "            output's background, its OxyOSShellSurface regions panels and overlays\n";
    return 2;
  }
  // The session's report file first, so that a fault from here on is said in it (crash-report.h).
  oxyos::openCrashReport();
  reportFatalSignals();
  // The application this process runs, as its report and the session's log name it: the id the
  // session launched it with, else its module (the shell's always: an id in its environment is
  // whatever started it passing its own on).
  const char* launchedId = std::getenv("OXYOS_APP_ID");
  const bool asShell = std::string(argv[3]) == "--shell";
  const std::string applicationName = launchedId && *launchedId && !asShell ? launchedId : argv[2];
  oxyos::Report::get().open(applicationName);
  stage("main");
  const std::string bundlePath = argv[1], moduleName = argv[2], outPath = argv[3];
  // The session's shell (docs/adr/0009-session-shell.mdx) is a window of another kind.
  const bool shellMode = outPath == "--shell";
  // For a scenario only (regress.py's `host_order`), the order in which the keyboard reaching a
  // shell region and JavaScript's accessibility focus moving into it arrive, forced
  // (docs/adr/0039-keyboard-access.mdx, decision 26): "focus-first", the focus sent before the
  // commit asking for the keyboard is mounted and the keyboard asked for only once the region's
  // tree is published with that focus; "grant-first", the focus held until the compositor has
  // given the region the keyboard; "popup-focus-first", the compositor's keyboard moves (its focus
  // in and out) held from a menu's opening until its tree is published with JavaScript's
  // accessibility focus, as a loaded machine reads the keyboard's enter on a menu after Bloom's
  // move into it; "window-focus-first", an application window's (never the shell's) commits mounted
  // only at the host's frame, so a dialog's mount, the focus it asks for its default control
  // (Bloom's `initialFocus`) and the host following it coming fall in one frame, before the dialog
  // has measured itself and shown (decision 27). Unset, nothing is forced.
  const std::string testOrder = [] {
    const char* order = std::getenv("OXYOS_HOST_TEST_ORDER");
    return std::string(order ? order : "");
  }();
  const bool focusFirst = shellMode && testOrder == "focus-first";
  const bool grantFirst = shellMode && testOrder == "grant-first";
  const bool popupFocusFirst = shellMode && testOrder == "popup-focus-first";
  const bool windowFocusFirst = !shellMode && testOrder == "window-focus-first";
  const bool windowed = outPath == "--window" || shellMode;
  const uint32_t output = shellMode && argc > 4 ? static_cast<uint32_t>(std::stoul(argv[4])) : 1;
  const char* socket = std::getenv("OXYOS_COMPOSITOR_SOCKET");
  if (windowed && !socket) {
    std::cerr << "--window needs OXYOS_COMPOSITOR_SOCKET\n";
    return 2;
  }
  const float width = !shellMode && argc > 5 ? std::stof(argv[4]) : 1024;
  const float height = !shellMode && argc > 5 ? std::stof(argv[5]) : 640;

  // What the application's fonts lack: Latin, Greek and Cyrillic and symbols (DejaVu, Noto Sans),
  // Chinese, Japanese and Korean (Noto Sans CJK, Debian's fonts-noto-cjk: a collection of a face
  // for each language, its regular and bold, docs/adr/0041-input-methods.mdx), and emoji in colour
  // (Noto Color Emoji, fonts-noto-color-emoji). A file missing is skipped.
  for (const char* path : {"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                           "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                           "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                           "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
                           "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"}) {
    if (std::ifstream(path).good()) oxyos::TextEngine::shared().addFallback(path);
  }
  ReactNativeFeatureFlags::override(std::make_unique<Flags>());
  // This thread is the UI thread (ui-thread.h), and holds the host's state from now on but while it
  // waits: JavaScript's thread, started next, reaches that state only between.
  oxyos::UiThread::adopt();
  oxyos::UiThread::wholeLogLines();
  oxyos::UiThread::Lock uiState;
  // Set while the loop waits for JavaScript to settle (a resize's layout): its thread wakes the loop
  // once it is idle.
  std::atomic<bool> wakeUiWhenIdle{false};
  // JavaScript's thread (js-thread.h): React Native's queue and its timers. Each stretch of its work
  // is said by the frame diagnostics and the report, named by what the loop's turn queued it for.
  // Fabric's beat is induced only by the loop, at the end of a turn: what one turn of the
  // compositor's set off (its events, read whole) reaches JavaScript in one batch, never split
  // between two of its stretches.
  std::shared_ptr<oxyos::JsThread> js;
  try {
    js = std::make_shared<oxyos::JsThread>(
        [] {
          if (!oxyos::UiThread::isCurrent()) return oxyos::JsThread::Cause{};
          auto& diagnostics = oxyos::FrameDiagnostics::get();
          // A turn that mounted JavaScript's commit hands it what the mount set off (a layout's
          // events, an image loaded).
          if (diagnostics.currentRank() == 0) return oxyos::JsThread::Cause{"its commit", 0};
          return oxyos::JsThread::Cause{diagnostics.currentTrigger(), diagnostics.currentRank()};
        },
        [](const oxyos::JsThread::Stretch& stretch) {
          auto started = Clock::now() - std::chrono::duration_cast<Clock::duration>(
                                            std::chrono::duration<double, std::milli>(stretch.span.wallMs));
          oxyos::FrameDiagnostics::get().javascriptWorked(stretch.span, started, stretch.cause.what, stretch.tasks);
          oxyos::Report::get().javascript(stretch.span, /* onItsThread */ true);
        },
        nullptr,
        // The loop is woken as JavaScript settles while it waits for that, or to hand over what came
        // meanwhile (events dispatched as what JavaScript did was applied among it).
        [&wakeUiWhenIdle](bool waiting) {
          if (waiting || wakeUiWhenIdle || Beat::pendingRequest()) oxyos::UiThread::wake();
        });
  } catch (const std::exception& error) {
    std::cerr << "[host] " << error.what() << "\n";
    return 1;
  }
  // Each delivery to React says the batch it reaches JavaScript in (Report::countTurnsBy): what the
  // loop queues while JavaScript works waits for it in one, however many turns of the loop's queue it.
  oxyos::Report::get().countTurnsBy([thread = std::weak_ptr(js)] {
    auto running = thread.lock();
    return running ? running->batch() : 0;
  });
  auto timerRegistry = js->makeTimers();
  auto* timers = timerRegistry.get();
  // The instance holds the only reference to its timer manager, as Android's JReactInstance gives
  // it (its timer executor keeps a weak one): the timers JavaScript leaves pending hold the
  // runtime's values, so they must go with the instance, before its runtime. Kept here past it,
  // they were destroyed after the runtime freed what they point into, and the host jumped through
  // freed memory as it exited. JavaScript's thread fires them through it while the instance lives.
  auto ownedTimerManager = std::make_shared<TimerManager>(std::move(timerRegistry));
  TimerManager& timerManager = *ownedTimerManager;
  js->fireTimersThrough(timerManager);
  // OXYOS_HOST_JS_PROFILE=FILE: Hermes' sampling profiler records the application's JavaScript
  // (at OXYOS_HOST_JS_PROFILE_HZ samples a second, 1000 unless set) and the profile is written to
  // FILE as the host ends (docs/performance.mdx). A directory (a path ending in `/`, or one that
  // exists) gets every host's own, <application>-<pid>.cpuprofile.json, as OXYOS_HOST_REPORT's: the
  // shell and an application profiled in one run. Off, Hermes has no sampler.
  const char* jsProfileEnv = std::getenv("OXYOS_HOST_JS_PROFILE");
  const std::string jsProfilePath =
      jsProfileEnv && *jsProfileEnv ? oxyos::perProcessPath(jsProfileEnv, applicationName, ".cpuprofile.json") : "";
  const char* jsProfile = jsProfilePath.empty() ? nullptr : jsProfilePath.c_str();
  auto* hermesRoot = jsProfile ? jsi::castInterface<facebook::hermes::IHermesRootAPI>(facebook::hermes::makeHermesRootAPI())
                               : nullptr;
  if (hermesRoot) {
    const char* hz = std::getenv("OXYOS_HOST_JS_PROFILE_HZ");
    hermesRoot->enableSamplingProfiler(hz && std::atof(hz) > 0 ? std::atof(hz) : 1000);
  }
  // Written while the runtime lives: the samples name its functions.
  auto writeJsProfile = [&] {
    if (!hermesRoot) return;
    try {
      hermesRoot->dumpSampledTraceToFile(jsProfile);
      std::cerr << "[host] JavaScript profile written to " << jsProfile << "\n";
    } catch (const std::exception& error) {
      std::cerr << "[host] could not write the JavaScript profile to " << jsProfile << ": " << error.what() << "\n";
    }
    hermesRoot->disableSamplingProfiler();
    hermesRoot = nullptr;
  };
  // OXYOS_HOST_GC_SANITIZE=RATE: with a Hermes built with HERMESVM_SANITIZE_HANDLES (its own build
  // directory, docs/bloom-host-sanitizers.mdx), each allocation collects the young generation and
  // moves every object in it, with probability RATE (0 to 1): a value held past a collection
  // without a root, or freed while it is still reachable, then fails at once instead of rarely.
  // Any other Hermes ignores it.
  auto runtimeConfig =
      ::hermes::vm::RuntimeConfig::Builder().withMicrotaskQueue(true).withEnableSampleProfiling(jsProfile != nullptr);
  if (const char* rate = std::getenv("OXYOS_HOST_GC_SANITIZE"); rate && std::atof(rate) > 0) {
    runtimeConfig.withGCConfig(
        ::hermes::vm::GCConfig::Builder()
            .withSanitizeConfig(
                ::hermes::vm::GCSanitizeConfig::Builder().withSanitizeRate(std::min(std::atof(rate), 1.0)).build())
            .build());
  }
  // Made on the thread it runs on: Hermes' sampling profiler samples, and its stack's guard bounds,
  // the thread that makes the runtime.
  std::unique_ptr<JSIRuntimeHolder> runtime;
  js->runOnQueueSync(
      [&] { runtime = std::make_unique<JSIRuntimeHolder>(facebook::hermes::makeHermesRuntime(runtimeConfig.build())); });
  // Counted on JavaScript's thread.
  std::atomic<int> jsErrors = 0;
  // React Native's runtime, once its bundle is loading, and whether the bundle ran: set on its thread.
  std::atomic<jsi::Runtime*> hermesRuntime = nullptr;
  std::atomic<bool> bundleRan = false;
  auto instance = std::make_unique<ReactInstance>(
      std::move(runtime), js, std::move(ownedTimerManager),
      [&](jsi::Runtime&, const JsErrorHandler::ProcessedError& error) noexcept {
        ++jsErrors;
        std::ostringstream said;
        said << error.message << "\n";
        for (const auto& frame : error.stack) {
          said << "    at " << frame.methodName << " (" << frame.file.value_or("?") << ":"
               << frame.lineNumber.value_or(0) << ")\n";
        }
        // A fatal one ends the application, as React Native's release builds do (crash-report.h).
        if (error.isFatal) oxyos::endForFatalJavaScriptError(said.str());
        std::cerr << "JS error: " << said.str();
      });
  stage("instance created");
  // What the loop hands JavaScript, handed over a turn at a time.
  ToJavaScript toJavaScript(*instance, *js);
  timerManager.setRuntimeExecutor(instance->getBufferedRuntimeExecutor());
  // The display's refresh, as the compositor tells it for the window (display-refresh.h): every
  // frame below is paced by it.
  oxyos::DisplayRefresh refresh;
  // requestAnimationFrame, once a frame (animation-frames.h); made after the instance, so its
  // callbacks go before the runtime does.
  oxyos::AnimationFrames animationFrames(toJavaScript.executor(instance->getBufferedRuntimeExecutor()), refresh);

  oxyos::Display display{width, height, 1.0f};
  auto invoker = toJavaScript.invoker(instance->createJSCallInvoker());
  // Blobs and the network (network.h): answers reach JavaScript as RCTDeviceEventEmitter events,
  // from the loop below, each with its arguments as one array, as Android emits them.
  oxyos::BlobStore blobs;
  oxyos::Networking networking(blobs, [&toJavaScript](const std::string& event, folly::dynamic args) {
    toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(event, std::move(args)));
  });
  // The user's files (files.h, docs/adr/0028-file-access.mdx), for an application's window: the
  // application's own folders are its data and cache directories (application.h, ADR 0030), only
  // if the session started it, and its bundle's folder is read-only. The session's shell has them
  // too, for the desktop (docs/adr/0047-finder-drags-and-the-desktop.mdx, decision 2), its own
  // folders its package's; the login screen has none. Events reach JavaScript as
  // RCTDeviceEventEmitter events from the loop below.
  const auto application = oxyos::Application::launched(bundlePath, moduleName);
  std::error_code bundleError;
  auto bundleFile = std::filesystem::canonical(bundlePath, bundleError);
  // A host run as a shell is known by its package (the manifest beside its bundle).
  const auto package =
      shellMode && !bundleError ? oxyos::Application::package(bundleFile) : oxyos::Application{};
  const bool shellFiles = package.id == oxyos::kShellPackage;
  oxyos::Files files(
      [&toJavaScript](const std::string& event, folly::dynamic payload) {
        toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                       folly::dynamic::array(event, std::move(payload)));
      },
      shellFiles ? package : application, bundleError ? std::string() : bundleFile.parent_path().string());
  // WebSockets (websocket.h), their events emitted from the loop below as Android emits them, one
  // map each; destroyed with the runtime, which closes the sockets still open (1001).
  oxyos::WebSockets webSockets(networking, blobs, [&toJavaScript](const std::string& event, folly::dynamic body) {
    toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(event, std::move(body)));
  });
  oxyos::HostServices services{blobs, networking};
  services.webSockets = &webSockets;
  // The application it runs, as its launcher started it (application.h).
  services.application = application;
  // The machine's Oxy session, followed from now in an application the session launched (never
  // the shell or the login screen), so its first snapshot is held before JavaScript reads it
  // (account-session.h; OxyOSAccountSession's `available`).
  if (application.recorded && !shellMode) {
    services.accountSession = std::make_unique<oxyos::AccountSession>();
    services.accountSession->follow();
  }
  services.emit = [&toJavaScript](const std::string& event, folly::dynamic body) {
    toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(event, std::move(body)));
  };
  RuntimeFacts runtimeOf;
  try {
    runtimeOf = runtimeFacts();
  } catch (const std::exception& error) {
    std::cerr << "[host] " << error.what() << "\n";
    // Nothing of JavaScript's runs past what main destroys as it returns.
    js->quitSynchronous();
    return 1;
  }
  services.runtime = {runtimeOf.abi, runtimeOf.api, runtimeOf.frameworkURL};
  oxyos::WorkletsHost worklets(refresh, runtimeOf.worklets, [thread = js.get()] { return thread->isCurrent(); });
  std::unique_ptr<Scheduler> scheduler;
  // Reduce motion as the session last told it (its first events, before the window's configure),
  // read when JavaScript installs Reanimated, as Android's NativeProxy reads its animator scale.
  oxyos::ReanimatedHost reanimated(
      worklets, [&scheduler] { return scheduler.get(); }, refresh,
      [&services] { return services.settings.drawing && services.settings.drawing()["reduceMotion"].asBool(); });
  // Animated's native driver (native-animated.h).
  oxyos::NativeAnimatedHost animated(refresh);
  // react-native-gesture-handler (gesture-handler.h), made once the view tree exists.
  std::unique_ptr<oxyos::GestureHandlerHost> gestures;
  instance->initializeRuntime({}, [&](jsi::Runtime& rt) {
    // console.* in React Native writes through nativeLoggingHook(message, level).
    rt.global().setProperty(rt, "nativeLoggingHook", jsi::Function::createFromHostFunction(
        rt, jsi::PropNameID::forAscii(rt, "nativeLoggingHook"), 2,
        [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
          static const char* levels[] = {"log", "info", "warn", "error"};
          int level = count > 1 && args[1].isNumber() ? static_cast<int>(args[1].getNumber()) : 0;
          std::cerr << "[console." << levels[std::clamp(level, 0, 3)] << "] "
                    << (count > 0 ? args[0].toString(rt).utf8(rt) : "") << "\n";
          return jsi::Value::undefined();
        }));
    // ECMA-402's Intl, which Hermes has on Android from the platform (intl.h).
    oxyos::installIntl(rt);
    // For the report (report.h, react_commits): each commit of React's reconciler, told through
    // the hook React's renderers announce themselves and their commits to (its DevTools' global
    // hook), whether or not the commit changed a view. Only while a report is written.
    if (oxyos::Report::get().enabled()) oxyos::Report::get().watchReactCommits(rt);
    // In place of React Native's, which runs callbacks as a zero-delay timer.
    animationFrames.install(rt);
    // Each module made once for the runtime and kept with it, as React Native's platforms keep
    // them (turbo-module-cache.h).
    TurboModuleBinding::install(
        rt, oxyos::cachedTurboModules(
                [&, invoker](jsi::Runtime& runtime, const std::string& name) -> std::shared_ptr<TurboModule> {
                  // Made from the host's state, the UI thread's (ui-thread.h): held meanwhile, as each
                  // of the host's modules holds it while its methods run (host-modules.h).
                  oxyos::UiThread::Lock uiState;
                  if (name == "WorkletsModule") return worklets.module(runtime, invoker);
                  if (name == "ReanimatedModule") return reanimated.module(runtime, invoker);
                  if (name == "NativeAnimatedModule") return animated.module(invoker);
                  if (name == "RNGestureHandlerModule" && gestures) return gestures->module(runtime, invoker);
                  if (auto module = oxyos::createHostModule(name, invoker, display, services)) return module;
                  if (auto module = DefaultTurboModules::getTurboModule(name, invoker)) return module;
                  // Said, once, by the cache (turbo-module-cache.h).
                  return nullptr;
                }));
  });

  auto context = std::make_shared<ContextContainer>();
  context->insert(ImageManagerKey, std::make_shared<ImageManager>(context));
  context->insert(RuntimeSchedulerKey, std::weak_ptr<RuntimeScheduler>(instance->getRuntimeScheduler()));
  // The window's size, which a modal starts at (modal.h); set as the surface is laid out.
  auto modalScreen = std::make_shared<oxyos::ModalScreen>();
  context->insert(oxyos::kModalScreenKey, modalScreen);
  SchedulerToolbox toolbox{};
  toolbox.contextContainer = context;
  // Fabric's own (a surface started or stopped, React Native's bindings installed), in order with
  // what the instance schedules itself as it starts (its runtime initialized, the bundle loaded).
  toolbox.runtimeExecutor = instance->getBufferedRuntimeExecutor();
  // Fabric's bindings (`nativeFabricUIManager`) are installed before the bundle runs, as iOS
  // installs them: code that runs as the bundle loads can then reach them, as Animated's native
  // module does when an app makes an animated component at the top of a module.
  toolbox.bridgelessBindingsExecutor = instance->getUnbufferedRuntimeExecutor();
  // The descriptor registry keeps a reference to its provider registry (to resolve components
  // on demand), so the provider registry must outlive every Scheduler that uses it.
  auto providers = std::make_shared<ComponentDescriptorProviderRegistry>();
  providers->add(concreteComponentDescriptorProvider<RootComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<oxyos::ViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ParagraphComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RawTextComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<TextComponentDescriptor>());
  // React Native's ScrollView, keeping its Android overScrollMode (overscroll.h).
  providers->add(concreteComponentDescriptorProvider<oxyos::ScrollViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ImageComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<oxyos::TextInputComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSPopupViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSDropTargetViewComponentDescriptor>());
  // React Native's Modal, laid out at the window's size from its first rendering (modal.h).
  providers->add(concreteComponentDescriptorProvider<oxyos::ModalHostViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoBlurViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoBlurTargetViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoImageComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RNCMaskedViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<KeyboardControllerViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSShellSurfaceComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSWindowViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSMinimizeTargetComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSScrollTargetComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RNCSafeAreaProviderComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RNCSafeAreaViewComponentDescriptor>());
  oxyos::addSvgComponents(*providers);
  oxyos::addAndroidComponents(*providers);
  toolbox.componentRegistryFactory = [providers](const EventDispatcher::Weak& dispatcher,
                                                 const std::shared_ptr<const ContextContainer>& container) {
    auto registry = providers->createComponentDescriptorRegistry(ComponentDescriptorParameters{
        .eventDispatcher = dispatcher, .contextContainer = container, .flavor = nullptr});
    // Components without an OxyOS implementation mount as plain views and are reported.
    std::const_pointer_cast<ComponentDescriptorRegistry>(registry)->setFallbackComponentDescriptor(
        std::make_shared<UnimplementedViewComponentDescriptor>(ComponentDescriptorParameters{
            .eventDispatcher = dispatcher,
            .contextContainer = container,
            .flavor = std::make_shared<const std::string>("UnimplementedNativeView")}));
    return registry;
  };
  auto runtimeScheduler = instance->getRuntimeScheduler();
  toolbox.eventBeatFactory = [runtimeScheduler](std::shared_ptr<EventBeat::OwnerBox> owner) {
    return std::make_unique<Beat>(std::move(owner), *runtimeScheduler);
  };

  oxyos::ViewTree tree;
  Mounter mounter(tree);
  // What gesture handlers recognize reaches JavaScript as Android's module sends it: device
  // events, or Reanimated's event handlers for worklets.
  gestures = std::make_unique<oxyos::GestureHandlerHost>(
      tree, oxyos::GestureHandlerHost::Delivery{
                .deviceEvent =
                    [&toJavaScript](const std::string& name, folly::dynamic payload) {
                      toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                                     folly::dynamic::array(name, std::move(payload)));
                    },
                .reanimatedEvent = [&reanimated](const std::string& name, int view,
                                                 const folly::dynamic& payload) {
                  reanimated.event(name, view, payload);
                },
            });
  // What Image views show (images.h), loaded at the size they are drawn; remote ones fetched with
  // the application's network, cached in its cache directory.
  oxyos::Images images(tree, refresh);
  images.connect(networking.handle(), application.cacheDirectory);
  services.images = &images;
  stage("fabric scheduler");
  scheduler = std::make_unique<Scheduler>(toolbox, nullptr, &mounter);
  mounter.attach(scheduler.get());
  mounter.runningBatch = [thread = js.get()] { return thread->running(); };
  mounter.wakeWhenIdle = &wakeUiWhenIdle;
  mounter.mountAtFrame = focusFirst || windowFocusFirst;
  // In a window, the compositor's configure decides the size the app lays out at.
  std::unique_ptr<oxyos::Window> window;
  // Settings' requests to the session, numbered alike: preferences and displays (ADR 0018, 0019).
  uint32_t settingsSerial = 0;
  // As the session's shell: what it knows of windows and workspaces, told from the first event.
  oxyos::ShellState shellState(output);
  if (shellMode) shellState.followApplications();
  // Settings follows the session's applications as the shell does (docs/adr/0030-installing-
  // applications.mdx): Open at Login and Notifications name each as the registry does, one installed,
  // updated or removed while Settings is open included (docs/adr/0060-login-items-and-background-
  // activity.mdx). Watched once the window is granted `settings`.
  std::unique_ptr<OxyOSApplicationsWatch, void (*)(OxyOSApplicationsWatch*)> settingsApplications{
      nullptr, oxyos_applications_watch_free};
  bool settingsApplicationsAsked = false;
  // The registry's listing as Settings was last told to read it (oxyos_applications' JSON).
  std::string toldApplications;
  auto applicationsListing = [] {
    int64_t length = oxyos_applications(nullptr, 0);
    if (length < 0) return std::string();
    std::string json(static_cast<size_t>(length) + 1, '\0');
    oxyos_applications(json.data(), json.size());
    json.resize(static_cast<size_t>(length));
    return json;
  };
  if (windowed) {
    // Ganesh's compiled pipelines are kept in the application's cache directory (skia-gpu.h); the
    // shell's and the login screen's, by their package's id (the manifest beside the bundle).
    // No manifest (a probe run as a shell): nothing is kept.
    std::string cacheDirectory = shellMode ? package.cacheDirectory : application.cacheDirectory;
    if (!cacheDirectory.empty()) oxyos::setPipelineCacheDirectory(cacheDirectory + "/skia");
    try {
      window = shellMode ? std::make_unique<oxyos::Window>(socket, oxyos::Window::Shell{.output = output})
                         : std::make_unique<oxyos::Window>(socket, static_cast<uint32_t>(width),
                                                           static_cast<uint32_t>(height));
      while (!window->configured()) {
        if (!window->dispatch([&shellState](const OxyOSEvent& event) { shellState.observe(event); })) {
          std::cerr << "[host] the compositor closed the connection before configuring the window\n";
          js->quitSynchronous();
          return 1;
        }
        oxyos::UiThread::Unlocked waiting;
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
      }
      // The compositor's pings are answered once JavaScript took what came before them (below).
      window->answerPingsItself();
      // The painter is started now, beside the runtime and the application loading.
      window->choosePainter();
      oxyos::Memory::get().setBufferTrim([&window] { return window->trimBuffers(); });
      oxyos::Memory::get().addRuntime("worklets", [&worklets] { return worklets.uiRuntime(); });
      // Where the window's frames spend their time, in the session's log (frame-diagnostics.h).
      oxyos::FrameDiagnostics::get().start(applicationName);
      window->resized();
      display = oxyos::Display{window->width(), window->height(), window->scale(),
                               window->drawing().dark ? oxyos::Display::kDarkWindowBackground
                                                      : oxyos::Display::kLightWindowBackground};
      // An application's window is named as its launcher says (the session's launch records;
      // the development compositor's scenarios), for the shell.
      const char* title = std::getenv("OXYOS_APP_TITLE");
      const char* appId = std::getenv("OXYOS_APP_ID");
      if (!shellMode && (title || appId)) window->nameWindow(title ? title : "", appId ? appId : "");
      services.applicationWindow = !shellMode;
      if (!shellMode || shellFiles) services.files = &files;
      // A file the session started it to open (its launch record's) is the person's to give.
      if (const char* opened = std::getenv("OXYOS_LAUNCH_URL"); !shellMode && opened &&
                                                                std::string_view(opened).starts_with("file:")) {
        files.give(opened);
      }
      services.shellWindow = shellMode;
      // Drag and drop between applications (docs/adr/0035-drag-and-drop.mdx): protocol version 10.
      services.drag.available = window->protocolVersion() >= 10;
      // The system's interface sounds (docs/adr/0044-system-sounds.mdx): protocol version 23, an
      // application's window (the session plays the shell's dock's own).
      services.soundsAvailable = !shellMode && window->protocolVersion() >= 23;
      // Search (docs/adr/0051-search.mdx): protocol version 31, every window and the shell.
      services.search.available = window->protocolVersion() >= 30;
      // A file chooser the session started (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): what
      // it shows, known before JavaScript starts, so Bloom's adapter is the chooser's from the start.
      if (const char* request = shellMode ? nullptr : std::getenv("OXYOS_FILE_CHOOSER")) {
        services.fileChooser.request = request;
      }
      // The session's preferences (docs/adr/0019-settings.mdx), from the connection's first events.
      services.settings = {
          .drawing = [&window] {
            const auto& drawing = window->drawing();
            return folly::dynamic::object("dark", drawing.dark)("reduceMotion", drawing.reduceMotion)(
                "reduceTransparency", drawing.reduceTransparency)("accent", drawing.accent)("caretBlink",
                                                                                          drawing.caretBlink);
          },
          .values = [&window]() -> folly::dynamic {
            if (!window->settingsGranted()) return nullptr;
            folly::dynamic values = folly::dynamic::object;
            for (const auto& [key, value] : window->preferences()) values[key] = value;
            return values;
          },
          // Only Settings may ask: the session gave it the preferences with its grant. Anything
          // else would be refused by the compositor, which closes the connection.
          .set = [&window, &settingsSerial](const std::string& key, const std::string& value) -> uint32_t {
            if (!window->settingsGranted()) return 0;
            ++settingsSerial;
            return window->setPreference(settingsSerial, key, value) ? settingsSerial : 0;
          },
          // The displays (docs/adr/0018-several-displays.mdx), as the session tells Settings them.
          .displays = [&window]() -> folly::dynamic {
            if (!window->settingsGranted()) return nullptr;
            folly::dynamic displays = folly::dynamic::array;
            for (const auto& [output, display] : window->displays()) {
              folly::dynamic modes = folly::dynamic::array, scales = folly::dynamic::array;
              for (const auto& mode : display.modes) {
                modes.push_back(folly::dynamic::object("width", mode.width)("height", mode.height)(
                    "refreshMhz", mode.refreshMhz)("preferred", mode.preferred));
              }
              for (uint32_t scale : display.scales) scales.push_back(scale);
              displays.push_back(folly::dynamic::object("output", output)("name", display.name)(
                  "builtin", display.builtin)("primary", display.primary)("x", display.x)("y", display.y)(
                  "width", display.width)("height", display.height)("scale", display.scale120)("mode", display.mode)(
                  "modes", std::move(modes))("scales", std::move(scales))("recommended", display.recommended)(
                  "widthMm", display.widthMm)("heightMm", display.heightMm));
            }
            const auto& trial = window->trial();
            return folly::dynamic::object("displays", std::move(displays))(
                "trial", trial.seconds ? folly::dynamic(folly::dynamic::object("output", trial.output)(
                                             "seconds", trial.seconds))
                                       : folly::dynamic(nullptr));
          },
          .display = [&window, &settingsSerial](const std::string& change,
                                                const std::vector<double>& numbers) -> uint32_t {
            if (!window->settingsGranted()) return 0;
            auto at = [&](size_t i) { return i < numbers.size() ? numbers[i] : 0.0; };
            auto whole = [&](size_t i) { return static_cast<uint32_t>(std::max(0.0, at(i))); };
            uint32_t serial = ++settingsSerial;
            bool sent = false;
            if (change == "mode" && numbers.size() == 4) {
              sent = window->setDisplayMode(serial, whole(0), whole(1), whole(2), whole(3));
            } else if (change == "scale" && numbers.size() == 2) {
              sent = window->setDisplayScale(serial, whole(0), whole(1));
            } else if (change == "origin" && numbers.size() == 3) {
              sent = window->setDisplayOrigin(serial, whole(0), static_cast<int32_t>(at(1)), static_cast<int32_t>(at(2)));
            } else if (change == "primary" && numbers.size() == 1) {
              sent = window->setPrimaryDisplay(serial, whole(0));
            } else if (change == "confirm" && numbers.size() == 1) {
              sent = window->confirmDisplays(serial, at(0) != 0);
            }
            return sent ? serial : 0;
          },
      };
    } catch (const std::exception& error) {
      std::cerr << "[host] " << error.what() << "\n";
      js->quitSynchronous();
      return 1;
    }
  }

  // What React Native's runtime holds, read on its thread (memory.h).
  oxyos::Memory::get().addRuntime(
      "hermes", [&hermesRuntime] { return hermesRuntime.load(); },
      [thread = js.get()](std::function<void()> task) { thread->runOnQueue(std::move(task)); });
  stage("load script");
  // The framework and the application's bundle are mapped from their files, never copied: the
  // framework's pages are the same for every application running on this runtime. The framework
  // is evaluated first, in the same turn (React Native's `beforeLoad`), so the application's
  // modules find React Native, React and Bloom defined, and React Native initialized.
  std::shared_ptr<const jsi::Buffer> framework = JSBigFileString::fromPath(runtimeOf.framework);
  instance->loadScript(
      JSBigFileString::fromPath(bundlePath), bundlePath,
      [framework, url = runtimeOf.frameworkURL, &hermesRuntime](jsi::Runtime& runtime) {
        // The application's runtime lives as long as the instance, which outlives the loop.
        hermesRuntime = &runtime;
        stage("framework");
        runtime.evaluateJavaScript(framework, url);
        stage("bundle");
      },
      [&bundleRan](jsi::Runtime&) { bundleRan = true; });

  stage("surface");
  SurfaceHandler surface(moduleName, 1);
  scheduler->registerSurface(surface);
  auto layOut = [&] {
    Size size{display.width, display.height};
    modalScreen->set(size);
    surface.constraintLayout(LayoutConstraints{.minimumSize = size, .maximumSize = size},
                             LayoutContext{.pointScaleFactor = display.scale});
  };
  layOut();
  surface.start();

  if (window) {
    // Serve the compositor until it closes the connection: lay out for each configured size,
    // and present whenever the tree changed and the previous frame was answered.
    // The first frame is the first mounted tree, never an empty window.
    bool dirty = false;
    // The GPU's painter has just taken over: the next frame is drawn whole.
    bool painterChanged = false;
    int status = 0;
    oxyos::AccessibilityBridge accessibility(tree, *window);
    oxyos::ShellSurfaceHost shellSurfaces(tree, *window, accessibility);
    // A popup's parent is the surface its anchor is drawn in: the root's, or a shell region's.
    oxyos::PopupHost popups(tree, *window, accessibility,
                            [&](facebook::react::Tag anchor) -> std::pair<uint32_t, facebook::react::Point> {
                              if (auto surface = shellSurfaces.surfaceOf(anchor)) {
                                if (auto content = shellSurfaces.content(*surface)) return {*surface, content->origin};
                              }
                              return {oxyos::Window::kToplevel, {}};
                            });
    oxyos::SafeAreas safeAreas;
    // React Native's Modal: laid out at the window's size, told it is shown, closed by Escape.
    oxyos::ModalHost modalHost(tree);
    oxyos::PointerInput pointer(tree, [&](uint32_t surface) -> std::optional<oxyos::SurfaceContent> {
      if (surface == oxyos::Window::kToplevel) return oxyos::SurfaceContent{.surface = surface, .view = tree.root()};
      if (auto content = shellSurfaces.content(surface)) return content;
      return popups.content(surface);
    });
    pointer.takesPointerModifiers(window->protocolVersion() >= 8);
    pointer.locatesSurfaces([&](facebook::react::Tag view) {
      uint32_t surface = popups.surfaceOf(view);
      if (surface == oxyos::Window::kToplevel) surface = shellSurfaces.surfaceOf(view).value_or(surface);
      return surface;
    });
    // Drags over this client's surfaces reach the drop targets under them, and its own drags start
    // from the press it holds (drag.h).
    oxyos::DragDrop dragDrop(tree, *window, [&](uint32_t surface) -> std::optional<oxyos::SurfaceContent> {
      if (surface == oxyos::Window::kToplevel) return oxyos::SurfaceContent{.surface = surface, .view = tree.root()};
      if (auto content = shellSurfaces.content(surface)) return content;
      return popups.content(surface);
    });
    // The window's drag (Bloom's useWindowDrag, the adapter's drag.watchWindow): a drag coming over
    // this client's surfaces and leaving them, told as a device event.
    dragDrop.watchWindow([&services](const folly::dynamic& event) { services.emit("OxyOSWindowDrag", event); });
    dragDrop.givesFiles([&files](const std::string& uri) { files.give(uri); });
    // While a popup is open, the pointer over the surface it hangs from reaches only the views it
    // tracks: a menu bar's titles (popup.h).
    pointer.admitsHover([&](uint32_t surface, facebook::react::Tag view) { return popups.tracks(surface, view); });
    oxyos::TextEditing editing(tree, [&](facebook::react::Tag view) -> std::optional<oxyos::Placement> {
      auto frame = oxyos::windowFrame(tree, view);
      if (!frame) return std::nullopt;
      uint32_t surface = popups.surfaceOf(view);
      if (surface == oxyos::Window::kToplevel) surface = shellSurfaces.surfaceOf(view).value_or(surface);
      // A popup's or a shell region's content is drawn relative to its surface's origin.
      if (surface != oxyos::Window::kToplevel) {
        auto content = popups.content(surface);
        if (!content) content = shellSurfaces.content(surface);
        if (!content) return std::nullopt;
        frame->origin.x -= content->origin.x;
        frame->origin.y -= content->origin.y;
      }
      return oxyos::Placement{.surface = surface, .frame = *frame};
    });
    // What an assistive technology's actions beyond activation and focus do (ADR 0003): scroll a
    // scroll view as the wheel does, close the node's modal (as Escape does) or popup (as the
    // compositor's dismissal does), select in an input; and what a node's snapshot shows of them.
    accessibility.use({
        .scrollRange = [&pointer](facebook::react::Tag tag) { return pointer.scrollRange(tag); },
        .scroll = [&pointer](facebook::react::Tag tag, float dx, float dy, uint32_t surface) {
          return pointer.scrollView(tag, dx, dy, surface);
        },
        .dismiss = [&](uint32_t surface, facebook::react::Tag tag) {
          for (auto at : oxyos::ancestry(tree, tag)) {
            const auto& view = tree.views().at(at).view;
            if (oxyos::isModal(view.componentName ? view.componentName : "")) return modalHost.requestClose();
          }
          if (!popups.content(surface)) return false;
          OxyOSEvent closed{};
          closed.kind = OXYOS_EVENT_POPUP_DONE;
          closed.surface = surface;
          popups.dismissed(closed);
          return true;
        },
        .select = [&editing](facebook::react::Tag tag, size_t start, size_t end) {
          return editing.setSelection(tag, start, end);
        },
        .selection = [&editing](facebook::react::Tag tag) { return editing.selection(tag); },
        .text = [&editing](facebook::react::Tag tag) { return editing.text(tag); },
    });
    // The pointer's shape over each surface, as the views under it want it.
    pointer.showsShapes([&window](uint32_t surface, uint8_t shape) { window->setPointerShape(surface, shape); });
    pointer.pacedBy(refresh);
    pointer.scrollsText([&editing](facebook::react::Tag view, float dx, float dy) {
      return editing.scrollText(view, dx, dy);
    });
    // A scroll no scroll view took, to the OxyOSScrollTarget around the pointer (the dock's icons,
    // docs/adr/0045-app-expose.mdx), with the number JavaScript gave it: { target, dx, dy, finger,
    // end } in logical pixels, `end` once a touchpad's fingers lift.
    pointer.scrollsTargets([&tree, &toJavaScript](facebook::react::Tag view, float dx, float dy, bool finger, bool end) {
      auto props = std::dynamic_pointer_cast<const facebook::react::OxyOSScrollTargetProps>(
          tree.views().at(view).view.props);
      if (!props) return;
      folly::dynamic payload =
          folly::dynamic::object("target", props->target)("dx", dx)("dy", dy)("finger", finger)("end", end);
      oxyos::Report::get().input("scroll.target", 0, payload);
      toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                     folly::dynamic::array("OxyOSScrollTarget", payload));
    });
    // The touch also goes to gesture handlers; one activating takes it from React Native and from
    // a text input's drag selection.
    pointer.feedsGestures([&gestures](int phase, const oxyos::SurfaceContent& content, float x, float y) {
      gestures->pointer(static_cast<oxyos::GestureHandlerHost::Phase>(phase), content, x, y);
    });
    pointer.setScale(display.scale);
    pointer.feedsFingers([&gestures](uint8_t action, size_t index, const oxyos::SurfaceContent& content,
                                     const std::vector<facebook::react::Point>& fingers) {
      gestures->fingers(action, index, content, fingers);
    });
    gestures->eventTimeMs = [&pointer] { return pointer.eventTimeMs(); };
    gestures->onCancelResponder = [&pointer] { pointer.cancelResponder(); };
    gestures->onIntercept = [&editing] { editing.releaseDrag(); };
    // Selected text drags out of an input as text, through the platform's drag, and every
    // editable input takes text dropped on it (docs/adr/0037-text-editing.mdx).
    editing.useDrag({.start = [&dragDrop](const std::string& text, std::optional<oxyos::DragImage> image,
                                          std::function<void(std::optional<std::string>)> done) {
      return dragDrop.start(oxyos::dragContents(folly::dynamic::object("text", text)),
                            OXYOS_DRAG_COPY | OXYOS_DRAG_MOVE, std::move(image), std::move(done));
    }});
    dragDrop.editsText(editing);
    dragDrop.takesPress([&pointer] { pointer.cancelResponder(); });
    editing.useClipboard({
        .copy = [&window](uint64_t serial, const std::string& text) { window->copyText(serial, text); },
        .copySensitive = [&window](uint64_t serial, const std::string& text) {
          window->copySensitiveText(serial, text);
        },
        .paste = [&window](uint64_t serial, std::function<void(std::string)> done) {
          window->pasteText(serial, std::move(done));
        },
        .hasText = [&window] { return window->clipboardHasText(); },
    });
    // A secondary click or a Control-click on a text input: its context menu (ADR 0037).
    pointer.opensTextMenus([&editing](facebook::react::Tag view, uint32_t surface, float x, float y) {
      editing.contextMenu(view, surface, x, y);
    });
    auto focusName = [&]() -> std::string {
      auto it = tree.views().find(editing.focused());
      return it == tree.views().end() ? std::string() : oxyos::Report::name(it->second.view);
    };
    // Compositor input as received, for the report (motion is left out: it is continuous).
    auto report = [&](const OxyOSEvent& event) {
      auto& r = oxyos::Report::get();
      if (!r.enabled()) return;
      static const char* pointer[] = {"", "pointer.enter", "pointer.leave", "", "pointer.press", "pointer.release",
                                      "pointer.cancel"};
      if (event.kind == OXYOS_EVENT_POINTER && event.detail != OXYOS_POINTER_MOTION && event.detail <= 6) {
        r.input(pointer[event.detail], event.surface);
      } else if (event.kind == OXYOS_EVENT_KEY) {
        r.input(event.flag ? "key.press" : "key.release", event.surface, folly::dynamic::object("code", event.code));
      } else if (event.kind == OXYOS_EVENT_TEXT) {
        r.input("text", event.surface, folly::dynamic::object("text", editing.logged(std::string(event.text))));
      } else if (event.kind == OXYOS_EVENT_PREEDIT) {
        r.input("preedit", event.surface,
                folly::dynamic::object("text", editing.logged(std::string(event.text)))("cursor", event.detail));
      } else if (event.kind == OXYOS_EVENT_SCROLL) {
        // Its source: a wheel's notch, a touchpad's fingers (their stop as they lift), or another
        // continuous device.
        static const char* source[] = {"", "wheel", "finger", "continuous"};
        r.input("scroll", event.surface,
                folly::dynamic::object("dx", event.x / 256.0)("dy", event.y / 256.0)(
                    "source", event.detail <= 3 ? source[event.detail] : "")("stop", event.flag != 0));
      } else if (event.kind == OXYOS_EVENT_HOLD && event.detail >= OXYOS_PINCH_BEGIN &&
                 event.detail <= OXYOS_PINCH_CANCEL) {
        static const char* hold[] = {"", "hold.begin", "", "hold.end", "hold.cancel"};
        if (*hold[event.detail]) r.input(hold[event.detail], event.surface, folly::dynamic::object("fingers", event.code));
      } else if (event.kind == OXYOS_EVENT_FOCUS) {
        r.input(event.flag ? "focus.in" : "focus.out", event.surface);
      } else if (event.kind == OXYOS_EVENT_ACCESSIBILITY_ACTION) {
        r.input("a11y.action", event.surface, folly::dynamic::object("index", event.code)("action", event.detail));
      } else if (event.kind == OXYOS_EVENT_POPUP_DONE) {
        r.input("popup.done", event.surface, folly::dynamic::object("reason", event.detail));
      }
    };
    // The latest press or key press: the input a popup opened by it cites for its grab.
    uint64_t lastPress = 0;
    // A command chosen in a text input's context menu cites the row's press or key.
    editing.useInputSerial([&lastPress] { return lastPress; });
    // The surface with the compositor's keyboard focus (0: none of this client's), and the text
    // field a shell region's surface was last asked to take it for (docs/adr/0024-system-shortcuts.mdx).
    uint32_t keyboardSurface = 0;
    facebook::react::Tag focusAskedFor = -1;
    // The region asking for the keyboard (its `keyboard` prop: the top bar driven from the keyboard),
    // the surface it was last asked for, and whether the keyboard reached it (the region or one of
    // its popups) since.
    uint32_t regionAskedFor = 0;
    bool regionHeldKeyboard = false;
    // The region that gave the keyboard back (the bar left), until the keyboard is on it again:
    // JavaScript's accessibility focus moves into it are declined meanwhile (releaseFocus below).
    uint32_t releasedRegion = 0;
    // A scenario's "focus-first" order: the region's request for the keyboard waits for its focus.
    bool keyboardHeldForFocus = false;
    // expo-clipboard's reads and writes carry the serial of the latest key or press the app
    // received (ADR 0007): the compositor refuses them unless the app has focus and that input is
    // recent. JavaScript hears when the clipboard changes.
    // The batch of JavaScript's that last asked for a drag (JsThread::running), held by the host's
    // state's lock (ui-thread.h).
    uint64_t dragAskedIn = 0;
    services.drag.start = [&dragDrop, &dragAskedIn, &wakeUiWhenIdle, &js](
                              const folly::dynamic& data, const std::vector<std::string>& actions, int preview,
                              int count, const folly::dynamic& options,
                              std::function<void(std::optional<std::string>)> done,
                              std::function<void(const std::string&)> cancelled) {
      auto vanish = options.isObject() ? options.getDefault("vanish", false) : folly::dynamic(false);
      // It starts on the UI thread once the batch asking has run and what it committed is mounted
      // (the loop, below), JavaScript's thread waking the loop as it settles.
      dragAskedIn = std::max(dragAskedIn, js->running());
      wakeUiWhenIdle = true;
      dragDrop.start(data, actions, preview, count, vanish.isBool() && vanish.asBool(), std::move(done),
                     std::move(cancelled));
    };
    // The system's interface sounds (Bloom's usePlatformSounds, docs/adr/0044-system-sounds.mdx),
    // citing the latest input the application received, as a Mac's Finder sounds for the person's
    // own action.
    services.playSystemSound = [&window, &lastPress, &services](const std::string& name) {
      uint32_t sound = name == "move-to-trash"  ? OXYOS_SYSTEM_SOUND_MOVE_TO_TRASH
                       : name == "empty-trash" ? OXYOS_SYSTEM_SOUND_EMPTY_TRASH
                                               : 0;
      if (sound == 0 || lastPress == 0 || !window || !services.soundsAvailable) return false;
      bool sent = window->playSystemSound(sound, lastPress);
      oxyos::Report::get().input("sound.play", oxyos::Window::kToplevel,
                                 folly::dynamic::object("sound", name)("sent", sent));
      return sent;
    };
    services.clipboard = {
        .copy = [&window, &lastPress](const std::string& text) { return window->copyText(lastPress, text); },
        .copySensitive = [&window, &lastPress](const std::string& text) {
          return window->copySensitiveText(lastPress, text);
        },
        .paste = [&window, &lastPress](std::function<void(std::string)> done) {
          window->pasteText(lastPress, std::move(done));
        },
        .hasText = [&window] { return window->clipboardHasText(); },
    };
    // react-native-keyboard-controller's dismiss: the focused input loses focus, as Android's
    // clears it.
    services.blurFocusedInput = [&editing, &tree] {
      auto focused = tree.views().find(editing.focused());
      if (focused != tree.views().end()) editing.command(focused->second.view, "blur", folly::dynamic::array());
    };
    // What announces: the lock screen's surface while the shell shows one, as only it is heard
    // while the session is locked (docs/adr/0010-lock-screen.mdx); else the window.
    auto announcer = [&shellSurfaces] { return shellSurfaces.lockSurface().value_or(oxyos::Window::kToplevel); };
    // An announcement is what the application says now: it interrupts what is being said, as
    // an announcement event does TalkBack's queue.
    // Said at once, as the one thread said it before the frame showing what JavaScript just did; but
    // while what JavaScript did in an earlier batch is held back (the input that came for it handed
    // first), behind that work (Mounter::announce), and out once the frame showing it published its
    // accessibility (announceAfterFrame): a focus moved by a press is said before what JavaScript
    // announces as the session answers it.
    services.announce = [&window, &mounter, &js, announcer](const std::string& text) {
      if (!oxyos::UiThread::isCurrent() && mounter.holdsEarlierThan(js->running())) {
        mounter.announce(text);
        return;
      }
      window->announce(announcer(), OXYOS_ASSERTIVE, text);
      oxyos::Report::get().input("announce", announcer(), folly::dynamic::object("text", text));
    };
    // The application's own window (docs/adr/0016-windows.mdx): its controls and drag regions act
    // through it. A close asked by the application ends it as the shell's close request does.
    bool closeAsked = false;
    folly::dynamic toldWindowState = nullptr;
    // The shell's state as JavaScript was last told it (OxyOSShellChanged): the compositor repeats
    // what has not changed (a window's state, its size, the battery each time it is read), and
    // only a change is told, as the window's state above.
    folly::dynamic toldShellState = nullptr;
    // The preferences as JavaScript was last told them (docs/adr/0019-settings.mdx).
    std::optional<oxyos::Window::Drawing> toldDrawing = window ? std::optional(window->drawing()) : std::nullopt;
    // Reduce transparency as the session told it at connect: expo-blur's views blur nothing.
    if (window) oxyos::setReducedTransparency(window->drawing().reduceTransparency);
    std::map<std::string, std::string> toldPreferences;
    if (window) toldPreferences = window->preferences();
    // The session's locale as JavaScript was last told it (docs/adr/0040-language-and-region.mdx):
    // the one told at connect is Intl's default before the bundle runs, and the report says it.
    std::string toldLocale = window ? window->locale() : std::string();
    if (!toldLocale.empty()) {
      oxyos::Report::get().input("locale", oxyos::Window::kToplevel, folly::dynamic::object("locale", toldLocale));
    }
    folly::dynamic toldDisplays = nullptr;
    if (!shellMode) {
      services.window = {
          .state = [&window] {
            return folly::dynamic::object("active", window->active())("maximized", window->maximized())(
                "fullscreen", window->fullscreen());
          },
          .close = [&closeAsked] {
            closeAsked = true;
            oxyos::FrameDiagnostics::get().closeAsked("the application");
          },
          .minimize = [&window] {
            oxyos::FrameDiagnostics::get().minimizeAsked();
            window->minimize();
          },
          .zoom = [&window] {
            oxyos::FrameDiagnostics::get().zoomAsked(!window->maximized());
            window->setMaximized(!window->maximized());
          },
          // From the press being handled: JavaScript asks while it handles that press, so the
          // latest press is it, and the compositor checks it is still held.
          .startMove = [&window, &lastPress] {
            if (lastPress == 0) return false;
            window->beginMove(lastPress);
            return true;
          },
          // Bloom declares the sizes as its layout mounts, and clears them as it unmounts, after
          // the window is gone.
          .setSizeLimits = [&window](uint32_t minWidth, uint32_t minHeight, uint32_t maxWidth, uint32_t maxHeight) {
            if (window) window->setSizeLimits(minWidth, minHeight, maxWidth, maxHeight);
          },
      };
    }
    // The focused text input as the window's first responder for the Edit menu's standard actions
    // (docs/adr/0021-menu-bar.mdx, decision 3): Bloom's `editing`, told each change below.
    auto responderState = [&editing]() -> folly::dynamic {
      auto now = editing.responder();
      if (!now) return nullptr;
      return folly::dynamic::object("undo", now->undo)("redo", now->redo)("cut", now->cut)("copy", now->copy)(
          "paste", now->paste)("pasteAndMatchStyle", now->pasteAndMatchStyle)("delete", now->deleteSelection)(
          "selectAll", now->selectAll);
    };
    std::optional<std::optional<oxyos::TextEditing::Commands>> toldResponder;
    if (!shellMode) {
      services.editing = {
          .state = responderState,
          .perform = [&editing](const std::string& action) { return editing.performAction(action); },
      };
    }
    // The window's menus (docs/adr/0021-menu-bar.mdx): each change a new commit, its serial
    // rising, which a choice in the shell's bar names.
    if (!shellMode) {
      services.menuBar = {
          .setMenus = [&window, serial = uint32_t{0}](const std::vector<oxyos::MenuEntry>& entries) mutable {
            std::vector<OxyOSMenuItem> items;
            items.reserve(entries.size());
            for (const auto& entry : entries) {
              items.push_back(OxyOSMenuItem{.parent = entry.parent,
                                            .kind = entry.kind,
                                            .flags = entry.flags,
                                            .role = entry.role,
                                            .label = entry.label.empty() ? nullptr : entry.label.c_str(),
                                            .shortcut = entry.shortcut.empty() ? nullptr : entry.shortcut.c_str()});
            }
            // Bloom describes its menus as none when it unmounts, after the window is gone.
            if (!window) return uint32_t{0};
            ++serial;
            window->setMenus(serial, items);
            oxyos::Report::get().input("menus.set", oxyos::Window::kToplevel,
                                       folly::dynamic::object("serial", serial)("entries", entries.size()));
            return serial;
          },
      };
    }
    // The Control Center's preference changes (the appearance, Do Not Disturb), numbered together:
    // each PREFERENCE_RESULT names one.
    uint32_t controlSerial = 0;
    // Launches and the URLs the shell opens are numbered together: each LAUNCH_RESULT names one.
    uint32_t launchSerial = 0;
    // An application's requests to open a URL (the platform's Settings at a page, or another
    // application's, docs/adr/0058-the-platform-api.mdx, step 6), by their serial, each told what
    // the session answered.
    std::map<uint32_t, std::function<void(const std::string&, const std::string&)>> urlOpens;
    if (!shellMode) {
      services.openUrl = [&window, &lastPress, &launchSerial, &urlOpens](
                             const std::string& url,
                             std::function<void(const std::string&, const std::string&)> done) {
        if (!window) return done("unavailable", "OxyOS opens a URL only from an application's window");
        // A file is opened by its type, never as a URL: the compositor closes the connection for one.
        if (!oxyos::urlOpensByScheme(url)) {
          return done("invalid", oxyos::urlScheme(url) == "file" ? "OxyOS opens a file by its type, never as a URL"
                                                                 : "it names no scheme");
        }
        // Settings' since version 27; any other scheme's since 34: an older compositor closes the
        // connection for it.
        std::string scheme = oxyos::urlScheme(url);
        uint32_t needs = scheme == OXYOS_PLATFORM_SETTINGS_SCHEME ? OXYOS_APPLICATION_OPENS_SETTINGS_VERSION
                                                                  : OXYOS_APPLICATION_OPENS_URLS_VERSION;
        if (window->protocolVersion() < needs) {
          return done("unavailable", "this session cannot open " + scheme + " links for an application (protocol version " +
                                 std::to_string(window->protocolVersion()) + ")");
        }
        if (lastPress == 0) return done("needsInput", "OxyOS opens a URL only for something the person did");
        ++launchSerial;
        bool sent = window->openUrl(launchSerial, lastPress, url);
        oxyos::Report::get().input("url.open", 0, folly::dynamic::object("url", url)("sent", sent));
        if (!sent) return done("unavailable", "the request to open it could not be sent");
        urlOpens[launchSerial] = std::move(done);
      };
      // The lock from an application (the Oxy SDK's sign in, docs/adr/0020-oxy-account-login.mdx):
      // only while its window is the active one, which the compositor checks too.
      services.lockSession = [&window]() {
        if (!window || window->protocolVersion() < OXYOS_APPLICATION_LOCKS_VERSION || !window->active()) return false;
        window->lockSession();
        oxyos::Report::get().input("session.lock", 0, folly::dynamic::object("sent", true));
        return true;
      };
    }
    if (shellMode) {
      services.shell = {
          .state = [&shellState] { return shellState.state(); },
          .manage = [&window](uint8_t action, uint32_t target, uint32_t workspace) {
            window->manage(action, target, workspace);
          },
          .lock = [&window] { window->lockSession(); },
          .launch = [&window, &launchSerial](const std::string& id) -> uint32_t {
            ++launchSerial;
            return window->launch(launchSerial, id) ? launchSerial : 0;
          },
          .authenticate = [&window, &shellSurfaces](const std::string& secret, const std::string& account,
                                                    bool passcode) {
            auto surface = shellSurfaces.lockSurface();
            return surface && window->authenticate(*surface, secret, account, passcode);
          },
          .activateMenuItem = [&window](uint32_t target, uint32_t serial, uint32_t index) {
            window->activateMenuItem(target, serial, index);
          },
          // The Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx).
          .setControlLevel =
              [&window](uint8_t control, uint32_t level) {
                oxyos::Report::get().input("control.set", 0, folly::dynamic::object("control", control)("level", level));
                window->setControlLevel(control, level);
              },
          .setControlMuted =
              [&window](uint8_t control, bool muted) {
                oxyos::Report::get().input("control.mute", 0, folly::dynamic::object("control", control)("muted", muted));
                window->setControlMuted(control, muted);
              },
          .setWifiEnabled =
              [&window](bool enabled) {
                oxyos::Report::get().input("control.wifi", 0, folly::dynamic::object("enabled", enabled));
                window->setWifiEnabled(enabled);
              },
          .setAppearance = [&window, &controlSerial](bool dark) {
            oxyos::Report::get().input("control.appearance", 0, folly::dynamic::object("dark", dark));
            window->setAppearance(++controlSerial, dark);
          },
          .refreshControls = [&window] { window->refreshControls(); },
          // A notification pressed in a banner or the Notification Center (ADR 0026), citing the
          // press (or key) it was pressed with.
          .activateNotification = [&window, &lastPress](uint32_t notification, uint8_t action,
                                                         const std::string& reply) {
            window->activateNotification(notification, action, lastPress, reply);
            oxyos::Report::get().input("notification.activate", 0,
                                       folly::dynamic::object("notification", notification)("action", action));
          },
          .removeNotification = [&window](uint32_t notification) { window->withdrawNotification(notification); },
          // Do Not Disturb, the Control Center's (docs/adr/0026-notifications.mdx): a preference
          // change as the appearance's, numbered with them.
          .setDoNotDisturb = [&window, &controlSerial](bool enabled) {
            oxyos::Report::get().input("control.do_not_disturb", 0, folly::dynamic::object("enabled", enabled));
            window->setDoNotDisturb(++controlSerial, enabled);
          },
          .announce = [&window, announcer](const std::string& text, bool polite) {
            window->announce(announcer(), polite ? OXYOS_POLITE : OXYOS_ASSERTIVE, text);
          },
          // Screenshots (docs/adr/0024-system-shortcuts.mdx, decision 10): the capture cites the
          // interface's serial, the opening of a saved one the press on its thumbnail.
          .capture = [&window](uint64_t serial, uint8_t kind, uint32_t output, const OxyOSRect& rect,
                               uint32_t target, uint8_t destination) {
            bool sent = window->capture(serial, kind, output, rect, target, destination);
            oxyos::Report::get().input(
                "capture", 0,
                folly::dynamic::object("kind", kind)("output", output)("window", target)("destination", destination)(
                    "rect", folly::dynamic::array(rect.x, rect.y, rect.width, rect.height))("sent", sent));
            return sent;
          },
          .endCapture = [&window](uint64_t serial) {
            bool sent = window->endCapture(serial);
            oxyos::Report::get().input("capture.end", 0, folly::dynamic::object("sent", sent));
            return sent;
          },
          .openScreenshot = [&window, &lastPress](uint32_t screenshot) {
            bool sent = window->openScreenshot(screenshot, lastPress);
            oxyos::Report::get().input("screenshot.open", 0,
                                       folly::dynamic::object("screenshot", screenshot)("sent", sent));
            return sent;
          },
          // The candidate window (docs/adr/0041-input-methods.mdx): a press on a candidate or a
          // chevron, which the compositor passes to the input method.
          .chooseCandidate = [&window](uint8_t index) {
            bool sent = window->chooseCandidate(index);
            oxyos::Report::get().input("candidates.choose", 0, folly::dynamic::object("index", index)("sent", sent));
            return sent;
          },
          .turnCandidatesPage = [&window](bool forward) {
            bool sent = window->turnCandidatesPage(forward);
            oxyos::Report::get().input("candidates.page", 0,
                                       folly::dynamic::object("forward", forward)("sent", sent));
            return sent;
          },
          // The dock's application menu (docs/adr/0034-the-dock.mdx): a list's change is a
          // preference change, numbered with the Control Center's; a URL cites the press that chose
          // it, numbered with the launches.
          .forceQuitApplication = [&window](uint32_t target) {
            bool sent = window->forceQuitApplication(target);
            oxyos::Report::get().input("application.force_quit", 0,
                                       folly::dynamic::object("window", target)("sent", sent));
            return sent;
          },
          .layOutWindow = [&window](uint32_t target, uint8_t layout) {
            bool sent = window->layOutWindow(target, layout);
            oxyos::Report::get().input("window.lay_out", 0,
                                       folly::dynamic::object("window", target)("layout", layout)("sent", sent));
            return sent;
          },
          .setApplicationList = [&window, &controlSerial](uint32_t list, const std::vector<std::string>& ids) {
            std::string text;
            for (const auto& id : ids) text += (text.empty() ? "" : "\n") + id;
            bool sent = window->setApplicationList(++controlSerial, list, text);
            folly::dynamic told = folly::dynamic::array();
            for (const auto& id : ids) told.push_back(id);
            oxyos::Report::get().input("application.list", 0,
                                       folly::dynamic::object("list", list)("ids", told)("sent", sent));
            return sent;
          },
          .openUrl = [&window, &lastPress, &launchSerial](const std::string& url) -> uint32_t {
            ++launchSerial;
            bool sent = window->openUrl(launchSerial, lastPress, url);
            oxyos::Report::get().input("url.open", 0, folly::dynamic::object("url", url)("sent", sent));
            return sent ? launchSerial : 0;
          },
          .emptyTrash = [&window, &lastPress] {
            bool sent = window->emptyTrash(lastPress);
            oxyos::Report::get().input("trash.empty", 0, folly::dynamic::object("sent", sent));
            return sent;
          },
          .trashFiles = [&window, &lastPress](const std::vector<std::string>& uris) {
            std::string list;
            for (const auto& uri : uris) list += uri + "\n";
            bool sent = window->trashFiles(lastPress, list);
            oxyos::Report::get().input("trash.files", 0, folly::dynamic::object("count", uris.size())("sent", sent));
            return sent;
          },
          .openFiles = [&window, &lastPress](const std::string& application, const std::vector<std::string>& uris) {
            std::string list;
            for (const auto& uri : uris) list += uri + "\n";
            bool sent = window->openFiles(lastPress, application, list);
            oxyos::Report::get().input("files.open", 0,
                                       folly::dynamic::object("application", application)("count", uris.size())(
                                           "sent", sent));
            return sent;
          },
          .focusDesktop = [&window, &lastPress] {
            if (lastPress == 0) return false;
            window->focusShellSurface(oxyos::Window::kToplevel, lastPress);
            oxyos::Report::get().input("focus.asked", oxyos::Window::kToplevel);
            return true;
          },
      };
    }
    // An application's notifications (docs/adr/0026-notifications.mdx), expo-notifications' on
    // OxyOS: the content the adapter gives, posted as its serial, and the responses the session
    // gives, kept until JavaScript takes them.
    oxyos::NotificationTexts notificationTexts;
    if (!shellMode && window) {
      services.notifications.post = [&window](uint32_t serial, const folly::dynamic& content) {
        auto text = [&](const folly::dynamic& object, const char* key) {
          auto value = object.get_ptr(key);
          return value && value->isString() ? value->getString() : std::string();
        };
        // The strings live until the call returns.
        std::vector<std::string> strings;
        strings.reserve(5 + 3 * 8);
        auto keep = [&](std::string value) { return strings.emplace_back(std::move(value)).c_str(); };
        std::vector<OxyOSNotificationAction> actions;
        if (auto list = content.get_ptr("actions"); list && list->isArray()) {
          for (const auto& action : *list) {
            if (actions.size() == 8) break;
            auto reply = action.get_ptr("reply");
            auto background = action.get_ptr("background");
            actions.push_back(OxyOSNotificationAction{
                .id = keep(text(action, "id")),
                .label = keep(text(action, "label")),
                .reply = reply && reply->isString() ? keep(reply->getString()) : nullptr,
                .background = background && background->isBool() && background->getBool(),
            });
          }
        }
        auto banner = content.get_ptr("banner");
        auto sound = content.get_ptr("sound");
        OxyOSNotification notification{
            .identifier = keep(text(content, "identifier")),
            .title = keep(text(content, "title")),
            .subtitle = keep(text(content, "subtitle")),
            .body = keep(text(content, "body")),
            .data = keep(text(content, "data")),
            .actions = actions.empty() ? nullptr : actions.data(),
            .action_count = actions.size(),
            .banner = banner && banner->isBool() && banner->getBool(),
            // The notification sound as it arrives (docs/adr/0044-system-sounds.mdx).
            .sound = sound && sound->isBool() && sound->getBool(),
        };
        bool posted = window->postNotification(serial, notification);
        if (!posted) std::cerr << "[host] the notification: " << oxyos_last_error() << "\n";
        oxyos::Report::get().input("notification.post", oxyos::Window::kToplevel,
                                   folly::dynamic::object("serial", serial)("sent", posted));
        return posted;
      };
      services.notifications.withdraw = [&window](uint32_t notification) {
        window->withdrawNotification(notification);
      };
    }
    // Installing applications (docs/adr/0030-installing-applications.mdx): what this launch may use,
    // the session's answers, the shell's sheets and capability results, assembled from chunks;
    // requests cite the latest press, as the session requires of what changes anything.
    auto& launchCapabilities = oxyos::LaunchCapabilities::current();
    oxyos::ChunkTexts systemTexts, sheetTexts;
    // A saved screenshot's path, assembled from its chunks (docs/adr/0024-system-shortcuts.mdx).
    oxyos::ChunkTexts screenshotPaths;
    uint32_t systemSerial = 0;
    std::map<uint32_t, std::string> askedCapabilities;
    // Login items and background activity (docs/adr/0060-login-items-and-background-activity.mdx):
    // the asks' serials, and whether this process may run on in the background once its window
    // closes, as the session last said (an answer, or a change the person made), and whether it is.
    uint32_t backgroundSerial = 0;
    bool backgroundEnabled = false;
    // The session asking it to quit before an end of the session
    // (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 5): whether JavaScript
    // answers (it registered a handler), and whether it answered quit, which ends the host.
    bool quitHandled = false;
    bool quitAnswered = false;
    // Sudden Termination (decision 10): JavaScript's holds, and what the compositor was told last.
    int suddenHolds = 0;
    std::optional<bool> suddenTold;
    // File choosers (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): the answers to this window's
    // requests, assembled from chunks. A request and an answer cite the latest press or key, as
    // the session requires.
    oxyos::ChunkTexts chosenTexts;
    uint32_t chooserSerial = 0;
    // The requests not answered yet: while one is, its sheet over the window is what takes the
    // window's keyboard (the window's text fields, ADR 0037).
    std::set<uint32_t> choosing;
    services.fileChooser.choose = [&window, &lastPress, &chooserSerial, &choosing](const std::string& text) -> uint32_t {
      ++chooserSerial;
      if (!window->chooseFiles(chooserSerial, lastPress, text)) return 0;
      choosing.insert(chooserSerial);
      return chooserSerial;
    };
    services.fileChooser.cancel = [&window, &choosing](uint32_t serial) {
      window->cancelChooseFiles(serial);
      choosing.erase(serial);
    };
    services.fileChooser.answer = [&window, &lastPress](const std::string& text) {
      // A choice cites the press or key that made it; a cancellation cites nothing.
      folly::dynamic answer;
      try {
        answer = folly::parseJson(text);
      } catch (const std::exception&) {
        return false;
      }
      bool chosen = answer.isObject() && answer.count("chosen") != 0;
      return window->answerFileChooser(chosen ? lastPress : 0, text);
    };
    // Search (docs/adr/0051-search.mdx): the index's answers, assembled from chunks. A request
    // cites no input (SearchRequest carries none); the compositor attaches who asks.
    oxyos::ChunkTexts searchTexts;
    uint32_t searchSerial = 0;
    services.search.request = [&window, &searchSerial](const std::string& text) -> uint32_t {
      // Serial 0 is the index's own (what is watched changed): a request never takes it.
      searchSerial = searchSerial == UINT32_MAX ? 1 : searchSerial + 1;
      return window->searchRequest(searchSerial, text) ? searchSerial : 0;
    };
    services.system = {
        .request = [&window, &lastPress, &systemSerial](const std::string& text) -> uint32_t {
          ++systemSerial;
          return window->systemRequest(systemSerial, lastPress, text) ? systemSerial : 0;
        },
        .answerSheet = [&window, &lastPress](uint32_t sheet, bool accepted) {
          window->sheetAnswer(sheet, accepted, lastPress);
        },
        .requestCapability = [&window, &lastPress, &systemSerial,
                              &askedCapabilities](const std::string& capability) -> uint32_t {
          uint8_t code = oxyos::capabilityCode(capability);
          if (code == 0) return 0;
          ++systemSerial;
          if (!window->requestCapability(systemSerial, lastPress, code)) return 0;
          askedCapabilities[systemSerial] = capability;
          return systemSerial;
        },
        .launch = [&launchCapabilities] { return launchCapabilities.describe(); },
        .allows = [&launchCapabilities](const std::string& capability) {
          return launchCapabilities.allows(capability);
        },
    };
    // An application's window of a compositor that has them (protocol version 33) asks about its
    // login item and background activity; the session decides, by its launch record.
    if (!shellMode && window && window->protocolVersion() >= 33) {
      services.backgroundItems.ask = [&window, &backgroundSerial](uint8_t kind, uint8_t action) -> uint32_t {
        backgroundSerial = backgroundSerial == UINT32_MAX ? 1 : backgroundSerial + 1;
        bool sent = window->backgroundItem(backgroundSerial, kind, action);
        oxyos::Report::get().input("background.ask", 0,
                                   folly::dynamic::object("kind", kind)("action", action)("sent", sent));
        return sent ? backgroundSerial : 0;
      };
    }
    // An application's window of a compositor that asks it to quit before logging out,
    // restarting or shutting down (protocol version 38): JavaScript answers while it handles the
    // question, the host at once otherwise.
    if (!shellMode && window && window->protocolVersion() >= OXYOS_QUIT_REQUESTS_VERSION) {
      services.quit.available = true;
      services.quit.handles = [&quitHandled](bool handles) { quitHandled = handles; };
      services.quit.holdSuddenTermination = [&suddenHolds](int delta) {
        suddenHolds = std::max(0, suddenHolds + delta);
      };
      services.quit.reply = [&window, &quitAnswered](uint32_t serial, bool quit) {
        bool sent = window->replyToQuit(serial, quit);
        oxyos::Report::get().input("quit.replied", 0,
                                   folly::dynamic::object("serial", serial)("quit", quit)("sent", sent));
        if (sent && quit) quitAnswered = true;
        return sent;
      };
    }
    window->onClipboardChanged([&toJavaScript, &window] {
      toJavaScript.callFunctionOnModule(
          "RCTDeviceEventEmitter", "emit",
          folly::dynamic::array("OxyOSClipboardChanged", folly::dynamic::object("text", window->clipboardHasText())));
    });
    oxyos::KeyChords keyChords;  // the window's keys as offered to JavaScript (key-chords.h)
    oxyos::TextInputEdit drawn;  // the focused input's caret and selection as last drawn
    // The focused input as the compositor was last told (ADR 0008), its caret, and each surface's
    // commit serial: a transaction made for an older one is for text the field no longer has.
    std::optional<oxyos::TextEditing::Field> toldField;
    std::optional<facebook::react::Rect> toldCaret;
    std::map<uint32_t, uint32_t> toldSerials;
    std::vector<std::string> shownTexts;  // the texts of the window's latest frame
    // The window's frame as last drawn: the next one draws only what changed (damage.h).
    oxyos::Canvas windowCanvas;
    oxyos::WindowDamage windowDamage;
    // OXYOS_HOST_DAMAGE_CHECK: every frame drawn from its damage is also drawn whole and compared.
    static const bool checkDamage = std::getenv("OXYOS_HOST_DAMAGE_CHECK") != nullptr;
    mounter.beforeApply = [&](const ShadowViewMutationList& mutations) {
      modalHost.leaving(mutations, display, drawn);
      editing.inserted(mutations);
    };
    mounter.onCommand = [&](const ShadowView& view, const std::string& name, const folly::dynamic& args) {
      editing.command(view, name, args);
      // A ScrollView's: scrollTo, scrollToEnd, flashScrollIndicators (input.h).
      pointer.command(view, name, args);
    };
    // Keyboard focus beyond text fields (keyboard-focus.h, docs/adr/0039-keyboard-access.mdx): the
    // control Tab reached, its ring, and where it goes back to when a modal leaves.
    oxyos::KeyboardFocus keyboardFocus(tree);
    // The modifiers held on the keyboard, for its navigation keys.
    bool heldShift = false, heldCtrl = false, heldAlt = false, heldMeta = false;
    // The view the keyboard navigates in a surface: the window's (its topmost modal while one is
    // shown) or a shell region's, either narrowed to a view marked modal; a popup's that holds a
    // dialog (Bloom's popover: its controls, as a Mac's popover's key view loop); none in any other
    // popup, a menu's, whose keys are its own (Bloom's popup navigation).
    auto dialogPopup = [&](uint32_t surface) -> facebook::react::Tag {
      auto content = popups.content(surface);
      if (!content) return -1;
      auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(content->view).view.props);
      bool dialog = props && (props->accessibilityRole == "dialog" || props->role == Role::Dialog);
      return dialog ? content->view : -1;
    };
    auto navigationScope = [&](uint32_t surface) -> facebook::react::Tag {
      if (surface == 0) return -1;
      if (auto dialog = dialogPopup(surface); dialog >= 0) return oxyos::navigationRoot(tree, dialog);
      if (shellMode) {
        auto content = shellSurfaces.content(surface);
        return content ? oxyos::navigationRoot(tree, content->view) : -1;
      }
      if (surface != oxyos::Window::kToplevel || tree.root() < 0) return -1;
      auto shown = oxyos::modals(tree);
      return oxyos::navigationRoot(tree, shown.empty() ? tree.root() : shown.back());
    };
    auto insideOf = [&](facebook::react::Tag tag, facebook::react::Tag root) {
      auto path = oxyos::ancestry(tree, tag);
      return std::find(path.begin(), path.end(), root) != path.end();
    };
    // The scroll views around a focused control bring it and its ring into view, as Android's
    // requestFocus asks its parents to (ScrollView.requestChildFocus), each in turn (input.h).
    auto reveal = [&](facebook::react::Tag tag, uint32_t surface) { pointer.reveal(tag, surface, oxyos::kFocusRingReach); };
    auto redrawFocus = [&] {
      dirty = true;
      popups.changed();
      shellSurfaces.changed();
    };
    // Keyboard focus to `target` (-1: none), its ring shown if the keyboard moved it: the control
    // left is told it blurred and the one reached that it is focused (React Native's onBlur and
    // onFocus, Pressability's), a text input through the text editing; the assistive technology's
    // focus follows, as VoiceOver's cursor follows the keyboard's.
    auto moveKeyboardFocus = [&](facebook::react::Tag target, bool ring, uint32_t surface) {
      auto previous = keyboardFocus.focused();
      if (previous == target) {
        if (ring != keyboardFocus.ringShown()) redrawFocus();
        keyboardFocus.set(target, ring);
        return;
      }
      auto emitterOf = [&](facebook::react::Tag tag) -> std::shared_ptr<const facebook::react::BaseViewEventEmitter> {
        auto it = tree.views().find(tag);
        if (it == tree.views().end()) return nullptr;
        return std::dynamic_pointer_cast<const facebook::react::BaseViewEventEmitter>(it->second.view.eventEmitter);
      };
      if (previous >= 0 && !oxyos::editableInput(tree, previous)) {
        if (auto emitter = emitterOf(previous)) {
          emitter->onBlur();
          oxyos::Report::get().deliver("blur", surface, tree.views().at(previous).view);
        }
      }
      keyboardFocus.set(target, ring);
      if (target >= 0 && oxyos::editableInput(tree, target)) {
        editing.focusFromKeyboard(target);
      } else {
        // The field left is blurred only when the keyboard moved off it within its surface (Tab
        // from a field to a button), never a field another surface focused meanwhile (a sheet's
        // password field, focused as it appears).
        auto field = editing.focused();
        if (field >= 0 && (field == previous || (target >= 0 && insideOf(field, navigationScope(surface))))) {
          editing.unfocus();
        }
        if (auto emitter = target >= 0 ? emitterOf(target) : nullptr) {
          emitter->onFocus();
          oxyos::Report::get().deliver("focus", surface, tree.views().at(target).view);
        }
      }
      if (target >= 0) {
        accessibility.focus(target);
        reveal(target, surface);
      }
      oxyos::Report::get().input("keyboard.focus", surface,
                                 folly::dynamic::object("ring", ring)(
                                     "target", target >= 0 ? oxyos::Report::name(tree.views().at(target).view) : ""));
      redrawFocus();
    };
    // The navigation's view as last seen for the surface with the keyboard: a modal coming takes the
    // keyboard to its first control and remembers where it was; one going gives it back there.
    // A view the application moved accessibility focus to while the keyboard navigates, which is
    // not focusable in the tree yet, or not in it at all: made so (or created) in the commit that
    // asks for its focus (a segmented control's newly chosen segment, its group's one Tab stop),
    // still to be mounted. The
    // keyboard follows it once that commit is, as Android's mounting layer handles the event
    // behind the mount items queued before it.
    facebook::react::Tag keyboardAfterMount = -1;
    // A modal that came over the control with the keyboard while it had no control of its own yet:
    // the keyboard goes to its first one as it mounts, unless something else took the keyboard.
    facebook::react::Tag modalFirstPending = -1;
    facebook::react::Tag lastScope = -1;
    uint32_t lastScopeSurface = 0;
    // The application's last move of the keyboard's focus while it navigated, and what had the
    // keyboard before it: a move into a modal the navigation had not followed yet (its default
    // control, focused as it mounts: Bloom's dialog's `initialFocus`) leaves the keyboard there,
    // and `from` is what the keyboard goes back to as the modal goes (docs/adr/0039-keyboard-access.mdx,
    // decision 27).
    struct ApplicationFocus {
      facebook::react::Tag to = -1, from = -1;
    } applicationFocus;
    // A dialog popup the keyboard went into from a control with its ring (a popover opened by Space
    // on its button), and that control, which has the keyboard back once the popup has gone.
    struct PopupReturn {
      facebook::react::Tag popup = -1, control = -1;
    } popupReturn;
    // Each shell region's focus request last carried out, while it holds the keyboard.
    std::map<facebook::react::Tag, std::optional<int>> focusRequestsApplied;
    auto followScope = [&] {
      auto scope = navigationScope(keyboardSurface);
      if (scope == lastScope && keyboardSurface == lastScopeSurface) {
        keyboardFocus.prune();
        return;
      }
      auto was = lastScope;
      bool sameSurface = keyboardSurface == lastScopeSurface;
      lastScope = scope;
      lastScopeSurface = keyboardSurface;
      if (!sameSurface && keyboardSurface != 0 && dialogPopup(keyboardSurface) >= 0) {
        // A popover took the keyboard: from a control with its ring, the keyboard goes to its first
        // control, as a Mac's popover takes the key focus, and comes back when it goes.
        keyboardFocus.prune();
        auto from = keyboardFocus.focused();
        if (keyboardFocus.ringShown() && from >= 0 && !insideOf(from, scope)) {
          popupReturn = {dialogPopup(keyboardSurface), from};
          auto first = oxyos::focusables(tree, scope);
          if (!first.empty()) moveKeyboardFocus(first.front(), true, keyboardSurface);
        }
        return;
      }
      if (!sameSurface && popupReturn.popup >= 0 && !tree.views().count(popupReturn.popup)) {
        // The popover went: the keyboard back on the control it opened from.
        auto back = std::exchange(popupReturn, PopupReturn{}).control;
        keyboardFocus.prune();
        if (back >= 0 && oxyos::focusable(tree, back) && scope >= 0 && insideOf(back, scope)) {
          moveKeyboardFocus(back, true, keyboardSurface);
          return;
        }
      }
      if (!sameSurface || scope < 0) {
        keyboardFocus.prune();
        // The keyboard went to another surface of this client's: focus left behind in a surface
        // that no longer has it goes (the dock left for the window), unless the keyboard went to a
        // popup, which gives it back (a menu opened from a focused button).
        auto focused = keyboardFocus.focused();
        if (scope >= 0 && focused >= 0 && !insideOf(focused, scope) && !oxyos::editableInput(tree, focused)) {
          moveKeyboardFocus(-1, false, lastScopeSurface);
        }
        return;
      }
      bool ring = keyboardFocus.ringShown();
      auto focused = keyboardFocus.focused();
      bool came = was >= 0 && tree.views().count(was) && insideOf(scope, was) && scope != was;
      if (came && focused >= 0 && insideOf(focused, scope)) {
        // A modal came whose own control the application had already focused (a dialog's default
        // button, focused in the commit that mounted it): that control keeps the keyboard, never the
        // modal's first in its place (a destructive Delete before the default Cancel), and the
        // keyboard goes back to what had it before as the modal goes.
        auto back = applicationFocus.to == focused ? applicationFocus.from : -1;
        bool outside = back >= 0 && tree.views().count(back) && !insideOf(back, scope);
        keyboardFocus.remember(scope, outside ? back : -1, outside);
      } else if (came && keyboardAfterMount >= 0 && tree.views().count(keyboardAfterMount) &&
                 insideOf(keyboardAfterMount, scope)) {
        // A modal came whose own control the application asked to focus before it could take the
        // keyboard (a dialog's buttons until it has measured itself and shows, a frame later): the
        // keyboard waits for that control (`keyboardAfterMount`), never the modal's first in its
        // place, and goes back to what it was on as the modal goes. The modal's first has it if
        // that control never can.
        keyboardFocus.remember(scope, focused, ring);
        modalFirstPending = ring ? scope : -1;
      } else if (came) {
        // A modal came over what had the keyboard.
        keyboardFocus.remember(scope, focused, ring);
        if (ring) {
          auto first = oxyos::focusables(tree, scope);
          moveKeyboardFocus(first.empty() ? -1 : first.front(), true, keyboardSurface);
          // Its controls not mounted yet (a list that draws its rows in a later commit): the
          // keyboard goes to the first once there is one, rather than nowhere.
          modalFirstPending = first.empty() ? scope : -1;
        }
      } else if (was >= 0) {
        // The modal went: the keyboard back where it was before it.
        auto [back, hadRing] = keyboardFocus.recall(was);
        keyboardFocus.prune();
        if (back >= 0 && oxyos::focusable(tree, back) && insideOf(back, scope)) {
          moveKeyboardFocus(back, hadRing, keyboardSurface);
        } else if (back >= 0 && oxyos::focusable(tree, back) && scope >= 0 && scope != tree.root() &&
                   tree.root() >= 0 && insideOf(back, tree.root())) {
          // Another modal came in the commit this one went in, over the control it gave the
          // keyboard back to (a choice that asks something at once: a display mode on trial, its
          // "Keep these display settings?"): it came over that control, which it remembers and gives
          // the keyboard back to as it goes, and takes the keyboard as a modal does.
          keyboardFocus.remember(scope, back, hadRing);
          if (hadRing) {
            auto first = oxyos::focusables(tree, scope);
            moveKeyboardFocus(first.empty() ? -1 : first.front(), true, keyboardSurface);
            modalFirstPending = first.empty() ? scope : -1;
          }
        }
      }
      keyboardFocus.prune();
    };
    enum class NavigationKey { Next, Previous, Activate, Increment, Decrement, ContextMenu, TextMenu };
    // The view whose context menu the keyboard opens from the focused control: the control, or its
    // nearest ancestor within `scope` that declares React Native's `longpress` accessibility action
    // (Bloom's context menu trigger, TalkBack's long press); -1 for none.
    auto contextMenuOf = [&](facebook::react::Tag focused, facebook::react::Tag scope) -> facebook::react::Tag {
      auto path = oxyos::ancestry(tree, focused);
      for (auto at = path.rbegin(); at != path.rend(); ++at) {
        auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(*at).view.props);
        if (props && std::any_of(props->accessibilityActions.begin(), props->accessibilityActions.end(),
                                 [](const auto& declared) { return declared.name == "longpress"; })) {
          return *at;
        }
        if (*at == scope) break;
      }
      return -1;
    };
    // What a key press is to the keyboard's navigation, if anything.
    auto navigationKey = [&](const OxyOSEvent& event) -> std::optional<NavigationKey> {
      if (event.kind != OXYOS_EVENT_KEY || !event.flag || heldCtrl || heldAlt || heldMeta) return std::nullopt;
      constexpr uint32_t kTab = 15, kSpace = 57, kReturn = 28, kNumpadEnter = 96, kUp = 103, kLeft = 105,
                         kRight = 106, kDown = 108, kF10 = 68, kMenu = 127;
      auto scope = navigationScope(event.surface);
      if (scope < 0) return std::nullopt;
      if (event.code == kTab) return heldShift ? NavigationKey::Previous : NavigationKey::Next;
      bool menuKey = (event.code == kMenu && !heldShift) || (event.code == kF10 && heldShift);
      // The focused text field's menu is the platform's (docs/adr/0037-text-editing.mdx, decision
      // 8): the Menu key and Shift+F10 open it at its caret, however the field took focus (a press
      // or Tab), as a PC's do and a Mac's Control-Return with Full Keyboard Access.
      if (menuKey && editing.focused() >= 0 && insideOf(editing.focused(), scope)) return NavigationKey::TextMenu;
      // A field the person focused with a press is where Tab goes on from, and keeps its keys.
      auto focused = editing.focused() >= 0 ? editing.focused() : keyboardFocus.focused();
      if (focused < 0 || !insideOf(focused, scope) || oxyos::editableInput(tree, focused) ||
          !oxyos::focusable(tree, focused) || !keyboardFocus.ringShown()) {
        return std::nullopt;
      }
      if (!heldShift && (event.code == kSpace || event.code == kReturn || event.code == kNumpadEnter)) {
        // Only a control that answers a click (Pressable's, a button's) takes them; any other (a
        // list that is one stop, its Space Quick Look and its Return a rename) has them as its
        // own keys, as a Mac's first responder passes on the keys it does not handle.
        auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(focused).view.props);
        if (props && props->events[facebook::react::ViewEvents::Offset::Click]) return NavigationKey::Activate;
        return std::nullopt;
      }
      // The Menu key and Shift+F10 open the focused control's context menu, as a PC keyboard's do
      // (a Mac's Control-Return with Full Keyboard Access): one the application declares, else the
      // key is the application's.
      if (menuKey && contextMenuOf(focused, scope) >= 0) {
        return NavigationKey::ContextMenu;
      }
      if (event.code == kUp || event.code == kDown || event.code == kLeft || event.code == kRight) {
        auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(focused).view.props);
        bool up = event.code == kUp || event.code == kRight;
        std::string action = up ? "increment" : "decrement";
        bool offers = props && std::any_of(props->accessibilityActions.begin(), props->accessibilityActions.end(),
                                           [&](const auto& declared) { return declared.name == action; });
        if (offers) return up ? NavigationKey::Increment : NavigationKey::Decrement;
      }
      return std::nullopt;
    };
    // Scrolling from the keyboard, as a Mac's scroll view takes the keys its first responder leaves
    // (Safari's steps, scroll-physics.h) and Android's ScrollView.executeKeyEvent takes them for
    // React: Page Down and Page Up, Space and Shift+Space a page; the arrows a line, ↑ ↓ down a
    // view, ← → across one; Home and End, Ctrl+↑ and Ctrl+↓, to its ends; each animated as
    // smoothScrollBy animates it.
    // The scroll view is the innermost around the keyboard's focus that moves that way (outward
    // when it cannot), else, with the focus in no scroll view, the window's main one (the largest
    // that scrolls that way). A focused text field keeps every one of these keys, and a focused
    // control keeps those it uses (Space on a button, the arrows on a slider, before this; Bloom's
    // own keys, offered to JavaScript first). A key taken reaches nothing else, nor its release.
    auto keyScroll = [&](const OxyOSEvent& event) -> bool {
      if (event.kind != OXYOS_EVENT_KEY || !event.flag || heldAlt || heldMeta) return false;
      constexpr uint32_t kSpace = 57, kPageUp = 104, kPageDown = 109, kHome = 102, kEnd = 107, kUp = 103,
                         kDown = 108, kLeft = 105, kRight = 106;
      if (heldShift && (event.code != kSpace || heldCtrl)) return false;
      // Ctrl is the applications' ⌘ (docs/adr/0037-text-editing.mdx): Ctrl+↑ and Ctrl+↓ go to the
      // ends, as ⌘↑ and ⌘↓ do in a Mac's scroll view.
      if (heldCtrl && event.code != kUp && event.code != kDown) return false;
      int axis = 1;
      enum class Step { Line, Page, Edge } step;
      int direction = 1;
      switch (event.code) {
        case kSpace: step = Step::Page; direction = heldShift ? -1 : 1; break;
        case kPageDown: step = Step::Page; break;
        case kPageUp: step = Step::Page; direction = -1; break;
        case kDown: step = heldCtrl ? Step::Edge : Step::Line; break;
        case kUp: step = heldCtrl ? Step::Edge : Step::Line; direction = -1; break;
        case kRight: step = Step::Line; axis = 0; break;
        case kLeft: step = Step::Line; axis = 0; direction = -1; break;
        case kEnd: step = Step::Edge; break;
        case kHome: step = Step::Edge; direction = -1; break;
        default: return false;
      }
      auto scope = navigationScope(event.surface);
      if (scope < 0) return false;
      if (editing.focused() >= 0 && insideOf(editing.focused(), scope)) return false;
      auto alongAxis = [&](facebook::react::Tag tag, int along) {
        auto range = pointer.scrollRange(tag);
        return range && (along ? range->second.y : range->second.x) > 0;
      };
      // Home and End go along the axis the view scrolls (a horizontal one's start and end).
      auto scrolls = [&](facebook::react::Tag tag) {
        return alongAxis(tag, axis) || (step == Step::Edge && alongAxis(tag, 0));
      };
      std::vector<facebook::react::Tag> candidates;
      auto focused = keyboardFocus.focused();
      if (focused >= 0 && insideOf(focused, scope)) {
        auto path = oxyos::ancestry(tree, focused);
        for (auto at = path.rbegin(); at != path.rend() && *at != scope; ++at) {
          if (scrolls(*at)) candidates.push_back(*at);
        }
      }
      if (candidates.empty()) {
        // The window's main scroll view: the largest shown that scrolls that way.
        facebook::react::Tag main = -1;
        float largest = 0;
        auto walk = [&](facebook::react::Tag tag, auto& self) -> void {
          auto it = tree.views().find(tag);
          if (it == tree.views().end()) return;
          auto props = std::dynamic_pointer_cast<const facebook::react::ViewProps>(it->second.view.props);
          if (props && props->opacity <= 0.001f) return;
          const auto& name = it->second.view.componentName ? std::string(it->second.view.componentName) : std::string();
          if (oxyos::ownSurface(name) && tag != scope) return;
          auto size = it->second.view.layoutMetrics.frame.size;
          if (name == "ScrollView" && scrolls(tag) && size.width * size.height > largest) {
            largest = size.width * size.height;
            main = tag;
          }
          for (auto child : it->second.children) self(child, self);
        };
        walk(scope, walk);
        if (main >= 0) candidates.push_back(main);
      }
      if (candidates.empty()) return false;
      facebook::react::Tag moved = -1;
      for (auto tag : candidates) {
        int along = alongAxis(tag, axis) ? axis : 0;
        auto size = tree.views().at(tag).view.layoutMetrics.frame.size;
        float length = along ? size.height : size.width;
        float distance = step == Step::Line ? oxyos::kLineStep
                         : step == Step::Page ? oxyos::pageStep(length)
                                              : std::numeric_limits<float>::max() / 4;
        float d = direction * distance;
        if (pointer.smoothScrollBy(tag, along ? 0 : d, along ? d : 0, event.surface)) {
          moved = tag;
          break;
        }
      }
      // A scroll view at its end still takes the key, as a Mac's does.
      auto target = moved >= 0 ? moved : candidates.front();
      oxyos::Report::get().input("keyboard.scroll", event.surface,
                                 folly::dynamic::object("key", event.code)("moved", moved >= 0)(
                                     "target", oxyos::Report::name(tree.views().at(target).view)));
      return true;
    };
    auto navigate = [&](const OxyOSEvent& event, NavigationKey key) {
      if (key == NavigationKey::Next || key == NavigationKey::Previous) {
        if (editing.focused() >= 0 && keyboardFocus.focused() != editing.focused()) {
          keyboardFocus.set(editing.focused(), false);
        }
        auto target = keyboardFocus.next(navigationScope(event.surface), key == NavigationKey::Previous);
        if (target >= 0) moveKeyboardFocus(target, true, event.surface);
        // The person moved the keyboard: a control the application asked for, not focusable yet, no
        // longer takes it.
        keyboardAfterMount = -1;
        return;
      }
      if (key == NavigationKey::TextMenu) {
        editing.contextMenuAtCaret();
        return;
      }
      const auto& shadow = tree.views().at(keyboardFocus.focused()).view;
      if (key == NavigationKey::Activate) {
        // As an assistive technology's activation: React Native's click, which Pressability
        // answers with onPress.
        shadow.eventEmitter->dispatchEvent("click", folly::dynamic::object());
        oxyos::Report::get().deliver("click", event.surface, shadow, folly::dynamic::object("key", event.code));
        return;
      }
      std::string action = key == NavigationKey::Increment   ? "increment"
                           : key == NavigationKey::Decrement ? "decrement"
                                                             : "longpress";
      // The context menu's view: the focused control's or an ancestor's (`contextMenuOf`).
      const auto& target = key == NavigationKey::ContextMenu
                               ? tree.views().at(contextMenuOf(keyboardFocus.focused(), navigationScope(event.surface))).view
                               : shadow;
      if (auto emitter = std::dynamic_pointer_cast<const facebook::react::BaseViewEventEmitter>(target.eventEmitter)) {
        emitter->onAccessibilityAction(action);
        oxyos::Report::get().deliver("accessibilityAction", event.surface, target,
                                     folly::dynamic::object("actionName", action));
      }
    };
    // A scenario's "grant-first" order (OXYOS_HOST_TEST_ORDER): JavaScript's focus moves into a
    // shell region that asks for the keyboard and does not have it yet wait for it, and are
    // delivered once the compositor has given it (or the region no longer asks).
    std::vector<std::pair<ShadowView, std::string>> heldForGrant;
    auto waitsForGrant = [&](const ShadowView& view, const std::string& type) {
      if (!grantFirst || type != "focus" || !tree.views().count(view.tag)) return false;
      auto in = shellSurfaces.surfaceOf(view.tag);
      return in && shellSurfaces.keyboardRegion() == *in && keyboardSurface != *in;
    };
    // A scenario's "popup-focus-first" order: from the moment a menu opens (before the compositor
    // can answer it) until its tree is published with JavaScript's focus or it is gone, the
    // compositor's keyboard moves are held, in order, and handled after.
    uint32_t keyboardHeldFor = 0;
    std::set<uint32_t> keyboardHeldOnce;
    std::vector<OxyOSEvent> keyboardHeld;
    // Accessibility focus, as Android's accessibility focus (separate from input focus): the
    // view a screen reader would be on, which the tree marks and the screen reader follows.
    // JavaScript's announcements applied, waiting for the frame that publishes what came before them.
    std::vector<std::string> announceAfterFrame;
    mounter.onAnnouncement = [&announceAfterFrame](const std::string& text) { announceAfterFrame.push_back(text); };
    mounter.onAccessibilityEvent = [&](const ShadowView& view, const std::string& type) {
      if (waitsForGrant(view, type)) {
        if (heldForGrant.empty()) oxyos::Report::get().input("test.focus.held", *shellSurfaces.surfaceOf(view.tag));
        heldForGrant.emplace_back(view, type);
        return;
      }
      std::string within = oxyos::nearestTestId(tree, view.tag);
      oxyos::Report::get().accessibilityEvent(type, view, within);
      // A region whose keyboard went back to a window (the bar left from the keyboard) takes no
      // accessibility focus from JavaScript while the keyboard is elsewhere: Bloom gives a closed
      // menu's trigger the focus back (a row chosen closes it after the bar gave the window the
      // keyboard), and the person is in the window, not on the bar. The assistive technology's own
      // focus moves (an action) are not these, and go anywhere. Nor is a move into a menu the
      // person opened over the region (its item pressed): the menu is where they are now. Whether
      // the view is in a popup is the tree's, which the commit carries, never the compositor's
      // answer: on a loaded machine Bloom's move into the menu reaches the host before the
      // compositor's keyboard enter on the popup does (docs/adr/0039-keyboard-access.mdx,
      // decision 26), and judged by that grant the move was declined, the menu's first row
      // unfocused and unsaid.
      if (type == "focus" && releasedRegion != 0 && keyboardSurface != releasedRegion &&
          shellSurfaces.keyboardRegion() != releasedRegion &&
          !(keyboardSurface != 0 && popups.content(keyboardSurface)) && !oxyos::inPopupView(tree, view.tag) &&
          shellSurfaces.surfaceOf(view.tag) == releasedRegion) {
        oxyos::Report::get().input("a11y.focus.declined", releasedRegion);
        return;
      }
      if (type == "focus") {
        accessibility.focus(view.tag);
        dirty = true;
        popups.changed();
        shellSurfaces.changed();
        // While the keyboard is navigating, the application's focus moves (Bloom giving a closed
        // menu's or dialog's trigger the focus back) take the keyboard's with them: its ring shows,
        // or a modal came with it showing and waits for a control of its own (`modalFirstPending`).
        // A view not in the tree yet is created by that commit (a field mounted with the focus, the
        // login screen's for the account just chosen): its place is known once it is mounted.
        auto scope = navigationScope(keyboardSurface);
        if ((keyboardFocus.ringShown() || modalFirstPending == scope) && scope >= 0 &&
            view.tag != keyboardFocus.focused()) {
          if (!tree.views().count(view.tag)) {
            keyboardAfterMount = view.tag;
          } else if (insideOf(view.tag, scope)) {
            if (oxyos::focusable(tree, view.tag)) {
              applicationFocus = {view.tag, keyboardFocus.focused()};
              moveKeyboardFocus(view.tag, true, keyboardSurface);
            } else {
              keyboardAfterMount = view.tag;
            }
          }
        }
      }
    };
    // The text input the assistive technology's focus went to as it took focus, while it has it,
    // and where that focus was before it.
    facebook::react::Tag fieldFollowed = -1, fieldFollowedFrom = -1;
    // A text input that takes focus, however it took it, takes the assistive technology's too
    // (textinput.h, `onFocus`): a field pressed is where the screen reader is, says itself and
    // echoes its editing (docs/adr/0037-text-editing.mdx, decision 7).
    editing.onFocus([&](facebook::react::Tag tag) {
      if (accessibility.focused() != tag) fieldFollowedFrom = accessibility.focused();
      fieldFollowed = tag;
      accessibility.focus(tag);
      dirty = true;
      popups.changed();
      shellSurfaces.changed();
    });
    int idleTurns = 0;  // turns in a row that found nothing to do
    // A key press offered to JavaScript first (Bloom's PlatformKeyboard: the menus' shortcuts, ADR
    // 0021) is answered on JavaScript's thread, which may be busy: the key, and every event of the
    // compositor's after it, wait for the answer in the order they came, as a browser holds key
    // events behind the one its page has not answered, while the loop goes on animating, scrolling
    // and drawing (docs/adr/0067-the-ui-thread.mdx). `answer` is -1 until it comes, then whether
    // JavaScript took the key.
    // Each with the number the report received it as (Report::received), the cause of what its handling
    // delivers.
    struct OfferedKey {
      OxyOSEvent event;
      uint64_t seq;
      std::shared_ptr<std::atomic<int>> answer;
      folly::dynamic offered;
      uint64_t number;  // in the order offered (KeyBurst)
    };
    // Shared with the offers' tasks, which may outlive the loop by their wait.
    auto keyBurst = std::make_shared<KeyBurst>();
    // The assistive technology is told what a turn of the compositor's did once all of it is
    // handled: while a key of it waits for JavaScript's answer, the window's accessibility waits with
    // it (an input method's commit and the Space it gave back said as one: "한", never "한" then
    // "espacio"), at most as long as Android holds a window's content changes (100 ms).
    std::optional<Clock::time_point> keyOfferedAt;
    bool accessibilityHeld = false;
    constexpr auto kAccessibilityHeldAtMost = std::chrono::milliseconds(100);
    auto publishWindowAccessibility = [&] {
      if (keyOfferedAt && Clock::now() - *keyOfferedAt < kAccessibilityHeldAtMost) {
        accessibilityHeld = true;
        return;
      }
      accessibility.publish(oxyos::Window::kToplevel, tree.root());
      accessibilityHeld = false;
    };
    uint64_t keysOffered = 0;
    std::optional<OfferedKey> offeredKey;
    // An event held behind it keeps what the connection says of it beyond the event itself, which
    // the library keeps only until it reads the next event: a text input's transaction and
    // AutoFill's fill, a long answer, the input method's candidates. Read as it is received, and
    // handled from that (`replaying`).
    struct Payload {
      bool transaction = false, filled = false;
      OxyOSTextTransaction change{};
      OxyOSTextFill fill{};
      std::string commit, preedit, fillText, pairedText;
      std::vector<OxyOSPreeditSegment> segments;
      std::optional<std::string> systemData;
      std::optional<oxyos::Window::Candidates> candidates;
      // The transaction and the fill, pointing into what is kept here.
      OxyOSTextTransaction kept() const {
        auto out = change;
        out.commit = commit.data();
        out.preedit = preedit.data();
        out.segments = segments.data();
        return out;
      }
      OxyOSTextFill keptFill() const {
        auto out = fill;
        out.fill = fillText.data();
        out.paired_fill = pairedText.data();
        return out;
      }
      // A fill is a credential: gone from memory as the payload goes.
      ~Payload() {
        std::fill(fillText.begin(), fillText.end(), '\0');
        std::fill(pairedText.begin(), pairedText.end(), '\0');
      }
    };
    struct Held {
      OxyOSEvent event;
      uint64_t seq;
      std::shared_ptr<const Payload> payload;
    };
    std::deque<Held> heldBehindKey;
    const Payload* replaying = nullptr;
    auto payloadOf = [&](const OxyOSEvent& event) -> std::shared_ptr<const Payload> {
      auto payload = std::make_shared<Payload>();
      if (event.kind == OXYOS_EVENT_TEXT_INPUT) {
        OxyOSTextTransaction change{};
        if (window->transaction(change)) {
          payload->transaction = true;
          payload->change = change;
          payload->commit.assign(change.commit ? change.commit : "", change.commit_length);
          payload->preedit.assign(change.preedit ? change.preedit : "", change.preedit_length);
          if (change.segments) payload->segments.assign(change.segments, change.segments + change.segment_count);
          OxyOSTextFill fill{};
          if (window->fill(fill)) {
            payload->filled = true;
            payload->fill = fill;
            if (fill.fill) payload->fillText.assign(fill.fill, fill.fill_length);
            if (fill.paired_fill) payload->pairedText.assign(fill.paired_fill, fill.paired_fill_length);
          }
        }
      } else if (event.kind == OXYOS_EVENT_SYSTEM_DATA) {
        payload->systemData = window->systemData();
      } else if (event.kind == OXYOS_EVENT_INPUT_CANDIDATES) {
        payload->candidates = window->inputCandidates();
      } else {
        return nullptr;
      }
      return payload;
    };
    // The number of the event being handled, as received.
    uint64_t handlingSeq = 0;
    auto offerKey = [&](const OxyOSEvent& event, folly::dynamic offered) {
      auto answer = std::make_shared<std::atomic<int>>(-1);
      uint64_t number = ++keysOffered;
      offeredKey = OfferedKey{event, handlingSeq, answer, offered, number};
      if (!keyOfferedAt) keyOfferedAt = Clock::now();
      toJavaScript.offerKey([listener = services.keyboard.listener, offered = std::move(offered), answer, number,
                             keyBurst, &hermesRuntime] {
        bool taken = false;
        if (auto* rt = hermesRuntime.load()) {
          try {
            auto value = listener->call(*rt, jsi::valueFromDynamic(*rt, offered));
            taken = value.isBool() && value.getBool();
          } catch (const jsi::JSError& error) {
            std::cerr << "[host] the key listener: " << error.getMessage() << "\n";
          }
        }
        answer->store(taken ? 1 : 0);
        oxyos::UiThread::wake();
        // The burst's last key: the microtasks of its keys, as React Native's scheduler runs a task's
        // (React schedules the render of what they changed from one, rendered once, after them).
        if (!keyBurst->awaitDecision(number)) {
          if (auto* rt = hermesRuntime.load()) {
            try {
              rt->drainMicrotasks();
            } catch (const jsi::JSError& error) {
              std::cerr << "[host] after the keys: " << error.getMessage() << "\n";
            }
          }
        }
      });
    };
    uint32_t toldRefresh = 0;  // the window's display's refresh, as last told (display-refresh.h)
    // A window that changes size answers its configure with a frame JavaScript laid out at that
    // size, as AppKit lays a window out during a live resize before the frame is shown: the new
    // size's layout reaches JavaScript (onLayout, the dimensions), whose commits can size the
    // tree again (an app shell measuring its room, then a panel group measuring its own). Until
    // JavaScript is quiet the window presents nothing and popups are not placed again: a resize's
    // first frame (a zoom, a window's size set, a live resize's start) carries the new layout,
    // waiting for JavaScript as long as it takes (at most a second, as long as a hold); the frames of
    // a live resize after it, at most a frame of the display (display-refresh.h: 16.7 ms at 60 Hz,
    // 8.3 at 120), so that a slow app never stalls the resize past its next refresh.
    std::optional<Clock::time_point> settleBy;
    Clock::time_point settleFrom;
    // Whether the resize under way has presented its first frame, and when its last configure came.
    bool resizeShown = true;
    std::optional<Clock::time_point> lastResized;
    uint64_t settleAfter = 0;  // what JavaScript was handed for the new size: settled once it ran
    bool firstShown = false;  // the window's first frame may be drawn (below)
    // The compositor's pings waiting for their answer (Window::answerPingsItself): the application is
    // responding once JavaScript took what came before a ping, as a Mac's window server reads an
    // application's main thread taking its events, so JavaScript stuck in its work is a hang
    // (docs/adr/0067-hangs-and-force-quit.mdx) while the UI thread goes on reading and drawing. A
    // ping read in a turn that queued JavaScript something is answered once JavaScript ran that batch
    // (JsThread::done); one read in a turn that queued nothing, once JavaScript is idle after what it
    // was running; never while a key offered to JavaScript waits for its answer.
    struct PingWaiting {
      uint64_t serial;
      uint64_t batch;
      bool idle;
    };
    std::vector<PingWaiting> pingsWaiting;
    int settleTurns = 0;
    try {
      // Where the turn's time goes, for the session's log (frame-diagnostics.h).
      auto& diagnostics = oxyos::FrameDiagnostics::get();
      using Part = oxyos::FrameDiagnostics::Part;
      using Scope = oxyos::FrameDiagnostics::Scope;
      // What runs JavaScript, timed for the report (Report::javascript) and the diagnostics.
      auto javascript = [](Part part, auto&& run) {
        Scope scope(part);
        if (!oxyos::Report::get().enabled()) return run();
        auto started = oxyos::ThreadSample::now();
        bool ran = run();
        oxyos::Report::get().javascript(oxyos::ThreadSpan::between(started, oxyos::ThreadSample::now()));
        return ran;
      };
      while (true) {
        oxyos::UiThread::woken();
        oxyos::Report::get().beginTurn();
        diagnostics.beginTurn();
        // What JavaScript handed over since the last turn (Mounter): its commits mounted, its view
        // commands and accessibility events handled, in order. JavaScript itself runs on its thread
        // and never holds the loop up (docs/adr/0067-the-ui-thread.mdx).
        // What JavaScript handed over is applied as one transaction once JavaScript is idle, after all
        // it ran with its commits (a layout effect's, an effect's, what a press set off), as Core
        // Animation commits what a run loop's turn changed at its end: a frame shows the whole of what
        // JavaScript did, never a step of it, as when the two took turns. What the UI thread moves
        // meanwhile (an animation, a scroll) goes on being drawn. (A commit made on the UI thread
        // applies the queue at once, being behind it.) The person's input that came while it worked
        // is part of that work: handed to it first, the frame showing the answer to all of it (a
        // quick Super+Tab never flashes the switcher it opened and closed), unless what JavaScript
        // did has waited a refresh (input that never stops is shown answered a refresh at a time).
        // Keys typed together are, until JavaScript has answered the last (the events held behind a
        // key): eight arrows typed at once move the selection once, said once, as when the loop
        // asked JavaScript for each key's answer in its turn and mounted what it did after.
        auto applyWaited = mounter.waitingSince();
        bool waitedARefresh = applyWaited && Clock::now() - *applyWaited >=
                                                 std::chrono::duration<double, std::milli>(refresh.periodMs());
        bool keysAnswering = offeredKey.has_value() || !heldBehindKey.empty();
        // (What the compositor tells counts: a burst of an input method's candidates is shown as its
        // last.)
        bool pulled = js->idle() && !keysAnswering && (js->stagedRank() < 1 || waitedARefresh) && mounter.apply();
        // What JavaScript has answered: the batches it ran through, and that whose commits the frame
        // shows now.
        diagnostics.javascriptApplied(std::max(js->done(), mounter.appliedBatch()));
        // A drag JavaScript asked for starts once the batch that asked has run with all it set off
        // (React's render of what the press selected, which the same batch's motion asked a drag in
        // before it), and what it committed is mounted: its image is the item as the press left it
        // (drag.h).
        if (dragDrop.asking() && js->done() >= dragAskedIn) {
          if (!mounter.mountAtFrame) pulled |= mounter.apply();
          pulled |= dragDrop.startAsked();
        }
        bool busy = pulled;
        if (busy) diagnostics.trigger("JavaScript", 0);
        bool uiFrame = false;
        {
          Scope animation(Part::Animation);
          bool animating = gestures->tick();
          // Worklets' frame is not work by itself (worklets-host.h): what it changes reaches the host
          // as a mount, a UI job or a frame asked for.
          uiFrame = javascript(Part::Animation, [&] { return worklets.tick(); });
          animating |= javascript(Part::Animation, [&] { return reanimated.tick(); });
          animating |= animated.tick();
          {
            oxyos::JsThread::CausedBy frame({"an animation frame", 0});
            animating |= animationFrames.tick(window->ready(oxyos::Window::kToplevel));
          }
          animating |= pointer.tick(window->ready(oxyos::Window::kToplevel));
          // expo-image views' transitions and animated pictures: a frame drawn again.
          if (images.tick(window->ready(oxyos::Window::kToplevel))) {
            animating = true;
            dirty = true;
            popups.changed();
            shellSurfaces.changed();
          }
          if (animating) diagnostics.trigger("an animation frame");
          busy |= animating;
        }
        bool arrived = networking.deliver();
        arrived |= webSockets.deliver();
        arrived |= files.deliver();
        arrived |= dragDrop.deliver();
        arrived |= deliverApplication(services);
        if (arrived) diagnostics.trigger("data arriving");
        busy |= arrived;
        // The wallpaper follows the clock (docs/adr/0048-wallpapers.mdx: a variant of the sun's,
        // a shuffle's next), and the applications the session's registry
        // (docs/adr/0030-installing-applications.mdx): each looked at.
        bool wallpaperChanged = shellMode && shellState.tick();
        bool applicationsChanged = shellMode && shellState.applicationsChanged();
        bool shellChanged = wallpaperChanged || applicationsChanged, closeRequested = false;
        std::optional<OxyOSEvent> authenticated;
        std::vector<OxyOSEvent> launchResults;
        std::vector<OxyOSEvent> preferenceResults;
        std::vector<OxyOSEvent> menuChoices;
        std::vector<OxyOSEvent> systemShortcuts;
        // The latest page of candidates an input method handed the system, for the candidate
        // window (docs/adr/0041-input-methods.mdx): only the last of a turn counts.
        std::optional<oxyos::Window::Candidates> candidates;
        // The screenshot interface's changes and the window under the pointer, in order, and the
        // screenshots saved with their paths (docs/adr/0024-system-shortcuts.mdx, decision 10).
        std::vector<OxyOSEvent> captures;
        std::vector<std::pair<OxyOSEvent, std::string>> screenshots;
        std::vector<OxyOSEvent> bezels;
        std::vector<OxyOSEvent> notificationResults, notificationsRemoved;
        std::vector<std::pair<uint32_t, std::string>> systemAnswers, systemData, sheets, filesChosen, searchAnswers;
        std::vector<OxyOSEvent> sheetsGone, capabilityResults, backgroundStates, quitRequests;
        bool responded = false;
        // The modifiers held, as the compositor last told them this turn (protocol version 38, a
        // Mac's flagsChanged:), if it did: a menu's alternates follow Option at once.
        std::optional<uint8_t> modifiersHeld;
        Part handling = diagnostics.enter(Part::Events);
        // The rest of an event's handling, once a key offered to JavaScript first is answered (whether
        // it took it: `keyTaken`); every other event goes straight on to it.
        auto finishEvent = [&](const OxyOSEvent& event, bool keyTaken, const std::optional<NavigationKey>& platformKey,
                               bool withheld) {
              if ((event.kind == OXYOS_EVENT_POINTER && event.detail == OXYOS_POINTER_PRESS) ||
                  (event.kind == OXYOS_EVENT_KEY && event.flag) ||
                  // An assistive technology's action, as the compositor counts it: any but moving
                  // its focus (a custom action, the desktop's Move to Trash, ADR 0047, cites it).
                  (event.kind == OXYOS_EVENT_ACCESSIBILITY_ACTION && event.detail != OXYOS_ACTION_FOCUS) ||
                  // A choice in the shell's bar is the person's input: Edit > Paste may cite it.
                  event.kind == OXYOS_EVENT_MENU_ITEM_ACTIVATED ||
                  // So is a system shortcut the shell shows: the search's field may cite it.
                  event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT ||
                  // And the screenshot interface opened or changed by the keyboard.
                  event.kind == OXYOS_EVENT_CAPTURE_MODE ||
                  // And a notification the person pressed (ADR 0026): its response may copy.
                  event.kind == OXYOS_EVENT_NOTIFICATION_RESPONSE ||
                  // And a drop (ADR 0035): the dock's Trash cites it to put the files in the Trash.
                  event.kind == OXYOS_EVENT_DRAG_DROP ||
                  // And, in the shell, a scroll (a dock icon's opens App Exposé, which takes the
                  // keyboard citing it, docs/adr/0045-app-expose.mdx); an application's scroll
                  // authorizes nothing.
                  (asShell && event.kind == OXYOS_EVENT_SCROLL)) {
                lastPress = event.serial;
              }
              report(event);
              if (keyTaken) return;
              if (event.kind == OXYOS_EVENT_POPUP_DONE) popups.dismissed(event);
              // An assistive technology's action; a focus move needs new snapshots.
              if (event.kind == OXYOS_EVENT_ACCESSIBILITY_ACTION && accessibility.act(event)) {
                dirty = true;
                popups.changed();
                shellSurfaces.changed();
              }
              if (event.kind == OXYOS_EVENT_KEY || event.kind == OXYOS_EVENT_TEXT) popups.key(event);
              // Full Keyboard Access: the key the platform took acts now, its press recorded.
              if (platformKey) {
                navigate(event, *platformKey);
                return;
              }
              // Scrolling from the keyboard (keyScroll): a key a scroll view takes, and its release,
              // go no further.
              if (keyScroll(event)) {
                keyChords.taken(event.code);
                return;
              }
              // Escape is the desktop's back: the topmost modal is asked to close, as Android's
              // back asks a modal's window.
              constexpr uint32_t kEscape = 1;
              if (event.kind == OXYOS_EVENT_KEY && event.flag && event.code == kEscape &&
                  event.surface == oxyos::Window::kToplevel) {
                modalHost.requestClose();
              }
              if (event.kind == OXYOS_EVENT_TEXT_INPUT) {
                OxyOSTextTransaction change{};
                bool transacted = replaying ? replaying->transaction : window->transaction(change);
                if (replaying && transacted) change = replaying->kept();
                if (transacted) {
                  auto& r = oxyos::Report::get();
                  if (r.enabled()) {
                    r.input("text_input", event.surface,
                            folly::dynamic::object("commit", editing.logged({change.commit, change.commit_length}))(
                                "preedit", editing.logged({change.preedit, change.preedit_length}))(
                                "delete", folly::dynamic::array(change.delete_before, change.delete_after)));
                  }
                  auto told = toldSerials.find(change.surface);
                  editing.transaction(change, told != toldSerials.end() && told->second == change.commit_serial);
                  // AutoFill's fill, the person's pick (docs/adr/0050-keychain-autofill-and-passkeys.mdx):
                  // the field and its pair replaced whole; its text is never reported.
                  OxyOSTextFill filled{};
                  bool fills = replaying ? replaying->filled : window->fill(filled);
                  if (replaying && fills) filled = replaying->keptFill();
                  if (fills) editing.fill(filled);
                }
              }
              if (withheld) return;
              dragDrop.observe(event);
              pointer.handle(event);
              editing.handle(event);
        };
        auto handleEvent = [&](const OxyOSEvent& event) {
              // What an event dispatches reaches JavaScript on Fabric's next beat: a turn later.
              busy = true;
              diagnostics.input(event.kind, event.detail, event.time_ns);
              shellChanged |= shellState.observe(event);
              if (event.kind == OXYOS_EVENT_AUTHENTICATION_RESULT) authenticated = event;
              if (event.kind == OXYOS_EVENT_LAUNCH_RESULT) launchResults.push_back(event);
              if (event.kind == OXYOS_EVENT_PREFERENCE_RESULT) preferenceResults.push_back(event);
              if (event.kind == OXYOS_EVENT_SYSTEM_TEXT) {
                if (auto text = systemTexts.add(event.code, event.detail, event.flag != 0, event.text)) {
                  systemAnswers.emplace_back(event.code, std::move(*text));
                }
              }
              // A long answer whole (ADR 0063), handed to JavaScript as text.
              if (event.kind == OXYOS_EVENT_SYSTEM_DATA) {
                if (auto text = replaying ? replaying->systemData : window->systemData()) {
                  systemData.emplace_back(event.code, std::move(*text));
                }
              }
              if (event.kind == OXYOS_EVENT_SEARCH_TEXT) {
                if (auto text = searchTexts.add(event.code, event.detail, event.flag != 0, event.text)) {
                  searchAnswers.emplace_back(event.code, std::move(*text));
                }
              }
              if (event.kind == OXYOS_EVENT_SHEET_TEXT) {
                if (auto text = sheetTexts.add(event.code, event.detail, event.flag != 0, event.text)) {
                  sheets.emplace_back(event.code, std::move(*text));
                }
              }
              if (event.kind == OXYOS_EVENT_SHEET_DISMISSED) sheetsGone.push_back(event);
              if (event.kind == OXYOS_EVENT_FILES_CHOSEN) {
                if (event.flag) choosing.erase(event.code);
                if (auto text = chosenTexts.add(event.code, event.detail, event.flag != 0, event.text)) {
                  filesChosen.emplace_back(event.code, std::move(*text));
                }
              }
              if (event.kind == OXYOS_EVENT_CAPABILITY_RESULT) capabilityResults.push_back(event);
              if (event.kind == OXYOS_EVENT_BACKGROUND_ITEM_STATE) backgroundStates.push_back(event);
              if (event.kind == OXYOS_EVENT_QUIT_REQUESTED && !shellMode) quitRequests.push_back(event);
              if (event.kind == OXYOS_EVENT_MODIFIERS) modifiersHeld = event.modifiers;
              if (event.kind == OXYOS_EVENT_MENU_ITEM_ACTIVATED) menuChoices.push_back(event);
              if (event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT) systemShortcuts.push_back(event);
              if (event.kind == OXYOS_EVENT_INPUT_CANDIDATES) {
                if (auto list = replaying ? replaying->candidates : window->inputCandidates()) candidates = std::move(*list);
              }
              if (event.kind == OXYOS_EVENT_CAPTURE_MODE || event.kind == OXYOS_EVENT_CAPTURE_TARGET) {
                captures.push_back(event);
              }
              if (event.kind == OXYOS_EVENT_SCREENSHOT_SAVED) {
                if (auto path = screenshotPaths.add(event.code, event.detail, event.flag != 0, event.text)) {
                  screenshots.emplace_back(event, std::move(*path));
                }
              }
              if (event.kind == OXYOS_EVENT_CONTROL_LEVEL && (event.flag & OXYOS_CONTROL_BEZEL)) {
                bezels.push_back(event);
              }
              // An application's notifications (ADR 0026): what came of a post, the responses
              // (their texts first) and those no longer kept.
              if (!shellMode) {
                notificationTexts.observe(event);
                if (event.kind == OXYOS_EVENT_NOTIFICATION_RESULT) notificationResults.push_back(event);
                if (event.kind == OXYOS_EVENT_NOTIFICATION_REMOVED) {
                  notificationTexts.forget(event.code);
                  notificationsRemoved.push_back(event);
                }
                if (event.kind == OXYOS_EVENT_NOTIFICATION_RESPONSE) {
                  auto response = oxyos::notificationResponse(event, notificationTexts.take(event.code));
                  oxyos::Report::get().input("notification.response", 0, response);
                  services.notifications.responses.push_back(std::move(response));
                  responded = true;
                }
              }
              if (event.kind == OXYOS_EVENT_FOCUS) {
                // The shell's desktop, its background, gaining or losing the keyboard
                // (docs/adr/0047-finder-drags-and-the-desktop.mdx, decision 5): its selection is
                // drawn in the accent's colour only while it has it.
                if (shellMode && event.surface == oxyos::Window::kToplevel) {
                  services.emit("OxyOSShellDesktopKeyboard", folly::dynamic::object("focused", event.flag != 0));
                }
                if (event.flag) {
                  keyboardSurface = event.surface;
                } else if (keyboardSurface == event.surface && !popups.closedByApplication(event.surface) &&
                           !shellSurfaces.closedByApplication(event.surface)) {
                  // A popup or a shell region this client closed itself (a menu replaced by the
                  // next one, the top bar's Right, Control Center closed by its Escape) keeps the
                  // keyboard as far as this client knows until the compositor gives it to the
                  // surface next in line: never, between the two, to nothing, which would read as
                  // the keyboard gone elsewhere (the region asking for it then let go, if the two
                  // events are read in two turns, as a loaded machine has them).
                  keyboardSurface = 0;
                }
              }
              // The shell asks the application to close its window: it finishes, as an Android
              // activity does when its task is closed.
              if (event.kind == OXYOS_EVENT_ACTIVATED) {
                oxyos::Report::get().input(event.flag ? "window.active" : "window.inactive", event.surface);
              }
              if (event.kind == OXYOS_EVENT_CLOSE_REQUESTED && event.surface == oxyos::Window::kToplevel) {
                closeRequested = true;
                diagnostics.closeAsked("the session");
                oxyos::Report::get().input("close.requested", event.surface);
              }
              if (event.kind == OXYOS_EVENT_KEY) {
                bool down = event.flag != 0;
                switch (event.code) {
                  case 42: case 54: heldShift = down; break;
                  case 29: case 97: heldCtrl = down; break;
                  case 56: case 100: heldAlt = down; break;
                  case 125: case 126: heldMeta = down; break;
                  default: break;
                }
              }
              if (event.kind == OXYOS_EVENT_FOCUS && !event.flag) heldShift = heldCtrl = heldAlt = heldMeta = false;
              // A press that only activated its window (a Mac's first mouse declined, ADR 0016,
              // decision 9), and its release, reach nothing of it: no view, no caret, no keyboard
              // focus moved. They are still the person's input, reported as received.
              bool withheld = pointer.withheld(event);
              // A press of the pointer hides the keyboard's ring, as Android's touch mode does, and
              // takes focus off a control that is no text input (a press focuses inputs itself).
              if (event.kind == OXYOS_EVENT_POINTER && event.detail == OXYOS_POINTER_PRESS && !withheld) {
                modalFirstPending = -1;
                keyboardAfterMount = -1;
              }
              if (event.kind == OXYOS_EVENT_POINTER && event.detail == OXYOS_POINTER_PRESS && !withheld &&
                  keyboardFocus.focused() >= 0) {
                if (oxyos::editableInput(tree, keyboardFocus.focused())) {
                  if (keyboardFocus.ringShown()) redrawFocus();
                  keyboardFocus.pressed();
                } else {
                  moveKeyboardFocus(-1, false, event.surface);
                }
              }
              // A key press is offered to JavaScript first (Bloom's menu shortcuts, ADR 0021); one
              // it takes, and that key's release, go no further.
              // Full Keyboard Access (keyboard-focus.h): Tab and Shift+Tab move through the controls
              // of the surface with the keyboard; Space and Return activate the focused control and
              // the arrows step an adjustable one, as a Mac's do with Full Keyboard Access on. The
              // focused control has these keys before the application's own (a Mac's first
              // responder before the window's key handling), so a list's Space never takes a
              // toolbar button's.
              auto platformKey = navigationKey(event);
              bool keyTaken = false;
              if (event.kind == OXYOS_EVENT_KEY && services.keyboard.listener && !platformKey) {
                if (keyChords.swallow(event)) {
                  keyTaken = true;
                } else if (auto offered = keyChords.press(event)) {
                  // Answered on JavaScript's thread: this key, and every event after it, wait for the
                  // answer (resumeAfterKey), while the loop goes on drawing.
                  offerKey(event, std::move(*offered));
                  return;
                }
              }
              finishEvent(event, keyTaken, platformKey, withheld);
            };
        // A scenario's "popup-focus-first" order: the keyboard's moves held for a menu's focus,
        // handled now that it has it (or is gone), before what came since.
        if (keyboardHeldFor != 0 &&
            (accessibility.publishedFocus(keyboardHeldFor) || !popups.content(keyboardHeldFor))) {
          oxyos::Report::get().input("test.keyboard.given", keyboardHeldFor);
          keyboardHeldFor = 0;
          for (const auto& held : std::exchange(keyboardHeld, {})) handleEvent(held);
        }
        auto dispatched = [&](const OxyOSEvent& event) {
          if (keyboardHeldFor != 0 && event.kind == OXYOS_EVENT_FOCUS) {
            keyboardHeld.push_back(event);
            return;
          }
          handleEvent(event);
        };
        // A key JavaScript answered: its handling ends, then the events that waited behind it go on, in
        // order, until one is another key offered to JavaScript.
        while (offeredKey && offeredKey->answer->load() >= 0) {
          auto key = std::move(*offeredKey);
          offeredKey.reset();
          busy = true;
          oxyos::Report::Handling keyHandling(key.event, key.seq);
          bool taken = key.answer->load() == 1;
          if (taken) {
            keyChords.taken(key.event.code);
            oxyos::Report::get().deliver("shortcut", key.event.surface, shellMode ? "shell" : "menu-bar", key.offered);
          }
          finishEvent(key.event, taken, std::nullopt, false);
          while (!offeredKey && !heldBehindKey.empty()) {
            auto held = std::move(heldBehindKey.front());
            heldBehindKey.pop_front();
            oxyos::Report::Handling heldHandling(held.event, held.seq);
            handlingSeq = held.seq;
            replaying = held.payload.get();
            dispatched(held.event);
            replaying = nullptr;
          }
          // The next key of the burst is offered, or there is none: JavaScript goes on.
          keyBurst->decided(key.number, offeredKey.has_value());
        }
        // Every key answered and the events behind them handled: what was held is published.
        if (!offeredKey && keyOfferedAt) {
          keyOfferedAt.reset();
          if (accessibilityHeld) dirty = true;
        } else if (accessibilityHeld && keyOfferedAt && Clock::now() - *keyOfferedAt >= kAccessibilityHeldAtMost) {
          dirty = true;
        }
        // The inputs below that hand JavaScript something (an event dispatched to a view, a device
        // event) are answered once it ran this turn's batch, and all it set off.
        toJavaScript.handed();
        bool connected = window->dispatch([&](const OxyOSEvent& event) {
          // Window::dispatch reported it received just before.
          handlingSeq = oxyos::Report::get().receivedSeq();
          if (offeredKey || !heldBehindKey.empty()) {
            heldBehindKey.push_back(Held{event, handlingSeq, payloadOf(event)});
            return;
          }
          dispatched(event);
        });
        // What the events dispatched to views reaches JavaScript now (Fabric's beat), and the inputs are
        // its to answer.
        bool handed = toJavaScript.beat();
        diagnostics.handedToJavaScript(handed || toJavaScript.handed(), js->batch());
        diagnostics.leave(handling);
        // A drag that left this client's last surface in those events has left the window.
        dragDrop.observed();
        if (!connected) break;
        if (!shellMode) diagnostics.windowState(window->maximized(), window->active());
        // The window's display's refresh, told before its first configure and when it changes.
        if (window->refreshMhz() != toldRefresh) {
          toldRefresh = window->refreshMhz();
          refresh.set(toldRefresh);
          diagnostics.setRefresh(refresh.periodMs());
          oxyos::Report::get().refresh(toldRefresh);
        }
        // The modifiers held changed (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision
        // 2): told to JavaScript as their own event, as a Mac's flagsChanged:, so the Oxy menu's
        // alternates follow Option the instant it goes down or up, not at the next key or move.
        if (modifiersHeld) {
          uint8_t held = *modifiersHeld;
          oxyos::Report::get().input("modifiers.changed", 0, folly::dynamic::object("modifiers", static_cast<int>(held)));
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSModifiersChanged",
                                    folly::dynamic::object("altKey", (held & OXYOS_MODIFIER_ALT) != 0)(
                                        "ctrlKey", (held & OXYOS_MODIFIER_CONTROL) != 0)(
                                        "metaKey", (held & OXYOS_MODIFIER_SUPER) != 0)(
                                        "shiftKey", (held & OXYOS_MODIFIER_SHIFT) != 0)));
        }
        // The session asks it to quit before logging out, restarting or shutting down
        // (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 5), as a Mac asks
        // `applicationShouldTerminate:`: JavaScript decides while it handles the question (an
        // unsaved document asks to be saved, or cancels the whole end); otherwise it quits at once.
        for (const auto& asked : quitRequests) {
          const char* reason = asked.detail == OXYOS_SESSION_END_LOG_OUT  ? "logOut"
                               : asked.detail == OXYOS_SESSION_END_RESTART ? "restart"
                                                                            : "shutDown";
          folly::dynamic told = folly::dynamic::object("serial", asked.code)("reason", reason);
          oxyos::Report::get().input("quit.requested", 0, told);
          if (quitHandled) {
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSQuitRequested", told));
          } else if (services.quit.reply) {
            services.quit.reply(asked.code, true);
          }
        }
        // Sudden Termination (docs/adr/0064, decision 10): it may be ended at once, without being
        // asked, while JavaScript handles no question to quit, holds nothing off and no storage
        // request is unanswered (a write not yet on the disk); told to the compositor as it changes.
        if (services.quit.available) {
          bool clean = !quitHandled && suddenHolds == 0 && (!services.storage || services.storage->outstanding() == 0);
          if (suddenTold != clean && window->setSuddenTermination(clean)) {
            suddenTold = clean;
            oxyos::Report::get().input("sudden.termination", 0, folly::dynamic::object("enabled", clean));
          }
        }
        // Answered quit (here or by JavaScript): it ends now, as for a close request, whatever its
        // background activity, since the session is ending. Its answer was sent as it was given.
        if (quitAnswered) {
          std::cerr << "[host] " << applicationName << " quits: the session is ending\n";
          break;
        }
        if (closeRequested || closeAsked) {
          if (closeAsked) oxyos::Report::get().input("close.asked", oxyos::Window::kToplevel);
          // Its background activity (docs/adr/0060-login-items-and-background-activity.mdx): an
          // application the session allows runs on without its window, its JavaScript going on (a
          // chat's connection, a sync), as a Mac's application with no window open keeps running;
          // the session ends it when the person turns that off or another of its processes is
          // already there. Any other ends with its window, as an Android activity finishes.
          if (!backgroundEnabled || window->toplevelClosed()) break;
          closeAsked = false;
          window->closeToplevel();
          services.appState = "background";
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("appStateDidChange", folly::dynamic::object("app_state", "background")));
          oxyos::Report::get().input("background.entered", 0, folly::dynamic::object());
          std::cerr << "[host] " << applicationName << " runs on in the background\n";
        }
        // The session's preferences (docs/adr/0019-settings.mdx), as React Native's own modules tell
        // a change on Android: the appearance (a configuration change), reduce motion; the rest to
        // Bloom's adapter; and, to Settings, every preference as kept and what came of a change.
        if (window && toldDrawing && !(*toldDrawing == window->drawing())) {
          const auto& now = window->drawing();
          auto emit = [&](const char* name, folly::dynamic payload) {
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(name, payload));
          };
          if (now.dark != toldDrawing->dark) {
            emit("appearanceChanged", folly::dynamic::object("colorScheme", now.dark ? "dark" : "light"));
            // The window's own background follows, drawn again whole.
            display.background =
                now.dark ? oxyos::Display::kDarkWindowBackground : oxyos::Display::kLightWindowBackground;
            windowCanvas = oxyos::Canvas{};
            dirty = true;
          }
          if (now.reduceMotion != toldDrawing->reduceMotion) emit("reduceMotionDidChange", now.reduceMotion);
          // Reduce transparency: expo-blur's views draw their tint alone from now on (or blur
          // again), every one of them, though none of their props changed: the window is drawn
          // again whole, as when the painter changes.
          if (now.reduceTransparency != toldDrawing->reduceTransparency) {
            oxyos::setReducedTransparency(now.reduceTransparency);
            painterChanged = true;
            dirty = true;
          }
          toJavaScript.tellState("OxyOSPreferencesChanged", services.settings.drawing());
          oxyos::Report::get().input("preferences", oxyos::Window::kToplevel, services.settings.drawing());
          toldDrawing = now;
        }
        if (window && window->preferences() != toldPreferences) {
          toldPreferences = window->preferences();
          toJavaScript.tellState("OxyOSSettingsChanged", services.settings.values());
        }
        // Settings' applications (`getApplications`): told to read them again as the registry
        // changes, and once as the watch begins, for a change before it. The watch says they may
        // have changed (a file of its directories written): told only when the listing did, so a
        // preference written beside them re-renders nothing.
        if (window && !shellMode && !settingsApplicationsAsked && window->settingsGranted()) {
          settingsApplicationsAsked = true;
          settingsApplications.reset(oxyos_applications_watch_new());
          if (!settingsApplications) {
            std::cerr << "[host] Settings' applications are not followed: " << oxyos_last_error() << "\n";
          }
          toldApplications = applicationsListing();
          toJavaScript.tellState("OxyOSApplicationsChanged", nullptr);
        } else if (settingsApplications && oxyos_applications_watch_changed(settingsApplications.get())) {
          if (auto listing = applicationsListing(); listing != toldApplications) {
            toldApplications = std::move(listing);
            toJavaScript.tellState("OxyOSApplicationsChanged", nullptr);
          }
        }
        // A change of the session's locale (docs/adr/0040-language-and-region.mdx, decision 4):
        // Intl's default is already the new one (the window set it); JavaScript is told, and
        // expo-localization's module tells its listeners, as Android's configuration change does.
        if (window && window->locale() != toldLocale) {
          toldLocale = window->locale();
          // The shell's applications by their names in the new language, and Settings': the
          // shell is told them now, so the bar's application menu and the dock say the new names
          // at once (docs/adr/0021-menu-bar.mdx).
          if (shellMode) {
            shellState.relocalize();
            shellChanged = true;
          }
          if (settingsApplications) {
            toJavaScript.tellState("OxyOSApplicationsChanged", nullptr);
          }
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSLocaleChanged",
                                                               folly::dynamic::object("locale", toldLocale)));
          oxyos::Report::get().input("locale", oxyos::Window::kToplevel, folly::dynamic::object("locale", toldLocale));
        }
        // The displays as the session tells Settings them (docs/adr/0018-several-displays.mdx).
        if (window && services.settings.displays) {
          auto displays = services.settings.displays();
          if (displays != toldDisplays) {
            toldDisplays = displays;
            toJavaScript.tellState("OxyOSDisplaysChanged", displays);
            if (!displays.isNull()) oxyos::Report::get().input("displays", oxyos::Window::kToplevel, displays);
          }
        }
        // Installing applications (ADR 0030): what the session answered, the shell's sheets, and
        // capability results; a capability granted holds for this process from now on.
        for (const auto& [serial, text] : systemAnswers) {
          auto answer = folly::parseJson(text);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSystemAnswer",
                                                               folly::dynamic::object("serial", serial)("answer", answer)));
          oxyos::Report::get().input("system", oxyos::Window::kToplevel,
                                     folly::dynamic::object("serial", serial)("answer", answer));
        }
        // A long answer (SystemData, docs/adr/0063-activity-monitor.mdx: Activity Monitor's samples, tens
        // of kilobytes every two seconds) as the text the session wrote, which JavaScript parses
        // itself (oxyos/system.js): Hermes' JSON.parse, never a folly::dynamic made into thousands
        // of JSI values; the report records its size, not its text.
        for (const auto& [serial, text] : systemData) {
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSystemAnswer",
                                                               folly::dynamic::object("serial", serial)("text", text)));
          oxyos::Report::get().input("system", oxyos::Window::kToplevel,
                                     folly::dynamic::object("serial", serial)("bytes", text.size()));
        }
        // Search (docs/adr/0051-search.mdx): the index's answers as it wrote them, JSON text the
        // window's search client reads (the platform's `search`).
        for (const auto& [serial, text] : searchAnswers) {
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSearchText",
                                                               folly::dynamic::object("serial", serial)("text", text)));
          oxyos::Report::get().input("search", oxyos::Window::kToplevel,
                                     folly::dynamic::object("serial", serial)("text", text));
        }
        for (const auto& [sheet, text] : sheets) {
          auto described = folly::parseJson(text);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSheet",
                                                               folly::dynamic::object("sheet", sheet)("text", text)));
          oxyos::Report::get().input("sheet", oxyos::Window::kToplevel,
                                     folly::dynamic::object("sheet", sheet)("kind", described["kind"]));
        }
        // File choosers (ADR 0029, decision 8): each request's answer, once.
        for (const auto& [serial, text] : filesChosen) {
          auto answer = folly::parseJson(text);
          // What the person chose is theirs to give: the application reaches it without `files`.
          if (auto chosen = answer.get_ptr("chosen"); chosen && chosen->isArray()) {
            for (const auto& uri : *chosen) {
              if (uri.isString()) files.give(uri.getString());
            }
          }
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSFilesChosen",
                                                               folly::dynamic::object("serial", serial)("answer", answer)));
          oxyos::Report::get().input("files_chosen", oxyos::Window::kToplevel,
                                     folly::dynamic::object("serial", serial)("answer", answer));
        }
        for (const auto& gone : sheetsGone) {
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSheetDismissed",
                                                               folly::dynamic::object("sheet", gone.code)));
        }
        // How its login item and background activity stand (docs/adr/0060-login-items-and-background-activity.mdx):
        // an answer, or a change the person made (serial 0), to JavaScript; and whether it may run
        // on without its window, which its close below follows.
        for (const auto& state : backgroundStates) {
          bool background = state.detail == OXYOS_BACKGROUND_ITEM_BACKGROUND;
          if (background) backgroundEnabled = state.flag == OXYOS_BACKGROUND_ENABLED;
          const char* status = state.flag == OXYOS_BACKGROUND_ENABLED             ? "enabled"
                               : state.flag == OXYOS_BACKGROUND_REQUIRES_APPROVAL ? "requiresApproval"
                               : state.flag == OXYOS_BACKGROUND_NOT_PERMITTED     ? "notPermitted"
                                                                                  : "notRegistered";
          folly::dynamic told = folly::dynamic::object("serial", state.code)(
              "kind", background ? "background" : "loginItem")("status", status);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSBackgroundItemState", told));
          oxyos::Report::get().input("background.state", 0, told);
        }
        for (const auto& result : capabilityResults) {
          auto asked = askedCapabilities.find(result.code);
          std::string capability = asked == askedCapabilities.end() ? "" : asked->second;
          if (asked != askedCapabilities.end()) askedCapabilities.erase(asked);
          if (result.flag != 0 && !capability.empty()) {
            launchCapabilities.grant(capability);
            if (capability == "files") files.grantThePersonsFiles();
          }
          folly::dynamic told = folly::dynamic::object("serial", result.code)("granted", result.flag != 0)("capability", capability);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSCapabilityResult", told));
          oxyos::Report::get().input("capability", oxyos::Window::kToplevel, told);
        }
        for (const auto& result : preferenceResults) {
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSSettingsResult",
                                    folly::dynamic::object("serial", result.code)("accepted", result.flag != 0)));
        }
        // The window's state, for its controls (docs/adr/0016-windows.mdx): whether it is active
        // (ACTIVATED), zoomed or full screen (its configures), told to JavaScript when it changes.
        if (services.window.state) {
          auto state = services.window.state();
          if (state != toldWindowState) {
            toldWindowState = state;
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSWindowChanged", state));
          }
        }
        if (shellChanged) {
          auto state = shellState.state();
          if (state != toldShellState) {
            toldShellState = state;
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellChanged", std::move(state)));
          }
        }
        // An application's notifications (ADR 0026): what came of its posts, those no longer kept,
        // and that responses wait (JavaScript takes them, each once).
        for (const auto& result : notificationResults) {
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSNotificationResult", oxyos::notificationResult(result)));
        }
        for (const auto& removed : notificationsRemoved) {
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSNotificationRemoved", folly::dynamic::object("notification", removed.code)));
        }
        if (responded) {
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSNotificationResponse", folly::dynamic::object()));
        }
        // Whether the applications the shell asked for were started (ADR 0013).
        for (const auto& result : launchResults) {
          oxyos::Report::get().input("launch.result", 0,
                                     folly::dynamic::object("serial", result.code)("launched", result.flag != 0));
          // An application's URL opened: its own answer.
          if (auto asked = urlOpens.find(result.code); asked != urlOpens.end()) {
            auto done = std::move(asked->second);
            urlOpens.erase(asked);
            if (result.flag != 0) {
              done("", "");
            } else {
              done("notOpened", "no application opened it");
            }
            continue;
          }
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSShellLaunch",
                                    folly::dynamic::object("serial", result.code)("launched", result.flag != 0)));
        }
        // A command of the window's menus chosen in the shell's bar (ADR 0021): to the adapter,
        // which runs the one Bloom declared at that index of that commit.
        for (const auto& choice : menuChoices) {
          oxyos::Report::Handling handling(choice);
          folly::dynamic payload = folly::dynamic::object("serial", choice.code)("index", choice.detail);
          oxyos::Report::get().input("menu.activated", choice.surface, payload);
          oxyos::Report::get().deliver("menuActivated", choice.surface, "menu-bar", payload);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSMenuActivated", payload));
        }
        // The system shortcuts the shell shows (docs/adr/0024-system-shortcuts.mdx): the
        // application switcher's and the search's, to JavaScript in order.
        for (const auto& shortcut : systemShortcuts) {
          static const char* actions[] = {"",
                                          "switcher-next",
                                          "switcher-previous",
                                          "switcher-quit",
                                          "switcher-hide",
                                          "switcher-done",
                                          "switcher-cancel",
                                          "search",
                                          "menu-bar",
                                          "dock",
                                          "status-menus",
                                          "notifications",
                                          "mission-control",
                                          "app-expose",
                                          "spotlight",
                                          "reveal"};
          const char* action = shortcut.detail <= OXYOS_SHORTCUT_REVEAL ? actions[shortcut.detail] : "";
          // What it asks of what it shows (docs/adr/0046-interactive-gestures.mdx): a key's toggle,
          // or a swipe's opening or closing it.
          const char* intent = shortcut.flag == OXYOS_SHORTCUT_INTENT_OPEN    ? "open"
                               : shortcut.flag == OXYOS_SHORTCUT_INTENT_CLOSE ? "close"
                                                                              : "toggle";
          folly::dynamic payload =
              folly::dynamic::object("action", action)("output", shortcut.code)("intent", intent);
          oxyos::Report::get().input("shortcut.system", 0, payload);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSShellShortcut", payload));
        }
        // The candidates to show, or none (docs/adr/0041-input-methods.mdx): { candidates, aux,
        // highlighted (-1: none), previous, next, vertical }. A whole state: those not yet delivered
        // give way to the newer.
        if (candidates) {
          folly::dynamic texts = folly::dynamic::array();
          for (const auto& text : candidates->candidates) texts.push_back(text);
          folly::dynamic payload = folly::dynamic::object("candidates", texts)("aux", candidates->aux)(
              "highlighted", candidates->highlighted)("previous", candidates->previous)("next", candidates->next)(
              "vertical", candidates->vertical);
          oxyos::Report::get().input("candidates", 0, payload);
          toJavaScript.tellState("OxyOSShellCandidates", std::move(payload));
        }
        // The screenshot interface (docs/adr/0024-system-shortcuts.mdx, decision 10): its changes
        // and the window under the pointer, to JavaScript in order; then the screenshots saved.
        for (const auto& capture : captures) {
          if (capture.kind == OXYOS_EVENT_CAPTURE_MODE) {
            static const char* modes[] = {"", "selection", "window", "toolbar", "confirm", "ended"};
            const char* mode = capture.detail <= OXYOS_CAPTURE_MODE_ENDED ? modes[capture.detail] : "";
            folly::dynamic payload = folly::dynamic::object("mode", mode)("serial", static_cast<double>(capture.serial))(
                "output", capture.code);
            oxyos::Report::get().input("capture.mode", 0, payload);
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellCapture", payload));
          } else {
            folly::dynamic payload = folly::dynamic::object("window", capture.code)("x", capture.x)("y", capture.y)(
                "width", capture.width)("height", capture.height);
            oxyos::Report::get().input("capture.target", 0, payload);
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellCaptureTarget", payload));
          }
        }
        for (const auto& [saved, path] : screenshots) {
          folly::dynamic payload = folly::dynamic::object("id", saved.code)("output", saved.x)("width", saved.width)(
              "height", saved.height)("path", path);
          oxyos::Report::get().input("screenshot.saved", 0, payload);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSShellScreenshot", payload));
        }
        // A hardware key's level for the shell's bezel (docs/adr/0025-hardware-keys-and-control-center.mdx),
        // after the state that holds it: to JavaScript in order, each a bezel shown again.
        for (const auto& bezel : bezels) {
          static const char* controls[] = {"", "output", "input", "display", "keyboard"};
          const char* control = bezel.detail <= OXYOS_CONTROL_KEYBOARD_BRIGHTNESS ? controls[bezel.detail] : "";
          folly::dynamic payload = folly::dynamic::object("control", control)("level", bezel.code / 65536.0)(
              "muted", (bezel.flag & OXYOS_CONTROL_MUTED) != 0);
          oxyos::Report::get().input("control.bezel", 0, payload);
          toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSShellBezel", payload));
        }
        // A text field JavaScript focused in one of the shell's regions (the launcher's search)
        // takes the keyboard there: the shell asks for its surface's focus, citing the latest
        // input, once the region has a surface and it is on the screen (a region shown in the
        // same commit has neither yet). A press already gave it; the compositor ignores an input
        // it did not give lately.
        if (shellMode && editing.focused() != focusAskedFor) {
          auto field = editing.focused();
          auto surface = field >= 0 ? shellSurfaces.surfaceOf(field) : std::nullopt;
          if (field < 0 || (surface && *surface == keyboardSurface)) {
            focusAskedFor = field;
          } else if (auto* shown = surface ? window->surface(*surface) : nullptr; shown && shown->shown) {
            window->focusShellSurface(*surface, lastPress);
            oxyos::Report::get().input("focus.asked", *surface);
            focusAskedFor = field;
          }
        }
        // A region asking for the keyboard (the top bar from Super+F2, docs/adr/0024-system-
        // shortcuts.mdx) takes it as a field does above, citing the latest input (the shortcut).
        // Once it held it, the keyboard leaving the region and its popups (a press in a window, the
        // overview) is told to JavaScript, which leaves the bar: what has the keyboard is never a
        // bar that no longer receives keys.
        if (shellMode) {
          // A scenario's "grant-first" order: the focus moves held for the keyboard, delivered now
          // that it is here (or no longer asked for).
          if (!heldForGrant.empty() && !waitsForGrant(heldForGrant.front().first, heldForGrant.front().second)) {
            auto held = std::exchange(heldForGrant, {});
            for (auto& [view, type] : held) mounter.onAccessibilityEvent(view, type);
          }
          uint32_t wanted = shellSurfaces.keyboardRegion().value_or(0);
          // The region gives the keyboard back (the bar left by Escape or a row chosen) or loses it
          // (a press elsewhere): the accessibility focus its keys moved over its titles goes with
          // it, as VoiceOver's leaves a Mac's menu bar for the window. What is focused is never a
          // title of a bar the keyboard has left.
          auto releaseFocus = [&](uint32_t region) {
            releasedRegion = region;
            auto focused = accessibility.focused();
            if (region == 0 || focused < 0) return;
            auto in = shellSurfaces.surfaceOf(focused);
            bool inPopup = popups.surfaceOf(focused) != oxyos::Window::kToplevel;
            if ((in && *in == region) || inPopup) {
              accessibility.focus(-1);
              shellSurfaces.changed();
              popups.changed();
              oxyos::Report::get().input("a11y.focus.released", region);
            }
          };
          if (wanted != regionAskedFor && wanted == 0 && regionHeldKeyboard) releaseFocus(regionAskedFor);
          // The keyboard back on the region, or the region asking for it again: it is the bar's.
          if (releasedRegion != 0 && (keyboardSurface == releasedRegion || wanted != 0)) releasedRegion = 0;
          if (wanted != regionAskedFor) {
            regionAskedFor = wanted;
            regionHeldKeyboard = false;
            if (wanted != 0 && wanted != keyboardSurface) {
              if (focusFirst && !accessibility.publishedFocus(wanted)) {
                // A scenario's "focus-first" order: asked for once the region's tree is published
                // with the focus JavaScript moved into it.
                if (!keyboardHeldForFocus) oxyos::Report::get().input("test.keyboard.held", wanted);
                keyboardHeldForFocus = true;
                regionAskedFor = 0;
              } else if (auto* shown = window->surface(wanted); shown && shown->shown) {
                keyboardHeldForFocus = false;
                window->focusShellSurface(wanted, lastPress);
                oxyos::Report::get().input("focus.asked", wanted);
              } else {
                regionAskedFor = 0;  // asked again once it is shown
              }
            }
          }
          // Held while the keyboard is on the region or a popup of this client's: anything but no
          // surface of this client's, the shell's background or another region. (A popup closing
          // is still the keyboard's surface until the compositor moves it back to the region.)
          bool elsewhere = keyboardSurface == 0 || keyboardSurface == oxyos::Window::kToplevel ||
                           (keyboardSurface != wanted && shellSurfaces.content(keyboardSurface).has_value());
          bool held = wanted != 0 && !elsewhere;
          if (held) {
            regionHeldKeyboard = true;
          } else if (regionHeldKeyboard) {
            regionHeldKeyboard = false;
            releaseFocus(wanted);
            oxyos::Report::get().input("keyboard.lost", wanted);
            toJavaScript.callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellKeyboardLost", folly::dynamic::object()));
          }
        }
        // What came of the lock screen's password (docs/adr/0010-lock-screen.mdx).
        if (authenticated) {
          static const char* outcomes[] = {"", "accepted", "rejected", "busy", "throttled"};
          const char* outcome = authenticated->detail <= OXYOS_AUTH_THROTTLED ? outcomes[authenticated->detail] : "";
          oxyos::Report::get().input("lock.authentication", authenticated->surface,
                                     folly::dynamic::object("outcome", outcome)("wait", authenticated->code));
          toJavaScript.callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSShellAuthentication",
                                    folly::dynamic::object("outcome", outcome)("wait", authenticated->code)));
        }
        // Focus, caret and selection moves draw even when the tree did not change, and so does the
        // caret's blink.
        if (window) editing.setCaretBlinks(window->drawing().caretBlink);
        // Whether the focused field's window is the key window, the one the person's keys are for
        // (docs/adr/0037-text-editing.mdx): out of it, no caret and a grey selection. An
        // application's window is key while the compositor says it is the active one (ACTIVATED):
        // the keyboard on its own surfaces or its popups (a menu, which leaves a Mac's key window
        // key), or on the shell's bar and its menus acting for it; not while a file chooser's
        // sheet it asked for has the keyboard, which is then the key window though the window
        // stays active, as a Mac's sheet is key over its main window; not while another window,
        // the launcher, an overlay of the shell or the lock screen has it, all of which leave no
        // window or another one active. The shell has no window: its field is key while the
        // keyboard is on the region it is drawn in or on one of the shell's popups.
        if (window) {
          bool key = window->active() && (keyboardSurface != 0 || choosing.empty());
          if (shellMode) {
            auto field = editing.field();
            key = !field || (keyboardSurface != 0 &&
                             (field->surface == keyboardSurface || popups.content(keyboardSurface).has_value()));
          }
          editing.setWindow(key, window->drawing().dark);
        }
        editing.tick(std::chrono::steady_clock::now());
        // A scroll view laid out again is clamped to its content before the keyboard's navigation
        // reads the tree.
        pointer.clampScrolls();
        // What the window's first responder can do, told to Bloom's `editing` as it changes (focus,
        // the input's text, selection or history, the clipboard), so the Edit menu's items are
        // enabled as the field validates them, in the shell's bar too.
        if (!shellMode) {
          auto responder = editing.responder();
          if (!toldResponder || *toldResponder != responder) {
            bool first = !toldResponder.has_value();
            toldResponder = responder;
            if (!first || responder) {
              auto payload = responderState();
              oxyos::Report::get().input("editing.responder", 0,
                                         folly::dynamic::object("state", payload));
              toJavaScript.tellState("OxyOSEditingChanged", payload);
            }
          }
        }
        // The keyboard's navigation follows a modal coming and going; its ring is drawn with the
        // focused control.
        followScope();
        if (modalFirstPending >= 0) {
          auto scope = navigationScope(keyboardSurface);
          // The ring was showing as it came (it is set only then); a focus in it since (a press, an
          // application's) ends the wait, and a control of its own the application asked for, not
          // laid out or shown yet, prolongs it.
          auto focused = keyboardFocus.focused();
          bool awaited = keyboardAfterMount >= 0 && tree.views().count(keyboardAfterMount) &&
                         insideOf(keyboardAfterMount, scope);
          if (scope != modalFirstPending || (focused >= 0 && insideOf(focused, scope))) {
            modalFirstPending = -1;
          } else if (auto first = oxyos::focusables(tree, scope); !awaited && !first.empty()) {
            modalFirstPending = -1;
            moveKeyboardFocus(first.front(), true, keyboardSurface);
          }
        }
        if (keyboardAfterMount >= 0) {
          auto tag = keyboardAfterMount;
          keyboardAfterMount = -1;
          auto scope = navigationScope(keyboardSurface);
          if ((keyboardFocus.ringShown() || modalFirstPending == scope) && scope >= 0 &&
              tag != keyboardFocus.focused() && tree.views().count(tag) && insideOf(tag, scope)) {
            if (oxyos::focusable(tree, tag)) {
              applicationFocus = {tag, keyboardFocus.focused()};
              moveKeyboardFocus(tag, true, keyboardSurface);
            } else if (modalFirstPending == scope && oxyos::focusableOnceShown(tree, tag)) {
              // A control of a modal coming that is not laid out or shown yet (a dialog's buttons
              // until it has measured itself, a frame later): the modal waits for it, and it takes
              // the keyboard once it can, unless the person moves the keyboard first.
              keyboardAfterMount = tag;
            }
          }
        }
        // A shell region given the keyboard puts its focus where it asks (its `focusTarget`).
        if (shellMode) {
          if (auto asked = shellSurfaces.focusRequest(); asked && keyboardSurface == asked->surface) {
            auto& applied = focusRequestsApplied[asked->region];
            if (!applied || *applied != asked->request) {
              auto root = oxyos::navigationRoot(tree, asked->region);
              auto all = oxyos::focusables(tree, root);
              facebook::react::Tag target = -1;
              for (auto tag : all) {
                auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(tag).view.props);
                if (asked->target == "first" || (props && props->testId == asked->target)) {
                  target = tag;
                  break;
                }
              }
              // Applied once its target is there: a region whose controls come a frame after it
              // has the keyboard (the overview's desktops, laid out once measured) is asked again.
              if (target >= 0) {
                applied = asked->request;
                moveKeyboardFocus(target, true, asked->surface);
              }
            }
          }
          for (auto it = focusRequestsApplied.begin(); it != focusRequestsApplied.end();) {
            auto asked = shellSurfaces.focusRequest();
            bool holding = asked && asked->region == it->first && keyboardSurface == asked->surface;
            it = holding ? std::next(it) : focusRequestsApplied.erase(it);
          }
        }
        auto shownEdit = editing.edit();
        // The ring shows while the surface the keyboard focused has the keyboard (or a popup of its
        // own, a menu opened from it): never in a window or a panel the keyboard left.
        bool ringHere = keyboardFocus.ringShown() && keyboardSurface != 0 &&
                        (popups.content(keyboardSurface) ||
                         insideOf(keyboardFocus.focused(), navigationScope(keyboardSurface)));
        shownEdit.focusRing = ringHere ? keyboardFocus.focused() : -1;
        // In the session's accent, light or dark, as a Mac draws its ring in the accent colour.
        if (window) {
          auto ring = runtimeOf.focusRings.find(window->drawing().accent);
          if (ring != runtimeOf.focusRings.end()) {
            shownEdit.focusRingColor = window->drawing().dark ? ring->second.second : ring->second.first;
          }
        }
        bool edited = shownEdit != drawn;
        // The ring's colour as it is drawn, for the scenarios (regress.py's keyboard_focus "color").
        if (shownEdit.focusRing >= 0 &&
            (drawn.focusRing < 0 || drawn.focusRingColor != shownEdit.focusRingColor)) {
          char color[8];
          std::snprintf(color, sizeof color, "#%06x", shownEdit.focusRingColor & 0xFFFFFF);
          oxyos::Report::get().input("keyboard.ring", keyboardSurface, folly::dynamic::object("color", color));
        }
        if (edited) {
          drawn = std::move(shownEdit);
          dirty = true;
          popups.changed();
          shellSurfaces.changed();
        }
        // The GPU's painter, started on a thread of its own, takes over: everything is drawn whole
        // on it.
        if (window->painterStarted()) {
          diagnostics.trigger("the GPU painter starting");
          painterChanged = true;
          dirty = true;
          busy = true;
          popups.changed();
          shellSurfaces.redrawWhole();
        }
        bool resized = window->resized();
        if (resized) {
          diagnostics.resized(window->resizedAt(), window->width(), window->height());
          display = oxyos::Display{window->width(), window->height(), window->scale(), display.background};
          pointer.setScale(display.scale);
          // useWindowDimensions and Dimensions follow the window (a resize, the output's scale or
          // size changed in Settings), as Android tells them on a configuration change.
          toJavaScript.tellState("didUpdateDimensions", oxyos::deviceDimensions(display));
          auto laying = Clock::now();
          {
            Scope fabric(Part::Layout);
            layOut();
          }
          double laidOutMs = std::chrono::duration<double, std::milli>(Clock::now() - laying).count();
          diagnostics.laidOut(laidOutMs);
          oxyos::Report::get().layout(display.width, display.height, laidOutMs);
          // A new size or scale needs a new frame even when the layout does not change (a
          // scale change keeps every logical frame); the old one would be discarded.
          dirty = true;
          busy = true;
        }
        // A scroll view's overlay scrollbars appearing, fading, growing or going (scrollbars.h).
        if (pointer.takeScrollbarsChanged()) {
          tree.setScrollbars(pointer.scrollbars());
          dirty = true;
          busy = true;
          popups.changed();
          shellSurfaces.changed();
        }
        // A scroll view stretched past its end, or springing back (overscroll, input.h), draws again.
        if (pointer.takeStretchChanged()) {
          tree.setStretches(pointer.stretches());
          dirty = true;
          busy = true;
          popups.changed();
          shellSurfaces.changed();
        }
        bool mounted = mounter.takeChanged();
        if (mounted) {
          editing.attached();
          // A drop target under a drag may have changed (its folder): the compositor answered anew.
          dragDrop.mounted();
          dirty = true;
          busy = true;
          popups.changed();
          shellSurfaces.changed();
          safeAreas.sync(tree);
        }
        // A field the assistive technology's focus followed (editing.onFocus) that lost its focus or
        // went (a rename cancelled, a dialog's field closed with it, a press on a list's row) leaves
        // that focus with the keyboard's, as VoiceOver's cursor stays with the keyboard's focus: the
        // control the keys are on again, else where it was before the field took it.
        if (fieldFollowed >= 0 && accessibility.focused() == fieldFollowed &&
            (editing.focused() != fieldFollowed || !tree.views().count(fieldFollowed))) {
          auto back = keyboardFocus.focused() >= 0 ? keyboardFocus.focused() : fieldFollowedFrom;
          if (back >= 0 && back != fieldFollowed && tree.views().count(back)) {
            accessibility.focus(back);
            dirty = true;
            popups.changed();
            shellSurfaces.changed();
          }
          fieldFollowed = -1;
        } else if (fieldFollowed >= 0 && accessibility.focused() != fieldFollowed) {
          fieldFollowed = -1;
        }
        // The new size's layout reaches JavaScript at once (its onLayout, through Fabric's beat, and the
        // dimensions): settled once JavaScript ran all of it and what that set off, its thread idle, and
        // the loop applied what it committed; or, but for a resize's first frame, once a refresh
        // passed. JavaScript's thread wakes the loop as it settles.
        if (resized) {
          toJavaScript.beat();
          settleFrom = Clock::now();
          // A resize after a pause in configures (a live resize's come a frame apart) begins anew.
          constexpr auto kResizeBegins = std::chrono::milliseconds(500);
          if (!lastResized || settleFrom - *lastResized >= kResizeBegins) resizeShown = false;
          lastResized = settleFrom;
          settleBy = settleFrom + (resizeShown ? std::chrono::duration_cast<Clock::duration>(refresh.period())
                                               : std::chrono::duration_cast<Clock::duration>(std::chrono::seconds(1)));
          settleAfter = js->batch();
          settleTurns = 0;
        } else if (settleBy) {
          ++settleTurns;
          bool quiet = js->done() >= settleAfter && js->idle() && !mounter.pending();
          bool timedOut = !quiet && Clock::now() >= *settleBy;
          if (quiet || timedOut) {
            settleBy.reset();
            resizeShown = true;
            double settledMs = std::chrono::duration<double, std::milli>(Clock::now() - settleFrom).count();
            diagnostics.settled(settledMs, timedOut);
            oxyos::Report::get().settled(settledMs, settleTurns, timedOut);
          }
        }
        bool settling = settleBy.has_value();
        // Images load at the size they are drawn; a picture arriving draws its view again.
        if (resized || mounted) images.sync(display.scale);
        if (images.deliver()) {
          diagnostics.trigger("an image arriving");
          dirty = true;
          busy = true;
          popups.changed();
          shellSurfaces.changed();
        }
        // A paste's text arriving is an edit of the focused input.
        if (window->deliverPastes()) busy = true;
        if (resized || mounted) shellSurfaces.sync();
        if (resized || mounted) modalHost.sync(display);
        // A modal's window animating in or out moves on once for each frame the window can take.
        if (window->ready(oxyos::Window::kToplevel) && modalHost.tick(display)) {
          dirty = true;
          busy = true;
        }
        if (!settling) popups.sync(lastPress);
        if (popupFocusFirst && keyboardHeldFor == 0) {
          if (auto opened = popups.awaitingFocus(); opened != 0 && keyboardHeldOnce.insert(opened).second) {
            oxyos::Report::get().input("test.keyboard.held", opened);
            keyboardHeldFor = opened;
          }
        }
        // The compositor is told about the focused input before the frame showing it, so its
        // state is in place when that frame is; the caret is only measured again when something
        // that moves it changed.
        if (auto field = editing.field(); field != toldField || (field && (edited || resized || mounted))) {
          auto caret = field ? editing.caretRect() : std::nullopt;
          if (field != toldField || caret != toldCaret) {
            if (toldField && (!field || field->surface != toldField->surface)) {
              window->textInput(toldField->surface, nullptr, false);
              toldSerials.erase(toldField->surface);
            }
            if (field) {
              OxyOSTextField told{.purpose = field->purpose,
                                  .hints = field->hints,
                                  .text = field->text.data(),
                                  .length = field->text.size(),
                                  .anchor = static_cast<uint32_t>(field->anchor),
                                  .cursor = static_cast<uint32_t>(field->cursor),
                                  .has_caret = caret.has_value()};
              if (caret) {
                told.caret_x = static_cast<int32_t>(std::floor(caret->origin.x));
                told.caret_y = static_cast<int32_t>(std::floor(caret->origin.y));
                told.caret_width = static_cast<uint32_t>(std::ceil(caret->size.width));
                told.caret_height = static_cast<uint32_t>(std::ceil(caret->size.height));
              }
              bool newField = !toldField || toldField->tag != field->tag || toldField->surface != field->surface;
              toldSerials[field->surface] =
                  window->textInput(field->surface, &told, newField, field->content, field->paired);
            }
            toldField = std::move(field);
            toldCaret = caret;
          }
        }
        // Every configure is answered with a frame, even one that changes nothing it shows.
        if (!window->answered(oxyos::Window::kToplevel)) dirty = true;
        // The window's first frame shows what the application first mounted, as the single thread's
        // did: it waits for JavaScript's first commit, or for JavaScript to have run its bundle and
        // gone idle without one (an application that draws nothing), never shows an empty window
        // before JavaScript had its turn.
        if (!firstShown && (mounter.transactions > 0 || (bundleRan && js->idle()))) firstShown = true;
        // A window closed to run on in the background (docs/adr/0060-login-items-and-background-activity.mdx)
        // is never drawn again: what changed in its tree waits for no frame. Left set, `dirty`
        // counted every turn as work, so the memory trim due a second after the last work was
        // armed again at each wake and woke the idle application once a second, for good.
        if (window->toplevelClosed()) dirty = false;
        if (dirty && !settling && firstShown && window->ready(oxyos::Window::kToplevel)) {
          Scope drawing(Part::Draw);
          auto started = Clock::now();
          auto damage = windowDamage.update(tree, display, drawn);
          if (painterChanged) damage.reset();
          painterChanged = false;
          if (damage && damage->empty() && window->answered(oxyos::Window::kToplevel)) {
            // Nothing visible changed (focus moved to a popup, say): nothing is drawn or presented.
            // The report still learns the focus, with the texts of the frame shown.
            oxyos::Report::get().frame(oxyos::Window::kToplevel, windowCanvas.width, windowCanvas.height, 0, shownTexts,
                                     focusName(), 0, false);
            publishWindowAccessibility();
            dirty = false;
          } else {
            auto stats = damage ? oxyos::drawDamaged(tree, display, windowCanvas, drawn, *damage)
                                : oxyos::draw(tree, display, windowCanvas, drawn);
            shownTexts = stats.textContent;
            double renderMs = std::chrono::duration<double, std::milli>(Clock::now() - started).count();
            if (checkDamage && damage) {
              oxyos::Canvas whole;
              oxyos::draw(tree, display, whole, drawn);
              auto wholePixels = oxyos::canvasPixels(whole), drawnPixels = oxyos::canvasPixels(windowCanvas);
              size_t differing = 0;
              for (size_t i = 0; i < wholePixels.size() && i < drawnPixels.size(); ++i) {
                differing += wholePixels[i] != drawnPixels[i];
              }
              if (differing) {
                std::cerr << "[host] a frame drawn from its damage differs in " << differing << " pixels\n";
                oxyos::Report::get().damageMismatch(oxyos::Window::kToplevel, differing);
              }
            }
            // Drawn on the GPU: the same frame drawn whole on the CPU must look the same (ADR 0031,
            // decision 13).
            static const bool checkRender = std::getenv("OXYOS_HOST_RENDER_CHECK") != nullptr;
            if (checkRender && windowCanvas.gpu) {
              oxyos::Canvas cpu;
              {
                oxyos::CpuPaintScope onCpu;
                oxyos::draw(tree, display, cpu, drawn);
              }
              auto gpuPixels = oxyos::canvasPixels(windowCanvas);
              auto difference = oxyos::compareFrames(gpuPixels, cpu.pixels, cpu.width);
              if (const char* dump = std::getenv("OXYOS_HOST_RENDER_DUMP"); dump && difference.largest > 32) {
                static int dumped = 0;
                oxyos::Canvas gpuFrame{windowCanvas.width, windowCanvas.height, gpuPixels, nullptr};
                oxyos::writePng(gpuFrame, std::string(dump) + "/gpu-" + std::to_string(dumped) + ".png");
                oxyos::writePng(cpu, std::string(dump) + "/cpu-" + std::to_string(dumped) + ".png");
                ++dumped;
              }
              oxyos::Report::get().renderCheck(oxyos::Window::kToplevel, difference);
            }
            // The damage in the canvas's pixels, rounded outward.
            std::vector<OxyOSRect> pixels;
            for (const auto& rect : damage.value_or(std::vector<facebook::react::Rect>{})) {
              auto x0 = static_cast<int32_t>(std::floor(rect.origin.x * display.scale));
              auto y0 = static_cast<int32_t>(std::floor(rect.origin.y * display.scale));
              auto x1 = static_cast<int32_t>(std::ceil((rect.origin.x + rect.size.width) * display.scale));
              auto y1 = static_cast<int32_t>(std::ceil((rect.origin.y + rect.size.height) * display.scale));
              pixels.push_back(OxyOSRect{x0, y0, static_cast<uint32_t>(x1 - x0), static_cast<uint32_t>(y1 - y0)});
            }
            // Its secure fields go with the frame: no screenshot captures them
            // (docs/adr/0024-system-shortcuts.mdx, decision 10).
            if (auto secure = oxyos::secureFields(tree, tree.root());
                window->setSecureRegions(oxyos::Window::kToplevel, secure)) {
              oxyos::Report::get().input("secure.regions", oxyos::Window::kToplevel,
                                         folly::dynamic::object("rects", oxyos::toDynamic(secure)));
            }
            window->present(oxyos::Window::kToplevel, windowCanvas, pixels);
            // The share of the window drawn again (overlaps counted once per rectangle, at most 1).
            double area = 0;
            for (const auto& rect : pixels) area += static_cast<double>(rect.width) * rect.height;
            double whole = static_cast<double>(windowCanvas.width) * windowCanvas.height;
            double damaged = pixels.empty() ? 1.0 : std::min(1.0, area / std::max(whole, 1.0));
            oxyos::Report::get().frame(oxyos::Window::kToplevel, windowCanvas.width, windowCanvas.height, renderMs,
                                     stats.textContent, focusName(), damaged);
            publishWindowAccessibility();
            dirty = false;
            busy = true;
          }
        }
        {
          Scope drawing(Part::Draw);
          popups.present(drawn, focusName());
          dragDrop.present();
          shellSurfaces.present(drawn, focusName());
        }
        // JavaScript's announcements, once what it did before them is published (the frame above):
        // a focus its work moved is said before them.
        if (!dirty) {
          for (const auto& text : std::exchange(announceAfterFrame, {})) services.announce(text);
        }
        // What the turn did may change what worklets' and Reanimated's next frames act on
        // (Reanimated's mappers, the updates it commits): they get one.
        if (busy) worklets.wantFrame();
        if (busy || uiFrame) reanimated.wantFrame();
        busy |= uiFrame;
        // What the turn dispatched to views after its events (a glide's scroll, an image loaded)
        // reaches JavaScript now; a drop delivered in it has had its onDrop queued, so the window's
        // drag it ended is told after it.
        toJavaScript.beat();
        dragDrop.ranJavaScript();
        // The pings this turn read wait for what it queued, or for what JavaScript runs now.
        for (uint64_t serial : window->takePings()) {
          bool staging = js->staging();
          pingsWaiting.push_back({serial, staging ? js->batch() : js->running(), !staging});
        }
        // What the turn queued for JavaScript runs, together, once JavaScript is idle.
        toJavaScript.handOver();
        if (!offeredKey) {
          std::erase_if(pingsWaiting, [&](const PingWaiting& ping) {
            if (js->done() < ping.batch || (ping.idle && !js->idle())) return false;
            window->pong(ping.serial);
            return true;
          });
        }
        // While the window waits for JavaScript to settle (a resize), an input for its answer or a
        // ping for JavaScript to take what came before it, JavaScript's thread wakes the loop as it
        // goes idle.
        wakeUiWhenIdle = settleBy.has_value() || !firstShown || diagnostics.awaitingJavaScript() || mounter.pending() ||
                         offeredKey.has_value() || !pingsWaiting.empty() || dragDrop.asking();
        oxyos::Report::get().endTurn();
        // With nothing to do, the loop sleeps until something can give it work, as Android's Looper
        // waits on its descriptors: the compositor (events, a frame's feedback), the network, the
        // image decoder, a paste's pipe, JavaScript's thread handing it a commit; or until the next
        // time something is due: a gesture handler's timer, a paste running out of time, the
        // wallpaper's next frame, the end of a resize's wait for JavaScript, or the next frame wanted
        // for worklets' or Reanimated's callbacks, Animated's native driver or requestAnimationFrame
        // (the compositor's answer to the window's frame wakes it for that too). Nothing else wakes
        // it: an application doing nothing never turns. It waits only after a second idle turn.
        idleTurns = busy ? 0 : idleTurns + 1;
        // What an idle window does not need is given back once it has been idle a while (memory.h).
        if (busy || dirty) {
          oxyos::Memory::get().worked(Clock::now());
        } else {
          oxyos::Memory::get().tick(Clock::now());
        }
        // A frame ready to draw whose window's frame before is not answered waits for the compositor.
        diagnostics.endTurn(dirty && !settling && !window->ready(oxyos::Window::kToplevel), idleTurns >= 2);
        if (idleTurns >= 2) {
          std::optional<Clock::time_point> deadline;
          std::optional<Clock::time_point> accessibilityDue;
          if (accessibilityHeld && keyOfferedAt) accessibilityDue = *keyOfferedAt + kAccessibilityHeldAtMost;
          for (auto due : {settleBy, accessibilityDue, gestures->nextDeadline(), window->pasteDeadline(), window->painterDeadline(),
                           dragDrop.nextDeadline(),
                           shellMode ? shellState.nextChange() : std::nullopt, worklets.nextFrame(),
                           reanimated.nextFrame(), animated.nextFrame(),
                           animationFrames.nextFrame(window->ready(oxyos::Window::kToplevel)),
                           pointer.nextFrame(window->ready(oxyos::Window::kToplevel)),
                           images.nextFrame(window->ready(oxyos::Window::kToplevel)), diagnostics.nextDeadline(),
                           editing.nextBlink(), oxyos::Memory::get().nextDeadline(),
                           services.accountSession ? services.accountSession->nextDeadline() : std::nullopt}) {
            if (due && (!deadline || *due < *deadline)) deadline = *due;
          }
          std::vector<pollfd> fds;
          // Another thread with work for the loop (ui-thread.h): JavaScript's commit, a worklet's job.
          fds.push_back({oxyos::UiThread::wakeFd(), POLLIN, 0});
          for (int fd : window->fds()) fds.push_back({fd, POLLIN, 0});
          for (int fd : dragDrop.fds()) fds.push_back({fd, POLLIN, 0});
          // The file service's is -1 until an application asks it something: poll ignores it.
          for (int fd : {networking.fd(), webSockets.fd(), images.fd(), files.fd()}) fds.push_back({fd, POLLIN, 0});
          for (int fd : applicationFds(services)) fds.push_back({fd, POLLIN, 0});
          if (int applications = shellMode ? shellState.applicationsFd() : -1; applications >= 0) {
            fds.push_back({applications, POLLIN, 0});
          }
          if (settingsApplications) {
            fds.push_back({oxyos_applications_watch_fd(settingsApplications.get()), POLLIN, 0});
          }
          // Asleep, the loop holds none of the host's state: JavaScript's calls into it go on.
          oxyos::UiThread::Unlocked asleep;
          if (!deadline) {
            ppoll(fds.data(), fds.size(), nullptr, nullptr);
          } else if (auto wait = std::chrono::duration_cast<std::chrono::nanoseconds>(*deadline - Clock::now()).count();
                     wait > 0) {
            // To the nanosecond: a frame due in 8.33 ms is not taken at 9.
            timespec timeout{static_cast<time_t>(wait / 1'000'000'000), static_cast<long>(wait % 1'000'000'000)};
            ppoll(fds.data(), fds.size(), &timeout, nullptr);
          }
        }
      }
    } catch (const std::exception& error) {
      std::cerr << "[host] " << error.what() << "\n";
      status = 1;
    }
    oxyos::FrameDiagnostics::get().leaving();
    writeJsProfile();
    gestures->onCancelResponder = nullptr;
    gestures->onIntercept = nullptr;
    std::cout << "{\"engine\":\"hermes\",\"fabric\":true,\"window\":true,\"transactions\":"
              << mounter.transactions << ",\"presented\":" << window->presented()
              << ",\"js_errors\":" << jsErrors << "}\n";
    services.keyboard.listener.reset();
    services.blurFocusedInput = nullptr;
    // The window outlives the JavaScript that can still run: stopping the surface unmounts the
    // application and draining runs what that queued (a cleanup, a module first loaded then, as
    // Reanimated's reduced motion asks AccessibilityInfo when it loads). The host's modules reach
    // the window through `services` (Appearance, AccessibilityInfo, OxyOSWindow and the rest), and
    // the accessibility bridge, the popups and the drag hold it: none may find it destroyed.
    surface.stop();
    scheduler->unregisterSurface(surface);
    // JavaScript runs what that queued (its calls into the host meanwhile hold its state), and its
    // thread ends: nothing of JavaScript's runs past the scheduler and the window.
    js->waitIdle(Clock::now() + std::chrono::seconds(2));
    js->quitSynchronous();
    scheduler.reset();
    window.reset();
    oxyos::FrameDiagnostics::get().ended();
    return status == 0 && jsErrors == 0 ? 0 : 1;
  }

  // Run until the app settles (no queued work and no one-shot timer due for a while) or a hard
  // deadline passes.
  auto deadline = Clock::now() + std::chrono::seconds(10);
  auto quietSince = Clock::now();
  // JavaScript's thread wakes the loop each time it is idle.
  wakeUiWhenIdle = true;
  while (Clock::now() < deadline) {
    oxyos::UiThread::woken();
    bool busy = !js->idle();
    if (!busy) busy |= mounter.apply();
    toJavaScript.beat();
    toJavaScript.handOver();
    worklets.tick();
    busy |= reanimated.tick();
    busy |= animated.tick();
    {
      oxyos::JsThread::CausedBy frame({"an animation frame", 0});
      busy |= animationFrames.tick(true);
    }
    if (busy) images.sync(display.scale);
    busy |= images.deliver();
    // A transition moves on while the render settles; an animated picture never keeps it busy.
    images.tick(true);
    busy |= networking.deliver();
    busy |= webSockets.deliver();
    busy |= deliverApplication(services);
    if (busy || timers->oneShotDueBefore(deadline)) {
      quietSince = Clock::now();
    } else if (Clock::now() - quietSince > std::chrono::milliseconds(500)) {
      break;
    }
    if (!busy) {
      oxyos::UiThread::Unlocked asleep;
      pollfd woken{oxyos::UiThread::wakeFd(), POLLIN, 0};
      ppoll(&woken, 1, std::array<timespec, 1>{timespec{0, 2'000'000}}.data(), nullptr);
    }
  }

  writeJsProfile();
  stage("render");
  auto stats = oxyos::render(tree, display, outPath);
  std::cout << "{\"engine\":\"hermes\",\"fabric\":true,\"bridgeless\":true,\"transactions\":"
            << mounter.transactions << ",\"views\":" << stats.views << ",\"texts\":" << stats.texts
            << ",\"unimplemented\":" << stats.unimplemented << ",\"js_errors\":" << jsErrors
            << ",\"output\":\"" << outPath << "\",\"written\":" << (stats.written ? "true" : "false") << "}\n";
  if (!stats.written) std::cerr << "[host] could not write " << outPath << "\n";

  surface.stop();
  scheduler->unregisterSurface(surface);
  js->waitIdle(Clock::now() + std::chrono::seconds(2));
  js->quitSynchronous();
  scheduler.reset();
  return jsErrors == 0 && mounter.transactions > 0 && stats.written ? 0 : 1;
}
