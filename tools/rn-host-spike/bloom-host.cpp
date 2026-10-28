// OxyOS React Native host: Hermes + bridgeless ReactInstance + Fabric on Linux, single-threaded.
// Loads a Metro bundle built for the `oxyos` platform, starts a surface and records Fabric's
// mounting transactions into a retained view tree that the renderer draws into a buffer.
#include "host-modules.h"
#include "renderer.h"
#include "text.h"

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
#include <fstream>
#include <iostream>
#include <map>
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

  bool pending() const { return !timers_.empty(); }

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

class Beat : public EventBeat {
 public:
  Beat(std::shared_ptr<OwnerBox> owner, RuntimeScheduler& scheduler)
      : EventBeat(std::move(owner), scheduler) {}
};

class Mounter : public SchedulerDelegate {
 public:
  explicit Mounter(oxy::ViewTree& tree) : tree_(tree) {}
  int transactions = 0;
  void schedulerDidFinishTransaction(const std::shared_ptr<const MountingCoordinator>&) override {}
  void schedulerShouldRenderTransactions(
      const std::shared_ptr<const MountingCoordinator>& coordinator) override {
    if (auto transaction = coordinator->pullTransaction()) {
      tree_.apply(transaction->getMutations());
      ++transactions;
    }
  }
  void schedulerShouldMergeReactRevision(SurfaceId) override {}
  void schedulerDidRequestPreliminaryViewAllocation(const ShadowNode&) override {}
  void schedulerDidDispatchCommand(const ShadowView&, const std::string&, const folly::dynamic&) override {}
  void schedulerDidSendAccessibilityEvent(const ShadowView&, const std::string&) override {}
  void schedulerDidSetIsJSResponder(const ShadowView&, bool, bool) override {}
  void schedulerShouldSynchronouslyUpdateViewOnUIThread(Tag, const folly::dynamic&) override {}
  void schedulerDidUpdateShadowTree(const std::unordered_map<Tag, folly::dynamic>&) override {}
  void schedulerDidCaptureViewSnapshot(Tag, SurfaceId) override {}
  void schedulerDidSetViewSnapshot(Tag, Tag, SurfaceId) override {}
  void schedulerDidClearPendingSnapshots() override {}

 private:
  oxy::ViewTree& tree_;
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
    std::cerr << "usage: oxy-bloom-host BUNDLE.js MODULE OUT.png [WIDTH HEIGHT] [--fonts DIR]\n";
    return 2;
  }
  std::signal(SIGSEGV, crashed);
  std::signal(SIGABRT, crashed);
  const std::string bundlePath = argv[1], moduleName = argv[2], outPath = argv[3];
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
  instance->initializeRuntime({}, [&](jsi::Runtime& rt) {
    TurboModuleBinding::install(
        rt, [&, invoker](jsi::Runtime&, const std::string& name) -> std::shared_ptr<TurboModule> {
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
  toolbox.componentRegistryFactory = [](const EventDispatcher::Weak& dispatcher,
                                        const std::shared_ptr<const ContextContainer>& container) {
    ComponentDescriptorProviderRegistry providers{};
    providers.add(concreteComponentDescriptorProvider<RootComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ViewComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ParagraphComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<RawTextComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<TextComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ScrollViewComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ImageComponentDescriptor>());
    auto registry = providers.createComponentDescriptorRegistry(ComponentDescriptorParameters{
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
  auto scheduler = std::make_unique<Scheduler>(toolbox, nullptr, &mounter);

  stage("load script");
  instance->loadScript(std::make_unique<JSBigStdString>(readFile(bundlePath)), bundlePath);

  stage("surface");
  SurfaceHandler surface(moduleName, 1);
  scheduler->registerSurface(surface);
  surface.constraintLayout(
      LayoutConstraints{.minimumSize = {width, height}, .maximumSize = {width, height}},
      LayoutContext{.pointScaleFactor = display.scale});
  surface.start();

  // Run until the app settles: no queued work for a while, or a hard deadline.
  auto deadline = Clock::now() + std::chrono::seconds(10);
  auto quietSince = Clock::now();
  while (Clock::now() < deadline) {
    bool busy = loop->drain();
    busy |= timers->fire(*timerManager);
    if (busy) {
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
            << ",\"output\":\"" << outPath << "\"}\n";

  surface.stop();
  scheduler->unregisterSurface(surface);
  loop->drain();
  scheduler.reset();
  return jsErrors == 0 && mounter.transactions > 0 ? 0 : 1;
}
