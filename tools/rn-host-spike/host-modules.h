// TurboModules the OxyOS host implements for React Native's start-up and Bloom.
#pragma once

#include "application.h"
#include "network.h"
#include "renderer.h"

#include <oxyos_client.h>

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
#include <vector>

namespace oxyos {

class Files;

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
  // At the login screen (docs/adr/0020-oxy-account-login.mdx) the password is for `account`, an
  // enrolled account's Unix user; empty, the session's own user.
  std::function<bool(const std::string& secret, const std::string& account)> authenticate;
  // Chooses the entry `index` of the commit `serial` of window `window`'s menus, as the shell was
  // told them (docs/adr/0021-menu-bar.mdx); the compositor delivers it only if they are still the
  // active window's.
  std::function<void(uint32_t window, uint32_t serial, uint32_t index)> activateMenuItem;
  // The Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx): a level by its
  // OXYOS_CONTROL_* (0 to 65536), a volume's mute, Wi-Fi on or off, the appearance light or dark,
  // and the system looked at again now. The session applies them and the state says what came of
  // it.
  std::function<void(uint8_t control, uint32_t level)> setControlLevel;
  std::function<void(uint8_t control, bool muted)> setControlMuted;
  std::function<void(bool enabled)> setWifiEnabled;
  std::function<void(bool dark)> setAppearance;
  std::function<void()> refreshControls;
  // Notifications (docs/adr/0026-notifications.mdx): the person pressed one (`action` 0) or one of
  // its actions, with the reply typed, citing the latest input; or removed one (0: all).
  std::function<void(uint32_t notification, uint8_t action, const std::string& reply)> activateNotification;
  std::function<void(uint32_t notification)> removeNotification;
  // Do Not Disturb on or off, from the Control Center: a change of the user's preference, as the
  // appearance is; the next NotificationPolicy says what came of it.
  std::function<void(bool enabled)> setDoNotDisturb;
  // What the shell says to assistive technologies: politely, after what they are saying (a
  // notification's banner), or at once.
  std::function<void(const std::string& text, bool polite)> announce;
  // Screenshots (docs/adr/0024-system-shortcuts.mdx, decision 10): a capture of what the person
  // chose in the screenshot interface, citing its serial (OXYOS_CAPTURE_SCREENS, _AREA with a
  // global logical rectangle of `output`, _WINDOW with the window's number; saved to
  // OXYOS_CAPTURE_DESKTOP or _DOCUMENTS), the interface ended without one, and a saved screenshot
  // opened in the application that opens it, citing the latest input. False if not sent.
  std::function<bool(uint64_t serial, uint8_t kind, uint32_t output, const OxyOSRect& rect, uint32_t window,
                     uint8_t destination)>
      capture;
  std::function<bool(uint64_t serial)> endCapture;
  std::function<bool(uint32_t screenshot)> openScreenshot;
};

// An application's notifications (docs/adr/0026-notifications.mdx), for expo-notifications' API
// (sample/oxyos/expo-adapter.js): a post as `serial` (its content as the adapter gives it:
// { identifier, title, subtitle, body, data, actions: [{ id, label, reply, background }], banner };
// false if it could not be sent; `OxyOSNotificationResult` answers), withdrawing one of its own
// (0: all), and the responses the session gave that JavaScript has not taken yet (each once;
// `OxyOSNotificationResponse` says one came). Unset without a connection to the compositor.
struct NotificationsService {
  std::function<bool(uint32_t serial, const folly::dynamic& content)> post;
  std::function<void(uint32_t notification)> withdraw;
  std::vector<folly::dynamic> responses;
};

// The application window's menus (docs/adr/0021-menu-bar.mdx), for Bloom's `menu-bar` capability:
// entries in order (OXYOS_MENU_* kinds and flags, a parent's index or OXYOS_NO_MENU_PARENT), sent as
// a new commit whose serial is returned (0: not sent). `OxyOSMenuActivated` ({ serial, index })
// says what the person chose in the shell's bar. Unset without an application window.
struct MenuEntry {
  uint32_t parent = OXYOS_NO_MENU_PARENT;
  uint8_t kind = 0, flags = 0;
  std::string label, shortcut;
};
struct MenuBarService {
  std::function<uint32_t(const std::vector<MenuEntry>& entries)> setMenus;
};

// The application's own window (docs/adr/0016-windows.mdx), for Bloom's `window` capability:
// its state ({ active, maximized, fullscreen }) and what its controls and drag regions do. Unset
// in the shell, which has no window of its own.
struct WindowService {
  std::function<folly::dynamic()> state;
  // The application asks to close its window: as the shell's close request, it finishes.
  std::function<void()> close;
  std::function<void()> minimize;
  // Zooms the window, or restores it if it is zoomed.
  std::function<void()> zoom;
  // Moves the window with the pointer from the press being handled (false if there is none).
  std::function<bool()> startMove;
  std::function<void(uint32_t minWidth, uint32_t minHeight, uint32_t maxWidth, uint32_t maxHeight)> setSizeLimits;
};

// The session's preferences (docs/adr/0019-settings.mdx): those every client draws by (`drawing`:
// dark, reduceMotion, reduceTransparency, accent), and for Settings alone every preference as
// kept (`values`, null without the grant) and a change (`set`: the request's serial, 0 if it could
// not be sent or this client is not Settings). Unset without a connection to the compositor.
struct SettingsService {
  std::function<folly::dynamic()> drawing;
  std::function<folly::dynamic()> values;
  std::function<uint32_t(const std::string& key, const std::string& value)> set;
  // The displays (docs/adr/0018-several-displays.mdx), for Settings alone (null without the
  // grant): `{displays: [...], trial: {output, seconds} | null}`; and a change of one
  // (`display`: "mode", "scale", "origin", "primary" or "confirm" with its numbers), the request's
  // serial, 0 if it could not be sent or this client is not Settings.
  std::function<folly::dynamic()> displays;
  std::function<uint32_t(const std::string& change, const std::vector<double>& numbers)> display;
};

// The window's key presses for Bloom's `PlatformKeyboard` (key-chords.h): JavaScript's one
// listener, called with each press before the focused view, which returns whether it took the key
// (a menu's shortcut ran). Set by JavaScript in an application's window; never in the shell.
struct KeyboardService {
  jsi::Runtime* runtime = nullptr;
  std::shared_ptr<jsi::Function> listener;
};

// Installing applications (docs/adr/0030-installing-applications.mdx): a request of the session's
// vocabulary (its serial, 0 if it was not sent), the shell's answer to a sheet, a capability asked
// at first use (its serial), what this launch may use, and whether it may use a capability now.
struct SystemService {
  std::function<uint32_t(const std::string& text)> request;
  std::function<void(uint32_t sheet, bool accepted)> answerSheet;
  std::function<uint32_t(const std::string& capability)> requestCapability;
  std::function<folly::dynamic()> launch;
  std::function<bool(const std::string& capability)> allows;
};

struct HostServices {
  BlobStore& blobs;
  Networking& networking;
  ClipboardService clipboard{};
  ShellService shell{};
  // Whether the host runs an application in a window of its own (not the shell, not offscreen):
  // known before JavaScript starts, so Bloom's adapter reports the `window` capability from it.
  bool applicationWindow = false;
  // Whether it runs the session's shell, whose surfaces' keys are offered to JavaScript too
  // (Launchpad's arrows and Return, docs/adr/0009-session-shell.mdx).
  bool shellWindow = false;
  WindowService window{};
  SettingsService settings{};
  NotificationsService notifications{};
  SystemService system{};
  MenuBarService menuBar{};
  KeyboardService keyboard{};
  // The user's files (files.h, docs/adr/0028-file-access.mdx): set for an application's window,
  // never for the shell or the login screen, whose expo-file-system and Bloom adapter have none.
  Files* files = nullptr;
  // AccessibilityInfo.announceForAccessibility (ADR 0003): to assistive technologies, from the
  // window; unset without one.
  std::function<void(const std::string& text)> announce;
  // The application it runs (application.h): its id, module, directories and embedded app config.
  Application application{};
  // Its AsyncStorage, made when it is first asked for (in its data directory), and the machine's
  // connectivity, watched once NetInfo asks; the host's loop waits on their descriptors and
  // delivers what they answer.
  std::unique_ptr<AsyncStorage> storage;
  std::unique_ptr<Connectivity> connectivity;
  // Takes focus from the focused text input, if any (react-native-keyboard-controller's dismiss);
  // unset without a window.
  std::function<void()> blurFocusedInput;
  // Sends an event to RCTDeviceEventEmitter, from the JavaScript thread.
  std::function<void(const std::string& event, folly::dynamic body)> emit;
  // The runtime this host is (docs/adr/0030-installing-applications.mdx, decisions 2 and 17): its
  // numbers, and where its framework is known from (its assets are beside it), `OxyOSRuntime`'s.
  struct Runtime {
    int abi = 0;
    int api = 0;
    std::string frameworkURL;
  } runtime{};
};

// React Native's Dimensions for a window of `display`'s size and scale: DeviceInfo's constant at
// start, and `didUpdateDimensions`' payload each time it changes, as Android's DeviceInfoModule
// emits it on a configuration change.
folly::dynamic deviceDimensions(const Display& display);

std::shared_ptr<facebook::react::TurboModule> createHostModule(
    const std::string& name,
    const std::shared_ptr<facebook::react::CallInvoker>& invoker,
    const Display& display,
    HostServices& services);

}  // namespace oxyos
