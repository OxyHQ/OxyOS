#include "animation-frames.h"

#include "report.h"

#include <react/timing/primitives.h>

using namespace facebook;

namespace oxyos {

AnimationFrames::AnimationFrames(react::RuntimeExecutor executor, const DisplayRefresh& refresh)
    : executor_(std::move(executor)), refresh_(refresh), state_(std::make_shared<State>()) {}

AnimationFrames::~AnimationFrames() {
  state_->waiting.clear();
  state_->running.clear();
}

void AnimationFrames::install(jsi::Runtime& runtime) {
  auto state = state_;
  runtime.global().setProperty(
      runtime, "requestAnimationFrame",
      jsi::Function::createFromHostFunction(
          runtime, jsi::PropNameID::forAscii(runtime, "requestAnimationFrame"), 1,
          [state](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
            if (count == 0 || !args[0].isObject() || !args[0].getObject(rt).isFunction(rt)) {
              throw jsi::JSError(rt, "The first argument to requestAnimationFrame must be a function.");
            }
            uint32_t handle = state->next++;
            state->waiting.emplace(handle, std::make_shared<jsi::Function>(args[0].getObject(rt).getFunction(rt)));
            return jsi::Value(static_cast<double>(handle));
          }));
  runtime.global().setProperty(
      runtime, "cancelAnimationFrame",
      jsi::Function::createFromHostFunction(
          runtime, jsi::PropNameID::forAscii(runtime, "cancelAnimationFrame"), 1,
          [state](jsi::Runtime&, const jsi::Value&, const jsi::Value* args, size_t count) {
            if (count > 0 && args[0].isNumber()) {
              auto handle = static_cast<uint32_t>(args[0].getNumber());
              state->waiting.erase(handle);
              state->running.erase(handle);
            }
            return jsi::Value::undefined();
          }));
}

bool AnimationFrames::tick(bool ready) {
  auto now = Clock::now();
  if (state_->waiting.empty() || !ready || (state_->last && now - *state_->last < refresh_.period())) return false;
  state_->last = now;
  // performance.now()'s clock: the steady clock, in milliseconds.
  double timestamp = react::HighResTimeStamp::fromChronoSteadyClockTimePoint(now).toDOMHighResTimeStamp();
  Report::get().animationFrame(timestamp, state_->waiting.size());
  for (auto& [handle, callback] : state_->waiting) {
    state_->running.emplace(handle, std::move(callback));
    // Each callback is a task of its own, as a browser runs them: its microtasks run before the next
    // callback, and one that throws is reported without keeping the others from running.
    executor_([state = state_, handle, timestamp](jsi::Runtime& runtime) {
      auto it = state->running.find(handle);
      if (it == state->running.end()) return;  // cancelled since the frame began
      auto callback = std::move(it->second);
      state->running.erase(it);
      callback->call(runtime, timestamp);
    });
  }
  state_->waiting.clear();
  return true;
}

std::optional<AnimationFrames::Clock::time_point> AnimationFrames::nextFrame(bool ready) const {
  if (state_->waiting.empty() || !ready) return std::nullopt;
  return state_->last ? *state_->last + refresh_.period() : Clock::now();
}

}  // namespace oxyos
