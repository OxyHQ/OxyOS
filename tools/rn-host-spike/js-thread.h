// The JavaScript thread (docs/adr/0067-the-ui-thread.mdx): React Native's runtime (Hermes), React and
// Fabric's commits run on a thread of their own, `oxyos-js`, as React Native's Android and iOS hosts
// run them on theirs; the host's loop, on the process's main thread, is the UI thread (ui-thread.h).
// The runtime is made on this thread (Hermes' sampling profiler and its stack's guard are the thread
// that makes it), with a stack as large as the main thread's and the crash report's stack for a
// fault on it (crash-report.h).
//
// React Native's queue (`MessageQueueThread`) and its timers (`PlatformTimerRegistry`) are this
// thread's: a task runs as soon as the thread is free, a timer fires on the thread when it is due, and
// the thread sleeps when it has neither. The loop never waits for it: what JavaScript commits reaches
// the loop as a mount it pulls (bloom-host.cpp's Mounter), what the loop has for JavaScript (input
// through Fabric's event beat, device events, requestAnimationFrame's frames) is queued here.
#pragma once

#include "thread-clock.h"

#include <cxxreact/MessageQueueThread.h>
#include <react/runtime/PlatformTimerRegistry.h>
#include <react/runtime/TimerManager.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <functional>
#include <map>
#include <mutex>
#include <optional>
#include <string>

#include <pthread.h>

namespace oxyos {

class JsThread : public facebook::react::MessageQueueThread {
 public:
  using Clock = std::chrono::steady_clock;

  // What queued a task, for the diagnostics of the stretch of work that runs it (frame-diagnostics.h):
  // the input, the timer, the frame. `rank` orders them as FrameDiagnostics::trigger does.
  struct Cause {
    std::string what;
    int rank = -1;
  };
  // A stretch of work: from the thread taking work up while idle to its being idle again.
  struct Stretch {
    ThreadSpan span;
    Cause cause;
    int tasks = 0;
  };

  // Starts the thread. `cause` names what queues a task from another thread (the loop's turn);
  // `worked` is called on the thread after each stretch; `idle` on the thread as it is about to
  // sleep, which may queue more (Fabric's beat for what its commits dispatched); `settled` on the
  // thread once it is idle (the loop, waiting for JavaScript to settle, is woken).
  JsThread(std::function<Cause()> cause, std::function<void(const Stretch&)> worked, std::function<void()> idle,
           std::function<void()> settled);
  // Ends it (quitSynchronous).
  ~JsThread() override;
  JsThread(const JsThread&) = delete;
  JsThread& operator=(const JsThread&) = delete;

  // React Native's queue: tasks run on the thread in the order they came. What the loop queues during
  // its turn (the UI thread: its events through Fabric's beat, device events, a frame's callbacks)
  // is handed over together as the turn ends (handOver), as one turn's work reached JavaScript
  // together when the two took turns: React then batches what a turn's events change into one render.
  void runOnQueue(std::function<void()>&& task) override;
  // The loop's turn ends: what it queued runs.
  void handOver();
  // Runs `task` on the thread after what is queued, and returns once it ran: at once when called on
  // the thread itself. The UI thread's state is let go meanwhile (ui-thread.h), since the task, or
  // what runs before it, may need it. An exception it throws is thrown here.
  void runOnQueueSync(std::function<void()>&& task) override;
  // Ends the thread once what it runs now is done; what is still queued never runs.
  void quitSynchronous() override;

  // Whether the caller is this thread.
  bool isCurrent() const;

  // The tasks queued so far and those run so far: a task queued when `queued()` was N has run once
  // `ran()` reaches N.
  uint64_t queued() const { return queued_.load(); }
  uint64_t ran() const { return ran_.load(); }
  // Nothing queued, nothing running, no timer due.
  bool idle() const;
  // Waits until the thread is idle (ui-thread.h's state let go meanwhile), or until `deadline`.
  bool waitIdle(Clock::time_point deadline);

  // React Native's timers (TimerManager's platform registry), fired on the thread: firing one queues
  // its callback (TimerManager::callTimer).
  class Timers : public facebook::react::PlatformTimerRegistry {
   public:
    explicit Timers(JsThread& thread) : thread_(thread) {}
    void createTimer(uint32_t id, double delayMs) override { add(id, delayMs, false); }
    void createRecurringTimer(uint32_t id, double delayMs) override { add(id, delayMs, true); }
    void deleteTimer(uint32_t id) override;

    // Whether a one-shot timer falls due before `deadline`: the application is not settled until
    // it fires. Recurring timers never settle, so they do not hold it.
    bool oneShotDueBefore(Clock::time_point deadline) const;

   private:
    friend class JsThread;
    struct Timer {
      Clock::time_point at;
      Clock::duration every;
      bool repeat;
    };
    void add(uint32_t id, double delayMs, bool repeat);
    // Those due, taken (a recurring one set for its next time); when the next is due.
    std::vector<uint32_t> due(Clock::time_point now);
    std::optional<Clock::time_point> next() const;
    JsThread& thread_;
    mutable std::mutex mutex_;
    std::map<uint32_t, Timer> timers_;
  };
  // The registry for React Native's TimerManager, and the manager its timers fire through: set
  // once, before JavaScript runs.
  std::unique_ptr<Timers> makeTimers();
  void fireTimersThrough(facebook::react::TimerManager& manager);

 private:
  struct Task {
    std::function<void()> run;
    Cause cause;
  };
  static void* main(void* self);
  void serve();
  // Takes the next task, or waits for one (or a timer due); false once quitting.
  bool next(Task& task, std::unique_lock<std::mutex>& lock);

  std::function<Cause()> cause_;
  std::function<void(const Stretch&)> worked_;
  std::function<void()> idleHook_, settledHook_;
  mutable std::mutex mutex_;
  std::condition_variable wake_, settled_;
  std::deque<Task> tasks_, staged_;
  bool running_ = false, quit_ = false, joined_ = false;
  std::atomic<uint64_t> queued_{0}, ran_{0};
  Timers* timers_ = nullptr;
  facebook::react::TimerManager* timerManager_ = nullptr;
  pthread_t thread_{};
};

}  // namespace oxyos
