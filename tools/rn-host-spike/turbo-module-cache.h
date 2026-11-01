// The runtime's TurboModules, one instance per name, kept for the runtime's life, as React Native's
// platforms keep them (Android's TurboModuleManager and iOS's RCTTurboModuleManager cache every
// module they make, and the cache goes with the instance).
//
// It is what makes a module's methods safe to keep. React Native's C++ modules make each method a
// host function that holds the module by a plain pointer (TurboModule::create captures `this`),
// and only the module's JavaScript object holds the module itself: made anew for each lookup, a
// module whose object JavaScript dropped was destroyed at the next collection while a method taken
// from it (`const {queueMicrotask} = NativeMicrotasks`) still pointed into it.
#pragma once

#include <ReactCommon/TurboModule.h>

#include <memory>
#include <string>
#include <unordered_map>

namespace oxyos {

// `provider`, made once per name: the first module it makes for a name is the one every later
// lookup of that name receives. A name it has no module for is asked again next time, as Android
// asks (a module can become available, the gesture handler's once the view tree exists). Install
// what this returns into one runtime (TurboModuleBinding::install): the binding holds it, so the
// modules are destroyed with the runtime.
inline facebook::react::TurboModuleProviderFunctionTypeWithRuntime cachedTurboModules(
    facebook::react::TurboModuleProviderFunctionTypeWithRuntime provider) {
  using Modules = std::unordered_map<std::string, std::shared_ptr<facebook::react::TurboModule>>;
  return [provider = std::move(provider), modules = std::make_shared<Modules>()](
             facebook::jsi::Runtime& runtime,
             const std::string& name) -> std::shared_ptr<facebook::react::TurboModule> {
    if (auto found = modules->find(name); found != modules->end()) return found->second;
    auto module = provider(runtime, name);
    if (module) modules->emplace(name, module);
    return module;
  };
}

}  // namespace oxyos
