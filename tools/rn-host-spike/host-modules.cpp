#include "host-modules.h"

#include "files.h"
#include "images.h"
#include "text.h"

#include <folly/dynamic.h>
#include <folly/json.h>
#include <oxyos_client.h>
#include <jsi/JSIDynamic.h>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>
#include <optional>
#include <random>
#include <set>
#include <tuple>
#include <unistd.h>

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
// cannot be sent. A multipart part that is a file (`{uri, type, name}`, as React Native's FormData
// gives one) and a body from a URI are read from the file as they are sent, opened through the file
// service's scope (`files`, an application's window only); a `data:` URI's base64 is its bytes.
struct Body {
  bool present = false;
  std::string bytes;
  // A body with files: its parts (`bytes` unused).
  std::vector<Networking::Part> parts;
  std::string contentType;  // set for multipart bodies
  std::string error;

  Body() = default;
  Body(const Body&) = delete;
  Body& operator=(const Body&) = delete;
  Body(Body&& other) noexcept { *this = std::move(other); }
  Body& operator=(Body&& other) noexcept {
    present = other.present;
    bytes = std::move(other.bytes);
    parts = std::exchange(other.parts, {});
    contentType = std::move(other.contentType);
    error = std::move(other.error);
    return *this;
  }
  // Files opened for a body never sent are closed.
  ~Body() {
    for (const auto& part : parts) {
      if (part.fd >= 0) ::close(part.fd);
    }
  }
  // The parts, handed to the network, which owns their files from then.
  std::vector<Networking::Part> release() { return std::exchange(parts, {}); }
};

// A multipart boundary no part is likely to contain: random, as OkHttp's.
std::string boundary() {
  static std::mt19937_64 random{std::random_device{}()};
  static constexpr char kHex[] = "0123456789abcdef";
  std::string boundary = "OxyOSFormBoundary";
  for (int i = 0; i < 32; ++i) boundary += kHex[random() & 15];
  return boundary;
}

// A file part's content from its URI: a file the scope allows, or a data URI's bytes; false with
// the reason in `error` otherwise.
bool filePart(Files* files, const std::string& uri, Networking::Part& part, std::string& error) {
  if (uri.rfind("data:", 0) == 0) {
    auto comma = uri.find(',');
    auto meta = uri.substr(0, comma == std::string::npos ? 0 : comma);
    if (comma == std::string::npos || meta.size() < 7 || meta.compare(meta.size() - 7, 7, ";base64") != 0) {
      error = "a data: URI in a request body must be base64";
      return false;
    }
    auto bytes = base64Decode(uri.substr(comma + 1));
    if (!bytes) {
      error = "a data: URI in a request body is not valid base64";
      return false;
    }
    part.bytes = std::move(*bytes);
    return true;
  }
  if (uri.rfind("file:", 0) != 0) {
    error = "a request body's file must be a file:// or data: URI on OxyOS, not " + uri;
    return false;
  }
  if (!files) {
    error = "files are reached from an application's window only";
    return false;
  }
  part.fd = files->openRead(uri, part.length, error);
  return part.fd >= 0;
}

Body requestBody(const BlobStore& blobs, Files* files, const folly::dynamic& data) {
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
    bool withFiles = false;
    // Bytes gather until a file: each file is a part of its own, read as it is sent.
    std::string pending;
    for (const auto& part : data["formData"]) {
      pending += "--" + boundary + "\r\n";
      bool typed = false;
      if (part.count("headers") && part["headers"].isArray()) {
        for (const auto& header : part["headers"]) {
          if (header.isArray() && header.size() == 2) {
            std::string name = header[0].asString(), lower = name;
            std::transform(lower.begin(), lower.end(), lower.begin(), [](unsigned char c) { return std::tolower(c); });
            typed |= lower == "content-type";
            pending += name + ": " + header[1].asString() + "\r\n";
          }
        }
      }
      pending += "\r\n";
      if (part.count("string") && part["string"].isString()) {
        pending += part["string"].asString();
      } else if (part.count("uri") && part["uri"].isString()) {
        // As Android: a file part says its type.
        if (!typed) {
          body.error = "Binary FormData part needs a content-type header.";
          return body;
        }
        Networking::Part file;
        if (!filePart(files, part["uri"].asString(), file, body.error)) return body;
        if (file.fd >= 0) {
          body.parts.push_back({.bytes = std::exchange(pending, {})});
          body.parts.push_back(std::move(file));
          withFiles = true;
        } else {
          pending += file.bytes;
        }
      } else {
        body.error = "Unrecognized FormData part.";
        return body;
      }
      pending += "\r\n";
    }
    pending += "--" + boundary + "--\r\n";
    if (withFiles) {
      body.parts.push_back({.bytes = std::move(pending)});
    } else {
      body.bytes = std::move(pending);
    }
    body.present = true;
    body.contentType = "multipart/form-data; boundary=" + boundary;
  } else if (data.count("uri") && data["uri"].isString()) {
    Networking::Part file;
    if (!filePart(files, data["uri"].asString(), file, body.error)) return body;
    body.present = true;
    if (file.fd >= 0) {
      body.parts.push_back(std::move(file));
    } else {
      body.bytes = std::move(file.bytes);
    }
  }
  return body;
}

// Bytes JavaScript gives: a string's UTF-8, an ArrayBuffer's or a typed array's view of one; none
// for undefined or null.
std::vector<uint8_t> givenBytes(jsi::Runtime& rt, const jsi::Value& value) {
  if (value.isString()) {
    std::string text = value.asString(rt).utf8(rt);
    return {text.begin(), text.end()};
  }
  if (!value.isObject()) return {};
  auto object = value.asObject(rt);
  if (object.isArrayBuffer(rt)) {
    auto buffer = object.getArrayBuffer(rt);
    return {buffer.data(rt), buffer.data(rt) + buffer.size(rt)};
  }
  auto backing = object.getProperty(rt, "buffer");
  if (!backing.isObject() || !backing.asObject(rt).isArrayBuffer(rt)) {
    throw jsi::JSError(rt, "OxyOSFiles: content is a string, an ArrayBuffer or a typed array");
  }
  auto buffer = backing.asObject(rt).getArrayBuffer(rt);
  auto offset = static_cast<size_t>(object.getProperty(rt, "byteOffset").asNumber());
  auto length = static_cast<size_t>(object.getProperty(rt, "byteLength").asNumber());
  if (offset + length > buffer.size(rt)) throw jsi::JSError(rt, "OxyOSFiles: a view beyond its buffer");
  return {buffer.data(rt) + offset, buffer.data(rt) + offset + length};
}

// A reply's bytes as JavaScript's memory, without a copy.
class ReplyBuffer : public jsi::MutableBuffer {
 public:
  explicit ReplyBuffer(std::shared_ptr<std::vector<uint8_t>> bytes) : bytes_(std::move(bytes)) {}
  size_t size() const override { return bytes_->size(); }
  uint8_t* data() override { return bytes_->data(); }

 private:
  std::shared_ptr<std::vector<uint8_t>> bytes_;
};

// A reply for JavaScript: `{json, bytes}` with its bytes as a Uint8Array, or, `asText`, as a
// string (UTF-8, which the service made valid).
jsi::Value replyValue(jsi::Runtime& rt, const Files::Reply& reply, bool asText) {
  jsi::Object out(rt);
  out.setProperty(rt, "json", jsi::String::createFromUtf8(rt, reply.json));
  if (asText) {
    out.setProperty(rt, "text", jsi::String::createFromUtf8(rt, reply.bytes->data(), reply.bytes->size()));
  } else {
    jsi::ArrayBuffer buffer(rt, std::make_shared<ReplyBuffer>(reply.bytes));
    out.setProperty(rt, "bytes", rt.global().getPropertyAsFunction(rt, "Uint8Array").callAsConstructor(rt, buffer));
  }
  return out;
}

// The bytes a typed array (a Uint8Array, or any integer TypedArray) views, writable in place.
std::pair<uint8_t*, size_t> typedBytes(jsi::Runtime& rt, const jsi::Value& value) {
  if (!value.isObject()) throw jsi::JSError(rt, "a typed array is needed");
  auto array = value.asObject(rt);
  auto buffer = array.getProperty(rt, "buffer");
  if (!buffer.isObject() || !buffer.asObject(rt).isArrayBuffer(rt)) throw jsi::JSError(rt, "a typed array is needed");
  auto bytes = buffer.asObject(rt).getArrayBuffer(rt);
  size_t offset = static_cast<size_t>(array.getProperty(rt, "byteOffset").asNumber());
  size_t length = static_cast<size_t>(array.getProperty(rt, "byteLength").asNumber());
  if (offset + length > bytes.size(rt)) throw jsi::JSError(rt, "the typed array is outside its buffer");
  return {bytes.data(rt) + offset, length};
}

// A digest by expo-crypto's algorithm name, or a JavaScript error naming what OxyOS lacks.
std::string digestOf(jsi::Runtime& rt, const std::string& algorithm, const uint8_t* data, size_t length) {
  std::string digest(64, '\0');
  int64_t size = oxyos_digest(algorithm.c_str(), data, length, reinterpret_cast<uint8_t*>(digest.data()), digest.size());
  if (size < 0) throw jsi::JSError(rt, oxyos_last_error());
  digest.resize(static_cast<size_t>(size));
  return digest;
}

std::string hex(const std::string& bytes) {
  static constexpr char kHex[] = "0123456789abcdef";
  std::string out;
  for (unsigned char byte : bytes) {
    out += kHex[byte >> 4];
    out += kHex[byte & 15];
  }
  return out;
}

// An AsyncStorage callback's error, as its Android module gives one: {message}.
folly::dynamic storageError(const std::string& message) { return folly::dynamic::object("message", message); }

}  // namespace

folly::dynamic deviceDimensions(const Display& display) {
  folly::dynamic window = folly::dynamic::object("width", display.width)("height", display.height)(
      "scale", display.scale)("fontScale", 1.0);
  return folly::dynamic::object("window", window)("screen", window);
}

std::shared_ptr<TurboModule> createHostModule(
    const std::string& name,
    const std::shared_ptr<CallInvoker>& invoker,
    const Display& display,
    HostServices& services) {
  auto module = std::make_shared<HostModule>(name, invoker);

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
    module->constants(folly::dynamic::object("Dimensions", deviceDimensions(display)));
  } else if (name == "OxyOSRuntime") {
    // The runtime's numbers (an application's package names the ones it was built for) and its
    // framework's URL, which the framework's assets are resolved beside (oxyos/framework-init.js).
    module->constants(folly::dynamic::object("abi", services.runtime.abi)("api", services.runtime.api)(
        "frameworkURL", services.runtime.frameworkURL));
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
             // The window's application (docs/adr/0024-system-shortcuts.mdx).
             {"hideApplication", OXYOS_MANAGE_HIDE_APPLICATION, true},
             {"quitApplication", OXYOS_MANAGE_QUIT_APPLICATION, true},
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
    // At the login screen, with the account it is for.
    module->method("authenticate", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 1 || !args[0].isString()) throw jsi::JSError(rt, "OxyOSShell.authenticate: a password");
      std::string account = count > 1 && args[1].isString() ? args[1].asString(rt).utf8(rt) : "";
      return jsi::Value(services.shell.authenticate(args[0].asString(rt).utf8(rt), account));
    });
    // The enrolled accounts (docs/adr/0020-oxy-account-login.mdx), from the accounts' service,
    // which answers only the login screen's user: [{ user, name, picture }], or null when they
    // cannot be read.
    module->method("accounts", [ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      std::string json(4096, '\0');
      for (int tries = 0; tries < 4; ++tries) {
        int64_t length = oxyos_accounts(json.data(), json.size());
        if (length < 0) {
          std::cerr << "[host] the accounts: " << oxyos_last_error() << "\n";
          return jsi::Value::null();
        }
        if (static_cast<size_t>(length) < json.size()) {
          json.resize(static_cast<size_t>(length));
          return jsi::valueFromDynamic(rt, folly::parseJson(json));
        }
        json.assign(static_cast<size_t>(length) + 1, '\0');
      }
      return jsi::Value::null();
    });
    // The Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx): controls by name,
    // levels from 0 to 1.
    auto control = [](jsi::Runtime& rt, const jsi::Value* args, size_t count) -> uint8_t {
      std::string name = count > 0 && args[0].isString() ? args[0].asString(rt).utf8(rt) : "";
      if (name == "output") return OXYOS_CONTROL_OUTPUT_VOLUME;
      if (name == "input") return OXYOS_CONTROL_INPUT_VOLUME;
      if (name == "display") return OXYOS_CONTROL_DISPLAY_BRIGHTNESS;
      if (name == "keyboard") return OXYOS_CONTROL_KEYBOARD_BRIGHTNESS;
      throw jsi::JSError(rt, "OxyOSShell: output, input, display or keyboard");
    };
    module->method("setControlLevel", [&services, ready, control](jsi::Runtime& rt, const jsi::Value* args,
                                                                   size_t count) {
      ready(rt);
      double level = count > 1 && args[1].isNumber() ? args[1].asNumber() : 0;
      services.shell.setControlLevel(control(rt, args, count),
                                     static_cast<uint32_t>(std::lround(std::clamp(level, 0.0, 1.0) * 65536)));
      return jsi::Value::undefined();
    });
    module->method("setControlMuted", [&services, ready, control](jsi::Runtime& rt, const jsi::Value* args,
                                                                   size_t count) {
      ready(rt);
      services.shell.setControlMuted(control(rt, args, count), count > 1 && args[1].isBool() && args[1].getBool());
      return jsi::Value::undefined();
    });
    module->method("setWifiEnabled", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      services.shell.setWifiEnabled(count > 0 && args[0].isBool() && args[0].getBool());
      return jsi::Value::undefined();
    });
    module->method("setAppearance", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      services.shell.setAppearance(count > 0 && args[0].isBool() && args[0].getBool());
      return jsi::Value::undefined();
    });
    module->method("refreshControls", [&services, ready](jsi::Runtime& rt, const jsi::Value*, size_t) {
      ready(rt);
      services.shell.refreshControls();
      return jsi::Value::undefined();
    });
    // A command of the active window's menus, as the shell was told them (ADR 0021).
    module->method("activateMenuItem", [&services, ready, number](jsi::Runtime& rt, const jsi::Value* args,
                                                                   size_t count) {
      ready(rt);
      services.shell.activateMenuItem(number(rt, args, count, 0), number(rt, args, count, 1),
                                      number(rt, args, count, 2));
      return jsi::Value::undefined();
    });
    // Notifications (docs/adr/0026-notifications.mdx): (notification, action, reply) pressed, and
    // (notification) removed, 0 for all.
    module->method("activateNotification", [&services, ready, number](jsi::Runtime& rt, const jsi::Value* args,
                                                                       size_t count) {
      ready(rt);
      std::string reply = count > 2 && args[2].isString() ? args[2].asString(rt).utf8(rt) : "";
      services.shell.activateNotification(number(rt, args, count, 0), static_cast<uint8_t>(number(rt, args, count, 1)),
                                          reply);
      return jsi::Value::undefined();
    });
    module->method("setDoNotDisturb", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      services.shell.setDoNotDisturb(count > 0 && args[0].isBool() && args[0].getBool());
      return jsi::Value::undefined();
    });
    module->method("removeNotification", [&services, ready, number](jsi::Runtime& rt, const jsi::Value* args,
                                                                     size_t count) {
      ready(rt);
      services.shell.removeNotification(number(rt, args, count, 0));
      return jsi::Value::undefined();
    });
    // (text, polite): to assistive technologies, after what they are saying or at once.
    module->method("announce", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count > 0 && args[0].isString()) {
        services.shell.announce(args[0].asString(rt).utf8(rt), count > 1 && args[1].isBool() && args[1].getBool());
      }
      return jsi::Value::undefined();
    });
    module->listeners();
  } else if (name == "OxyOSSystem") {
    // Installing applications (sample/oxyos/system.js, docs/adr/0030-installing-applications.mdx):
    // a request of the session's vocabulary, answered by `OxyOSSystemAnswer`s naming its serial
    // (0: the update state, unasked); the shell's answer to a sheet (`OxyOSSheet`,
    // `OxyOSSheetDismissed`); a capability asked at first use, answered by
    // `OxyOSCapabilityResult`; and what this launch may use. Requests cite the latest press.
    auto ready = [&services](jsi::Runtime& rt) {
      if (!services.system.request) throw jsi::JSError(rt, "OxyOSSystem: no connection to the session");
    };
    module->method("request", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 1 || !args[0].isString()) throw jsi::JSError(rt, "OxyOSSystem.request: the request's JSON");
      return jsi::Value(static_cast<double>(services.system.request(args[0].asString(rt).utf8(rt))));
    });
    module->method("answerSheet", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 2 || !args[0].isNumber() || !args[1].isBool()) {
        throw jsi::JSError(rt, "OxyOSSystem.answerSheet: a sheet and whether it is accepted");
      }
      services.system.answerSheet(static_cast<uint32_t>(args[0].asNumber()), args[1].getBool());
      return jsi::Value::undefined();
    });
    module->method("requestCapability", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 1 || !args[0].isString()) throw jsi::JSError(rt, "OxyOSSystem.requestCapability: a capability");
      return jsi::Value(static_cast<double>(services.system.requestCapability(args[0].asString(rt).utf8(rt))));
    });
    module->method("getLaunch", [&services](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return services.system.launch ? jsi::valueFromDynamic(rt, services.system.launch()) : jsi::Value::null();
    });
    module->listeners();
  } else if (name == "OxyOSNotifications") {
    // An application's notifications, behind expo-notifications (sample/oxyos/expo-adapter.js,
    // docs/adr/0026-notifications.mdx): posting, withdrawing, and the responses the session gave.
    auto ready = [&services](jsi::Runtime& rt) {
      if (!services.notifications.post) {
        throw jsi::JSError(rt, "OxyOSNotifications: notifications need a connection to the compositor on OxyOS");
      }
    };
    module->method("post", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      if (count < 2 || !args[0].isNumber() || !args[1].isObject()) {
        throw jsi::JSError(rt, "OxyOSNotifications.post: a serial and the notification");
      }
      // An installed version posts only once its account granted it (ADR 0030, decision 8).
      if (services.system.allows && !services.system.allows("notifications")) return jsi::Value(false);
      return jsi::Value(services.notifications.post(static_cast<uint32_t>(args[0].asNumber()),
                                                    jsi::dynamicFromValue(rt, args[1])));
    });
    module->method("withdraw", [&services, ready](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ready(rt);
      services.notifications.withdraw(count > 0 && args[0].isNumber() ? static_cast<uint32_t>(args[0].asNumber()) : 0);
      return jsi::Value::undefined();
    });
    // Each response once: taken, it is JavaScript's.
    module->method("takeResponses", [&services](jsi::Runtime& rt, const jsi::Value*, size_t) {
      folly::dynamic taken = folly::dynamic::array();
      for (auto& response : services.notifications.responses) taken.push_back(std::move(response));
      services.notifications.responses.clear();
      return jsi::valueFromDynamic(rt, taken);
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
    // The displays (docs/adr/0018-several-displays.mdx): as the session tells them, and a change of
    // one, answered as a preference's (`OxyOSSettingsResult`); `OxyOSDisplaysChanged` follows.
    module->method("getDisplays", [&services](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return services.settings.displays ? jsi::valueFromDynamic(rt, services.settings.displays()) : jsi::Value::null();
    });
    for (const char* change : {"mode", "scale", "origin", "primary", "confirm"}) {
      static const std::map<std::string, std::string> methods = {{"mode", "setDisplayMode"},
                                                                 {"scale", "setDisplayScale"},
                                                                 {"origin", "setDisplayOrigin"},
                                                                 {"primary", "setPrimaryDisplay"},
                                                                 {"confirm", "confirmDisplays"}};
      const std::string& method = methods.at(change);
      module->method(method, [&services, change = std::string(change), method](jsi::Runtime& rt, const jsi::Value* args,
                                                                              size_t count) {
        std::vector<double> numbers;
        for (size_t i = 0; i < count; ++i) {
          if (args[i].isBool()) {
            numbers.push_back(args[i].getBool() ? 1 : 0);
          } else if (args[i].isNumber()) {
            numbers.push_back(args[i].asNumber());
          } else {
            throw jsi::JSError(rt, "OxyOSSettings." + method + ": numbers");
          }
        }
        if (!services.settings.display) throw jsi::JSError(rt, "OxyOSSettings: no connection to the session");
        return jsi::Value(static_cast<double>(services.settings.display(change, numbers)));
      });
    }
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
    // repeat }) returning true for a key it took. An application's window has it, and the shell, for
    // the keys its surfaces receive (Launchpad's arrows and Return).
    bool available = services.applicationWindow || services.shellWindow;
    module->constants(folly::dynamic::object("available", available));
    module->method("setListener", [&services](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (!services.applicationWindow && !services.shellWindow) {
        throw jsi::JSError(rt, "OxyOSKeyboard: only an application's window or the shell has it");
      }
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
      Body body = requestBody(services.blobs, services.files, jsi::dynamicFromValue(rt, args[4]));
      request.responseType = text(rt, args[5]);
      request.incremental = args[6].isBool() && args[6].getBool();
      request.timeoutMs = args[7].isNumber() ? static_cast<long>(args[7].getNumber()) : 0;
      request.withCredentials = !args[8].isBool() || args[8].getBool();
      if (!body.error.empty()) {
        services.networking.reject(request.id, body.error);
        return jsi::Value::undefined();
      }
      // As Android: a body from a URI says its type.
      if (body.present && !typed && body.contentType.empty() && jsi::dynamicFromValue(rt, args[4]).count("uri")) {
        services.networking.reject(request.id, "Payload is set but no content-type header specified");
        return jsi::Value::undefined();
      }
      request.hasBody = body.present;
      request.body = std::move(body.bytes);
      request.parts = body.release();
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
  } else if (name == "OxyOSFiles") {
    // The user's files (files.h, docs/adr/0028-file-access.mdx), for an application's window, beneath
    // expo-file-system and the Bloom adapter: `call(json, content?, as?)` answers at once,
    // `request(id, json, content?, as?)` with a promise, each `{json, bytes}` (`{json, text}` when
    // `as` is "text"); `OxyOSFilesEvent` says what watches saw and transfers' progress. Absent
    // elsewhere, so a call fails visibly.
    if (!services.files) return nullptr;
    auto asText = [](jsi::Runtime& rt, const jsi::Value* args, size_t count, size_t at) {
      return count > at && args[at].isString() && args[at].asString(rt).utf8(rt) == "text";
    };
    module->method("call", [&services, asText](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 1 || !args[0].isString()) throw jsi::JSError(rt, "OxyOSFiles.call: a request as JSON text");
      auto bytes = count > 1 ? givenBytes(rt, args[1]) : std::vector<uint8_t>{};
      auto reply = services.files->call(args[0].asString(rt).utf8(rt), bytes.data(), bytes.size());
      return replyValue(rt, reply, asText(rt, args, count, 2));
    });
    module->method("request", [&services, asText, invoker](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 2 || !args[0].isNumber() || !args[1].isString()) {
        throw jsi::JSError(rt, "OxyOSFiles.request: an id and a request as JSON text");
      }
      auto id = static_cast<uint64_t>(args[0].getNumber());
      auto json = args[1].asString(rt).utf8(rt);
      auto bytes = count > 2 ? givenBytes(rt, args[2]) : std::vector<uint8_t>{};
      bool text = asText(rt, args, count, 3);
      // Settled on the JavaScript thread, through the module's invoker, once the service answers.
      auto start = jsi::Function::createFromHostFunction(
          rt, jsi::PropNameID::forAscii(rt, "request"), 2,
          [&services, invoker, id, json, bytes, text](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args,
                                                        size_t) mutable {
            auto resolve = std::make_shared<jsi::Value>(rt, args[0]);
            services.files->request(id, json, std::move(bytes), [invoker, resolve, text](Files::Reply reply) {
              invoker->invokeAsync([resolve, reply = std::move(reply), text](jsi::Runtime& rt) {
                resolve->asObject(rt).asFunction(rt).call(rt, replyValue(rt, reply, text));
              });
            });
            return jsi::Value::undefined();
          });
      return rt.global().getPropertyAsFunction(rt, "Promise").callAsConstructor(rt, start);
    });
    module->listeners();
  } else if (name == "OxyOSApplication") {
    // The application this host runs (application.h), for the Expo modules built on it
    // (sample/oxyos/expo-adapter.js): its id (the registry entry's), whether the session started
    // it, its module, its embedded app config (expo-constants' manifest), its directories, and the
    // machine's name and version for expo-constants.
    const auto& application = services.application;
    auto orNull = [](const std::string& text) { return text.empty() ? folly::dynamic(nullptr) : folly::dynamic(text); };
    module->constants(folly::dynamic::object("id", orNull(application.id))("recorded", application.recorded)(
        "module", application.module)("manifest", application.manifest ? folly::dynamic(*application.manifest)
                                                                       : folly::dynamic(nullptr))(
        "dataDirectory", orNull(application.dataDirectory))("cacheDirectory", orNull(application.cacheDirectory))(
        "deviceName", firstLine("/etc/hostname"))("systemVersion", assignment("/etc/os-release", "VERSION_ID")));
  } else if (name == "IntentAndroid") {
    // React Native's Linking, as its Android module answers it. OxyOS starts an application from
    // its registry entry, never from a link, so it was opened by none; and no application on
    // OxyOS opens links yet (there is no browser, nor a handler for a scheme), so none can be
    // opened: asked, it says so, and opening one fails with the reason.
    module->method("getInitialURL", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return resolved(rt, jsi::Value::null());
    });
    module->method("canOpenURL", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return resolved(rt, jsi::Value(false));
    });
    module->method("openURL", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      std::string url = count > 0 ? text(rt, args[0]) : "";
      auto colon = url.find(':');
      std::string scheme = colon == std::string::npos ? url : url.substr(0, colon);
      return rejected(rt, "OxyOS has no application that opens " + scheme + " links yet (" + url + ")");
    });
    module->method("openSettings", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return rejected(rt, "OxyOS has no settings page for an application yet");
    });
    module->method("sendIntent", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return rejected(rt, "OxyOS has no Android intents");
    });
    module->listeners();
  } else if (name == "RNCAsyncStorage") {
    // @react-native-async-storage/async-storage (application.h, crates/oxyos-app-storage): kept in
    // the application's data directory, by its id, on a thread of its own; each callback as its
    // Android module calls it. Without an id the session gave it there is nowhere to keep anything:
    // every call fails.
    auto ask = [&services, invoker](jsi::Runtime& rt, folly::dynamic request, const jsi::Value* callback,
                                    std::function<void(jsi::Runtime&, jsi::Function&, const folly::dynamic&)> answer) {
      if (!callback || !callback->isObject() || !callback->asObject(rt).isFunction(rt)) return;
      auto done = std::make_shared<jsi::Function>(callback->asObject(rt).asFunction(rt));
      if (services.application.dataDirectory.empty()) {
        answer(rt, *done, folly::dynamic::object("error", "OxyOS keeps nothing for this application: the session did not start it with an application id"));
        return;
      }
      if (!services.storage) services.storage = std::make_unique<AsyncStorage>(services.application.dataDirectory);
      services.storage->request(std::move(request), [invoker, done, answer](folly::dynamic result) {
        invoker->invokeAsync([done, answer, result = std::move(result)](jsi::Runtime& rt) { answer(rt, *done, result); });
      });
    };
    auto keys = [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      return count > 0 ? jsi::dynamicFromValue(rt, args[0]) : folly::dynamic::array();
    };
    // Changes and multiGet: an array of errors, or null.
    auto errors = [](jsi::Runtime& rt, const folly::dynamic& result) {
      return result.count("error") ? jsi::valueFromDynamic(rt, folly::dynamic::array(storageError(result["error"].asString())))
                                   : jsi::Value::null();
    };
    module->method("multiGet", [ask, keys, errors](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ask(rt, folly::dynamic::object("op", "get")("keys", keys(rt, args, count)), count > 1 ? &args[1] : nullptr,
          [errors](jsi::Runtime& rt, jsi::Function& done, const folly::dynamic& result) {
            done.call(rt, errors(rt, result), result.count("ok") ? jsi::valueFromDynamic(rt, result["ok"]) : jsi::Value::null());
          });
      return jsi::Value::undefined();
    });
    for (const char* change : {"multiSet", "multiMerge", "multiRemove"}) {
      std::string op = std::string(change) == "multiSet" ? "set" : std::string(change) == "multiMerge" ? "merge" : "remove";
      module->method(change, [ask, keys, errors, op](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
        folly::dynamic request = folly::dynamic::object("op", op)(op == "remove" ? "keys" : "pairs", keys(rt, args, count));
        ask(rt, std::move(request), count > 1 ? &args[1] : nullptr,
            [errors](jsi::Runtime& rt, jsi::Function& done, const folly::dynamic& result) { done.call(rt, errors(rt, result)); });
        return jsi::Value::undefined();
      });
    }
    // getAllKeys and clear: one error, or null.
    auto error = [](jsi::Runtime& rt, const folly::dynamic& result) {
      return result.count("error") ? jsi::valueFromDynamic(rt, storageError(result["error"].asString())) : jsi::Value::null();
    };
    module->method("getAllKeys", [ask, error](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ask(rt, folly::dynamic::object("op", "keys"), count > 0 ? &args[0] : nullptr,
          [error](jsi::Runtime& rt, jsi::Function& done, const folly::dynamic& result) {
            done.call(rt, error(rt, result), result.count("ok") ? jsi::valueFromDynamic(rt, result["ok"]) : jsi::Value::null());
          });
      return jsi::Value::undefined();
    });
    module->method("clear", [ask, error](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      ask(rt, folly::dynamic::object("op", "clear"), count > 0 ? &args[0] : nullptr,
          [error](jsi::Runtime& rt, jsi::Function& done, const folly::dynamic& result) { done.call(rt, error(rt, result)); });
      return jsi::Value::undefined();
    });
  } else if (name == "RNCNetInfo") {
    // @react-native-community/netinfo (application.h, crates/oxyos-net): the machine's
    // connectivity from the kernel, and `netInfo.networkStatusDidChange` when it changes, as its
    // Android module tells them. Whether the internet answers is NetInfo's own check over HTTP.
    auto connectivity = [&services]() -> Connectivity& {
      if (!services.connectivity) {
        services.connectivity = std::make_unique<Connectivity>([&services](const folly::dynamic& state) {
          if (services.emit) services.emit("netInfo.networkStatusDidChange", state);
        });
      }
      return *services.connectivity;
    };
    // Its configuration is NetInfo's reachability check, which runs in JavaScript.
    module->method("configure", [](jsi::Runtime&, const jsi::Value*, size_t) { return jsi::Value::undefined(); });
    module->method("getCurrentState", [connectivity](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      folly::dynamic state = connectivity().state();
      // An interface asked for that is not the one in use is not connected, as on Android.
      if (count > 0 && args[0].isString()) {
        std::string requested = args[0].asString(rt).utf8(rt);
        if (requested != state["type"].asString()) {
          state = folly::dynamic::object("type", requested)("isConnected", false)("details", nullptr);
        }
      }
      return resolved(rt, jsi::valueFromDynamic(rt, state));
    });
    module->listeners();
  } else if (name == "OxyOSCrypto") {
    // expo-crypto (sample/oxyos/expo-adapter.js): the kernel's CSPRNG and digests in Rust
    // (oxyos-client-ffi): SHA-1, SHA-256, SHA-384 and SHA-512. Expo's MD2, MD4 and MD5 are not
    // provided: asked for, they throw.
    module->method("getRandomValues", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 1) throw jsi::JSError(rt, "getRandomValues: a typed array is needed");
      auto [data, length] = typedBytes(rt, args[0]);
      if (oxyos_random_bytes(data, length) != 0) throw jsi::JSError(rt, oxyos_last_error());
      return jsi::Value(rt, args[0]);
    });
    // The digest of a string's UTF-8, as hex or base64.
    module->method("digestString", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 3) throw jsi::JSError(rt, "digestString: an algorithm, the data and an encoding");
      std::string data = text(rt, args[1]);
      std::string digest = digestOf(rt, text(rt, args[0]), reinterpret_cast<const uint8_t*>(data.data()), data.size());
      return jsi::String::createFromAscii(rt, text(rt, args[2]) == "base64" ? base64Encode(digest) : hex(digest));
    });
    // The digest of a typed array's bytes, into another.
    module->method("digest", [](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
      if (count < 3) throw jsi::JSError(rt, "digest: an algorithm, an output and the data");
      auto [data, length] = typedBytes(rt, args[2]);
      std::string digest = digestOf(rt, text(rt, args[0]), data, length);
      auto [out, room] = typedBytes(rt, args[1]);
      if (room < digest.size()) throw jsi::JSError(rt, "digest: the output is too small");
      std::copy(digest.begin(), digest.end(), out);
      return jsi::Value::undefined();
    });
  } else if (name == "KeyboardController") {
    // react-native-keyboard-controller, as its Android module answers with the software keyboard
    // hidden, which on OxyOS it always is: there is none (a hardware keyboard and the input
    // method, ADR 0008, need no room). Its events never come, its keyboard has no corners, and
    // dismissing it takes focus from the focused input unless asked to keep it. Its modes and the
    // status bar's translucency change nothing here. Moving focus to the next or previous input
    // and measuring a view are not provided yet: they throw.
    module->constants(folly::dynamic::object("keyboardBorderRadius", 0));
    module->unsupported({"setInputMode", "setDefaultMode", "preload", "setTranslucent"});
    module->method("dismiss", [&services](jsi::Runtime&, const jsi::Value* args, size_t count) {
      bool keepFocus = count > 0 && args[0].isBool() && args[0].getBool();
      if (!keepFocus && services.blurFocusedInput) services.blurFocusedInput();
      return jsi::Value::undefined();
    });
    module->method("setFocusTo", [](jsi::Runtime& rt, const jsi::Value*, size_t) -> jsi::Value {
      throw jsi::JSError(rt, "KeyboardController.setFocusTo is not available on OxyOS yet");
    });
    module->method("viewPositionInWindow", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return rejected(rt, "KeyboardController.viewPositionInWindow is not available on OxyOS yet");
    });
    module->listeners();
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
