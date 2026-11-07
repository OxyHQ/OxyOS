#include "worklets-host.h"

#include "host-modules.h"
#include "report.h"

#include <cxxreact/JSBigString.h>

#include <worklets/NativeModules/WorkletsModuleProxy.h>
#include <worklets/NativeModules/WorkletsModuleProxyInitializer.h>
#include <worklets/Networking/NetworkingBackend.h>
#include <worklets/Tools/JSScheduler.h>
#include <worklets/Tools/PlatformLogger.h>
#include <worklets/Tools/RNRuntimeStatus.h>
#include <worklets/Tools/UIScheduler.h>
#include <worklets/WorkletRuntime/RuntimeBindings.h>

#include <algorithm>
#include <chrono>
#include <iostream>

using namespace facebook;

namespace worklets {

void PlatformLogger::log(const char* str) { std::cerr << "[worklets] " << str << "\n"; }
void PlatformLogger::log(const std::string& str) { log(str.c_str()); }
void PlatformLogger::log(const double d) { std::cerr << "[worklets] " << d << "\n"; }
void PlatformLogger::log(const int i) { std::cerr << "[worklets] " << i << "\n"; }
void PlatformLogger::log(const bool b) { std::cerr << "[worklets] " << (b ? "true" : "false") << "\n"; }

}  // namespace worklets

namespace oxyos {

namespace {

double nowMs() {
  using namespace std::chrono;
  return duration<double, std::milli>(steady_clock::now().time_since_epoch()).count();
}

// The host's single thread is both the JS and the UI thread.
class LoopUIScheduler : public worklets::UIScheduler {
 public:
  // Whether UI jobs are queued for the next triggerUI.
  bool pending() { return !uiJobs_.empty(); }

 protected:
  bool queryIsOnUIThread() const override { return true; }
};

// Networking from worklet runtimes has no OxyOS backend yet: every request fails explicitly.
class UnsupportedNetworking : public worklets::NetworkingBackend {
 public:
  void sendRequest(uint64_t, worklets::RequestConfig&&,
                   const std::shared_ptr<worklets::NetworkRequestListener>& listener) override {
    listener->onError(worklets::RequestError::Network, "Networking from worklet runtimes is not supported on OxyOS yet");
  }
  void abortRequest(uint64_t) override {}
};

}  // namespace

WorkletsHost::WorkletsHost(const DisplayRefresh& refresh, std::string compiled)
    : refresh_(refresh),
      ui_(std::make_shared<LoopUIScheduler>()),
      frames_(std::make_shared<std::vector<std::function<void(double)>>>()),
      wanted_(std::make_shared<bool>(true)),
      compiled_(std::move(compiled)) {}

WorkletsHost::~WorkletsHost() {
  if (initializer_) initializer_->invalidate();
}

std::shared_ptr<react::TurboModule> WorkletsHost::module(
    jsi::Runtime& runtime,
    const std::shared_ptr<react::CallInvoker>& invoker) {
  if (!initializer_) {
    auto frames = frames_;
    auto bindings = std::make_shared<worklets::RuntimeBindings>(worklets::RuntimeBindings{
        .requestAnimationFrame = [this, frames](std::function<void(const double)>&& callback) {
          frames->push_back(std::move(callback));
          // Worklets' frame loop starts as the UI runtime sets its frame queue up, which is when
          // its requestAnimationFrame is there to watch.
          if (!watching_) watchRequests();
        },
        .nativeLoggingHook =
            [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
              if (count > 0) std::cerr << "[worklets console] " << args[0].toString(rt).utf8(rt) << "\n";
              return jsi::Value::undefined();
            },
        .networkingBackend = std::make_shared<UnsupportedNetworking>()});
    initializer_ = std::make_shared<worklets::WorkletsModuleProxyInitializer>(
        std::make_shared<worklets::JSScheduler>(runtime, invoker, [] { return true; }), ui_, bindings,
        std::make_shared<worklets::RNRuntimeStatus>());
    initializer_->prepareProxy();
  }
  auto module = std::make_shared<HostModule>("WorkletsModule", invoker);
  module->method("installTurboModule", [this](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
    bool bundleMode = count > 0 && args[0].isBool() && args[0].getBool();
    if (bundleMode) {
      throw jsi::JSError(rt, "Worklets bundle mode is not supported on OxyOS yet");
    }
    proxy_ = initializer_->finalize(rt, false, [] { return worklets::BundleModeConfig{}; });
    if (auto runtime = proxy_ ? proxy_->getUIWorkletRuntime() : nullptr) makeWorkletsCompiled(runtime->getJSIRuntime());
    return jsi::Value(true);
  });
  module->method("start", [this](jsi::Runtime&, const jsi::Value*, size_t) {
    if (proxy_) proxy_->start();
    return jsi::Value(proxy_ != nullptr);
  });
  module->method("prepareBundleMode", [](jsi::Runtime&, const jsi::Value*, size_t) { return jsi::Value(false); });
  module->unsupported({"toggleSlowAnimationsOnUIRuntime"});
  return module;
}

void WorkletsHost::makeWorkletsCompiled(jsi::Runtime& ui) {
  // Worklets make a worklet on the UI runtime the first time it runs there, from its text: with
  // `evalWithSourceUrl(code, "worklet_<hash>")` where the runtime has one (worklets' own
  // valueUnpacker), else `eval`. Hermes compiling each from source cost about a quarter of a
  // millisecond, hundreds of them before Settings' first frame showed its text (docs/performance.mdx).
  // The framework's are compiled ahead (`__oxyosWorklets`, by that URL), evaluated here from their
  // file, mapped and shared by every application on this runtime; a worklet the framework does
  // not have (an application's own) is compiled from its text, as worklets would.
  if (!compiled_.empty()) {
    try {
      ui.evaluateJavaScript(react::JSBigFileString::fromPath(compiled_), compiled_);
    } catch (const std::exception& error) {
      std::cerr << "[host] the framework's worklets compiled ahead could not be read (" << error.what()
                << "): each is compiled from its text\n";
    }
  }
  ui.global().setProperty(
      ui, "evalWithSourceUrl",
      jsi::Function::createFromHostFunction(
          ui, jsi::PropNameID::forAscii(ui, "evalWithSourceUrl"), 2,
          [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) -> jsi::Value {
            if (count < 2 || !args[0].isString() || !args[1].isString()) {
              throw jsi::JSError(rt, "evalWithSourceUrl: a worklet's text and its URL");
            }
            std::string url = args[1].getString(rt).utf8(rt);
            auto table = rt.global().getProperty(rt, "__oxyosWorklets");
            if (table.isObject()) {
              auto compiled = table.getObject(rt).getProperty(rt, url.c_str());
              if (compiled.isObject()) {
                Report::get().worklet(url, true);
                return compiled;
              }
            }
            Report::get().worklet(url, false);
            return rt.evaluateJavaScript(std::make_shared<jsi::StringBuffer>(args[0].getString(rt).utf8(rt)), url);
          }));
}

void WorkletsHost::watchRequests() {
  watching_ = true;
  if (!proxy_) {
    std::cerr << "[host] worklets' frame loop started before its runtime: every frame runs\n";
    return;
  }
  // requestAnimationFrame as worklets set it up, kept under a name of the host's, and in its
  // place one that wants the next frame first. Whatever asks for a frame later (worklets' timers,
  // Reanimated's animations and mappers) finds this one.
  auto& rt = proxy_->getUIWorkletRuntime()->getJSIRuntime();
  auto global = rt.global();
  auto request = global.getProperty(rt, "requestAnimationFrame");
  if (!request.isObject() || !request.getObject(rt).isFunction(rt)) {
    std::cerr << "[host] worklets' UI runtime has no requestAnimationFrame to watch: every frame runs\n";
    return;
  }
  global.setProperty(rt, "__oxyosRequestAnimationFrame", request);
  auto wanted = wanted_;
  global.setProperty(
      rt, "requestAnimationFrame",
      jsi::Function::createFromHostFunction(
          rt, jsi::PropNameID::forAscii(rt, "requestAnimationFrame"), 1,
          [wanted](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
            *wanted = true;
            return rt.global().getPropertyAsFunction(rt, "__oxyosRequestAnimationFrame").call(rt, args, count);
          }));
  watched_ = true;
}

facebook::jsi::Runtime* WorkletsHost::uiRuntime() const {
  if (!proxy_) return nullptr;
  auto runtime = proxy_->getUIWorkletRuntime();
  return runtime ? &runtime->getJSIRuntime() : nullptr;
}

bool WorkletsHost::tick() {
  // UIScheduler::triggerUI drains every queued UI job; one may set what the next frame acts on.
  bool ran = std::static_pointer_cast<LoopUIScheduler>(ui_)->pending();
  ui_->triggerUI();
  if (ran) *wanted_ = true;
  double now = nowMs();
  if (!frames_->empty() && (*wanted_ || !watched_) && now - lastFrame_ >= refresh_.periodMs()) {
    lastFrame_ = now;
    // What the frame asks for wants the one after it.
    *wanted_ = false;
    auto callbacks = std::move(*frames_);
    frames_->clear();
    for (auto& callback : callbacks) callback(now);
    ran = true;
  }
  return ran;
}

std::optional<std::chrono::steady_clock::time_point> WorkletsHost::nextFrame() const {
  if (frames_->empty() || (watched_ && !*wanted_)) return std::nullopt;
  double wait = std::max(0.0, lastFrame_ + refresh_.periodMs() - nowMs());
  return std::chrono::steady_clock::now() +
         std::chrono::duration_cast<std::chrono::steady_clock::duration>(std::chrono::duration<double, std::milli>(wait));
}

}  // namespace oxyos
