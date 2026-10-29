#include "intl.h"

#include "intl-js.h"

#include <oxyos_client.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace jsi = facebook::jsi;

namespace oxyos {

namespace {

// A result of the C ABI: it gives the length it needs, and is called again with that much room.
// A refusal is thrown as JavaScript's RangeError, with the ABI's reason.
std::string call(jsi::Runtime& rt, const std::function<int64_t(char*, size_t)>& fill) {
  std::string out(64, '\0');
  for (;;) {
    int64_t length = fill(out.data(), out.size());
    if (length < 0) {
      auto error = rt.global().getPropertyAsFunction(rt, "RangeError").callAsConstructor(rt, oxyos_last_error());
      throw jsi::JSError(rt, std::move(error));
    }
    if (static_cast<size_t>(length) < out.size()) {
      out.resize(static_cast<size_t>(length));
      return out;
    }
    out.assign(static_cast<size_t>(length) + 1, '\0');
  }
}

// Parts, "type\x1Fvalue" joined by '\x1E', as an array of [type, value] pairs.
jsi::Array parts(jsi::Runtime& rt, const std::string& text) {
  std::vector<std::pair<std::string, std::string>> found;
  size_t start = 0;
  while (start < text.size()) {
    size_t end = text.find('\x1E', start);
    if (end == std::string::npos) end = text.size();
    std::string part = text.substr(start, end - start);
    size_t split = part.find('\x1F');
    if (split != std::string::npos) found.emplace_back(part.substr(0, split), part.substr(split + 1));
    start = end + 1;
  }
  jsi::Array out(rt, found.size());
  for (size_t i = 0; i < found.size(); ++i) {
    jsi::Array pair(rt, 2);
    pair.setValueAtIndex(rt, 0, jsi::String::createFromUtf8(rt, found[i].first));
    pair.setValueAtIndex(rt, 1, jsi::String::createFromUtf8(rt, found[i].second));
    out.setValueAtIndex(rt, i, std::move(pair));
  }
  return out;
}

std::string text(jsi::Runtime& rt, const jsi::Value* args, size_t count, size_t index) {
  return index < count ? args[index].asString(rt).utf8(rt) : std::string();
}

double number(const jsi::Value* args, size_t count, size_t index) {
  return index < count && args[index].isNumber() ? args[index].getNumber() : 0;
}

void function(jsi::Runtime& rt, jsi::Object& target, const char* name, unsigned arity, jsi::HostFunctionType body) {
  target.setProperty(rt, name,
                     jsi::Function::createFromHostFunction(rt, jsi::PropNameID::forAscii(rt, name), arity,
                                                           std::move(body)));
}

}  // namespace

void installIntl(jsi::Runtime& rt) {
  jsi::Object native(rt);
  function(rt, native, "defaultLocale", 0, [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value*, size_t) {
    return jsi::String::createFromUtf8(rt, call(rt, [](char* out, size_t capacity) {
                                         return oxyos_intl_default_locale(out, capacity);
                                       }));
  });
  function(rt, native, "canonicalize", 1,
           [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
             std::string tag = text(rt, args, count, 0);
             return jsi::String::createFromUtf8(rt, call(rt, [&](char* out, size_t capacity) {
                                                  return oxyos_intl_canonicalize(tag.data(), tag.size(), out, capacity);
                                                }));
           });
  function(rt, native, "resolve", 1, [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
    std::string requested = text(rt, args, count, 0);
    return jsi::String::createFromUtf8(rt, call(rt, [&](char* out, size_t capacity) {
                                         return oxyos_intl_resolve(requested.data(), requested.size(), out, capacity);
                                       }));
  });
  function(rt, native, "formatDate", 3,
           [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
             std::string locale = text(rt, args, count, 0), options = text(rt, args, count, 1);
             double time = number(args, count, 2);
             return jsi::Value(rt, parts(rt, call(rt, [&](char* out, size_t capacity) {
                                         return oxyos_intl_format_date(locale.data(), locale.size(), options.data(),
                                                                       options.size(), time, out, capacity);
                                       })));
           });
  function(rt, native, "resolveDate", 2,
           [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
             std::string locale = text(rt, args, count, 0), options = text(rt, args, count, 1);
             return jsi::String::createFromUtf8(rt, call(rt, [&](char* out, size_t capacity) {
                                                  return oxyos_intl_resolve_date(locale.data(), locale.size(),
                                                                                 options.data(), options.size(), out,
                                                                                 capacity);
                                                }));
           });
  function(rt, native, "formatNumber", 3,
           [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
             std::string locale = text(rt, args, count, 0), options = text(rt, args, count, 1);
             double value = number(args, count, 2);
             return jsi::Value(rt, parts(rt, call(rt, [&](char* out, size_t capacity) {
                                         return oxyos_intl_format_number(locale.data(), locale.size(), options.data(),
                                                                         options.size(), value, out, capacity);
                                       })));
           });
  function(rt, native, "resolveNumber", 2,
           [](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
             std::string locale = text(rt, args, count, 0), options = text(rt, args, count, 1);
             return jsi::String::createFromUtf8(rt, call(rt, [&](char* out, size_t capacity) {
                                                  return oxyos_intl_resolve_number(locale.data(), locale.size(),
                                                                                   options.data(), options.size(), out,
                                                                                   capacity);
                                                }));
           });
  rt.global().setProperty(rt, "__oxyosIntl", std::move(native));
  rt.evaluateJavaScript(std::make_shared<jsi::StringBuffer>(kIntlJs), "oxyos-intl.js");
}

}  // namespace oxyos
