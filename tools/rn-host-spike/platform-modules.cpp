#include "platform-modules.h"

#include <OxyOSPlatformSpecJSI.h>

#include <utility>

namespace oxyos {

namespace {

using facebook::react::NativeOxyOSApplicationCxxSpec;
using facebook::react::NativeOxyOSClipboardCxxSpec;
using facebook::react::NativeOxyOSNotificationsCxxSpec;
using facebook::react::NativeOxyOSSoundsCxxSpec;

// The system's interface sounds, behind `sounds` (docs/adr/0044-system-sounds.mdx): an application
// asks by meaning, the session plays its own, citing the latest input the application received.
class SoundsModule : public NativeOxyOSSoundsCxxSpec<SoundsModule> {
 public:
  SoundsModule(std::shared_ptr<CallInvoker> invoker, HostServices& services)
      : NativeOxyOSSoundsCxxSpec(std::move(invoker)), services_(services) {}

  jsi::Object getConstants(jsi::Runtime& rt) {
    jsi::Object constants(rt);
    constants.setProperty(rt, "available", services_.soundsAvailable);
    return constants;
  }

  bool play(jsi::Runtime&, std::string name) {
    return services_.playSystemSound && services_.playSystemSound(name);
  }

 private:
  HostServices& services_;
};

// The clipboard, behind `pasteboard` (docs/adr/0007-clipboard.mdx), with the serial of the latest
// key or press the application received: the compositor refuses it unless the application has the
// keyboard focus and that input is one of its latest 16. Every call throws at once where the
// application has no window. `OxyOSClipboardChanged` says another application copied.
class ClipboardModule : public NativeOxyOSClipboardCxxSpec<ClipboardModule> {
 public:
  ClipboardModule(std::shared_ptr<CallInvoker> invoker, HostServices& services)
      : NativeOxyOSClipboardCxxSpec(std::move(invoker)), services_(services) {}

  jsi::Value hasString(jsi::Runtime& rt) {
    ready(rt);
    return resolved(rt, jsi::Value(services_.clipboard.hasText()));
  }

  jsi::Value setString(jsi::Runtime& rt, std::string text) {
    ready(rt);
    return resolved(rt, jsi::Value(services_.clipboard.copy(text)));
  }

  // The text arrives later, from the clipboard's owner: the promise settles on the JavaScript
  // thread, through the module's invoker. A refused paste reads nothing: "".
  jsi::Value getString(jsi::Runtime& rt) {
    ready(rt);
    auto start = jsi::Function::createFromHostFunction(
        rt, jsi::PropNameID::forAscii(rt, "paste"), 2,
        [&services = services_, invoker = jsInvoker_](
            jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t) {
          auto resolve = std::make_shared<jsi::Value>(rt, args[0]);
          services.clipboard.paste([invoker, resolve](std::string pasted) {
            invoker->invokeAsync([resolve, pasted = std::move(pasted)](jsi::Runtime& rt) {
              resolve->asObject(rt).asFunction(rt).call(rt, jsi::String::createFromUtf8(rt, pasted));
            });
          });
          return jsi::Value::undefined();
        });
    return rt.global().getPropertyAsFunction(rt, "Promise").callAsConstructor(rt, start);
  }

  // The host emits its events whether or not anything listens: nothing to count.
  void addListener(jsi::Runtime&, std::string) {}
  void removeListeners(jsi::Runtime&, double) {}

 private:
  void ready(jsi::Runtime& rt) const {
    if (!services_.clipboard.copy) throw jsi::JSError(rt, "OxyOSClipboard: the clipboard needs a window on OxyOS");
  }

  HostServices& services_;
};

// An application's notifications, behind `notifications` (docs/adr/0026-notifications.mdx):
// posting, withdrawing, and the responses the session gave, each taken once. A post's outcome
// arrives as `OxyOSNotificationResult`, naming its serial.
class NotificationsModule : public NativeOxyOSNotificationsCxxSpec<NotificationsModule> {
 public:
  NotificationsModule(std::shared_ptr<CallInvoker> invoker, HostServices& services)
      : NativeOxyOSNotificationsCxxSpec(std::move(invoker)), services_(services) {}

  jsi::String post(jsi::Runtime& rt, double serial, jsi::Object notification) {
    ready(rt);
    // An installed version posts only once its account granted it (ADR 0030, decision 8).
    if (services_.system.allows && !services_.system.allows("notifications")) {
      return jsi::String::createFromAscii(rt, "notPermitted");
    }
    bool sent = services_.notifications.post(static_cast<uint32_t>(serial),
                                             jsi::dynamicFromValue(rt, jsi::Value(rt, notification)));
    return jsi::String::createFromAscii(rt, sent ? "sent" : "notSent");
  }

  void withdraw(jsi::Runtime& rt, double notification) {
    ready(rt);
    services_.notifications.withdraw(static_cast<uint32_t>(notification));
  }

  // Each response once: taken, it is JavaScript's.
  jsi::Array takeResponses(jsi::Runtime& rt) {
    folly::dynamic taken = folly::dynamic::array();
    for (auto& response : services_.notifications.responses) taken.push_back(std::move(response));
    services_.notifications.responses.clear();
    return jsi::valueFromDynamic(rt, taken).asObject(rt).asArray(rt);
  }

  // The host emits its events whether or not anything listens: nothing to count.
  void addListener(jsi::Runtime&, std::string) {}
  void removeListeners(jsi::Runtime&, double) {}

 private:
  void ready(jsi::Runtime& rt) const {
    if (!services_.notifications.post) {
      throw jsi::JSError(rt, "OxyOSNotifications: notifications need a connection to the compositor on OxyOS");
    }
  }

  HostServices& services_;
};

// The application this host runs, behind `application` (docs/adr/0058-the-platform-api.mdx): what
// it is (applicationConstants), and a URL opened with the application whose entry declares its
// scheme, as a Mac's `NSWorkspace.open`: asked of the session from its window, citing the person's
// latest press or key, the answer `{opened}` or `{refused: {reason, message}}`, never a rejection.
class ApplicationModule : public NativeOxyOSApplicationCxxSpec<ApplicationModule> {
 public:
  ApplicationModule(std::shared_ptr<CallInvoker> invoker, HostServices& services)
      : NativeOxyOSApplicationCxxSpec(std::move(invoker)), services_(services) {}

  jsi::Object getConstants(jsi::Runtime& rt) {
    return jsi::valueFromDynamic(rt, applicationConstants(services_.application)).asObject(rt);
  }

  jsi::Value open(jsi::Runtime& rt, std::string url) {
    auto answer = [](const std::string& refusal, const std::string& message) -> folly::dynamic {
      if (refusal.empty()) return folly::dynamic::object("opened", true)("refused", nullptr);
      return folly::dynamic::object("opened", false)("refused",
                                                     folly::dynamic::object("reason", refusal)("message", message));
    };
    if (!services_.openUrl) {
      auto none = answer("unavailable", "OxyOS opens a URL only from an application's window");
      return resolved(rt, jsi::valueFromDynamic(rt, none));
    }
    auto start = jsi::Function::createFromHostFunction(
        rt, jsi::PropNameID::forAscii(rt, "open"), 2,
        [&services = services_, invoker = jsInvoker_, url, answer](jsi::Runtime& rt, const jsi::Value&,
                                                                   const jsi::Value* args, size_t) {
          auto resolve = std::make_shared<jsi::Value>(rt, args[0]);
          services.openUrl(url, [invoker, resolve, answer](const std::string& refusal, const std::string& message) {
            invoker->invokeAsync([resolve, told = answer(refusal, message)](jsi::Runtime& rt) {
              resolve->asObject(rt).asFunction(rt).call(rt, jsi::valueFromDynamic(rt, told));
            });
          });
          return jsi::Value::undefined();
        });
    return rt.global().getPropertyAsFunction(rt, "Promise").callAsConstructor(rt, start);
  }

 private:
  HostServices& services_;
};

}  // namespace

std::shared_ptr<TurboModule> createPlatformModule(
    const std::string& name,
    const std::shared_ptr<CallInvoker>& invoker,
    HostServices& services) {
  if (name == NativeOxyOSSoundsCxxSpec<SoundsModule>::kModuleName) {
    return std::make_shared<SoundsModule>(invoker, services);
  }
  if (name == NativeOxyOSClipboardCxxSpec<ClipboardModule>::kModuleName) {
    return std::make_shared<ClipboardModule>(invoker, services);
  }
  if (name == NativeOxyOSApplicationCxxSpec<ApplicationModule>::kModuleName) {
    return std::make_shared<ApplicationModule>(invoker, services);
  }
  if (name == NativeOxyOSNotificationsCxxSpec<NotificationsModule>::kModuleName) {
    return std::make_shared<NotificationsModule>(invoker, services);
  }
  return nullptr;
}

}  // namespace oxyos
