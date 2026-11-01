// OxyOS React Native host: Hermes + bridgeless ReactInstance + Fabric on Linux, single-threaded.
// Loads a Metro bundle built for the `oxyos` platform, starts a surface and records Fabric's
// mounting transactions into a retained view tree that the renderer draws into a buffer.
#include "accessibility.h"
#include "android-components.h"
#include "application.h"
#include "damage.h"
#include "expo-blur.h"
#include "files.h"
#include "frame-diagnostics.h"
#include "keyboard-controller.h"
#include "masked-view.h"
#include "gesture-handler.h"
#include "host-modules.h"
#include "turbo-module-cache.h"
#include "key-chords.h"
#include "images.h"
#include "input.h"
#include "android-view-props.h"
#include "overscroll.h"
#include "intl.h"
#include "modal.h"
#include "network.h"
#include "popup.h"
#include "drag.h"
#include "safe-area.h"
#include "notifications.h"
#include "system.h"
#include "shell-state.h"
#include "shell-surface.h"
#include "svg.h"
#include "report.h"
#include "renderer.h"
#include "text.h"
#include "textinput.h"
#include "window.h"
#include "animation-frames.h"
#include "native-animated.h"
#include "reanimated-host.h"
#include "worklets-host.h"

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
#include <react/renderer/components/modal/ModalHostViewComponentDescriptor.h>
#include <react/renderer/components/root/RootComponentDescriptor.h>
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
#include <react/renderer/scheduler/SurfaceHandler.h>
#include <react/runtime/JSRuntimeFactory.h>
#include <react/runtime/ReactInstance.h>
#include <react/runtime/TimerManager.h>
#include <react/utils/ContextContainer.h>

#include <chrono>
#include <filesystem>
#include <optional>
#include <poll.h>
#include <cmath>
#include <csignal>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <map>
#include <set>
#include <queue>
#include <sstream>
#include <thread>
#include <csignal>
#include <execinfo.h>
#include <sys/prctl.h>
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

// The JS thread is this process's only thread: tasks run when the loop drains the queue.
class Loop : public MessageQueueThread {
 public:
  void runOnQueue(std::function<void()>&& task) override { tasks_.push(std::move(task)); }
  void runOnQueueSync(std::function<void()>&& task) override { task(); }
  void quitSynchronous() override {}

  bool drain() {
    bool ran = false;
    while (!tasks_.empty()) {
      auto task = std::move(tasks_.front());
      tasks_.pop();
      task();
      ran = true;
    }
    return ran;
  }

 private:
  std::queue<std::function<void()>> tasks_;
};

class Timers : public PlatformTimerRegistry {
 public:
  void createTimer(uint32_t id, double delay) override { add(id, delay, false); }
  void createRecurringTimer(uint32_t id, double delay) override { add(id, delay, true); }
  void deleteTimer(uint32_t id) override { timers_.erase(id); }

  // Fires due timers; returns whether any fired.
  bool fire(TimerManager& manager) {
    auto now = Clock::now();
    std::vector<uint32_t> due;
    for (auto& [id, timer] : timers_) {
      if (timer.at <= now) due.push_back(id);
    }
    for (auto id : due) {
      auto it = timers_.find(id);
      if (it == timers_.end()) continue;
      if (it->second.repeat) {
        it->second.at = now + it->second.every;
      } else {
        timers_.erase(it);
      }
      manager.callTimer(static_cast<TimerHandle>(id));
    }
    return !due.empty();
  }

  // When the next timer falls due, if one is set.
  std::optional<Clock::time_point> next() const {
    std::optional<Clock::time_point> next;
    for (const auto& [id, timer] : timers_) {
      if (!next || timer.at < *next) next = timer.at;
    }
    return next;
  }

  // Whether a one-shot timer falls due before `deadline`: the app is not settled until it fires.
  // Recurring timers never settle, so they do not hold the loop.
  bool oneShotDueBefore(Clock::time_point deadline) const {
    for (const auto& [id, timer] : timers_) {
      if (!timer.repeat && timer.at < deadline) return true;
    }
    return false;
  }

 private:
  struct Timer {
    Clock::time_point at;
    Clock::duration every;
    bool repeat;
  };
  void add(uint32_t id, double delay, bool repeat) {
    auto every = std::chrono::duration_cast<Clock::duration>(
        std::chrono::duration<double, std::milli>(delay));
    timers_[id] = Timer{Clock::now() + every, every, repeat};
  }
  std::map<uint32_t, Timer> timers_;
};

// Fabric's event beat: the host loop ticks every live beat, as Android's Choreographer does, so
// queued events (onLayout, ...) reach JavaScript.
class Beat : public EventBeat {
 public:
  Beat(std::shared_ptr<OwnerBox> owner, RuntimeScheduler& scheduler)
      : EventBeat(std::move(owner), scheduler) {
    live().insert(this);
  }
  ~Beat() override { live().erase(this); }

  static void tickAll() {
    for (auto* beat : live()) beat->induce();
  }

 private:
  static std::set<Beat*>& live() {
    static std::set<Beat*> beats;
    return beats;
  }
};

class Mounter : public SchedulerDelegate {
 public:
  explicit Mounter(oxyos::ViewTree& tree) : tree_(tree) {}
  void attach(const Scheduler* scheduler) { scheduler_ = scheduler; }
  int transactions = 0;
  void schedulerDidFinishTransaction(const std::shared_ptr<const MountingCoordinator>& coordinator) override {
    coordinators_[coordinator->getSurfaceId()] = coordinator;
  }
  void schedulerShouldRenderTransactions(
      const std::shared_ptr<const MountingCoordinator>& coordinator) override {
    coordinators_[coordinator->getSurfaceId()] = coordinator;
    pull();
  }
  // The host's frame: pull whatever each surface committed since the last frame.
  bool pull() {
    bool mounted = false;
    for (auto& [surface, coordinator] : coordinators_) {
      // Reanimated's layout-animation proxy overrides every pull, so an empty transaction is
      // returned even when nothing was committed.
      auto transaction = coordinator->pullTransaction();
      if (transaction && !transaction->getMutations().empty()) {
        oxyos::FrameDiagnostics::Scope mounting(oxyos::FrameDiagnostics::Part::Mount);
        // Where its commit's time went (Fabric's telemetry): the commit, its layout (Yoga) and
        // the text measured in that layout.
        const auto& telemetry = transaction->getTelemetry();
        auto since = [](auto from, auto to) {
          return std::max(0.0, std::chrono::duration<double, std::milli>(to - from).count());
        };
        oxyos::FrameDiagnostics::get().committed(
            since(telemetry.getCommitStartTime(), telemetry.getCommitEndTime()),
            since(telemetry.getLayoutStartTime(), telemetry.getLayoutEndTime()),
            std::chrono::duration<double, std::milli>(telemetry.getTextMeasureTime()).count());
        if (beforeApply) beforeApply(transaction->getMutations());
        oxyos::Report::get().committed(transaction->getMutations());
        tree_.apply(transaction->getMutations());
        ++transactions;
        // As each platform's mounting layer does: report the mount, which runs the UIManager's
        // mount hooks. Reanimated resumes its own commits there.
        if (scheduler_) scheduler_->reportMount(surface);
        mounted = true;
        changed_ = true;
      }
    }
    return mounted;
  }
  // Whether the tree changed since the last call, whether the mount came from the host's loop or
  // from the Scheduler's own callback.
  bool takeChanged() {
    bool changed = changed_;
    changed_ = false;
    return changed;
  }
  void schedulerShouldMergeReactRevision(SurfaceId) override {}
  void schedulerDidRequestPreliminaryViewAllocation(const ShadowNode&) override {}
  // A transaction about to be applied, its views still in the tree (a modal's last frame).
  std::function<void(const ShadowViewMutationList&)> beforeApply;
  // View commands from JavaScript (a text input's focus, blur, ...), for the host's native side.
  std::function<void(const ShadowView&, const std::string&, const folly::dynamic&)> onCommand;
  void schedulerDidDispatchCommand(const ShadowView& view, const std::string& name, const folly::dynamic& args) override {
    if (onCommand) onCommand(view, name, args);
  }
  // Accessibility events from JavaScript (AccessibilityInfo.sendAccessibilityEvent): moving
  // accessibility focus, for the host's accessibility state.
  std::function<void(const ShadowView&, const std::string&)> onAccessibilityEvent;
  void schedulerDidSendAccessibilityEvent(const ShadowView& view, const std::string& type) override {
    if (onAccessibilityEvent) onAccessibilityEvent(view, type);
  }
  void schedulerDidSetIsJSResponder(const ShadowView&, bool, bool) override {}
  // Animated's native driver setting a view's props between commits (native-animated.h), as
  // Android's mounting layer updates a native view: the tree changed, as with a mount.
  void schedulerShouldSynchronouslyUpdateViewOnUIThread(Tag tag, const folly::dynamic& props) override {
    if (scheduler_ && oxyos::updateViewProps(tree_, *scheduler_, tag, props)) changed_ = true;
  }
  void schedulerDidUpdateShadowTree(const std::unordered_map<Tag, folly::dynamic>&) override {}
  void schedulerDidCaptureViewSnapshot(Tag, SurfaceId) override {}
  void schedulerDidSetViewSnapshot(Tag, Tag, SurfaceId) override {}
  void schedulerDidClearPendingSnapshots() override {}

 private:
  oxyos::ViewTree& tree_;
  const Scheduler* scheduler_ = nullptr;
  bool changed_ = false;
  std::map<SurfaceId, std::shared_ptr<const MountingCoordinator>> coordinators_;
};

// A fatal signal's report in the session's log: the signal and, for a bad access, the address it
// touched; the thread it hit (by name: the JavaScript thread is the main one, Hermes' collector
// is `hades`); the registers at the fault, which tell a bad pointer from a bad index; and the
// stack, its frames' offsets to symbolise against the build's unstripped objects.
void crashed(int signal, siginfo_t* info, void* context) {
  void* frames[64];
  int count = backtrace(frames, 64);
  char thread[17] = {};
  prctl(PR_GET_NAME, thread, 0, 0, 0);
  // A fault the kernel raised has an address (si_code > 0); a signal sent by a process, abort()'s
  // among them, has its sender in the same storage instead.
  if (info->si_code > 0 && signal != SIGABRT) {
    dprintf(2, "[host] fatal signal %d (code %d) at address %p, on thread %d '%s'\n", signal, info->si_code,
            info->si_addr, static_cast<int>(gettid()), thread);
  } else {
    dprintf(2, "[host] fatal signal %d (code %d) sent by process %d, on thread %d '%s'\n", signal, info->si_code,
            static_cast<int>(info->si_pid), static_cast<int>(gettid()), thread);
  }
  const auto& machine = static_cast<const ucontext_t*>(context)->uc_mcontext;
#if defined(__x86_64__)
  const auto* r = machine.gregs;
  dprintf(2,
          "[host] rip %llx rsp %llx rbp %llx rax %llx rbx %llx rcx %llx rdx %llx rsi %llx rdi %llx r8 %llx r9 %llx "
          "r10 %llx r11 %llx r12 %llx r13 %llx r14 %llx r15 %llx\n",
          r[REG_RIP], r[REG_RSP], r[REG_RBP], r[REG_RAX], r[REG_RBX], r[REG_RCX], r[REG_RDX], r[REG_RSI], r[REG_RDI],
          r[REG_R8], r[REG_R9], r[REG_R10], r[REG_R11], r[REG_R12], r[REG_R13], r[REG_R14], r[REG_R15]);
#elif defined(__aarch64__)
  dprintf(2, "[host] pc %llx sp %llx", machine.pc, machine.sp);
  for (int i = 0; i < 31; ++i) dprintf(2, " x%d %llx", i, machine.regs[i]);
  dprintf(2, "\n");
#endif
  backtrace_symbols_fd(frames, count, 2);
  _exit(128 + signal);
}

// The report above, installed on the JavaScript thread, also for a crash that overflowed its stack:
// the handler runs on a stack of its own.
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
}

// The runtime this host is (docs/adr/0030-installing-applications.mdx, decisions 2 and 17): the
// directory its executable is in, its runtime.json's numbers, and its framework, which every
// application's JavaScript runtime evaluates before the application's own bundle.
struct RuntimeFacts {
  int abi = 0;
  int api = 0;
  std::string framework;
  std::string frameworkURL;
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
  if (!std::ifstream(facts.framework).good()) {
    throw std::runtime_error("runtime " + std::to_string(facts.abi) + " has no framework: " + facts.framework);
  }
  return facts;
}

void stage(const char* name) {
  if (std::getenv("OXYOS_HOST_TRACE")) std::cerr << "[host] " << name << "\n";
  oxyos::Report::get().stage(name);
}

}  // namespace

// What the application's storage and the connectivity answered (application.h), to JavaScript.
bool deliverApplication(oxyos::HostServices& services) {
  bool busy = services.storage && services.storage->deliver();
  if (services.connectivity) busy |= services.connectivity->deliver();
  return busy;
}

// Their descriptors, which wake the loop when they have something.
std::vector<int> applicationFds(const oxyos::HostServices& services) {
  std::vector<int> fds;
  if (services.storage) fds.push_back(services.storage->fd());
  if (services.connectivity && services.connectivity->fd() >= 0) fds.push_back(services.connectivity->fd());
  return fds;
}

int main(int argc, char** argv) {
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
  const bool windowed = outPath == "--window" || shellMode;
  const uint32_t output = shellMode && argc > 4 ? static_cast<uint32_t>(std::stoul(argv[4])) : 1;
  const char* socket = std::getenv("OXYOS_COMPOSITOR_SOCKET");
  if (windowed && !socket) {
    std::cerr << "--window needs OXYOS_COMPOSITOR_SOCKET\n";
    return 2;
  }
  const float width = !shellMode && argc > 5 ? std::stof(argv[4]) : 1024;
  const float height = !shellMode && argc > 5 ? std::stof(argv[5]) : 640;

  for (const char* path : {"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                           "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"}) {
    if (std::ifstream(path).good()) oxyos::TextEngine::shared().addFallback(path);
  }
  ReactNativeFeatureFlags::override(std::make_unique<Flags>());
  auto loop = std::make_shared<Loop>();
  auto timerRegistry = std::make_unique<Timers>();
  auto* timers = timerRegistry.get();
  // The instance holds the only reference to its timer manager, as Android's JReactInstance gives
  // it (its timer executor keeps a weak one): the timers JavaScript leaves pending hold the
  // runtime's values, so they must go with the instance, before its runtime. Kept here past it,
  // they were destroyed after the runtime freed what they point into, and the host jumped through
  // freed memory as it exited. The loop below fires them through `timerManager` while the
  // instance lives.
  auto ownedTimerManager = std::make_shared<TimerManager>(std::move(timerRegistry));
  TimerManager& timerManager = *ownedTimerManager;
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
  auto runtime =
      std::make_unique<JSIRuntimeHolder>(facebook::hermes::makeHermesRuntime(runtimeConfig.build()));
  int jsErrors = 0;
  auto instance = std::make_unique<ReactInstance>(
      std::move(runtime), loop, std::move(ownedTimerManager),
      [&](jsi::Runtime&, const JsErrorHandler::ProcessedError& error) noexcept {
        ++jsErrors;
        std::cerr << "JS error: " << error.message << "\n";
        for (const auto& frame : error.stack) {
          std::cerr << "    at " << frame.methodName << " (" << frame.file.value_or("?") << ":"
                    << frame.lineNumber.value_or(0) << ")\n";
        }
      });
  stage("instance created");
  timerManager.setRuntimeExecutor(instance->getBufferedRuntimeExecutor());
  // The display's refresh, as the compositor tells it for the window (display-refresh.h): every
  // frame below is paced by it.
  oxyos::DisplayRefresh refresh;
  // requestAnimationFrame, once a frame (animation-frames.h); made after the instance, so its
  // callbacks go before the runtime does.
  oxyos::AnimationFrames animationFrames(instance->getBufferedRuntimeExecutor(), refresh);
  loop->drain();

  oxyos::Display display{width, height, 1.0f};
  auto invoker = instance->createJSCallInvoker();
  // Blobs and the network (network.h): answers reach JavaScript as RCTDeviceEventEmitter events,
  // from the loop below, each with its arguments as one array, as Android emits them.
  oxyos::BlobStore blobs;
  oxyos::Networking networking(blobs, [&instance](const std::string& event, folly::dynamic args) {
    instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(event, std::move(args)));
  });
  // The user's files (files.h, docs/adr/0028-file-access.mdx), for an application's window: the
  // application's own folders are its data and cache directories (application.h, ADR 0030), only
  // if the session started it, and its bundle's folder is read-only. Events reach JavaScript as
  // RCTDeviceEventEmitter events from the loop below.
  const auto application = oxyos::Application::launched(bundlePath, moduleName);
  std::error_code bundleError;
  auto bundleFile = std::filesystem::canonical(bundlePath, bundleError);
  oxyos::Files files(
      [&instance](const std::string& event, folly::dynamic payload) {
        instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                       folly::dynamic::array(event, std::move(payload)));
      },
      application, bundleError ? std::string() : bundleFile.parent_path().string());
  oxyos::HostServices services{blobs, networking};
  // The application it runs, as its launcher started it (application.h).
  services.application = application;
  services.emit = [&instance](const std::string& event, folly::dynamic body) {
    instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(event, std::move(body)));
  };
  RuntimeFacts runtimeOf;
  try {
    runtimeOf = runtimeFacts();
  } catch (const std::exception& error) {
    std::cerr << "[host] " << error.what() << "\n";
    return 1;
  }
  services.runtime = {runtimeOf.abi, runtimeOf.api, runtimeOf.frameworkURL};
  oxyos::WorkletsHost worklets(refresh);
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
                  if (name == "WorkletsModule") return worklets.module(runtime, invoker);
                  if (name == "ReanimatedModule") return reanimated.module(runtime, invoker);
                  if (name == "NativeAnimatedModule") return animated.module(invoker);
                  if (name == "RNGestureHandlerModule" && gestures) return gestures->module(runtime, invoker);
                  if (auto module = oxyos::createHostModule(name, invoker, display, services)) return module;
                  if (auto module = DefaultTurboModules::getTurboModule(name, invoker)) return module;
                  std::cerr << "[host] TurboModule unavailable: " << name << "\n";
                  return nullptr;
                }));
  });

  auto context = std::make_shared<ContextContainer>();
  context->insert(ImageManagerKey, std::make_shared<ImageManager>(context));
  context->insert(RuntimeSchedulerKey, std::weak_ptr<RuntimeScheduler>(instance->getRuntimeScheduler()));
  SchedulerToolbox toolbox{};
  toolbox.contextContainer = context;
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
  providers->add(concreteComponentDescriptorProvider<ModalHostViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoBlurViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoBlurTargetViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RNCMaskedViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<KeyboardControllerViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSShellSurfaceComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSWindowViewComponentDescriptor>());
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
                    [&instance](const std::string& name, folly::dynamic payload) {
                      instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                                     folly::dynamic::array(name, std::move(payload)));
                    },
                .reanimatedEvent = [&reanimated](const std::string& name, int view,
                                                 const folly::dynamic& payload) {
                  reanimated.event(name, view, payload);
                },
            });
  // What Image views show (images.h), loaded at the size they are drawn.
  oxyos::Images images(tree);
  stage("fabric scheduler");
  scheduler = std::make_unique<Scheduler>(toolbox, nullptr, &mounter);
  mounter.attach(scheduler.get());

  // In a window, the compositor's configure decides the size the app lays out at.
  std::unique_ptr<oxyos::Window> window;
  // Settings' requests to the session, numbered alike: preferences and displays (ADR 0018, 0019).
  uint32_t settingsSerial = 0;
  // As the session's shell: what it knows of windows and workspaces, told from the first event.
  oxyos::ShellState shellState(output);
  if (shellMode) shellState.followApplications();
  if (windowed) {
    try {
      window = shellMode ? std::make_unique<oxyos::Window>(socket, oxyos::Window::Shell{.output = output})
                         : std::make_unique<oxyos::Window>(socket, static_cast<uint32_t>(width),
                                                           static_cast<uint32_t>(height));
      while (!window->configured()) {
        if (!window->dispatch([&shellState](const OxyOSEvent& event) { shellState.observe(event); })) {
          std::cerr << "[host] the compositor closed the connection before configuring the window\n";
          return 1;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
      }
      // The painter is started now, beside the runtime and the application loading.
      window->choosePainter();
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
      if (!shellMode) services.files = &files;
      services.shellWindow = shellMode;
      // Drag and drop between applications (docs/adr/0035-drag-and-drop.mdx): protocol version 10.
      services.drag.available = window->protocolVersion() >= 10;
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
      return 1;
    }
  }

  stage("load script");
  // The framework and the application's bundle are mapped from their files, never copied: the
  // framework's pages are the same for every application running on this runtime. The framework
  // is evaluated first, in the same turn (React Native's `beforeLoad`), so the application's
  // modules find React Native, React and Bloom defined, and React Native initialized.
  std::shared_ptr<const jsi::Buffer> framework = JSBigFileString::fromPath(runtimeOf.framework);
  instance->loadScript(
      JSBigFileString::fromPath(bundlePath), bundlePath,
      [framework, url = runtimeOf.frameworkURL](jsi::Runtime& runtime) {
        stage("framework");
        runtime.evaluateJavaScript(framework, url);
        stage("bundle");
      },
      nullptr);

  stage("surface");
  SurfaceHandler surface(moduleName, 1);
  scheduler->registerSurface(surface);
  auto layOut = [&] {
    Size size{display.width, display.height};
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
    // Drags over this client's surfaces reach the drop targets under them, and its own drags start
    // from the press it holds (drag.h).
    oxyos::DragDrop dragDrop(tree, *window, [&](uint32_t surface) -> std::optional<oxyos::SurfaceContent> {
      if (surface == oxyos::Window::kToplevel) return oxyos::SurfaceContent{.surface = surface, .view = tree.root()};
      if (auto content = shellSurfaces.content(surface)) return content;
      return popups.content(surface);
    });
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
    });
    // The pointer's shape over each surface, as the views under it want it.
    pointer.showsShapes([&window](uint32_t surface, uint8_t shape) { window->setPointerShape(surface, shape); });
    pointer.pacedBy(refresh);
    pointer.scrollsText([&editing](facebook::react::Tag view, float dx, float dy) {
      return editing.scrollText(view, dx, dy);
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
    editing.useClipboard({
        .copy = [&window](uint64_t serial, const std::string& text) { window->copyText(serial, text); },
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
        r.input("scroll", event.surface, folly::dynamic::object("dy", event.y / 256.0));
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
    // expo-clipboard's reads and writes carry the serial of the latest key or press the app
    // received (ADR 0007): the compositor refuses them unless the app has focus and that input is
    // recent. JavaScript hears when the clipboard changes.
    services.drag.start = [&dragDrop](const folly::dynamic& data, const std::vector<std::string>& actions, int preview,
                                      std::function<void(std::optional<std::string>)> done) {
      dragDrop.start(data, actions, preview, std::move(done));
    };
    services.clipboard = {
        .copy = [&window, &lastPress](const std::string& text) { return window->copyText(lastPress, text); },
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
    // An announcement is what the application says now: it interrupts what is being said, as
    // an announcement event does TalkBack's queue.
    services.announce = [&window](const std::string& text) {
      window->announce(oxyos::Window::kToplevel, OXYOS_ASSERTIVE, text);
      oxyos::Report::get().input("announce", oxyos::Window::kToplevel, folly::dynamic::object("text", text));
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
    std::map<std::string, std::string> toldPreferences;
    if (window) toldPreferences = window->preferences();
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
          .authenticate = [&window, &shellSurfaces](const std::string& secret, const std::string& account) {
            auto surface = shellSurfaces.lockSurface();
            return surface && window->authenticate(*surface, secret, account);
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
          .announce = [&window](const std::string& text, bool polite) {
            window->announce(oxyos::Window::kToplevel, polite ? OXYOS_POLITE : OXYOS_ASSERTIVE, text);
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
          // The dock's application menu (docs/adr/0034-the-dock.mdx): a list's change is a
          // preference change, numbered with the Control Center's; a URL cites the press that chose
          // it, numbered with the launches.
          .forceQuitApplication = [&window](uint32_t target) {
            bool sent = window->forceQuitApplication(target);
            oxyos::Report::get().input("application.force_quit", 0,
                                       folly::dynamic::object("window", target)("sent", sent));
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
        OxyOSNotification notification{
            .identifier = keep(text(content, "identifier")),
            .title = keep(text(content, "title")),
            .subtitle = keep(text(content, "subtitle")),
            .body = keep(text(content, "body")),
            .data = keep(text(content, "data")),
            .actions = actions.empty() ? nullptr : actions.data(),
            .action_count = actions.size(),
            .banner = banner && banner->isBool() && banner->getBool(),
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
    auto launchCapabilities = oxyos::LaunchCapabilities::fromEnvironment();
    oxyos::ChunkTexts systemTexts, sheetTexts;
    // A saved screenshot's path, assembled from its chunks (docs/adr/0024-system-shortcuts.mdx).
    oxyos::ChunkTexts screenshotPaths;
    uint32_t systemSerial = 0;
    std::map<uint32_t, std::string> askedCapabilities;
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
    window->onClipboardChanged([&instance, &window] {
      instance->callFunctionOnModule(
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
    };
    // Accessibility focus, as Android's accessibility focus (separate from input focus): the
    // view a screen reader would be on. No assistive technology reads it yet; the tree marks it.
    mounter.onAccessibilityEvent = [&](const ShadowView& view, const std::string& type) {
      std::string within;
      auto path = oxyos::ancestry(tree, view.tag);
      for (auto at = path.rbegin(); at != path.rend() && within.empty(); ++at) {
        auto props = std::dynamic_pointer_cast<const ViewProps>(tree.views().at(*at).view.props);
        if (props && !props->testId.empty()) within = props->testId;
      }
      oxyos::Report::get().accessibilityEvent(type, view, within);
      // A region whose keyboard went back to a window (the bar left from the keyboard) takes no
      // accessibility focus from JavaScript while the keyboard is elsewhere: Bloom gives a closed
      // menu's trigger the focus back (a row chosen closes it after the bar gave the window the
      // keyboard), and the person is in the window, not on the bar. The assistive technology's own
      // focus moves (an action) are not these, and go anywhere.
      if (type == "focus" && releasedRegion != 0 && keyboardSurface != releasedRegion &&
          shellSurfaces.keyboardRegion() != releasedRegion &&
          !(keyboardSurface != 0 && popups.content(keyboardSurface)) &&
          shellSurfaces.surfaceOf(view.tag) == releasedRegion) {
        oxyos::Report::get().input("a11y.focus.declined", releasedRegion);
        return;
      }
      if (type == "focus") {
        accessibility.focus(view.tag);
        dirty = true;
        popups.changed();
        shellSurfaces.changed();
      }
    };
    int idleTurns = 0;  // turns in a row that found nothing to do
    uint32_t toldRefresh = 0;  // the window's display's refresh, as last told (display-refresh.h)
    // A window that changes size answers its configure with a frame JavaScript laid out at that
    // size, as AppKit lays a window out during a live resize before the frame is shown: the new
    // size's layout reaches JavaScript (onLayout, the dimensions), whose commits can size the
    // tree again (an app shell measuring its room, then a panel group measuring its own). Until
    // JavaScript is quiet, or a frame of the display has passed (display-refresh.h: 16.7 ms at
    // 60 Hz, 8.3 at 120) so that a slow app never stalls the resize past its next refresh, the
    // window presents nothing and popups are not placed again.
    std::optional<Clock::time_point> settleBy;
    Clock::time_point settleFrom;
    int quietTurns = 0;  // turns in a row in which JavaScript ran nothing and nothing mounted
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
        auto started = Clock::now();
        bool ran = run();
        oxyos::Report::get().javascript(std::chrono::duration<double, std::milli>(Clock::now() - started).count());
        return ran;
      };
      while (true) {
        oxyos::Report::get().beginTurn();
        diagnostics.beginTurn();
        {
          Scope beat(Part::JavaScript);
          Beat::tickAll();
        }
        bool ranJs = javascript(Part::JavaScript, [&] { return loop->drain(); });
        bool pulled = mounter.pull();
        bool busy = ranJs || pulled;
        if (busy) diagnostics.trigger("JavaScript", 0);
        // A timer's callback, a requestAnimationFrame's, runs on the next drain.
        if (timers->fire(timerManager)) {
          diagnostics.trigger("a timer");
          busy = true;
        }
        bool uiFrame = false;
        {
          Scope animation(Part::Animation);
          bool animating = gestures->tick();
          // Worklets' frame is not work by itself (worklets-host.h): what it changes reaches the host
          // as a mount, a UI job or a frame asked for.
          uiFrame = javascript(Part::Animation, [&] { return worklets.tick(); });
          animating |= javascript(Part::Animation, [&] { return reanimated.tick(); });
          animating |= animated.tick();
          animating |= animationFrames.tick(window->ready(oxyos::Window::kToplevel));
          animating |= pointer.tick(window->ready(oxyos::Window::kToplevel));
          if (animating) diagnostics.trigger("an animation frame");
          busy |= animating;
        }
        bool arrived = networking.deliver();
        arrived |= files.deliver();
        arrived |= dragDrop.deliver();
        arrived |= deliverApplication(services);
        if (arrived) diagnostics.trigger("data arriving");
        busy |= arrived;
        // The wallpaper follows the sun (docs/adr/0012-solar-wallpaper.mdx), and the applications
        // the session's registry (docs/adr/0030-installing-applications.mdx): each looked at.
        bool wallpaperChanged = shellMode && shellState.tick();
        bool applicationsChanged = shellMode && shellState.applicationsChanged();
        bool shellChanged = wallpaperChanged || applicationsChanged, closeRequested = false;
        std::optional<OxyOSEvent> authenticated;
        std::vector<OxyOSEvent> launchResults;
        std::vector<OxyOSEvent> preferenceResults;
        std::vector<OxyOSEvent> menuChoices;
        std::vector<OxyOSEvent> systemShortcuts;
        // The screenshot interface's changes and the window under the pointer, in order, and the
        // screenshots saved with their paths (docs/adr/0024-system-shortcuts.mdx, decision 10).
        std::vector<OxyOSEvent> captures;
        std::vector<std::pair<OxyOSEvent, std::string>> screenshots;
        std::vector<OxyOSEvent> bezels;
        std::vector<OxyOSEvent> notificationResults, notificationsRemoved;
        std::vector<std::pair<uint32_t, std::string>> systemAnswers, sheets, filesChosen;
        std::vector<OxyOSEvent> sheetsGone, capabilityResults;
        bool responded = false;
        Part handling = diagnostics.enter(Part::Events);
        bool connected = window->dispatch([&](const OxyOSEvent& event) {
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
              if (event.kind == OXYOS_EVENT_MENU_ITEM_ACTIVATED) menuChoices.push_back(event);
              if (event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT) systemShortcuts.push_back(event);
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
                if (event.flag) {
                  keyboardSurface = event.surface;
                } else if (keyboardSurface == event.surface && !popups.closedByApplication(event.surface)) {
                  // A popup this client closed itself (a menu replaced by the next one, the top
                  // bar's Right) keeps the keyboard as far as this client knows until the
                  // compositor gives it to the surface next in line: never, between the two, to
                  // nothing, which would read as the keyboard gone elsewhere.
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
              // A key press is offered to JavaScript first (Bloom's menu shortcuts, ADR 0021); one
              // it takes, and that key's release, go no further.
              bool keyTaken = false;
              if (event.kind == OXYOS_EVENT_KEY && services.keyboard.listener) {
                if (keyChords.swallow(event)) {
                  keyTaken = true;
                } else if (auto offered = keyChords.press(event)) {
                  jsi::Runtime& rt = *services.keyboard.runtime;
                  try {
                    auto answer = services.keyboard.listener->call(rt, jsi::valueFromDynamic(rt, *offered));
                    rt.drainMicrotasks();
                    keyTaken = answer.isBool() && answer.getBool();
                  } catch (const jsi::JSError& error) {
                    std::cerr << "[host] the key listener: " << error.getMessage() << "\n";
                  }
                  if (keyTaken) {
                    keyChords.taken(event.code);
                    oxyos::Report::get().deliver("shortcut", event.surface, shellMode ? "shell" : "menu-bar",
                                                 *offered);
                  }
                }
              }
              if ((event.kind == OXYOS_EVENT_POINTER && event.detail == OXYOS_POINTER_PRESS) ||
                  (event.kind == OXYOS_EVENT_KEY && event.flag) ||
                  (event.kind == OXYOS_EVENT_ACCESSIBILITY_ACTION && event.detail == OXYOS_ACTION_ACTIVATE) ||
                  // A choice in the shell's bar is the person's input: Edit > Paste may cite it.
                  event.kind == OXYOS_EVENT_MENU_ITEM_ACTIVATED ||
                  // So is a system shortcut the shell shows: the search's field may cite it.
                  event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT ||
                  // And the screenshot interface opened or changed by the keyboard.
                  event.kind == OXYOS_EVENT_CAPTURE_MODE ||
                  // And a notification the person pressed (ADR 0026): its response may copy.
                  event.kind == OXYOS_EVENT_NOTIFICATION_RESPONSE ||
                  // And a drop (ADR 0035): the dock's Trash cites it to put the files in the Trash.
                  event.kind == OXYOS_EVENT_DRAG_DROP) {
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
              // Escape is the desktop's back: the topmost modal is asked to close, as Android's
              // back asks a modal's window.
              constexpr uint32_t kEscape = 1;
              if (event.kind == OXYOS_EVENT_KEY && event.flag && event.code == kEscape &&
                  event.surface == oxyos::Window::kToplevel) {
                modalHost.requestClose();
              }
              if (event.kind == OXYOS_EVENT_TEXT_INPUT) {
                OxyOSTextTransaction change{};
                if (window->transaction(change)) {
                  auto& r = oxyos::Report::get();
                  if (r.enabled()) {
                    r.input("text_input", event.surface,
                            folly::dynamic::object("commit", editing.logged({change.commit, change.commit_length}))(
                                "preedit", editing.logged({change.preedit, change.preedit_length}))(
                                "delete", folly::dynamic::array(change.delete_before, change.delete_after)));
                  }
                  auto told = toldSerials.find(change.surface);
                  editing.transaction(change, told != toldSerials.end() && told->second == change.commit_serial);
                }
              }
              dragDrop.observe(event);
              pointer.handle(event);
              editing.handle(event);
            });
        diagnostics.leave(handling);
        if (!connected) break;
        if (!shellMode) diagnostics.windowState(window->maximized(), window->active());
        // The window's display's refresh, told before its first configure and when it changes.
        if (window->refreshMhz() != toldRefresh) {
          toldRefresh = window->refreshMhz();
          refresh.set(toldRefresh);
          diagnostics.setRefresh(refresh.periodMs());
          oxyos::Report::get().refresh(toldRefresh);
        }
        if (closeRequested || closeAsked) {
          if (closeAsked) oxyos::Report::get().input("close.asked", oxyos::Window::kToplevel);
          break;
        }
        // The session's preferences (docs/adr/0019-settings.mdx), as React Native's own modules tell
        // a change on Android: the appearance (a configuration change), reduce motion; the rest to
        // Bloom's adapter; and, to Settings, every preference as kept and what came of a change.
        if (window && toldDrawing && !(*toldDrawing == window->drawing())) {
          const auto& now = window->drawing();
          auto emit = [&](const char* name, folly::dynamic payload) {
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(name, payload));
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
          emit("OxyOSPreferencesChanged", services.settings.drawing());
          oxyos::Report::get().input("preferences", oxyos::Window::kToplevel, services.settings.drawing());
          toldDrawing = now;
        }
        if (window && window->preferences() != toldPreferences) {
          toldPreferences = window->preferences();
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSettingsChanged", services.settings.values()));
        }
        // The displays as the session tells Settings them (docs/adr/0018-several-displays.mdx).
        if (window && services.settings.displays) {
          auto displays = services.settings.displays();
          if (displays != toldDisplays) {
            toldDisplays = displays;
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSDisplaysChanged", displays));
            if (!displays.isNull()) oxyos::Report::get().input("displays", oxyos::Window::kToplevel, displays);
          }
        }
        // Installing applications (ADR 0030): what the session answered, the shell's sheets, and
        // capability results; a capability granted holds for this process from now on.
        for (const auto& [serial, text] : systemAnswers) {
          auto answer = folly::parseJson(text);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSystemAnswer",
                                                               folly::dynamic::object("serial", serial)("answer", answer)));
          oxyos::Report::get().input("system", oxyos::Window::kToplevel,
                                     folly::dynamic::object("serial", serial)("answer", answer));
        }
        for (const auto& [sheet, text] : sheets) {
          auto described = folly::parseJson(text);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSheet",
                                                               folly::dynamic::object("sheet", sheet)("text", text)));
          oxyos::Report::get().input("sheet", oxyos::Window::kToplevel,
                                     folly::dynamic::object("sheet", sheet)("kind", described["kind"]));
        }
        // File choosers (ADR 0029, decision 8): each request's answer, once.
        for (const auto& [serial, text] : filesChosen) {
          auto answer = folly::parseJson(text);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSFilesChosen",
                                                               folly::dynamic::object("serial", serial)("answer", answer)));
          oxyos::Report::get().input("files_chosen", oxyos::Window::kToplevel,
                                     folly::dynamic::object("serial", serial)("answer", answer));
        }
        for (const auto& gone : sheetsGone) {
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSSheetDismissed",
                                                               folly::dynamic::object("sheet", gone.code)));
        }
        for (const auto& result : capabilityResults) {
          auto asked = askedCapabilities.find(result.code);
          std::string capability = asked == askedCapabilities.end() ? "" : asked->second;
          if (asked != askedCapabilities.end()) askedCapabilities.erase(asked);
          if (result.flag != 0 && !capability.empty()) launchCapabilities.grant(capability);
          folly::dynamic told = folly::dynamic::object("serial", result.code)("granted", result.flag != 0)("capability", capability);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSCapabilityResult", told));
          oxyos::Report::get().input("capability", oxyos::Window::kToplevel, told);
        }
        for (const auto& result : preferenceResults) {
          instance->callFunctionOnModule(
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
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSWindowChanged", state));
          }
        }
        if (shellChanged) {
          auto state = shellState.state();
          if (state != toldShellState) {
            toldShellState = state;
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellChanged", std::move(state)));
          }
        }
        // An application's notifications (ADR 0026): what came of its posts, those no longer kept,
        // and that responses wait (JavaScript takes them, each once).
        for (const auto& result : notificationResults) {
          instance->callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSNotificationResult", oxyos::notificationResult(result)));
        }
        for (const auto& removed : notificationsRemoved) {
          instance->callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSNotificationRemoved", folly::dynamic::object("notification", removed.code)));
        }
        if (responded) {
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSNotificationResponse", folly::dynamic::object()));
        }
        // Whether the applications the shell asked for were started (ADR 0013).
        for (const auto& result : launchResults) {
          oxyos::Report::get().input("launch.result", 0,
                                     folly::dynamic::object("serial", result.code)("launched", result.flag != 0));
          instance->callFunctionOnModule(
              "RCTDeviceEventEmitter", "emit",
              folly::dynamic::array("OxyOSShellLaunch",
                                    folly::dynamic::object("serial", result.code)("launched", result.flag != 0)));
        }
        // A command of the window's menus chosen in the shell's bar (ADR 0021): to the adapter,
        // which runs the one Bloom declared at that index of that commit.
        for (const auto& choice : menuChoices) {
          folly::dynamic payload = folly::dynamic::object("serial", choice.code)("index", choice.detail);
          oxyos::Report::get().input("menu.activated", choice.surface, payload);
          oxyos::Report::get().deliver("menuActivated", choice.surface, "menu-bar", payload);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
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
                                          "menu-bar"};
          const char* action = shortcut.detail <= OXYOS_SHORTCUT_MENU_BAR ? actions[shortcut.detail] : "";
          folly::dynamic payload = folly::dynamic::object("action", action)("output", shortcut.code);
          oxyos::Report::get().input("shortcut.system", 0, payload);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSShellShortcut", payload));
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
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellCapture", payload));
          } else {
            folly::dynamic payload = folly::dynamic::object("window", capture.code)("x", capture.x)("y", capture.y)(
                "width", capture.width)("height", capture.height);
            oxyos::Report::get().input("capture.target", 0, payload);
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellCaptureTarget", payload));
          }
        }
        for (const auto& [saved, path] : screenshots) {
          folly::dynamic payload = folly::dynamic::object("id", saved.code)("output", saved.x)("width", saved.width)(
              "height", saved.height)("path", path);
          oxyos::Report::get().input("screenshot.saved", 0, payload);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
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
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
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
              if (auto* shown = window->surface(wanted); shown && shown->shown) {
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
            instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                           folly::dynamic::array("OxyOSShellKeyboardLost", folly::dynamic::object()));
          }
        }
        // What came of the lock screen's password (docs/adr/0010-lock-screen.mdx).
        if (authenticated) {
          static const char* outcomes[] = {"", "accepted", "rejected", "busy", "throttled"};
          const char* outcome = authenticated->detail <= OXYOS_AUTH_THROTTLED ? outcomes[authenticated->detail] : "";
          oxyos::Report::get().input("lock.authentication", authenticated->surface,
                                     folly::dynamic::object("outcome", outcome)("wait", authenticated->code));
          instance->callFunctionOnModule(
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
        bool edited = editing.edit() != drawn;
        if (edited) {
          drawn = editing.edit();
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
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("didUpdateDimensions", oxyos::deviceDimensions(display)));
          auto laying = Clock::now();
          {
            Scope fabric(Part::JavaScript);
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
          dirty = true;
          busy = true;
          popups.changed();
          shellSurfaces.changed();
          safeAreas.sync(tree);
        }
        // Two quiet turns: what one turn's layout queues (onLayout) reaches JavaScript on the next.
        if (resized) {
          settleFrom = Clock::now();
          settleBy = settleFrom + refresh.period();
          quietTurns = settleTurns = 0;
        } else if (settleBy) {
          ++settleTurns;
          quietTurns = ranJs || pulled || mounted ? 0 : quietTurns + 1;
          bool timedOut = Clock::now() >= *settleBy;
          if (quietTurns >= 2 || timedOut) {
            settleBy.reset();
            double settledMs = std::chrono::duration<double, std::milli>(Clock::now() - settleFrom).count();
            diagnostics.settled(settledMs, timedOut && quietTurns < 2);
            oxyos::Report::get().settled(settledMs, settleTurns, timedOut && quietTurns < 2);
          }
        }
        bool settling = settleBy.has_value();
        if (settling) busy = true;
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
              toldSerials[field->surface] = window->textInput(field->surface, &told, newField);
            }
            toldField = std::move(field);
            toldCaret = caret;
          }
        }
        // Every configure is answered with a frame, even one that changes nothing it shows.
        if (!window->answered(oxyos::Window::kToplevel)) dirty = true;
        if (dirty && !settling && window->ready(oxyos::Window::kToplevel)) {
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
            accessibility.publish(oxyos::Window::kToplevel, tree.root());
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
            accessibility.publish(oxyos::Window::kToplevel, tree.root());
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
        // What the turn did may change what worklets' and Reanimated's next frames act on
        // (Reanimated's mappers, the updates it commits): they get one.
        if (busy) worklets.wantFrame();
        if (busy || uiFrame) reanimated.wantFrame();
        busy |= uiFrame;
        oxyos::Report::get().endTurn();
        // With nothing to do, the loop sleeps until something can give it work, as Android's Looper
        // waits on its descriptors: the compositor (events, a frame's feedback), the network, the
        // image decoder, a paste's pipe; or until the next time something is due: a timer of
        // JavaScript's or a gesture handler's, a paste running out of time, the wallpaper's next
        // frame, or the next frame wanted for worklets' or Reanimated's callbacks, Animated's
        // native driver or requestAnimationFrame (the compositor's answer to the window's frame
        // wakes it for that too). Nothing else wakes it: an application doing nothing never turns.
        // It waits only after a second idle turn, since what one turn dispatches (an event for
        // Fabric's beat) is delivered on the next.
        idleTurns = busy ? 0 : idleTurns + 1;
        // A frame ready to draw whose window's frame before is not answered waits for the compositor.
        diagnostics.endTurn(dirty && !settling && !window->ready(oxyos::Window::kToplevel), idleTurns >= 2);
        if (idleTurns >= 2) {
          std::optional<Clock::time_point> deadline;
          for (auto due : {timers->next(), gestures->nextDeadline(), window->pasteDeadline(), window->painterDeadline(),
                           dragDrop.nextDeadline(),
                           shellMode ? shellState.nextChange() : std::nullopt, worklets.nextFrame(),
                           reanimated.nextFrame(), animated.nextFrame(),
                           animationFrames.nextFrame(window->ready(oxyos::Window::kToplevel)),
                           pointer.nextFrame(window->ready(oxyos::Window::kToplevel)), diagnostics.nextDeadline(),
                           editing.nextBlink()}) {
            if (due && (!deadline || *due < *deadline)) deadline = *due;
          }
          std::vector<pollfd> fds;
          for (int fd : window->fds()) fds.push_back({fd, POLLIN, 0});
          for (int fd : dragDrop.fds()) fds.push_back({fd, POLLIN, 0});
          // The file service's is -1 until an application asks it something: poll ignores it.
          for (int fd : {networking.fd(), images.fd(), files.fd()}) fds.push_back({fd, POLLIN, 0});
          for (int fd : {networking.fd(), images.fd()}) fds.push_back({fd, POLLIN, 0});
          for (int fd : applicationFds(services)) fds.push_back({fd, POLLIN, 0});
          if (int applications = shellMode ? shellState.applicationsFd() : -1; applications >= 0) {
            fds.push_back({applications, POLLIN, 0});
          }
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
    window.reset();
    surface.stop();
    scheduler->unregisterSurface(surface);
    loop->drain();
    scheduler.reset();
    oxyos::FrameDiagnostics::get().ended();
    return status == 0 && jsErrors == 0 ? 0 : 1;
  }

  // Run until the app settles (no queued work and no one-shot timer due for a while) or a hard
  // deadline passes.
  auto deadline = Clock::now() + std::chrono::seconds(10);
  auto quietSince = Clock::now();
  while (Clock::now() < deadline) {
    Beat::tickAll();
    bool busy = loop->drain();
    busy |= mounter.pull();
    busy |= timers->fire(timerManager);
    worklets.tick();
    busy |= reanimated.tick();
    busy |= animated.tick();
    busy |= animationFrames.tick(true);
    if (busy) images.sync(display.scale);
    busy |= images.deliver();
    busy |= networking.deliver();
    busy |= deliverApplication(services);
    if (busy || timers->oneShotDueBefore(deadline)) {
      quietSince = Clock::now();
    } else if (Clock::now() - quietSince > std::chrono::milliseconds(500)) {
      break;
    }
    if (!busy) std::this_thread::sleep_for(std::chrono::milliseconds(2));
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
  loop->drain();
  scheduler.reset();
  return jsErrors == 0 && mounter.transactions > 0 && stats.written ? 0 : 1;
}
