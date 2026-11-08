#include "platform-modules.h"

#include <OxyOSPlatformSpecJSI.h>

#include <utility>

namespace oxyos {

namespace {

using facebook::react::NativeOxyOSClipboardCxxSpec;
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
  return nullptr;
}

}  // namespace oxyos
