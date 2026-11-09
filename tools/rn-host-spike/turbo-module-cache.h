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

#include <unistd.h>

#include <algorithm>
#include <array>
#include <functional>
#include <memory>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>

namespace oxyos {

// The names OxyOS has no module for that React Native's and Expo's packages look up as they load,
// each with a fallback of its own, so their absence is no news and is not said
// (docs/react-native-android-adaptation.mdx, "What OxyOS does not provide", which lists the same):
// Expo's optional modules (`requireOptionalNativeModule`, or `requireNativeModule` in a try, or
// `NativeModules.X` tested), the iOS names React Native's Linking and AccessibilityInfo look up
// beside the Android ones the host serves, React Native's own optional loggers, and
// async-storage's first two names before the one the host serves (`RNCAsyncStorage`).
inline constexpr std::array<std::string_view, 16> kProbedWithFallback = {
    // Expo's.
    "ExpoSplashScreen", "ExpoUpdates", "EXDevLauncher", "ExpoGo", "ExpoFontUtils", "ExpoObserve", "ExpoStatusBar",
    "ExpoBlur", "NativeUnimoduleProxy", "RNSModule",
    // React Native's own: its optional loggers, and the iOS names beside IntentAndroid and AccessibilityInfo.
    "SoundManager", "FrameRateLogger", "LinkingManager", "AccessibilityManager",
    // async-storage's, before RNCAsyncStorage.
    "PlatformLocalStorage", "RNC_AsyncSQLiteDBStorage"};

inline bool probedWithFallback(std::string_view name) {
  return std::find(kProbedWithFallback.begin(), kProbedWithFallback.end(), name) != kProbedWithFallback.end();
}

// What is said of a name the runtime has no module for: one line in the session's log, in one write
// (the session's other processes write to the same log while an application starts).
inline void reportUnavailable(const std::string& name) {
  std::string line = "[host] TurboModule unavailable: " + name + "\n";
  for (size_t done = 0; done < line.size();) {
    ssize_t n = ::write(STDERR_FILENO, line.data() + done, line.size() - done);
    if (n <= 0) break;
    done += static_cast<size_t>(n);
  }
}

// `provider`, made once per name: the first module it makes for a name is the one every later
// lookup of that name receives. A name it has no module for is asked again next time, as Android
// asks (a module can become available, the gesture handler's once the view tree exists), and is
// told to `unavailable` the first time only, unless it is one of `kProbedWithFallback`. Install
// what this returns into one runtime (TurboModuleBinding::install): the binding holds it, so the
// modules are destroyed with the runtime, and each name is said at most once per runtime.
inline facebook::react::TurboModuleProviderFunctionTypeWithRuntime cachedTurboModules(
    facebook::react::TurboModuleProviderFunctionTypeWithRuntime provider,
    std::function<void(const std::string&)> unavailable = reportUnavailable) {
  using Modules = std::unordered_map<std::string, std::shared_ptr<facebook::react::TurboModule>>;
  return [provider = std::move(provider), unavailable = std::move(unavailable),
          modules = std::make_shared<Modules>(), said = std::make_shared<std::unordered_set<std::string>>()](
             facebook::jsi::Runtime& runtime,
             const std::string& name) -> std::shared_ptr<facebook::react::TurboModule> {
    if (auto found = modules->find(name); found != modules->end()) return found->second;
    auto module = provider(runtime, name);
    if (module) {
      modules->emplace(name, module);
    } else if (!probedWithFallback(name) && said->insert(name).second && unavailable) {
      unavailable(name);
    }
    return module;
  };
}

}  // namespace oxyos
