// turbo-module-cache.h's test: a runtime's TurboModules are one instance per name, and a method
// taken from a module keeps working after JavaScript has dropped the module's object, through a
// full collection and thousands of lookups. The module is a C++ TurboModule as React Native
// generates them (a method map whose host functions hold the module by `this`), so a module
// destroyed under its method is a use after free, which a build with AddressSanitizer reports
// (docs/bloom-host-sanitizers.mdx); --uncached runs the same without the cache, to see it.
#include "turbo-module-cache.h"

#include <ReactCommon/TurboModuleBinding.h>
#include <hermes/hermes.h>
#include <jsi/instrumentation.h>

#include <cstring>
#include <iostream>
#include <memory>
#include <string>

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
    TurboModuleBinding::install(rt, cached ? oxyos::cachedTurboModules(std::move(provider)) : std::move(provider));

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
  }
  check(alive == 0, "the module was destroyed with the runtime (alive " + std::to_string(alive) + ")");
  std::cout << (failures ? "turbo module cache: FAILED\n" : "turbo module cache: passed\n");
  return failures ? 1 : 0;
}
