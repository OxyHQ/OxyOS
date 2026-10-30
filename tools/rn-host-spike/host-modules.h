// TurboModules the OxyOS host implements for React Native's start-up and Bloom.
#pragma once

#include "network.h"
#include "renderer.h"

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>

#include <jsi/JSIDynamic.h>
#include <folly/dynamic.h>

#include <functional>
#include <iostream>
#include <map>
#include <memory>
#include <set>
#include <string>

namespace oxyos {

using facebook::react::CallInvoker;
using facebook::react::TurboModule;
namespace jsi = facebook::jsi;

// A TurboModule whose methods are host lambdas.
class HostModule : public TurboModule {
 public:
  using Method = std::function<jsi::Value(jsi::Runtime&, const jsi::Value*, size_t)>;

  HostModule(const std::string& name, std::shared_ptr<CallInvoker> invoker)
      : facebook::react::TurboModule(name, std::move(invoker)), name_(name) {}

  HostModule& method(const std::string& name, Method fn) {
    methods_[name] = std::move(fn);
    return *this;
  }

  HostModule& constants(folly::dynamic value) {
    return method("getConstants", [value](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return jsi::valueFromDynamic(rt, value);
    });
  }

  // Listener bookkeeping for NativeEventEmitter: the host emits its events whether or not anything
  // listens, so there is nothing to count.
  HostModule& listeners() {
    method("addListener", [](jsi::Runtime&, const jsi::Value*, size_t) { return jsi::Value::undefined(); });
    return method("removeListeners", [](jsi::Runtime&, const jsi::Value*, size_t) { return jsi::Value::undefined(); });
  }

  // Calls that OxyOS has no counterpart for: accepted, reported once, no effect.
  HostModule& unsupported(std::initializer_list<const char*> names) {
    for (const char* method : names) {
      std::string qualified = name_ + "." + method;
      this->method(method, [qualified](jsi::Runtime&, const jsi::Value*, size_t) {
        static std::set<std::string> reported;
        if (reported.insert(qualified).second) std::cerr << "[host] unsupported on OxyOS: " << qualified << "\n";
        return jsi::Value::undefined();
      });
    }
    return *this;
  }

  jsi::Value create(jsi::Runtime& rt, const jsi::PropNameID& prop) override {
    auto it = methods_.find(prop.utf8(rt));
    if (it == methods_.end()) return jsi::Value::undefined();
    auto fn = it->second;
    return jsi::Function::createFromHostFunction(
        rt, prop, 0, [fn](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
          return fn(rt, args, count);
        });
  }

 private:
  std::string name_;
  std::map<std::string, Method> methods_;
};


// What modules share with the host: blobs, and the network.
// The clipboard for JavaScript (expo-clipboard), with the serial of the latest key or press the
// app received (ADR 0007): set by the host once its window exists; unset without one.
struct ClipboardService {
  std::function<bool(const std::string& text)> copy;
  std::function<void(std::function<void(std::string)> done)> paste;
  std::function<bool()> hasText;
};

// The session's shell (ADR 0009): what it knows of windows and workspaces, and what it asks.
struct ShellService {
  std::function<folly::dynamic()> state;
  std::function<void(uint8_t action, uint32_t window, uint32_t workspace)> manage;
  // The lock (docs/adr/0010-lock-screen.mdx): locking the session, and the password typed in the
  // lock screen, which the host sends from its lock surface (false if there is none, or it could
  // not be sent). `OxyOSShellAuthentication` answers.
  std::function<void()> lock;
  // Asks the session for an application of its registry (ADR 0013): the request's serial, 0 if it
  // could not be sent. `OxyOSShellLaunch` answers.
  std::function<uint32_t(const std::string& id)> launch;
  std::function<bool(const std::string& secret)> authenticate;
};

struct HostServices {
  BlobStore& blobs;
  Networking& networking;
  ClipboardService clipboard{};
  ShellService shell{};
  // AccessibilityInfo.announceForAccessibility (ADR 0003): to assistive technologies, from the
  // window; unset without one.
  std::function<void(const std::string& text)> announce;
};

std::shared_ptr<facebook::react::TurboModule> createHostModule(
    const std::string& name,
    const std::shared_ptr<facebook::react::CallInvoker>& invoker,
    const Display& display,
    HostServices& services);

}  // namespace oxyos
