#include "reanimated-host.h"

#include "host-modules.h"
#include "worklets-host.h"

#include <reanimated/NativeModules/ReanimatedModuleProxy.h>
#include <reanimated/RuntimeDecorators/RNRuntimeDecorator.h>
#include <reanimated/Tools/PlatformDepMethodsHolder.h>
#include <worklets/Compat/StableApi.h>
#include <worklets/NativeModules/WorkletsModuleProxy.h>

#include <react/renderer/core/EventListener.h>
#include <react/renderer/scheduler/Scheduler.h>
#include <react/renderer/uimanager/UIManager.h>

#include <chrono>
#include <cstdlib>
#include <iostream>

using namespace facebook;

namespace oxyos {

namespace {

double nowMs() {
  using namespace std::chrono;
  return duration<double, std::milli>(steady_clock::now().time_since_epoch()).count();
}

void unsupported(const char* what) {
  static std::set<std::string> reported;
  if (reported.insert(what).second) std::cerr << "[host] unsupported on OxyOS: Reanimated " << what << "\n";
}

}  // namespace

ReanimatedHost::ReanimatedHost(
    WorkletsHost& worklets,
    std::function<react::Scheduler*()> scheduler,
    const DisplayRefresh& refresh)
    : worklets_(worklets),
      scheduler_(std::move(scheduler)),
      refresh_(refresh),
      renders_(std::make_shared<std::vector<std::function<void(double)>>>()) {}

ReanimatedHost::~ReanimatedHost() = default;

std::shared_ptr<react::TurboModule> ReanimatedHost::module(
    jsi::Runtime&,
    const std::shared_ptr<react::CallInvoker>& invoker) {
  auto module = std::make_shared<HostModule>("ReanimatedModule", invoker);
  module->method("installTurboModule", [this, invoker](jsi::Runtime& rt, const jsi::Value*, size_t) {
    auto workletsProxy = worklets_.proxy();
    if (!workletsProxy) throw jsi::JSError(rt, "Reanimated needs Worklets to be installed first");
    auto uiRuntime = workletsProxy->getUIWorkletRuntime();
    auto renders = renders_;
    reanimated::PlatformDepMethodsHolder platform{
        .requestRender = [renders](std::function<void(const double)> callback) {
          renders->push_back(std::move(callback));
        },
        .synchronouslyUpdateUIPropsFunction = nullptr,  // disabled by Reanimated's static flags
        .getAnimationTimestamp = [] { return nowMs(); },
        .registerSensor = [](int, int, int, std::function<void(double[], int)>) {
          unsupported("sensors");
          return -1;
        },
        .unregisterSensor = [](int) {},
        .setGestureStateFunction = [](int, int) { unsupported("gesture state"); },
        .subscribeForKeyboardEvents = [](std::function<void(int, int)>, bool, bool) {
          unsupported("keyboard events");
          return -1;
        },
        .unsubscribeFromKeyboardEvents = [](int) {},
        .maybeFlushUIUpdatesQueueFunction = [] {},
        .attachPseudoSelector = [](react::Tag, reanimated::PseudoSelector, std::function<void(bool)>) {
          unsupported("pseudo selectors");
        },
        .detachPseudoSelector = [](react::Tag, reanimated::PseudoSelector) {},
        .platformTransitionBackend = nullptr,
        .platformAnimationFactory = nullptr,
    };
    proxy_ = std::make_shared<reanimated::ReanimatedModuleProxy>(
        uiRuntime, workletsProxy->getUIScheduler(), rt, invoker, platform, /*isReducedMotion=*/false);
    proxy_->init(platform);
    auto* scheduler = scheduler_();
    proxy_->initializeFabric(scheduler->getUIManager());
    // As Reanimated's iOS module does: every Fabric event reaches its worklet event handlers
    // (useAnimatedScrollHandler, ...) first. The host's one thread is the UI thread.
    std::weak_ptr<reanimated::ReanimatedModuleProxy> weak = proxy_;
    scheduler->addEventListener(std::make_shared<react::EventListener>([weak](const react::RawEvent& event) {
      auto proxy = weak.lock();
      if (proxy && std::getenv("OXYOS_HOST_TRACE") && event.eventTarget) {
        std::string type = event.type.rfind("top", 0) == 0 ? "on" + event.type.substr(3) : event.type;
        std::cerr << "[reanimated] event " << event.type << " tag " << event.eventTarget->getTag() << " handled "
                  << proxy->isAnyHandlerWaitingForEvent(type, event.eventTarget->getTag()) << "\n";
      }
      return proxy && proxy->handleRawEvent(event, nowMs());
    }));
    reanimated::RNRuntimeDecorator::decorate(rt, worklets::getJSIRuntimeFromWorkletRuntime(uiRuntime), proxy_);
    return jsi::Value(true);
  });
  return module;
}

void ReanimatedHost::event(const std::string& name, int tag, const folly::dynamic& payload) {
  if (!proxy_) return;
  auto& ui = worklets::getJSIRuntimeFromWorkletRuntime(worklets_.proxy()->getUIWorkletRuntime());
  proxy_->handleEvent(name, tag, jsi::valueFromDynamic(ui, payload), nowMs());
}

bool ReanimatedHost::tick() {
  double now = nowMs();
  if (!proxy_ || (!wanted_ && renders_->empty()) || now - lastFrame_ < refresh_.periodMs()) return false;
  lastFrame_ = now;
  wanted_ = false;
  // As Android's frame callback does: run requested render callbacks, then always let Reanimated
  // perform its operations, which also resumes React commits it paused.
  bool rendered = !renders_->empty();
  auto callbacks = std::move(*renders_);
  renders_->clear();
  for (auto& callback : callbacks) callback(now);
  proxy_->performOperations();
  return rendered;
}

std::optional<std::chrono::steady_clock::time_point> ReanimatedHost::nextFrame() const {
  if (!proxy_ || (!wanted_ && renders_->empty())) return std::nullopt;
  double wait = std::max(0.0, lastFrame_ + refresh_.periodMs() - nowMs());
  return std::chrono::steady_clock::now() +
         std::chrono::duration_cast<std::chrono::steady_clock::duration>(std::chrono::duration<double, std::milli>(wait));
}

}  // namespace oxyos
