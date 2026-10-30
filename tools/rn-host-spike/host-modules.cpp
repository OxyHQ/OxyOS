#include "host-modules.h"

#include "images.h"
#include "text.h"

#include <folly/dynamic.h>
#include <oxyos_client.h>
#include <jsi/JSIDynamic.h>

#include <algorithm>
#include <cctype>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>
#include <optional>
#include <random>
#include <set>
#include <tuple>

using namespace facebook;
using namespace facebook::react;

namespace oxyos {

namespace {

jsi::Value call(jsi::Runtime& rt, const jsi::Value& callback, const jsi::Value& argument) {
  if (callback.isObject() && callback.asObject(rt).isFunction(rt)) {
    callback.asObject(rt).asFunction(rt).call(rt, argument);
  }
  return jsi::Value::undefined();
}

// A promise resolved with `value`.
jsi::Value resolved(jsi::Runtime& rt, const jsi::Value& value) {
  return rt.global().getPropertyAsObject(rt, "Promise").getPropertyAsFunction(rt, "resolve").call(rt, value);
}

// A promise rejected with an Error carrying `message`.
jsi::Value rejected(jsi::Runtime& rt, const std::string& message) {
  auto error = rt.global().getPropertyAsFunction(rt, "Error").callAsConstructor(rt, message);
  return rt.global().getPropertyAsObject(rt, "Promise").getPropertyAsFunction(rt, "reject").call(rt, error);
}

// The first line of a file, or "" (a file that is not there says nothing).
std::string firstLine(const char* path) {
  std::ifstream in(path);
  std::string line;
  std::getline(in, line);
  return line;
}

// A `KEY=value` line's value in a shell-style file (os-release, /etc/default/keyboard), unquoted.
std::string assignment(const char* path, const std::string& key) {
  std::ifstream in(path);
  for (std::string line; std::getline(in, line);) {
    if (line.rfind(key + "=", 0) != 0) continue;
    std::string value = line.substr(key.size() + 1);
    if (value.size() >= 2 && (value.front() == '"' || value.front() == '\'') && value.back() == value.front()) {
      value = value.substr(1, value.size() - 2);
    }
    return value;
  }
  return "";
}

// What OxyOS's Settings shows of the machine (apps/settings, docs/adr/0019-settings.mdx), read
// from where the system keeps it: the computer's name, os-release's PRETTY_NAME, the processor's
// model and the memory (/proc), the time zone (/etc/localtime's zone), whether systemd-timesyncd
// has set the clock, and the keyboard's layout (/etc/default/keyboard). Read only: the system's
// services change them (ADR 0019 decision 2). The display is the session's, not the host's: "".
folly::dynamic systemFacts() {
  std::string processor;
  {
    std::ifstream in("/proc/cpuinfo");
    for (std::string line; std::getline(in, line);) {
      if (line.rfind("model name", 0) == 0) {
        auto colon = line.find(':');
        if (colon != std::string::npos) processor = line.substr(line.find_first_not_of(' ', colon + 1));
        break;
      }
    }
  }
  double memory = 0;
  {
    std::ifstream in("/proc/meminfo");
    std::string key;
    long kib = 0;
    if (in >> key >> kib && key == "MemTotal:") memory = static_cast<double>(kib) * 1024;
  }
  std::string zone;
  {
    std::error_code error;
    auto target = std::filesystem::read_symlink("/etc/localtime", error).string();
    auto at = target.find("zoneinfo/");
    if (!error && at != std::string::npos) zone = target.substr(at + 9);
  }
  std::string layout = assignment("/etc/default/keyboard", "XKBLAYOUT");
  std::string variant = assignment("/etc/default/keyboard", "XKBVARIANT");
  if (!variant.empty()) layout += " (" + variant + ")";
  std::error_code error;
  bool synchronized = std::filesystem::exists("/run/systemd/timesync/synchronized", error);
  return folly::dynamic::object("computerName", firstLine("/etc/hostname"))(
      "version", assignment("/etc/os-release", "PRETTY_NAME"))("processor", processor)("memoryBytes", memory)(
      "display", "")("timeZone", zone)("automaticTime", synchronized)("keyboardLayout", layout);
}

std::string text(jsi::Runtime& rt, const jsi::Value& value) {
  return value.isString() ? value.asString(rt).utf8(rt) : value.isUndefined() ? "" : value.toString(rt).utf8(rt);
}

// The bytes a blob descriptor ({blobId, offset, size}) names.
std::optional<std::string> blobBytes(const BlobStore& blobs, const folly::dynamic& blob) {
  if (!blob.isObject() || !blob.count("blobId")) return std::nullopt;
  size_t offset = blob.count("offset") && blob["offset"].isNumber() ? static_cast<size_t>(blob["offset"].asDouble()) : 0;
  size_t size = blob.count("size") && blob["size"].isNumber() ? static_cast<size_t>(blob["size"].asDouble())
                                                               : std::string::npos;
  return blobs.slice(blob["blobId"].asString(), offset, size);
}

// A request's body from React Native's description of it (convertRequestBody), or the reason it
// cannot be sent. Multipart parts that are files are not supported on OxyOS yet.
struct Body {
  bool present = false;
  std::string bytes;
  std::string contentType;  // set for multipart bodies
  std::string error;
};

// A multipart boundary no part is likely to contain: random, as OkHttp's.
std::string boundary() {
  static std::mt19937_64 random{std::random_device{}()};
  static constexpr char kHex[] = "0123456789abcdef";
  std::string boundary = "OxyOSFormBoundary";
  for (int i = 0; i < 32; ++i) boundary += kHex[random() & 15];
  return boundary;
}

Body requestBody(const BlobStore& blobs, const folly::dynamic& data) {
  Body body;
  if (!data.isObject()) return body;
  if (data.count("string") && data["string"].isString()) {
    body.present = true;
    body.bytes = data["string"].asString();
  } else if (data.count("base64") && data["base64"].isString()) {
    auto bytes = base64Decode(data["base64"].asString());
    if (!bytes) {
      body.error = "the request body is not valid base64";
    } else {
      body.present = true;
      body.bytes = *bytes;
    }
  } else if (data.count("blob")) {
    auto bytes = blobBytes(blobs, data["blob"]);
    if (!bytes) {
      body.error = "the request body's blob does not exist";
    } else {
      body.present = true;
      body.bytes = *bytes;
    }
  } else if (data.count("formData") && data["formData"].isArray()) {
    std::string boundary = oxyos::boundary();
    for (const auto& part : data["formData"]) {
      if (!part.count("string")) {
        body.error = "file parts in multipart bodies are not supported on OxyOS yet";
        return body;
      }
      body.bytes += "--" + boundary + "\r\n";
      if (part.count("headers") && part["headers"].isArray()) {
        for (const auto& header : part["headers"]) {
          if (header.isArray() && header.size() == 2) {
            body.bytes += header[0].asString() + ": " + header[1].asString() + "\r\n";
          }
        }
      }
      body.bytes += "\r\n" + part["string"].asString() + "\r\n";
    }
    body.bytes += "--" + boundary + "--\r\n";
    body.present = true;
    body.contentType = "multipart/form-data; boundary=" + boundary;
  } else if (data.count("uri")) {
    body.error = "request bodies from a URI are not supported on OxyOS yet";
  }
  return body;
}

}  // namespace

std::shared_ptr<TurboModule> createHostModule(
    const std::string& name,
    const std::shared_ptr<CallInvoker>& invoker,
    const Display& display,
    HostServices& services) {
  auto module = std::make_shared<HostModule>(name, invoker);
  folly::dynamic window = folly::dynamic::object("width", display.width)("height", display.height)(
      "scale", display.scale)("fontScale", 1.0);

  if (name == "PlatformConstants") {
    // React Native's Android spec, answered for OxyOS. `Version` is not a device's Android API
    // level (OxyOS is not Android): it is the level whose React Native code paths the host follows,
    // React Native 0.88's target SDK, which core compares only to trust keyboard events (OxyOS has
    // no software keyboard and sends none). See docs/react-native-android-adaptation.mdx.
    module->constants(folly::dynamic::object("isTesting", false)("isDisableAnimations", false)(
        "reactNativeVersion", folly::dynamic::object("major", 0)("minor", 88)("patch", 0)("prerelease", nullptr))(
        "Version", 36)("Release", "OxyOS")("Serial", "")("Fingerprint", "")("Model", "OxyOS")("uiMode", "normal")(
        "Brand", "Oxy")("Manufacturer", "Oxy"));
    // There is no Android ID to give: callers learn that rather than receive a fake one.
    module->method("getAndroidID", [](jsi::Runtime& rt, const jsi::Value*, size_t) -> jsi::Value {
      throw jsi::JSError(rt, "OxyOS has no Android ID");
    });
  } else if (name == "DeviceInfo") {
    module->constants(folly::dynamic::object("Dimensions", folly::dynamic::object("window", window)("screen", window)));
  } else if (name == "SourceCode") {
    const char* url = std::getenv("OXYOS_SCRIPT_URL");
    module->constants(folly::dynamic::object("scriptURL", url ? url : ""));
  } else if (name == "I18nManager") {
    module->constants(folly::dynamic::object("isRTL", false)("doLeftAndRightSwapInRTL", true)(
        "localeIdentifier", "en-US"));
    module->unsupported({"allowRTL", "forceRTL", "swapLeftAndRightInRTL"});
  } else if (name == "AppState") {
    module->constants(folly::dynamic::object("initialAppState", "active"));
    module->method("getCurrentAppState", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      jsi::Object state(rt);
      state.setProperty(rt, "app_state", "active");
      return count > 0 ? call(rt, args[0], jsi::Value(rt, state)) : jsi::Value::undefined();
    });
    module->listeners();
  } else if (name == "Appearance") {
    // The session's appearance (docs/adr/0019-settings.mdx), an automatic one resolved by the
    // session; `appearanceChanged` when it changes, as Android's configuration change tells it.
    module->method("getColorScheme", [&services](jsi::Runtime& rt, const jsi::Value*, size_t) {
      bool dark = services.settings.drawing && services.settings.drawing()["dark"].asBool();
      return jsi::String::createFromAscii(rt, dark ? "dark" : "light");
    });
    // The application's own choice stays its own (Bloom's theme), as on Android an app's night mode
    // override does not change the system's.
    module->unsupported({"setColorScheme"});
    module->listeners();
  } else if (name == "AccessibilityInfo") {
    // Reduce motion is the session's preference (docs/adr/0019-settings.mdx), told with
    // `reduceMotionDidChange` when it changes, as Android tells its animator duration scale.
    module->method("isReduceMotionEnabled", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      bool reduced = services.settings.drawing && services.settings.drawing()["reduceMotion"].asBool();
      return count > 0 ? call(rt, args[0], jsi::Value(reduced)) : jsi::Value::undefined();
    });
    // The other assistive-technology and display settings do not exist on OxyOS yet: off.
    for (const char* query : {"isTouchExplorationEnabled", "isAccessibilityServiceEnabled",
                              "isHighTextContrastEnabled", "isGrayscaleEnabled", "isInvertColorsEnabled",
                              "isDarkerSystemColorsEnabled"}) {
      module->method(query, [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
        return count > 0 ? call(rt, args[0], jsi::Value(false)) : jsi::Value::undefined();
      });
    }
    // No accessibility timeout preference exists on OxyOS, so the recommended timeout is the one
    // asked about, as on Android without a user preference.
    module->method("getRecommendedTimeoutMillis", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      return count > 1 ? call(rt, args[1], jsi::Value(args[0].isNumber() ? args[0].getNumber() : 0))
                       : jsi::Value::undefined();
    });
    // An announcement reaches assistive technologies through the compositor, as Android's
    // reaches TalkBack; without a window there is no one to tell.
    module->method("announceForAccessibility", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count > 0 && args[0].isString() && services.announce) services.announce(args[0].asString(rt).utf8(rt));
      return jsi::Value::undefined();
    });
    module->unsupported({"setAccessibilityFocus"});
  } else if (name == "ExceptionsManager") {
    auto report = [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count > 0 && args[0].isObject()) {
        auto data = args[0].asObject(rt);
        std::cerr << "[js exception] " << text(rt, data.getProperty(rt, "message")) << "\n";
        auto stack = data.getProperty(rt, "stack");
        if (stack.isObject() && stack.asObject(rt).isArray(rt)) {
          auto frames = stack.asObject(rt).asArray(rt);
          for (size_t i = 0; i < std::min<size_t>(frames.size(rt), 12); ++i) {
            auto frame = frames.getValueAtIndex(rt, i);
            if (!frame.isObject()) continue;
            auto f = frame.asObject(rt);
            std::cerr << "    at " << text(rt, f.getProperty(rt, "methodName")) << " ("
                      << text(rt, f.getProperty(rt, "file")) << ":" << text(rt, f.getProperty(rt, "lineNumber"))
                      << ")\n";
          }
        }
      } else if (count > 0) {
        std::cerr << "[js exception] " << text(rt, args[0]) << "\n";
      }
      return jsi::Value::undefined();
    };
    module->method("reportException", report);
    module->method("reportFatalException", report);
    module->method("reportSoftException", report);
    module->unsupported({"updateExceptionMessage", "dismissRedbox"});
  } else if (name == "DeviceEventManager") {
    module->unsupported({"invokeDefaultBackPressHandler"});
  } else if (name == "StatusBarManager") {
    module->constants(folly::dynamic::object("HEIGHT", 0)("DEFAULT_BACKGROUND_COLOR", 0));
    module->unsupported({"setColor", "setTranslucent", "setStyle", "setHidden"});
  } else if (name == "KeyboardObserver") {
    module->listeners();
  } else if (name == "OxyOSShell") {
    // The session's shell (sample/oxyos/shell-state.js): the compositor's windows and workspaces,
    // and the shell's actions on them (docs/adr/0009-session-shell.mdx). Only a host run as the
    // shell (`--shell`, with the shell grant) has them.
    auto ready = [&services](jsi::Runtime& rt) {
      if (!services.shell.manage) throw jsi::JSError(rt, "OxyOSShell: only the session's shell has it");
    };
    module->method("getState", [&services, ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      return jsi::valueFromDynamic(rt, services.shell.state());
    });
    auto number = [](jsi::Runtime& rt, const jsi::Value* args, size_t count, size_t at) {
      return count > at && args[at].isNumber() ? static_cast<uint32_t>(args[at].asNumber()) : 0u;
    };
    for (auto [method, action, withWindow] : std::initializer_list<std::tuple<const char*, uint8_t, bool>>{
             {"focusWindow", OXYOS_MANAGE_FOCUS, true},
             {"closeWindow", OXYOS_MANAGE_CLOSE, true},
             {"minimizeWindow", OXYOS_MANAGE_MINIMIZE, true},
             {"moveWindowToWorkspace", OXYOS_MANAGE_MOVE, true},
             {"activateWorkspace", OXYOS_MANAGE_ACTIVATE_WORKSPACE, false},
             {"addWorkspace", OXYOS_MANAGE_ADD_WORKSPACE, false},
             {"removeWorkspace", OXYOS_MANAGE_REMOVE_WORKSPACE, false}}) {
      module->method(method, [&services, ready, number, action, withWindow](jsi::Runtime& rt, const jsi::Value* args,
                                                                            size_t count) {
        ready(rt);
        // (window[, workspace]) or (workspace).
        uint32_t window = withWindow ? number(rt, args, count, 0) : 0;
        uint32_t workspace = number(rt, args, count, withWindow ? 1 : 0);
        services.shell.manage(action, window, workspace);
        return jsi::Value::undefined();
      });
    }
    module->method("launch", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 1 || !args[0].isString()) throw jsi::JSError(rt, "OxyOSShell.launch: an application id");
      return jsi::Value(static_cast<double>(services.shell.launch(args[0].asString(rt).utf8(rt))));
    });
    module->method("lockSession", [&services, ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      services.shell.lock();
      return jsi::Value::undefined();
    });
    // The password reaches the compositor and the session, nothing else: never logged or reported.
    module->method("authenticate", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 1 || !args[0].isString()) throw jsi::JSError(rt, "OxyOSShell.authenticate: a password");
      return jsi::Value(services.shell.authenticate(args[0].asString(rt).utf8(rt)));
    });
    // A command of the active window's menus, as the shell was told them (ADR 0021).
    module->method("activateMenuItem", [&services, ready, number](jsi::Runtime& rt, const jsi::Value* args,
                                                                   size_t count) {
      ready(rt);
      services.shell.activateMenuItem(number(rt, args, count, 0), number(rt, args, count, 1),
                                      number(rt, args, count, 2));
      return jsi::Value::undefined();
    });
    module->listeners();
  } else if (name == "OxyOSWindow") {
    // The application's own window (sample/oxyos/platform-surface.js, docs/adr/0016-windows.mdx).
    // `metrics` are the platform's: where the controls are and the room they take, the same for
    // every application, and the window's corner radius, which the compositor draws.
    auto ready = [&services](jsi::Runtime& rt) {
      if (!services.window.state) throw jsi::JSError(rt, "OxyOSWindow: only an application's window has it");
    };
    module->constants(folly::dynamic::object("available", services.applicationWindow)(
        "metrics", folly::dynamic::object("controls", folly::dynamic::object("start", 14)("top", 16)("width", 68)(
                                                          "height", 20))("room", folly::dynamic::object("top", 52)(
                                                          "leading", 88))("radius", 16)));
    module->method("getState", [&services, ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      return jsi::valueFromDynamic(rt, services.window.state());
    });
    for (auto [method, action] : std::initializer_list<std::pair<const char*, std::function<void()> WindowService::*>>{
             {"close", &WindowService::close}, {"minimize", &WindowService::minimize}, {"zoom", &WindowService::zoom}}) {
      module->method(method, [&services, ready, action](jsi::Runtime& rt, const jsi::Value*, size_t) {
        ready(rt);
        (services.window.*action)();
        return jsi::Value::undefined();
      });
    }
    module->method("startMove", [&services, ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      return jsi::Value(services.window.startMove());
    });
    module->method("setSizeLimits", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      auto at = [&](size_t i) { return count > i && args[i].isNumber() ? static_cast<uint32_t>(args[i].asNumber()) : 0u; };
      services.window.setSizeLimits(at(0), at(1), at(2), at(3));
      return jsi::Value::undefined();
    });
    module->listeners();
  } else if (name == "OxyOSPreferences") {
    // What every application draws by that React Native's own modules do not carry: Bloom's accent
    // colour and reduce transparency (sample/oxyos/platform-surface.js). `OxyOSPreferencesChanged`.
    module->method("getState", [&services](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return services.settings.drawing ? jsi::valueFromDynamic(rt, services.settings.drawing()) : jsi::Value::null();
    });
    module->listeners();
  } else if (name == "OxyOSSettings") {
    // OxyOS's Settings (apps/settings): every preference as the session keeps it, a change asked of
    // the session, and the system's facts. Only the client the session granted `settings` has its
    // preferences; `OxyOSSettingsChanged` and `OxyOSSettingsResult` follow.
    module->method("getValues", [&services](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return services.settings.values ? jsi::valueFromDynamic(rt, services.settings.values()) : jsi::Value::null();
    });
    module->method("setPreference", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 2 || !args[0].isString() || !args[1].isString()) {
        throw jsi::JSError(rt, "OxyOSSettings.setPreference: a key and a value");
      }
      if (!services.settings.set) throw jsi::JSError(rt, "OxyOSSettings: no connection to the session");
      return jsi::Value(static_cast<double>(
          services.settings.set(args[0].asString(rt).utf8(rt), args[1].asString(rt).utf8(rt))));
    });
    module->method("getSystem", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return jsi::valueFromDynamic(rt, systemFacts());
    });
    module->listeners();
  } else if (name == "OxyOSMenuBar") {
    // The application window's menus (sample/oxyos/platform-surface.js, docs/adr/0021-menu-bar.mdx):
    // `setMenus([{ parent, kind, flags, label, shortcut }])` sends them as a new commit and returns
    // its serial; `OxyOSMenuActivated` ({ serial, index }) is a choice in the shell's bar. The
    // adapter keeps Bloom's ids for each commit's indices: the compositor never sees them.
    module->constants(folly::dynamic::object("available", services.applicationWindow));
    module->method("setMenus", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (!services.menuBar.setMenus) throw jsi::JSError(rt, "OxyOSMenuBar: only an application's window has menus");
      if (count < 1 || !args[0].isObject()) throw jsi::JSError(rt, "OxyOSMenuBar.setMenus: the entries");
      auto value = jsi::dynamicFromValue(rt, args[0]);
      if (!value.isArray()) throw jsi::JSError(rt, "OxyOSMenuBar.setMenus: the entries");
      std::vector<MenuEntry> entries;
      for (const auto& item : value) {
        MenuEntry entry;
        auto parent = item.getDefault("parent", -1);
        entry.parent = parent.isNumber() && parent.asDouble() >= 0 ? static_cast<uint32_t>(parent.asDouble())
                                                                    : OXYOS_NO_MENU_PARENT;
        entry.kind = static_cast<uint8_t>(item.getDefault("kind", 0).asInt());
        entry.flags = static_cast<uint8_t>(item.getDefault("flags", 0).asInt());
        auto label = item.getDefault("label", "");
        auto shortcut = item.getDefault("shortcut", "");
        if (label.isString()) entry.label = label.getString();
        if (shortcut.isString()) entry.shortcut = shortcut.getString();
        entries.push_back(std::move(entry));
      }
      return jsi::Value(static_cast<double>(services.menuBar.setMenus(entries)));
    });
    module->listeners();
  } else if (name == "OxyOSKeyboard") {
    // The window's keys, offered to JavaScript before the focused view (sample/oxyos/platform-surface.js,
    // docs/adr/0021-menu-bar.mdx): `setListener(fn)`, fn({ key, code, altKey, ctrlKey, metaKey, shiftKey,
    // repeat }) returning true for a key it took. Only an application's window has it.
    module->constants(folly::dynamic::object("available", services.applicationWindow));
    module->method("setListener", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (!services.applicationWindow) throw jsi::JSError(rt, "OxyOSKeyboard: only an application's window has it");
      if (count < 1 || !args[0].isObject() || !args[0].asObject(rt).isFunction(rt)) {
        throw jsi::JSError(rt, "OxyOSKeyboard.setListener: a function");
      }
      services.keyboard.runtime = &rt;
      services.keyboard.listener = std::make_shared<jsi::Function>(args[0].asObject(rt).asFunction(rt));
      return jsi::Value::undefined();
    });
  } else if (name == "OxyOSClipboard") {
    // The clipboard behind expo-clipboard (sample/oxyos/expo-adapter.js), with the serial of the
    // latest key or press the app received: the compositor refuses it unless the app has keyboard
    // focus and that input is one of its latest 16 (docs/adr/0007-clipboard.mdx).
    auto ready = [&services](jsi::Runtime& rt) {
      if (!services.clipboard.copy) throw jsi::JSError(rt, "OxyOSClipboard: the clipboard needs a window on OxyOS");
    };
    module->method("hasString", [&services, ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      return resolved(rt, jsi::Value(services.clipboard.hasText()));
    });
    module->method("setString", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      return resolved(rt, jsi::Value(services.clipboard.copy(count > 0 ? text(rt, args[0]) : "")));
    });
    // The text arrives later, from the clipboard's owner: the promise settles on the JavaScript
    // thread, through the module's invoker. A refused paste reads nothing: "".
    module->method("getString", [&services, ready, invoker](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      auto start = jsi::Function::createFromHostFunction(
          rt, jsi::PropNameID::forAscii(rt, "paste"), 2,
          [&services, invoker](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t) {
            auto resolve = std::make_shared<jsi::Value>(rt, args[0]);
            services.clipboard.paste([invoker, resolve](std::string pasted) {
              invoker->invokeAsync([resolve, pasted = std::move(pasted)](jsi::Runtime& rt) {
                resolve->asObject(rt).asFunction(rt).call(rt, jsi::String::createFromUtf8(rt, pasted));
              });
            });
            return jsi::Value::undefined();
          });
      return rt.global().getPropertyAsFunction(rt, "Promise").callAsConstructor(rt, start);
    });
    module->listeners();
  } else if (name == "ImageLoader") {
    // Image sizes from the files' headers (images.h resolves the sources, oxyos-image reads them):
    // a promise of {width, height} in pixels, or of true for a prefetch.
    auto sized = [](bool prefetch) {
      return [prefetch](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
        auto source = resolveImageSource(count > 0 ? text(rt, args[0]) : "");
        if (!source.error.empty()) return rejected(rt, source.error);
        uint32_t width = 0, height = 0;
        if (oxyos_image_size(source.path.data(), source.path.size(), &width, &height) != 0) {
          return rejected(rt, oxyos_last_error());
        }
        // A local image needs no fetching: it is ready if it can be read.
        if (prefetch) return resolved(rt, jsi::Value(true));
        return resolved(rt, jsi::valueFromDynamic(rt, folly::dynamic::object("width", width)("height", height)));
      };
    };
    module->method("getSize", sized(false));
    module->method("getSizeWithHeaders", sized(false));
    module->method("prefetchImage", sized(true));
    module->method("queryCache", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return resolved(rt, jsi::Object(rt));
    });
    // A prefetch has finished when it returns: there is nothing to abort.
    module->method("abortRequest", [](jsi::Runtime&, const jsi::Value*, size_t) { return jsi::Value::undefined(); });
  } else if (name == "RNCSafeAreaContext") {
    // An OxyOS window has no system insets.
    folly::dynamic zero = folly::dynamic::object("top", 0)("right", 0)("bottom", 0)("left", 0);
    module->constants(folly::dynamic::object(
        "initialWindowMetrics", folly::dynamic::object("insets", zero)(
                                    "frame", folly::dynamic::object("x", 0)("y", 0)("width", display.width)(
                                                 "height", display.height))));
  } else if (name == "Networking") {
    // React Native's Android networking (XMLHttpRequest, so fetch and axios): network.h.
    module->method("sendRequest", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 9) throw jsi::JSError(rt, "Networking.sendRequest: missing arguments");
      Networking::Request request;
      request.method = text(rt, args[0]);
      request.url = text(rt, args[1]);
      request.id = static_cast<int64_t>(args[2].getNumber());
      folly::dynamic headers = jsi::dynamicFromValue(rt, args[3]);
      bool typed = false;
      if (headers.isArray()) {
        for (const auto& header : headers) {
          if (!header.isArray() || header.size() != 2) continue;
          std::string key = header[0].asString(), lower = key;
          std::transform(lower.begin(), lower.end(), lower.begin(), [](unsigned char c) { return std::tolower(c); });
          typed |= lower == "content-type";
          request.headers.emplace_back(key, header[1].asString());
        }
      }
      Body body = requestBody(services.blobs, jsi::dynamicFromValue(rt, args[4]));
      request.responseType = text(rt, args[5]);
      request.incremental = args[6].isBool() && args[6].getBool();
      request.timeoutMs = args[7].isNumber() ? static_cast<long>(args[7].getNumber()) : 0;
      request.withCredentials = !args[8].isBool() || args[8].getBool();
      if (!body.error.empty()) {
        services.networking.reject(request.id, body.error);
        return jsi::Value::undefined();
      }
      request.hasBody = body.present;
      request.body = std::move(body.bytes);
      if (!body.contentType.empty() && !typed) request.headers.emplace_back("Content-Type", body.contentType);
      services.networking.send(std::move(request));
      return jsi::Value::undefined();
    });
    module->method("abortRequest", [&services](jsi::Runtime&, const jsi::Value* args, size_t count) {
      if (count > 0 && args[0].isNumber()) services.networking.abort(static_cast<int64_t>(args[0].getNumber()));
      return jsi::Value::undefined();
    });
    module->method("clearCookies", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      services.networking.clearCookies();
      return count > 0 ? call(rt, args[0], jsi::Value(true)) : jsi::Value::undefined();
    });
    module->listeners();
  } else if (name == "BlobModule") {
    module->constants(folly::dynamic::object("BLOB_URI_SCHEME", "content")("BLOB_URI_HOST", nullptr));
    // Responses become blobs in the network module itself.
    module->method("addNetworkingHandler", [](jsi::Runtime&, const jsi::Value*, size_t) {
      return jsi::Value::undefined();
    });
    module->method("createFromParts", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 2) return jsi::Value::undefined();
      folly::dynamic parts = jsi::dynamicFromValue(rt, args[0]);
      std::string bytes;
      for (const auto& part : parts) {
        std::string type = part.count("type") ? part["type"].asString() : "";
        if (type == "string") {
          bytes += part["data"].asString();
        } else if (type == "blob") {
          auto slice = blobBytes(services.blobs, part["data"]);
          if (!slice) throw jsi::JSError(rt, "BlobModule.createFromParts: a part's blob does not exist");
          bytes += *slice;
        } else {
          throw jsi::JSError(rt, "BlobModule.createFromParts: unknown part type " + type);
        }
      }
      services.blobs.put(text(rt, args[1]), std::move(bytes));
      return jsi::Value::undefined();
    });
    module->method("release", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count > 0) services.blobs.release(text(rt, args[0]));
      return jsi::Value::undefined();
    });
    // There is no WebSocket module yet, so no socket to send a blob over.
    module->unsupported({"addWebSocketHandler", "removeWebSocketHandler"});
    module->method("sendOverSocket", [](jsi::Runtime& rt, const jsi::Value*, size_t) -> jsi::Value {
      throw jsi::JSError(rt, "BlobModule.sendOverSocket: OxyOS has no WebSocket module yet");
    });
  } else if (name == "FileReaderModule") {
    module->method("readAsText", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      auto bytes = count > 0 ? blobBytes(services.blobs, jsi::dynamicFromValue(rt, args[0])) : std::nullopt;
      if (!bytes) return rejected(rt, "FileReader: the blob does not exist");
      // UTF-8, whatever encoding is asked for: the one React Native's Android reader uses too.
      return resolved(rt, jsi::String::createFromUtf8(rt, *bytes));
    });
    module->method("readAsDataURL", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      folly::dynamic blob = count > 0 ? jsi::dynamicFromValue(rt, args[0]) : folly::dynamic(nullptr);
      auto bytes = blobBytes(services.blobs, blob);
      if (!bytes) return rejected(rt, "FileReader: the blob does not exist");
      std::string type = blob.count("type") && blob["type"].isString() && !blob["type"].asString().empty()
                             ? blob["type"].asString()
                             : "application/octet-stream";
      return resolved(rt, jsi::String::createFromUtf8(rt, "data:" + type + ";base64," + base64Encode(*bytes)));
    });
  } else if (name == "OxyOSFontRegistry") {
    // Registers a font file under a family name with the host's text engine.
    module->method("registerFont", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 2) return jsi::Value(false);
      std::string family = text(rt, args[0]), uri = text(rt, args[1]);
      if (uri.rfind("file://", 0) == 0) uri = uri.substr(7);
      return jsi::Value(TextEngine::shared().registerFamily(family, uri));
    });
  } else {
    return nullptr;
  }
  return module;
}

}  // namespace oxyos
