#include "platform-modules.h"

#include "account-session.h"

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

  // A password or a code (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 8): marked
  // sensitive, so no clipboard manager keeps it, and cleared after 90 seconds if still there
  // (docs/adr/0007-clipboard.mdx, decision 6). The text is forgotten here once copied.
  jsi::Value setSensitiveString(jsi::Runtime& rt, std::string text) {
    ready(rt);
    if (!services_.clipboard.copySensitive) {
      forget(text);
      return resolved(rt, jsi::Value(false));
    }
    bool sent = !text.empty() && services_.clipboard.copySensitive(text);
    forget(text);
    return resolved(rt, jsi::Value(sent));
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

  // Read when JavaScript first asks, once the host has connected (whether it can ask about its
  // background items is the connection's).
  jsi::Object getConstants(jsi::Runtime& rt) {
    folly::dynamic constants = applicationConstants(services_.application);
    // Its login item and background activity can be asked about (protocol version 33).
    constants["backgroundItems"] = static_cast<bool>(services_.backgroundItems.ask);
    // The session asks it to quit before an end of the session (protocol version 38,
    // docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 5).
    constants["quitRequests"] = services_.quit.available;
    return jsi::valueFromDynamic(rt, constants).asObject(rt);
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

  // Login items and background activity (`application.loginItem`, `application.backgroundActivity`,
  // docs/adr/0060-login-items-and-background-activity.mdx): how one of its items stands, or
  // registering or unregistering it, by name (`loginItem`, `background`; `status`, `register`,
  // `unregister`): its serial, 0 if it was not sent; `OxyOSBackgroundItemState` answers.
  double backgroundItem(jsi::Runtime& rt, std::string kind, std::string action) {
    uint8_t kindCode = kind == "loginItem"    ? OXYOS_BACKGROUND_ITEM_LOGIN_ITEM
                       : kind == "background" ? OXYOS_BACKGROUND_ITEM_BACKGROUND
                                              : 0;
    uint8_t actionCode = action == "status"       ? OXYOS_BACKGROUND_ITEM_STATUS
                         : action == "register"   ? OXYOS_BACKGROUND_ITEM_REGISTER
                         : action == "unregister" ? OXYOS_BACKGROUND_ITEM_UNREGISTER
                                                  : 0;
    if (kindCode == 0 || actionCode == 0) {
      throw jsi::JSError(rt, "OxyOSApplication.backgroundItem: an item ('loginItem' or 'background') and an "
                             "action ('status', 'register' or 'unregister')");
    }
    if (!services_.backgroundItems.ask) return 0;
    return static_cast<double>(services_.backgroundItems.ask(kindCode, actionCode));
  }

  // Whether JavaScript answers the session's requests to quit (`application.onQuitRequest`,
  // docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx): while it does, `OxyOSQuitRequested`
  // ({serial, reason}) tells it; while it does not, the host answers quit at once.
  void handlesQuit(jsi::Runtime&, bool handles) {
    if (services_.quit.handles) services_.quit.handles(handles);
  }

  // Sudden Termination (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 10), as a
  // Mac's NSProcessInfo counter: held while the application has unsaved work, so the session asks it
  // to quit instead of ending it at once; each disable is matched by an enable.
  void disableSuddenTermination(jsi::Runtime&) {
    if (services_.quit.holdSuddenTermination) services_.quit.holdSuddenTermination(1);
  }
  void enableSuddenTermination(jsi::Runtime&) {
    if (services_.quit.holdSuddenTermination) services_.quit.holdSuddenTermination(-1);
  }

  // The application's answer to the request `serial`; whether it was sent. Answering quit ends the
  // host, as a close request does, background activity or not.
  bool replyToQuit(jsi::Runtime&, double serial, bool quit) {
    if (!services_.quit.reply || serial < 1 || serial > 4294967295.0) return false;
    return services_.quit.reply(static_cast<uint32_t>(serial), quit);
  }

  // The host emits its events whether or not anything listens: nothing to count.
  void addListener(jsi::Runtime&, std::string) {}
  void removeListeners(jsi::Runtime&, double) {}

 private:
  HostServices& services_;
};

}  // namespace

std::shared_ptr<TurboModule> createPlatformModule(
    const std::string& name,
    const std::shared_ptr<CallInvoker>& invoker,
    HostServices& services) {
  if (name == NativeOxyOSSoundsCxxSpec<SoundsModule>::kModuleName) {
    return std::make_shared<UiLocked<SoundsModule>>(invoker, services);
  }
  if (name == NativeOxyOSClipboardCxxSpec<ClipboardModule>::kModuleName) {
    return std::make_shared<UiLocked<ClipboardModule>>(invoker, services);
  }
  if (name == NativeOxyOSApplicationCxxSpec<ApplicationModule>::kModuleName) {
    return std::make_shared<UiLocked<ApplicationModule>>(invoker, services);
  }
  if (name == NativeOxyOSNotificationsCxxSpec<NotificationsModule>::kModuleName) {
    return std::make_shared<UiLocked<NotificationsModule>>(invoker, services);
  }
  return nullptr;
}

}  // namespace oxyos
