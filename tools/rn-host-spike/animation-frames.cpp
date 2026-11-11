#include "animation-frames.h"

#include "report.h"
#include "ui-thread.h"

#include <react/timing/primitives.h>

using namespace facebook;

namespace oxyos {

AnimationFrames::AnimationFrames(react::RuntimeExecutor executor, const DisplayRefresh& refresh)
    : executor_(std::move(executor)), refresh_(refresh), state_(std::make_shared<State>()) {}

AnimationFrames::~AnimationFrames() {
  std::lock_guard lock(state_->mutex);
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
            auto callback = std::make_shared<jsi::Function>(args[0].getObject(rt).getFunction(rt));
            uint32_t handle;
            bool first;
            {
              std::lock_guard lock(state->mutex);
              handle = state->next++;
              first = state->waiting.empty();
              state->waiting.emplace(handle, std::move(callback));
            }
            // The frame clock is the UI thread's (ui-thread.h): told of the first callback waiting.
            if (first) UiThread::wake();
            return jsi::Value(static_cast<double>(handle));
          }));
  runtime.global().setProperty(
      runtime, "cancelAnimationFrame",
      jsi::Function::createFromHostFunction(
          runtime, jsi::PropNameID::forAscii(runtime, "cancelAnimationFrame"), 1,
          [state](jsi::Runtime&, const jsi::Value&, const jsi::Value* args, size_t count) {
            if (count > 0 && args[0].isNumber()) {
              auto handle = static_cast<uint32_t>(args[0].getNumber());
              std::lock_guard lock(state->mutex);
              state->waiting.erase(handle);
              state->running.erase(handle);
            }
            return jsi::Value::undefined();
          }));
}

bool AnimationFrames::tick(bool ready) {
  auto now = Clock::now();
  std::vector<uint32_t> handles;
  {
    std::lock_guard lock(state_->mutex);
    if (state_->waiting.empty() || !ready || (state_->last && now - *state_->last < refresh_.period())) return false;
    state_->last = now;
    for (auto& [handle, callback] : state_->waiting) {
      state_->running.emplace(handle, std::move(callback));
      handles.push_back(handle);
    }
    state_->waiting.clear();
  }
  // performance.now()'s clock: the steady clock, in milliseconds.
  double timestamp = react::HighResTimeStamp::fromChronoSteadyClockTimePoint(now).toDOMHighResTimeStamp();
  Report::get().animationFrame(timestamp, handles.size());
  for (auto handle : handles) {
    // Each callback is a task of its own, as a browser runs them: its microtasks run before the next
    // callback, and one that throws is reported without keeping the others from running.
    executor_([state = state_, handle, timestamp](jsi::Runtime& runtime) {
      std::shared_ptr<jsi::Function> callback;
      {
        std::lock_guard lock(state->mutex);
        auto it = state->running.find(handle);
        if (it == state->running.end()) return;  // cancelled since the frame began
        callback = std::move(it->second);
        state->running.erase(it);
      }
      callback->call(runtime, timestamp);
    });
  }
  return true;
}

std::optional<AnimationFrames::Clock::time_point> AnimationFrames::nextFrame(bool ready) const {
  std::lock_guard lock(state_->mutex);
  if (state_->waiting.empty() || !ready) return std::nullopt;
  return state_->last ? *state_->last + refresh_.period() : Clock::now();
}

}  // namespace oxyos
