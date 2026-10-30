// OxyOS React Native host: Hermes + bridgeless ReactInstance + Fabric on Linux, single-threaded.
// Loads a Metro bundle built for the `oxyos` platform, starts a surface and records Fabric's
// mounting transactions into a retained view tree that the renderer draws into a buffer.
#include "accessibility.h"
#include "android-components.h"
#include "damage.h"
#include "expo-blur.h"
#include "masked-view.h"
#include "gesture-handler.h"
#include "host-modules.h"
#include "key-chords.h"
#include "images.h"
#include "input.h"
#include "intl.h"
#include "modal.h"
#include "network.h"
#include "popup.h"
#include "safe-area.h"
#include "notifications.h"
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

std::string readFile(const std::string& path) {
  std::ifstream file(path, std::ios::binary);
  std::stringstream content;
  content << file.rdbuf();
  return content.str();
}

void crashed(int signal) {
  void* frames[64];
  int count = backtrace(frames, 64);
  dprintf(2, "[host] fatal signal %d\n", signal);
  backtrace_symbols_fd(frames, count, 2);
  _exit(128 + signal);
}

void stage(const char* name) {
  if (std::getenv("OXYOS_HOST_TRACE")) std::cerr << "[host] " << name << "\n";
  oxyos::Report::get().stage(name);
}

}  // namespace

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
  std::signal(SIGSEGV, crashed);
  std::signal(SIGABRT, crashed);
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
  auto timerManager = std::make_shared<TimerManager>(std::move(timerRegistry));
  auto runtime = std::make_unique<JSIRuntimeHolder>(facebook::hermes::makeHermesRuntime(
      ::hermes::vm::RuntimeConfig::Builder().withMicrotaskQueue(true).build()));
  int jsErrors = 0;
  auto instance = std::make_unique<ReactInstance>(
      std::move(runtime), loop, timerManager,
      [&](jsi::Runtime&, const JsErrorHandler::ProcessedError& error) noexcept {
        ++jsErrors;
        std::cerr << "JS error: " << error.message << "\n";
        for (const auto& frame : error.stack) {
          std::cerr << "    at " << frame.methodName << " (" << frame.file.value_or("?") << ":"
                    << frame.lineNumber.value_or(0) << ")\n";
        }
      });
  stage("instance created");
  timerManager->setRuntimeExecutor(instance->getBufferedRuntimeExecutor());
  // requestAnimationFrame, once a frame (animation-frames.h); made after the instance, so its
  // callbacks go before the runtime does.
  oxyos::AnimationFrames animationFrames(instance->getBufferedRuntimeExecutor());
  loop->drain();

  oxyos::Display display{width, height, 1.0f};
  auto invoker = instance->createJSCallInvoker();
  // Blobs and the network (network.h): answers reach JavaScript as RCTDeviceEventEmitter events,
  // from the loop below, each with its arguments as one array, as Android emits them.
  oxyos::BlobStore blobs;
  oxyos::Networking networking(blobs, [&instance](const std::string& event, folly::dynamic args) {
    instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit", folly::dynamic::array(event, std::move(args)));
  });
  oxyos::HostServices services{blobs, networking};
  oxyos::WorkletsHost worklets;
  std::unique_ptr<Scheduler> scheduler;
  oxyos::ReanimatedHost reanimated(worklets, [&scheduler] { return scheduler.get(); });
  // Animated's native driver (native-animated.h).
  oxyos::NativeAnimatedHost animated;
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
    // In place of React Native's, which runs callbacks as a zero-delay timer.
    animationFrames.install(rt);
    TurboModuleBinding::install(
        rt, [&, invoker](jsi::Runtime& runtime, const std::string& name) -> std::shared_ptr<TurboModule> {
          if (name == "WorkletsModule") return worklets.module(runtime, invoker);
          if (name == "ReanimatedModule") return reanimated.module(runtime, invoker);
          if (name == "NativeAnimatedModule") return animated.module(invoker);
          if (name == "RNGestureHandlerModule" && gestures) return gestures->module(runtime, invoker);
          if (auto module = oxyos::createHostModule(name, invoker, display, services)) return module;
          if (auto module = DefaultTurboModules::getTurboModule(name, invoker)) return module;
          std::cerr << "[host] TurboModule unavailable: " << name << "\n";
          return nullptr;
        });
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
  providers->add(concreteComponentDescriptorProvider<ViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ParagraphComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RawTextComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<TextComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ScrollViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ImageComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<oxyos::TextInputComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyOSPopupViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ModalHostViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoBlurViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<ExpoBlurTargetViewComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<RNCMaskedViewComponentDescriptor>());
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
      services.shellWindow = shellMode;
      // The session's preferences (docs/adr/0019-settings.mdx), from the connection's first events.
      services.settings = {
          .drawing = [&window] {
            const auto& drawing = window->drawing();
            return folly::dynamic::object("dark", drawing.dark)("reduceMotion", drawing.reduceMotion)(
                "reduceTransparency", drawing.reduceTransparency)("accent", drawing.accent);
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
  instance->loadScript(std::make_unique<JSBigStdString>(readFile(bundlePath)), bundlePath);

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
    gestures->onCancelResponder = [&pointer] { pointer.cancelResponder(); };
    gestures->onIntercept = [&editing] { editing.releaseDrag(); };
    editing.useClipboard({
        .copy = [&window](uint64_t serial, const std::string& text) { window->copyText(serial, text); },
        .paste = [&window](uint64_t serial, std::function<void(std::string)> done) {
          window->pasteText(serial, std::move(done));
        },
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
    // The surface with the compositor's keyboard focus (0: none of this client's), and the text
    // field a shell region's surface was last asked to take it for (docs/adr/0024-system-shortcuts.mdx).
    uint32_t keyboardSurface = 0;
    facebook::react::Tag focusAskedFor = -1;
    // expo-clipboard's reads and writes carry the serial of the latest key or press the app
    // received (ADR 0007): the compositor refuses them unless the app has focus and that input is
    // recent. JavaScript hears when the clipboard changes.
    services.clipboard = {
        .copy = [&window, &lastPress](const std::string& text) { return window->copyText(lastPress, text); },
        .paste = [&window, &lastPress](std::function<void(std::string)> done) {
          window->pasteText(lastPress, std::move(done));
        },
        .hasText = [&window] { return window->clipboardHasText(); },
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
          .close = [&closeAsked] { closeAsked = true; },
          .minimize = [&window] { window->minimize(); },
          .zoom = [&window] { window->setMaximized(!window->maximized()); },
          // From the press being handled: JavaScript asks while it handles that press, so the
          // latest press is it, and the compositor checks it is still held.
          .startMove = [&window, &lastPress] {
            if (lastPress == 0) return false;
            window->beginMove(lastPress);
            return true;
          },
          .setSizeLimits = [&window](uint32_t minWidth, uint32_t minHeight, uint32_t maxWidth, uint32_t maxHeight) {
            window->setSizeLimits(minWidth, minHeight, maxWidth, maxHeight);
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
    if (shellMode) {
      services.shell = {
          .state = [&shellState] { return shellState.state(); },
          .manage = [&window](uint8_t action, uint32_t target, uint32_t workspace) {
            window->manage(action, target, workspace);
          },
          .lock = [&window] { window->lockSession(); },
          .launch = [&window, serial = uint32_t{0}](const std::string& id) mutable -> uint32_t {
            ++serial;
            return window->launch(serial, id) ? serial : 0;
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
      if (type == "focus") {
        accessibility.focus(view.tag);
        dirty = true;
        popups.changed();
        shellSurfaces.changed();
      }
    };
    int idleTurns = 0;  // turns in a row that found nothing to do
    // A window that changes size answers its configure with a frame JavaScript laid out at that
    // size, as AppKit lays a window out during a live resize before the frame is shown: the new
    // size's layout reaches JavaScript (onLayout, the dimensions), whose commits can size the
    // tree again (an app shell measuring its room, then a panel group measuring its own). Until
    // JavaScript is quiet, or kResizeSettle has passed so that a slow app never stalls the resize,
    // the window presents nothing and popups are not placed again.
    constexpr auto kResizeSettle = std::chrono::milliseconds(16);
    std::optional<Clock::time_point> settleBy;
    Clock::time_point settleFrom;
    int quietTurns = 0;  // turns in a row in which JavaScript ran nothing and nothing mounted
    int settleTurns = 0;
    try {
      // What runs JavaScript, timed for the report (Report::javascript).
      auto javascript = [](auto&& run) {
        if (!oxyos::Report::get().enabled()) return run();
        auto started = Clock::now();
        bool ran = run();
        oxyos::Report::get().javascript(std::chrono::duration<double, std::milli>(Clock::now() - started).count());
        return ran;
      };
      while (true) {
        oxyos::Report::get().beginTurn();
        Beat::tickAll();
        bool ranJs = javascript([&] { return loop->drain(); });
        bool pulled = mounter.pull();
        bool busy = ranJs || pulled;
        busy |= timers->fire(*timerManager);
        busy |= gestures->tick();
        // Worklets' frame is not work by itself (worklets-host.h): what it changes reaches the host
        // as a mount, a UI job or a frame asked for.
        bool uiFrame = javascript([&] { return worklets.tick(); });
        busy |= javascript([&] { return reanimated.tick(); });
        busy |= animated.tick();
        busy |= animationFrames.tick(window->ready(oxyos::Window::kToplevel));
        busy |= networking.deliver();
        // The wallpaper follows the sun (docs/adr/0012-solar-wallpaper.mdx).
        bool shellChanged = shellMode && shellState.tick(), closeRequested = false;
        std::optional<OxyOSEvent> authenticated;
        std::vector<OxyOSEvent> launchResults;
        std::vector<OxyOSEvent> preferenceResults;
        std::vector<OxyOSEvent> menuChoices;
        std::vector<OxyOSEvent> systemShortcuts;
        std::vector<OxyOSEvent> bezels;
        std::vector<OxyOSEvent> notificationResults, notificationsRemoved;
        bool responded = false;
        if (!window->dispatch([&](const OxyOSEvent& event) {
              // What an event dispatches reaches JavaScript on Fabric's next beat: a turn later.
              busy = true;
              shellChanged |= shellState.observe(event);
              if (event.kind == OXYOS_EVENT_AUTHENTICATION_RESULT) authenticated = event;
              if (event.kind == OXYOS_EVENT_LAUNCH_RESULT) launchResults.push_back(event);
              if (event.kind == OXYOS_EVENT_PREFERENCE_RESULT) preferenceResults.push_back(event);
              if (event.kind == OXYOS_EVENT_MENU_ITEM_ACTIVATED) menuChoices.push_back(event);
              if (event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT) systemShortcuts.push_back(event);
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
                } else if (keyboardSurface == event.surface) {
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
                  // And a notification the person pressed (ADR 0026): its response may copy.
                  event.kind == OXYOS_EVENT_NOTIFICATION_RESPONSE) {
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
              pointer.handle(event);
              editing.handle(event);
            })) {
          break;
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
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSShellChanged", shellState.state()));
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
                                          "search"};
          const char* action = shortcut.detail <= OXYOS_SHORTCUT_SEARCH ? actions[shortcut.detail] : "";
          folly::dynamic payload = folly::dynamic::object("action", action)("output", shortcut.code);
          oxyos::Report::get().input("shortcut.system", 0, payload);
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("OxyOSShellShortcut", payload));
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
        // Focus, caret and selection moves draw even when the tree did not change.
        bool edited = editing.edit() != drawn;
        if (edited) {
          drawn = editing.edit();
          dirty = true;
          popups.changed();
          shellSurfaces.changed();
        }
        bool resized = window->resized();
        if (resized) {
          display = oxyos::Display{window->width(), window->height(), window->scale(), display.background};
          pointer.setScale(display.scale);
          // useWindowDimensions and Dimensions follow the window (a resize, the output's scale or
          // size changed in Settings), as Android tells them on a configuration change.
          instance->callFunctionOnModule("RCTDeviceEventEmitter", "emit",
                                         folly::dynamic::array("didUpdateDimensions", oxyos::deviceDimensions(display)));
          auto laying = Clock::now();
          layOut();
          oxyos::Report::get().layout(display.width, display.height,
                                      std::chrono::duration<double, std::milli>(Clock::now() - laying).count());
          // A new size or scale needs a new frame even when the layout does not change (a
          // scale change keeps every logical frame); the old one would be discarded.
          dirty = true;
          busy = true;
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
          settleBy = settleFrom + kResizeSettle;
          quietTurns = settleTurns = 0;
        } else if (settleBy) {
          ++settleTurns;
          quietTurns = ranJs || pulled || mounted ? 0 : quietTurns + 1;
          bool timedOut = Clock::now() >= *settleBy;
          if (quietTurns >= 2 || timedOut) {
            settleBy.reset();
            oxyos::Report::get().settled(
                std::chrono::duration<double, std::milli>(Clock::now() - settleFrom).count(), settleTurns,
                timedOut && quietTurns < 2);
          }
        }
        bool settling = settleBy.has_value();
        if (settling) busy = true;
        // Images load at the size they are drawn; a picture arriving draws its view again.
        if (resized || mounted) images.sync(display.scale);
        if (images.deliver()) {
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
          auto started = Clock::now();
          auto damage = windowDamage.update(tree, display, drawn);
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
              size_t differing = 0;
              for (size_t i = 0; i < whole.pixels.size() && i < windowCanvas.pixels.size(); ++i) {
                differing += whole.pixels[i] != windowCanvas.pixels[i];
              }
              if (differing) {
                std::cerr << "[host] a frame drawn from its damage differs in " << differing << " pixels\n";
                oxyos::Report::get().damageMismatch(oxyos::Window::kToplevel, differing);
              }
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
        popups.present(drawn, focusName());
        shellSurfaces.present(drawn, focusName());
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
        if (idleTurns >= 2) {
          std::optional<Clock::time_point> deadline;
          for (auto due : {timers->next(), gestures->nextDeadline(), window->pasteDeadline(),
                           shellMode ? shellState.nextChange() : std::nullopt, worklets.nextFrame(),
                           reanimated.nextFrame(), animated.nextFrame(),
                           animationFrames.nextFrame(window->ready(oxyos::Window::kToplevel))}) {
            if (due && (!deadline || *due < *deadline)) deadline = *due;
          }
          std::vector<pollfd> fds;
          for (int fd : window->fds()) fds.push_back({fd, POLLIN, 0});
          for (int fd : {networking.fd(), images.fd()}) fds.push_back({fd, POLLIN, 0});
          if (!deadline) {
            ppoll(fds.data(), fds.size(), nullptr, nullptr);
          } else if (auto wait = std::chrono::duration_cast<std::chrono::nanoseconds>(*deadline - Clock::now()).count();
                     wait > 0) {
            // To the nanosecond: a frame due in 16.7 ms is not taken at 17.
            timespec timeout{static_cast<time_t>(wait / 1'000'000'000), static_cast<long>(wait % 1'000'000'000)};
            ppoll(fds.data(), fds.size(), &timeout, nullptr);
          }
        }
      }
    } catch (const std::exception& error) {
      std::cerr << "[host] " << error.what() << "\n";
      status = 1;
    }
    gestures->onCancelResponder = nullptr;
    gestures->onIntercept = nullptr;
    std::cout << "{\"engine\":\"hermes\",\"fabric\":true,\"window\":true,\"transactions\":"
              << mounter.transactions << ",\"presented\":" << window->presented()
              << ",\"js_errors\":" << jsErrors << "}\n";
    services.keyboard.listener.reset();
    window.reset();
    surface.stop();
    scheduler->unregisterSurface(surface);
    loop->drain();
    scheduler.reset();
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
    busy |= timers->fire(*timerManager);
    worklets.tick();
    busy |= reanimated.tick();
    busy |= animated.tick();
    busy |= animationFrames.tick(true);
    if (busy) images.sync(display.scale);
    busy |= images.deliver();
    busy |= networking.deliver();
    if (busy || timers->oneShotDueBefore(deadline)) {
      quietSince = Clock::now();
    } else if (Clock::now() - quietSince > std::chrono::milliseconds(500)) {
      break;
    }
    if (!busy) std::this_thread::sleep_for(std::chrono::milliseconds(2));
  }

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
