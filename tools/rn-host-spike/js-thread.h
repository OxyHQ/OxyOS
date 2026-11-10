// The JavaScript thread: React Native's runtime (Hermes), React and Fabric's commits run on a thread
// of their own, `oxyos-js`, as React Native's Android and iOS hosts run them on theirs; the host's
// loop, on the process's main thread, is the UI thread. The runtime is made on this thread (Hermes'
// sampling profiler and its stack's guard are the thread that makes it), with a stack of its own as
// large as the main thread's and the crash report's stack for a fault on it (crash-report.h).
//
// React Native's queue (`MessageQueueThread`) and its timers (`PlatformTimerRegistry`) are this
// thread's. For now the loop and the thread run in lockstep: what is queued runs when the loop asks
// (`drain`), and the loop waits for it, as the single thread did; every order of the loop's turn
// is the one it had.
#pragma once

#include "thread-clock.h"

#include <cxxreact/MessageQueueThread.h>
#include <react/runtime/PlatformTimerRegistry.h>
#include <react/runtime/TimerManager.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <exception>
#include <functional>
#include <map>
#include <mutex>
#include <optional>

#include <pthread.h>

namespace oxyos {

class JsThread : public facebook::react::MessageQueueThread {
 public:
  using Clock = std::chrono::steady_clock;

  // Starts the thread, which waits for work.
  JsThread();
  // Ends it (quitSynchronous).
  ~JsThread() override;
  JsThread(const JsThread&) = delete;
  JsThread& operator=(const JsThread&) = delete;

  // React Native's queue: tasks run on the thread in the order they came, when the loop drains it.
  void runOnQueue(std::function<void()>&& task) override;
  // Runs `task` on the thread and returns once it ran: at once when called on the thread itself.
  // An exception it throws is thrown here.
  void runOnQueueSync(std::function<void()>&& task) override;
  // Ends the thread once what it runs now is done; what is still queued never runs.
  void quitSynchronous() override;

  // Whether the caller is this thread.
  bool isCurrent() const;

  // What a drain ran: whether any task, and the thread's time doing it (thread-clock.h), measured
  // on the thread.
  struct Work {
    bool ran = false;
    ThreadSpan span;
  };
  // Runs every task queued, those they queue included, on the thread; returns once they ran.
  Work drain();

  // React Native's timers (TimerManager's platform registry). The loop fires those due; firing one
  // queues its callback, which the next drain runs.
  class Timers : public facebook::react::PlatformTimerRegistry {
   public:
    void createTimer(uint32_t id, double delayMs) override { add(id, delayMs, false); }
    void createRecurringTimer(uint32_t id, double delayMs) override { add(id, delayMs, true); }
    void deleteTimer(uint32_t id) override;

    // Fires those due; whether any was.
    bool fire(facebook::react::TimerManager& manager);
    // When the next falls due, if one is set.
    std::optional<Clock::time_point> next() const;
    // Whether a one-shot timer falls due before `deadline`: the application is not settled until
    // it fires. Recurring timers never settle, so they do not hold it.
    bool oneShotDueBefore(Clock::time_point deadline) const;
    // App Nap's timer throttling (docs/adr/0066-quality-of-service.mdx, decision 5): while set,
    // timers fall due only on whole multiples of `align` of the steady clock, every timer due by
    // then together, as a browser runs a hidden page's timers once a second: a napping application
    // wakes at most once an `align` for them, however many it set and however short.
    void throttle(std::optional<Clock::duration> align);

   private:
    struct Timer {
      Clock::time_point at;
      Clock::duration every;
      bool repeat;
    };
    void add(uint32_t id, double delayMs, bool repeat);
    mutable std::mutex mutex_;
    std::map<uint32_t, Timer> timers_;
    std::optional<Clock::duration> align_;
  };

 private:
  static void* main(void* self);
  void serve();
  // Runs `job` on the thread and waits for it.
  void perform(std::function<void()> job);

  mutable std::mutex mutex_;
  std::condition_variable asked_, done_;
  std::deque<std::function<void()>> tasks_;
  // What the loop asked the thread to run, and whether it has.
  std::function<void()> job_;
  bool jobDone_ = false;
  std::exception_ptr jobFailed_;
  bool quit_ = false, joined_ = false;
  pthread_t thread_{};
};

}  // namespace oxyos
