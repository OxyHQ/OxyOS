#include "host-modules.h"

#include "images.h"
#include "text.h"

#include <folly/dynamic.h>
#include <jsi/JSIDynamic.h>

#include <cstdlib>
#include <iostream>
#include <map>
#include <set>

using namespace facebook;
using namespace facebook::react;

namespace oxy {

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

std::string text(jsi::Runtime& rt, const jsi::Value& value) {
  return value.isString() ? value.asString(rt).utf8(rt) : value.isUndefined() ? "" : value.toString(rt).utf8(rt);
}

}  // namespace

std::shared_ptr<TurboModule> createHostModule(
    const std::string& name,
    const std::shared_ptr<CallInvoker>& invoker,
    const Display& display) {
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
    const char* url = std::getenv("OXY_SCRIPT_URL");
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
    module->method("getColorScheme", [](jsi::Runtime& rt, const jsi::Value*, size_t) {
      return jsi::String::createFromAscii(rt, "light");
    });
    module->unsupported({"setColorScheme"});
    module->listeners();
  } else if (name == "AccessibilityInfo") {
    // No assistive-technology or reduced-motion settings exist on OxyOS yet: report them off.
    for (const char* query : {"isReduceMotionEnabled", "isTouchExplorationEnabled", "isAccessibilityServiceEnabled",
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
    module->unsupported({"setAccessibilityFocus", "announceForAccessibility"});
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
  } else if (name == "ImageLoader") {
    // Image sizes from the files' headers (images.h resolves the sources, oxy-image reads them):
    // a promise of {width, height} in pixels, or of true for a prefetch.
    auto sized = [](bool prefetch) {
      return [prefetch](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
        auto source = resolveImageSource(count > 0 ? text(rt, args[0]) : "");
        if (!source.error.empty()) return rejected(rt, source.error);
        uint32_t width = 0, height = 0;
        if (oxy_image_size(source.path.data(), source.path.size(), &width, &height) != 0) {
          return rejected(rt, oxy_last_error());
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
  } else if (name == "OxyFontRegistry") {
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

}  // namespace oxy
