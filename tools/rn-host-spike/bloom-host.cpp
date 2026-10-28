// OxyOS React Native host: Hermes + bridgeless ReactInstance + Fabric on Linux, single-threaded.
// Loads a Metro bundle built for the `oxyos` platform, starts a surface and records Fabric's
// mounting transactions into a retained view tree that the renderer draws into a buffer.
#include "android-components.h"
#include "host-modules.h"
#include "input.h"
#include "popup.h"
#include "renderer.h"
#include "text.h"
#include "textinput.h"
#include "window.h"
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
  explicit Mounter(oxy::ViewTree& tree) : tree_(tree) {}
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
  // View commands from JavaScript (a text input's focus, blur, ...), for the host's native side.
  std::function<void(const ShadowView&, const std::string&, const folly::dynamic&)> onCommand;
  void schedulerDidDispatchCommand(const ShadowView& view, const std::string& name, const folly::dynamic& args) override {
    if (onCommand) onCommand(view, name, args);
  }
  void schedulerDidSendAccessibilityEvent(const ShadowView&, const std::string&) override {}
  void schedulerDidSetIsJSResponder(const ShadowView&, bool, bool) override {}
  void schedulerShouldSynchronouslyUpdateViewOnUIThread(Tag, const folly::dynamic&) override {}
  void schedulerDidUpdateShadowTree(const std::unordered_map<Tag, folly::dynamic>&) override {}
  void schedulerDidCaptureViewSnapshot(Tag, SurfaceId) override {}
  void schedulerDidSetViewSnapshot(Tag, Tag, SurfaceId) override {}
  void schedulerDidClearPendingSnapshots() override {}

 private:
  oxy::ViewTree& tree_;
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
  if (std::getenv("OXY_HOST_TRACE")) std::cerr << "[host] " << name << "\n";
}

}  // namespace

int main(int argc, char** argv) {
  if (argc < 4) {
    std::cerr << "usage: oxy-bloom-host BUNDLE.js MODULE (OUT.png | --window) [WIDTH HEIGHT]\n"
                 "  OUT.png   run until the app settles, then draw it into OUT.png\n"
                 "  --window  present it in a window on the compositor at $OXY_COMPOSITOR_SOCKET\n";
    return 2;
  }
  std::signal(SIGSEGV, crashed);
  std::signal(SIGABRT, crashed);
  const std::string bundlePath = argv[1], moduleName = argv[2], outPath = argv[3];
  const bool windowed = outPath == "--window";
  const char* socket = std::getenv("OXY_COMPOSITOR_SOCKET");
  if (windowed && !socket) {
    std::cerr << "--window needs OXY_COMPOSITOR_SOCKET\n";
    return 2;
  }
  const float width = argc > 5 ? std::stof(argv[4]) : 1024;
  const float height = argc > 5 ? std::stof(argv[5]) : 640;

  for (const char* path : {"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                           "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"}) {
    if (std::ifstream(path).good()) oxy::TextEngine::shared().addFallback(path);
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
  loop->drain();

  oxy::Display display{width, height, 1.0f};
  auto invoker = instance->createJSCallInvoker();
  oxy::WorkletsHost worklets;
  std::unique_ptr<Scheduler> scheduler;
  oxy::ReanimatedHost reanimated(worklets, [&scheduler] { return scheduler.get(); });
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
    TurboModuleBinding::install(
        rt, [&, invoker](jsi::Runtime& runtime, const std::string& name) -> std::shared_ptr<TurboModule> {
          if (name == "WorkletsModule") return worklets.module(runtime, invoker);
          if (name == "ReanimatedModule") return reanimated.module(runtime, invoker);
          if (auto module = oxy::createHostModule(name, invoker, display)) return module;
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
  providers->add(concreteComponentDescriptorProvider<oxy::TextInputComponentDescriptor>());
  providers->add(concreteComponentDescriptorProvider<OxyPopupViewComponentDescriptor>());
  oxy::addAndroidComponents(*providers);
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

  oxy::ViewTree tree;
  Mounter mounter(tree);
  stage("fabric scheduler");
  scheduler = std::make_unique<Scheduler>(toolbox, nullptr, &mounter);
  mounter.attach(scheduler.get());

  // In a window, the compositor's configure decides the size the app lays out at.
  std::unique_ptr<oxy::Window> window;
  if (windowed) {
    try {
      window = std::make_unique<oxy::Window>(socket, static_cast<uint32_t>(width), static_cast<uint32_t>(height));
      while (!window->configured()) {
        if (!window->dispatch([](const OxyEvent&) {})) {
          std::cerr << "[host] the compositor closed the connection before configuring the window\n";
          return 1;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
      }
      window->resized();
      display = oxy::Display{window->width(), window->height(), window->scale()};
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
    oxy::PopupHost popups(tree, *window);
    oxy::PointerInput pointer(tree, [&](uint32_t surface) -> std::optional<oxy::SurfaceContent> {
      if (surface == oxy::Window::kToplevel) return oxy::SurfaceContent{.view = tree.root()};
      return popups.content(surface);
    });
    oxy::TextEditing editing(tree);
    // The latest press or key press: the input a popup opened by it cites for its grab.
    uint64_t lastPress = 0;
    facebook::react::Tag caret = -1;
    mounter.onCommand = [&](const ShadowView& view, const std::string& name, const folly::dynamic& args) {
      editing.command(view, name, args);
    };
    try {
      while (true) {
        Beat::tickAll();
        bool busy = loop->drain();
        busy |= mounter.pull();
        busy |= timers->fire(*timerManager);
        worklets.tick();
        busy |= reanimated.tick();
        if (!window->dispatch([&](const OxyEvent& event) {
              if ((event.kind == OXY_EVENT_POINTER && event.detail == OXY_POINTER_PRESS) ||
                  (event.kind == OXY_EVENT_KEY && event.flag)) {
                lastPress = event.serial;
              }
              if (event.kind == OXY_EVENT_POPUP_DONE) popups.dismissed(event);
              pointer.handle(event);
              editing.handle(event);
            })) {
          break;
        }
        // Focus moves draw (or remove) a caret even when the tree did not change.
        if (editing.focused() != caret) {
          caret = editing.focused();
          dirty = true;
          popups.changed();
        }
        if (window->resized()) {
          display = oxy::Display{window->width(), window->height(), window->scale()};
          layOut();
          busy = true;
        }
        if (mounter.takeChanged()) {
          dirty = true;
          busy = true;
          popups.changed();
        }
        popups.sync(lastPress);
        if (dirty && window->ready(oxy::Window::kToplevel)) {
          oxy::Canvas canvas;
          oxy::draw(tree, display, canvas, caret);
          window->present(oxy::Window::kToplevel, canvas);
          dirty = false;
          busy = true;
        }
        popups.present(caret);
        if (!busy) std::this_thread::sleep_for(std::chrono::milliseconds(2));
      }
    } catch (const std::exception& error) {
      std::cerr << "[host] " << error.what() << "\n";
      status = 1;
    }
    std::cout << "{\"engine\":\"hermes\",\"fabric\":true,\"window\":true,\"transactions\":"
              << mounter.transactions << ",\"presented\":" << window->presented()
              << ",\"js_errors\":" << jsErrors << "}\n";
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
    if (busy || timers->oneShotDueBefore(deadline)) {
      quietSince = Clock::now();
    } else if (Clock::now() - quietSince > std::chrono::milliseconds(500)) {
      break;
    }
    if (!busy) std::this_thread::sleep_for(std::chrono::milliseconds(2));
  }

  stage("render");
  auto stats = oxy::render(tree, display, outPath);
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
