#include "gesture-handler.h"

#include "host-modules.h"
#include "report.h"

#include <functional>

#include <RNGHRuntimeDecorator.h>

#include <chrono>
#include <iostream>
#include <vector>

using namespace facebook::react;
namespace jsi = facebook::jsi;

namespace oxyos {

namespace {

// How a handler's events reach JavaScript (`attachGestureHandler`), RNGH's ActionType.
constexpr int kReanimatedWorklet = 1;
constexpr int kNativeAnimatedEvent = 2;
constexpr int kJsFunctionOldApi = 3;
constexpr int kJsFunctionNewApi = 4;

// Relations JavaScript gives in a handler's settings when it creates it.
const std::set<std::string> kRelationKeys = {"waitFor", "simultaneousHandlers", "blocksHandlers"};

void reportOnce(const std::string& what) {
  static std::set<std::string> reported;
  if (reported.insert(what).second) std::cerr << "[gestures] " << what << "\n";
}

const char* stateName(int state) {
  static const char* names[] = {"UNDETERMINED", "FAILED", "BEGAN", "CANCELLED", "ACTIVE", "END"};
  return state >= 0 && state <= 5 ? names[state] : "?";
}

std::vector<int32_t> tags(jsi::Runtime& rt, const jsi::Value& list) {
  std::vector<int32_t> out;
  if (!list.isObject() || !list.asObject(rt).isArray(rt)) return out;
  auto array = list.asObject(rt).asArray(rt);
  for (size_t i = 0; i < array.size(rt); ++i) {
    auto tag = array.getValueAtIndex(rt, i);
    if (tag.isNumber()) out.push_back(static_cast<int32_t>(tag.asNumber()));
  }
  return out;
}

}  // namespace

GestureHandlerHost::GestureHandlerHost(const ViewTree& tree, Delivery delivery)
    : tree_(tree), delivery_(std::move(delivery)), gestures_(oxyos_gestures_new()) {}

GestureHandlerHost::~GestureHandlerHost() { oxyos_gestures_free(gestures_); }

uint64_t GestureHandlerHost::now() {
  using namespace std::chrono;
  return static_cast<uint64_t>(duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count());
}

std::shared_ptr<TurboModule> GestureHandlerHost::module(jsi::Runtime& runtime,
                                                         const std::shared_ptr<CallInvoker>& invoker) {
  // As RNGH's bindings installer: `_setGestureStateAsync` and the rest on the React Native runtime.
  gesturehandler::RNGHRuntimeDecorator::installRNRuntimeBindings(
      runtime, 0, [this](int tag, int state) { setState(tag, state); });
  auto module = std::make_shared<HostModule>("RNGestureHandlerModule", invoker);
  module->method("createGestureHandler", [this](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
    if (count >= 3) {
      configure(rt, static_cast<int>(args[1].asNumber()), args[2], true, args[0].asString(rt).utf8(rt), true);
    }
    return jsi::Value::undefined();
  });
  module->method("attachGestureHandler", [this](jsi::Runtime&, const jsi::Value* args, size_t count) {
    if (count < 3) return jsi::Value::undefined();
    int tag = static_cast<int>(args[0].asNumber());
    auto view = static_cast<Tag>(args[1].asNumber());
    int actionType = static_cast<int>(args[2].asNumber());
    // A handler not created (one not ported) was reported when it was.
    if (!names_.count(tag)) return jsi::Value::undefined();
    if (oxyos_gestures_attach(gestures_, tag, view) != 0) {
      reportOnce(oxyos_last_error());
      return jsi::Value::undefined();
    }
    actionTypes_[tag] = actionType;
    views_[tag] = view;
    if (actionType == kNativeAnimatedEvent) {
      reportOnce("Animated.event with the native driver is not available on OxyOS: " + names_[tag] +
                 " updates are not sent");
    } else if (actionType != kReanimatedWorklet && actionType != kJsFunctionOldApi &&
               actionType != kJsFunctionNewApi) {
      reportOnce("gesture handler action type " + std::to_string(actionType) +
                 " (the newer API's native detectors) is not available on OxyOS");
    }
    deliver();
    return jsi::Value::undefined();
  });
  module->method("setGestureHandlerConfig", [this](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
    int tag = count >= 2 ? static_cast<int>(args[0].asNumber()) : -1;
    if (names_.count(tag)) configure(rt, tag, args[1], false, names_[tag], true);
    return jsi::Value::undefined();
  });
  module->method("updateGestureHandlerConfig", [this](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
    int tag = count >= 2 ? static_cast<int>(args[0].asNumber()) : -1;
    if (names_.count(tag)) configure(rt, tag, args[1], false, names_[tag], false);
    return jsi::Value::undefined();
  });
  module->method("configureRelations", [this](jsi::Runtime& rt, const jsi::Value* args, size_t count) {
    int tag = count >= 2 ? static_cast<int>(args[0].asNumber()) : -1;
    if (!names_.count(tag) || !args[1].isObject()) return jsi::Value::undefined();
    auto relations = args[1].asObject(rt);
    auto waitFor = tags(rt, relations.getProperty(rt, "waitFor"));
    auto simultaneous = tags(rt, relations.getProperty(rt, "simultaneousHandlers"));
    auto blocks = tags(rt, relations.getProperty(rt, "blocksHandlers"));
    oxyos_gestures_relate(gestures_, tag, waitFor.data(), waitFor.size(), simultaneous.data(), simultaneous.size(),
                          blocks.data(), blocks.size());
    return jsi::Value::undefined();
  });
  module->method("dropGestureHandler", [this](jsi::Runtime&, const jsi::Value* args, size_t count) {
    if (count < 1) return jsi::Value::undefined();
    int tag = static_cast<int>(args[0].asNumber());
    oxyos_gestures_drop(gestures_, tag);
    names_.erase(tag);
    actionTypes_.erase(tag);
    views_.erase(tag);
    deliver();
    return jsi::Value::undefined();
  });
  // Operations take effect as they are called, as on Android.
  module->method("flushOperations", [](jsi::Runtime&, const jsi::Value*, size_t) { return jsi::Value::undefined(); });
  // `_setGestureStateSync` on the UI runtime, found as RNGH finds it (the holder JavaScript set).
  module->method("installUIRuntimeBindings", [this](jsi::Runtime& rt, const jsi::Value*, size_t) {
    auto resolved = gesturehandler::RNGHRuntimeDecorator::tryFindUIRuntime(rt);
    if (!resolved.runtime) return jsi::Value(false);
    gesturehandler::RNGHRuntimeDecorator::installUIRuntimeBindings(
        *resolved.runtime, [this](int tag, int state) { setState(tag, state); });
    return jsi::Value(true);
  });
  return module;
}

void GestureHandlerHost::configure(jsi::Runtime& rt, int tag, const jsi::Value& config, bool create,
                                   const std::string& name, bool reset) {
  std::vector<std::string> keys;
  std::vector<OxyOSGestureValue> values;
  std::map<std::string, std::vector<int32_t>> relations;
  if (config.isObject()) {
    auto object = config.asObject(rt);
    auto names = object.getPropertyNames(rt);
    for (size_t i = 0; i < names.size(rt); ++i) {
      std::string key = names.getValueAtIndex(rt, i).asString(rt).utf8(rt);
      auto value = object.getProperty(rt, key.c_str());
      // Relations count when a handler is created (Android's configureInteractions); its
      // factory ignores them after.
      if (kRelationKeys.count(key)) {
        if (create) relations[key] = tags(rt, value);
        continue;
      }
      OxyOSGestureValue setting{};
      if (value.isBool()) {
        setting.kind = OXYOS_GESTURE_BOOL;
        setting.number = value.getBool() ? 1 : 0;
      } else if (value.isNumber()) {
        setting.kind = OXYOS_GESTURE_NUMBER;
        setting.number = value.getNumber();
      } else if (value.isString()) {
        setting.kind = OXYOS_GESTURE_TEXT;
      } else if (value.isObject() && value.asObject(rt).isArray(rt)) {
        setting.kind = OXYOS_GESTURE_LIST;
        auto list = value.asObject(rt).asArray(rt);
        for (size_t at = 0; at < list.size(rt) && at < 6; ++at) {
          auto entry = list.getValueAtIndex(rt, at);
          if (!entry.isNumber()) continue;
          setting.list[at] = entry.getNumber();
          setting.list_set |= static_cast<uint8_t>(1u << at);
        }
      } else if (value.isNull() || value.isUndefined()) {
        setting.kind = OXYOS_GESTURE_NULL;
      } else {
        continue;
      }
      keys.push_back(std::move(key));
      values.push_back(setting);
    }
  }
  for (size_t i = 0; i < values.size(); ++i) {
    values[i].key = keys[i].data();
    values[i].key_length = keys[i].size();
  }
  std::vector<uint8_t> refused(values.size());
  int status = create ? oxyos_gestures_create(gestures_, name.data(), name.size(), tag, values.data(), values.size(),
                                              refused.data())
                      : oxyos_gestures_configure(gestures_, tag, values.data(), values.size(), reset, refused.data());
  if (status != 0) {
    reportOnce(oxyos_last_error());
    return;
  }
  if (create) names_[tag] = name;
  for (size_t i = 0; i < refused.size(); ++i) {
    if (!refused[i]) continue;
    reportOnce(name + " " + keys[i] + " is not available on OxyOS: only the primary mouse button reaches gesture handlers");
  }
  if (!relations.empty()) {
    auto& waitFor = relations["waitFor"];
    auto& simultaneous = relations["simultaneousHandlers"];
    auto& blocks = relations["blocksHandlers"];
    oxyos_gestures_relate(gestures_, tag, waitFor.data(), waitFor.size(), simultaneous.data(), simultaneous.size(),
                          blocks.data(), blocks.size());
  }
  // Disabling a handler that takes part cancels it.
  deliver();
}

void GestureHandlerHost::setState(int tag, int state) {
  oxyos_gestures_set_state(gestures_, tag, state);
  deliver();
}

uint8_t GestureHandlerHost::frameOf(void* context, int32_t view, OxyOSGestureFrame* out) {
  auto* self = static_cast<GestureHandlerHost*>(context);
  auto frame = windowFrame(self->tree_, view);
  if (!frame) return 0;
  *out = OxyOSGestureFrame{frame->origin.x, frame->origin.y, frame->size.width, frame->size.height};
  return 1;
}

bool GestureHandlerHost::reaches(Tag view, float x, float y) const {
  bool attached = false;
  for (const auto& [tag, at] : views_) attached |= at == view;
  if (!attached) return false;
  auto frame = windowFrame(tree_, view);
  if (!frame) return false;
  OxyOSGestureFrame bounds{frame->origin.x, frame->origin.y, frame->size.width, frame->size.height};
  return oxyos_gestures_reaches(gestures_, view, bounds, x, y) != 0;
}

void GestureHandlerHost::pointer(Phase phase, const SurfaceContent& content, float x, float y) {
  // The recognizers work in root coordinates, where React Native lays views out.
  OxyOSGesturePointer point{0, content.origin.x + x, content.origin.y + y};
  std::vector<int32_t> hits;
  if (phase == Phase::Down) {
    Tag target = gestureHitTest(tree_, content, x, y, [&](Tag view) { return reaches(view, point.x, point.y); });
    if (target >= 0) {
      auto path = ancestry(tree_, target);
      hits.assign(path.rbegin(), path.rend());
    }
  }
  static const uint8_t actions[] = {OXYOS_GESTURE_DOWN, OXYOS_GESTURE_MOVE, OXYOS_GESTURE_UP, OXYOS_GESTURE_CANCEL};
  // The pointer is a mouse's (RNGH's pointer type 2).
  if (oxyos_gestures_touch(gestures_, actions[static_cast<int>(phase)], 0, &point, 1, now(), 2, hits.data(),
                           hits.size(), &GestureHandlerHost::frameOf, this) != 0) {
    reportOnce(oxyos_last_error());
  }
  deliver();
}

void GestureHandlerHost::fingers(uint8_t action, size_t index, const SurfaceContent& content,
                                 const std::vector<Point>& points) {
  if (index >= points.size()) return;
  std::vector<OxyOSGesturePointer> pointers;
  for (size_t i = 0; i < points.size(); ++i) {
    pointers.push_back({static_cast<uint32_t>(i), content.origin.x + points[i].x, content.origin.y + points[i].y});
  }
  // A finger going down finds the handlers under it, as the first did.
  std::vector<int32_t> hits;
  if (action == OXYOS_GESTURE_DOWN || action == OXYOS_GESTURE_POINTER_DOWN) {
    const auto& point = pointers[index];
    Tag target = gestureHitTest(tree_, content, points[index].x, points[index].y,
                                [&](Tag view) { return reaches(view, point.x, point.y); });
    if (target >= 0) {
      auto path = ancestry(tree_, target);
      hits.assign(path.rbegin(), path.rend());
    }
  }
  if (oxyos_gestures_touch(gestures_, action, index, pointers.data(), pointers.size(), now(), 0, hits.data(),
                           hits.size(), &GestureHandlerHost::frameOf, this) != 0) {
    reportOnce(oxyos_last_error());
  }
  deliver();
}

std::optional<std::chrono::steady_clock::time_point> GestureHandlerHost::nextDeadline() const {
  int64_t at = oxyos_gestures_next_deadline(gestures_);
  if (at < 0) return std::nullopt;
  // now() counts milliseconds of the steady clock.
  return std::chrono::steady_clock::time_point(std::chrono::milliseconds(at));
}

bool GestureHandlerHost::tick() {
  oxyos_gestures_tick(gestures_, now());
  return deliver();
}

bool GestureHandlerHost::deliver() {
  bool delivered = false;
  OxyOSGestureOutput out{};
  // What is delivered may set states (a worklet's `manager.activate()`), adding outputs: they are
  // delivered in turn.
  while (oxyos_gestures_output(gestures_, &out) == 1) {
    delivered = true;
    auto& report = Report::get();
    auto handler = names_.count(out.tag) ? names_[out.tag] : std::string();
    if (out.kind == OXYOS_GESTURE_INTERCEPT) {
      report.gesture(folly::dynamic::object("event", "intercept"));
      if (onIntercept) onIntercept();
      continue;
    }
    if (out.kind == OXYOS_GESTURE_CANCEL_RESPONDER) {
      report.gesture(folly::dynamic::object("event", "cancel_responder")("handler", handler));
      if (onCancelResponder) onCancelResponder();
      continue;
    }
    if (out.kind == OXYOS_GESTURE_TOUCH) {
      deliverTouches(out, handler);
      continue;
    }
    bool stateChange = out.kind == OXYOS_GESTURE_STATE_CHANGE;
    folly::dynamic payload = folly::dynamic::object("handlerTag", out.tag)("state", out.state)(
        "numberOfPointers", out.number_of_pointers)("pointerType", out.pointer_type);
    if (stateChange) payload["oldState"] = out.old_state;
    if (out.fields & OXYOS_GESTURE_HAS_POSITION) {
      payload["x"] = out.x;
      payload["y"] = out.y;
      payload["absoluteX"] = out.absolute_x;
      payload["absoluteY"] = out.absolute_y;
    }
    if (out.fields & OXYOS_GESTURE_HAS_TRANSLATION) {
      payload["translationX"] = out.translation_x;
      payload["translationY"] = out.translation_y;
    }
    if (out.fields & OXYOS_GESTURE_HAS_VELOCITY) {
      payload["velocityX"] = out.velocity_x;
      payload["velocityY"] = out.velocity_y;
    }
    if (out.fields & OXYOS_GESTURE_HAS_DURATION) payload["duration"] = out.duration;
    if (out.fields & OXYOS_GESTURE_HAS_POINTER_INSIDE) payload["pointerInside"] = out.pointer_inside != 0;
    // A pinch's and a rotation's data, as RNGH's Android PinchGestureHandler and
    // RotationGestureHandler send it.
    if (out.fields & OXYOS_GESTURE_HAS_SCALE) {
      payload["scale"] = out.scale;
      payload["focalX"] = out.focal_x;
      payload["focalY"] = out.focal_y;
      payload["velocity"] = out.rate;
    }
    if (out.fields & OXYOS_GESTURE_HAS_ROTATION) {
      payload["rotation"] = out.rotation;
      payload["anchorX"] = out.anchor_x;
      payload["anchorY"] = out.anchor_y;
      payload["velocity"] = out.rate;
    }
    if (report.enabled()) {
      auto view = tree_.views().find(out.view);
      folly::dynamic line = payload;
      line["event"] = stateChange ? "state" : "update";
      line["handler"] = handler;
      line["view"] = view == tree_.views().end() ? std::to_string(out.view) : Report::name(view->second.view);
      line["state"] = stateName(out.state);
      if (stateChange) line["oldState"] = stateName(out.old_state);
      report.gesture(line);
    }
    std::string name = stateChange ? "onGestureHandlerStateChange" : "onGestureHandlerEvent";
    auto type = actionTypes_.find(out.tag);
    switch (type == actionTypes_.end() ? 0 : type->second) {
      case kReanimatedWorklet:
        if (delivery_.reanimatedEvent) delivery_.reanimatedEvent(name, out.view, payload);
        break;
      case kNativeAnimatedEvent:
        // State changes still reach JavaScript; updates were for the native driver (reported).
        if (stateChange && delivery_.deviceEvent) delivery_.deviceEvent(name, std::move(payload));
        break;
      case kJsFunctionOldApi:
      case kJsFunctionNewApi:
        if (delivery_.deviceEvent) delivery_.deviceEvent(name, std::move(payload));
        break;
      default:
        break;
    }
  }
  return delivered;
}

void GestureHandlerHost::deliverTouches(const OxyOSGestureOutput& out, const std::string& handler) {
  auto list = [this](uint8_t all) {
    std::vector<OxyOSGestureTouch> touches(oxyos_gestures_touches(gestures_, all, nullptr, 0));
    oxyos_gestures_touches(gestures_, all, touches.data(), touches.size());
    folly::dynamic array = folly::dynamic::array;
    for (const auto& touch : touches) {
      array.push_back(folly::dynamic::object("id", touch.id)("x", touch.x)("y", touch.y)("absoluteX", touch.absolute_x)(
          "absoluteY", touch.absolute_y));
    }
    return array;
  };
  // RNGestureHandlerTouchEvent's data.
  folly::dynamic payload = folly::dynamic::object("handlerTag", out.tag)("state", out.state)(
      "numberOfTouches", out.number_of_pointers)("eventType", out.event_type)("pointerType", out.pointer_type)(
      "changedTouches", list(0))("allTouches", list(1));
  auto& report = Report::get();
  if (report.enabled()) {
    static const char* types[] = {"", "down", "move", "up", "cancelled"};
    auto view = tree_.views().find(out.view);
    report.gesture(folly::dynamic::object("event", "touch")("type", out.event_type <= 4 ? types[out.event_type] : "?")(
        "handler", handler)("state", stateName(out.state))("touches", out.number_of_pointers)(
        "view", view == tree_.views().end() ? std::to_string(out.view) : Report::name(view->second.view)));
  }
  // As RNGH's dispatcher: worklets through Reanimated, the newer JavaScript API as device events.
  auto type = actionTypes_.find(out.tag);
  switch (type == actionTypes_.end() ? 0 : type->second) {
    case kReanimatedWorklet:
      if (delivery_.reanimatedEvent) delivery_.reanimatedEvent("onGestureHandlerEvent", out.view, payload);
      break;
    case kJsFunctionNewApi:
      if (delivery_.deviceEvent) delivery_.deviceEvent("onGestureHandlerEvent", std::move(payload));
      break;
    default:
      break;
  }
}

}  // namespace oxyos
