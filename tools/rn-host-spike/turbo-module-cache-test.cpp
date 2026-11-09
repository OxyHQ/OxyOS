// turbo-module-cache.h's test: a runtime's TurboModules are one instance per name, and a method
// taken from a module keeps working after JavaScript has dropped the module's object, through a
// full collection and thousands of lookups. The module is a C++ TurboModule as React Native
// generates them (a method map whose host functions hold the module by `this`), so a module
// destroyed under its method is a use after free, which a build with AddressSanitizer reports
// (docs/bloom-host-sanitizers.mdx); --uncached runs the same without the cache, to see it. It also
// checks what the cache says of names it has no module for: each once per runtime, none of the
// probes with a fallback of their own (kProbedWithFallback).
#include "turbo-module-cache.h"

#include <ReactCommon/TurboModuleBinding.h>
#include <hermes/hermes.h>
#include <jsi/instrumentation.h>

#include <cstring>
#include <iostream>
#include <algorithm>
#include <memory>
#include <string>
#include <vector>

namespace jsi = facebook::jsi;
using facebook::react::CallInvoker;
using facebook::react::TurboModule;
using facebook::react::TurboModuleBinding;

namespace {

int made = 0;
int alive = 0;

// `add(x)` answers x plus the module's own number, read through the module.
class ProbeModule : public TurboModule {
 public:
  explicit ProbeModule(std::shared_ptr<CallInvoker> invoker)
      : TurboModule("OxyOSTurboModuleProbe", std::move(invoker)), base_(std::make_unique<double>(40)) {
    ++made;
    ++alive;
    methodMap_["add"] = MethodMetadata{1, [](jsi::Runtime&, TurboModule& module, const jsi::Value* args, size_t count) {
      return jsi::Value(*static_cast<ProbeModule&>(module).base_ + (count > 0 ? args[0].asNumber() : 0));
    }};
  }
  ~ProbeModule() override { --alive; }

 private:
  std::unique_ptr<double> base_;
};

int failures = 0;
void check(bool ok, const std::string& what) {
  std::cout << (ok ? "ok: " : "FAIL: ") << what << "\n";
  if (!ok) ++failures;
}

}  // namespace

int main(int argc, char** argv) {
  const bool cached = !(argc > 1 && std::strcmp(argv[1], "--uncached") == 0);
  {
    auto runtime = facebook::hermes::makeHermesRuntime();
    jsi::Runtime& rt = *runtime;
    // As ReactInstance sets it: modules are reached through `nativeModuleProxy`.
    rt.global().setProperty(rt, "RN$Bridgeless", true);
    facebook::react::TurboModuleProviderFunctionTypeWithRuntime provider =
        [](jsi::Runtime&, const std::string& name) -> std::shared_ptr<TurboModule> {
      if (name == "OxyOSTurboModuleProbe") return std::make_shared<ProbeModule>(nullptr);
      return nullptr;
    };
    // What the cache says of a name it has no module for, in order.
    std::vector<std::string> said;
    auto unavailable = [&said](const std::string& name) { said.push_back(name); };
    TurboModuleBinding::install(
        rt, cached ? oxyos::cachedTurboModules(std::move(provider), unavailable) : std::move(provider));

    auto run = [&rt](const char* code) {
      return rt.evaluateJavaScript(std::make_shared<jsi::StringBuffer>(code), "turbo-module-cache-test.js");
    };
    check(run("nativeModuleProxy.OxyOSTurboModuleProbe === nativeModuleProxy.OxyOSTurboModuleProbe").getBool(),
          "two lookups give the same module object");
    check(made == 1, "the module was made once for two lookups (made " + std::to_string(made) + ")");

    // A method kept, the module's object dropped, then many lookups, each object dropped too, and
    // modules the provider has none for.
    run("var add = nativeModuleProxy.OxyOSTurboModuleProbe.add;"
        "for (var i = 0; i < 10000; i++) { nativeModuleProxy.OxyOSTurboModuleProbe; nativeModuleProxy['Missing' + (i % 7)]; }");
    rt.instrumentation().collectGarbage("turbo-module-cache-test");
    check(made == 1, "10000 lookups more, the module is still the one made (made " + std::to_string(made) + ")");
    check(alive == 1, "after a full collection the module is alive (alive " + std::to_string(alive) + ")");
    jsi::Value sum = run("add(2)");
    check(sum.isNumber() && sum.getNumber() == 42, "the method kept still reads its module: add(2) is 42");
    check(run("nativeModuleProxy.OxyOSTurboModuleProbe.add(-40)").getNumber() == 0,
          "a lookup after the collection reaches the same module");
    check(run("nativeModuleProxy.Missing0 === null").getBool(), "a module the provider has none for is null");

    if (cached) {
      std::vector<std::string> missing;
      for (int i = 0; i < 7; ++i) missing.push_back("Missing" + std::to_string(i));
      std::vector<std::string> sorted = said;
      std::sort(sorted.begin(), sorted.end());
      check(sorted == missing, "10000 lookups of 7 missing names: each said once (" + std::to_string(said.size()) +
                                   " said)");
      check(!said.empty() && said.front() == "Missing0", "said in the order first asked");
      // The probes with a fallback, looked up as their packages do (TurboModuleRegistry.get, and
      // `NativeModules.X` through the same proxy), twice: nothing said, and each still null.
      said.clear();
      std::string probes;
      for (auto name : oxyos::kProbedWithFallback) {
        probes += "nativeModuleProxy['" + std::string(name) + "'] === null && ";
        probes += "nativeModuleProxy['" + std::string(name) + "'] === null && ";
      }
      check(run((probes + "true").c_str()).getBool(), "every probe with a fallback is null");
      check(said.empty(), "no probe with a fallback is said (" + std::to_string(said.size()) + " said)");
      // An application's call of a documented absent module is worth a line: said, once.
      run("nativeModuleProxy.ToastAndroid; nativeModuleProxy.ToastAndroid; nativeModuleProxy.Vibration;");
      check(said == std::vector<std::string>{"ToastAndroid", "Vibration"},
            "a module an application calls is said once each (" + std::to_string(said.size()) + " said)");
    }
  }
  check(alive == 0, "the module was destroyed with the runtime (alive " + std::to_string(alive) + ")");
  std::cout << (failures ? "turbo module cache: FAILED\n" : "turbo module cache: passed\n");
  return failures ? 1 : 0;
}
