// Feasibility host: Hermes V1 + React Native 0.88 Fabric (cxx platform) + Yoga on Linux.
// A JS script builds a tree through the real nativeFabricUIManager binding; Fabric commits it,
// Yoga lays it out and the mounting coordinator delivers mutations, which are printed.
#include <hermes/hermes.h>
#include <jsi/jsi.h>
#include <ReactCommon/RuntimeExecutor.h>
#include <react/featureflags/ReactNativeFeatureFlags.h>
#include <react/featureflags/ReactNativeFeatureFlagsDefaults.h>
#include <react/renderer/componentregistry/ComponentDescriptorProviderRegistry.h>
#include <react/renderer/components/root/RootComponentDescriptor.h>
#include <react/renderer/components/scrollview/ScrollViewComponentDescriptor.h>
#include <react/renderer/components/text/ParagraphComponentDescriptor.h>
#include <react/renderer/components/text/RawTextComponentDescriptor.h>
#include <react/renderer/components/text/TextComponentDescriptor.h>
#include <react/renderer/components/view/ViewComponentDescriptor.h>
#include <react/renderer/core/EventBeat.h>
#include <react/renderer/mounting/MountingCoordinator.h>
#include <react/renderer/runtimescheduler/RuntimeScheduler.h>
#include <react/renderer/scheduler/Scheduler.h>
#include <react/renderer/scheduler/SchedulerDelegate.h>
#include <react/renderer/scheduler/SchedulerToolbox.h>
#include <react/renderer/scheduler/SurfaceHandler.h>
#include <react/utils/ContextContainer.h>
#include <fstream>
#include <iostream>
#include <queue>
#include <sstream>

using namespace facebook;
using namespace facebook::react;

namespace {

class Flags : public ReactNativeFeatureFlagsDefaults {
 public:
  bool enableBridgelessArchitecture() override { return true; }
};

class Queue {
 public:
  void push(std::function<void()> task) { tasks_.push(std::move(task)); }
  void flush() {
    while (!tasks_.empty()) {
      auto task = std::move(tasks_.front());
      tasks_.pop();
      task();
    }
  }
 private:
  std::queue<std::function<void()>> tasks_;
};

class Beat : public EventBeat {
 public:
  Beat(std::shared_ptr<OwnerBox> owner, RuntimeScheduler& scheduler)
      : EventBeat(std::move(owner), scheduler) {}
};

const char* kind(ShadowViewMutation::Type type) {
  switch (type) {
    case ShadowViewMutation::Create: return "create";
    case ShadowViewMutation::Delete: return "delete";
    case ShadowViewMutation::Insert: return "insert";
    case ShadowViewMutation::Remove: return "remove";
    case ShadowViewMutation::Update: return "update";
  }
  return "?";
}

class Printer : public SchedulerDelegate {
 public:
  int mutations = 0;
  void schedulerDidFinishTransaction(const std::shared_ptr<const MountingCoordinator>&) override {}
  void schedulerShouldRenderTransactions(
      const std::shared_ptr<const MountingCoordinator>& coordinator) override {
    auto transaction = coordinator->pullTransaction();
    if (!transaction) return;
    for (const auto& m : transaction->getMutations()) {
      const auto& view = m.type == ShadowViewMutation::Delete || m.type == ShadowViewMutation::Remove
          ? m.oldChildShadowView : m.newChildShadowView;
      const auto& f = view.layoutMetrics.frame;
      std::cout << "{\"mutation\":\"" << kind(m.type) << "\",\"component\":\"" << view.componentName
                << "\",\"tag\":" << view.tag << ",\"parent\":" << m.parentTag
                << ",\"frame\":[" << f.origin.x << "," << f.origin.y << "," << f.size.width << ","
                << f.size.height << "]}\n";
      ++mutations;
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
};

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::cerr << "usage: oxy-rn-spike SCRIPT.js\n";
    return 2;
  }
  std::ifstream file(argv[1]);
  std::stringstream source;
  source << file.rdbuf();

  ReactNativeFeatureFlags::override(std::make_unique<Flags>());
  auto runtime = facebook::hermes::makeHermesRuntime(
      ::hermes::vm::RuntimeConfig::Builder().withMicrotaskQueue(true).build());
  Queue queue;
  RuntimeExecutor executor = [&](std::function<void(jsi::Runtime&)>&& callback) {
    queue.push([&, callback = std::move(callback)]() mutable { callback(*runtime); });
  };
  auto runtimeScheduler = std::make_shared<RuntimeScheduler>(
      executor, [] { return HighResTimeStamp::now(); },
      [](jsi::Runtime&, jsi::JSError& error) { std::cerr << "JS error: " << error.what() << "\n"; });
  auto context = std::make_shared<ContextContainer>();
  context->insert(RuntimeSchedulerKey, std::weak_ptr<RuntimeScheduler>(runtimeScheduler));

  SchedulerToolbox toolbox{};
  toolbox.contextContainer = context;
  toolbox.runtimeExecutor = executor;
  toolbox.componentRegistryFactory = [](const EventDispatcher::Weak& dispatcher,
                                        const std::shared_ptr<const ContextContainer>& container) {
    ComponentDescriptorProviderRegistry providers{};
    providers.add(concreteComponentDescriptorProvider<RootComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ViewComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ParagraphComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<RawTextComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<TextComponentDescriptor>());
    providers.add(concreteComponentDescriptorProvider<ScrollViewComponentDescriptor>());
    return providers.createComponentDescriptorRegistry(ComponentDescriptorParameters{
        .eventDispatcher = dispatcher, .contextContainer = container, .flavor = nullptr});
  };
  toolbox.eventBeatFactory = [&](std::shared_ptr<EventBeat::OwnerBox> owner) {
    return std::make_unique<Beat>(std::move(owner), *runtimeScheduler);
  };

  Printer printer;
  auto scheduler = std::make_unique<Scheduler>(toolbox, nullptr, &printer);
  queue.flush();  // installs nativeFabricUIManager

  runtime->global().setProperty(*runtime, "print", jsi::Function::createFromHostFunction(
      *runtime, jsi::PropNameID::forAscii(*runtime, "print"), 1,
      [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
        std::cerr << "[js] " << (count ? args[0].toString(rt).utf8(rt) : "") << "\n";
        return jsi::Value::undefined();
      }));
  runtime->evaluateJavaScript(std::make_shared<jsi::StringBuffer>(source.str()), argv[1]);
  SurfaceHandler surface("OxySpike", 11);
  scheduler->registerSurface(surface);
  surface.constraintLayout(
      LayoutConstraints{.minimumSize = {800, 600}, .maximumSize = {800, 600}},
      LayoutContext{.pointScaleFactor = 1});
  surface.start();
  for (int tick = 0; tick < 8; ++tick) {
    runtimeScheduler->scheduleWork([](jsi::Runtime&) {});
    queue.flush();
  }
  std::cout << "{\"summary\":{\"engine\":\"hermes\",\"fabric\":true,\"mutations\":" << printer.mutations
            << ",\"surface\":[800,600]}}\n";
  surface.stop();
  scheduler->unregisterSurface(surface);
  queue.flush();
  scheduler.reset();
  return printer.mutations > 0 ? 0 : 1;
}
