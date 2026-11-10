#include "js-thread.h"

#include "scheduling.h"
#include "ui-thread.h"

#include <algorithm>
#include <csignal>
#include <exception>
#include <stdexcept>
#include <utility>
#include <vector>

#include <sys/prctl.h>

namespace oxyos {

namespace {

// As large as the main thread's (its RLIMIT_STACK on Debian): React's recursion through a deep tree
// and Hermes' own frames need what JavaScript had on the main thread.
constexpr size_t kStackSize = 8 * 1024 * 1024;

thread_local bool tJsThread = false;
// What started the stretch the thread is in: a task it queues itself (a promise's continuation, a
// timer it set) is part of it.
thread_local JsThread::Cause tCause;
// What another thread's tasks are for while a CausedBy lives there.
thread_local std::optional<JsThread::Cause> tQueuedFor;

}  // namespace

JsThread::CausedBy::CausedBy(Cause cause) : previous_(std::exchange(tQueuedFor, std::move(cause))) {}

JsThread::CausedBy::~CausedBy() { tQueuedFor = std::move(previous_); }

JsThread::JsThread(std::function<Cause()> cause, std::function<void(const Stretch&)> worked, std::function<void()> idle,
                   std::function<void(bool)> settled)
    : cause_(std::move(cause)),
      worked_(std::move(worked)),
      idleHook_(std::move(idle)),
      settledHook_(std::move(settled)) {
  pthread_attr_t attributes;
  pthread_attr_init(&attributes);
  pthread_attr_setstacksize(&attributes, kStackSize);
  int failed = pthread_create(&thread_, &attributes, &JsThread::main, this);
  pthread_attr_destroy(&attributes);
  if (failed) throw std::runtime_error("the JavaScript thread could not be started");
}

JsThread::~JsThread() { quitSynchronous(); }

void* JsThread::main(void* self) {
  // Its name, as the crash report and the profilers tell threads apart (docs/bloom-host-sanitizers.mdx).
  prctl(PR_SET_NAME, "oxyos-js", 0, 0, 0);
  tJsThread = true;
  // Input waits on it as on the UI thread: its own short slice (scheduling.h).
  requestInteractiveSlice();
  // The fatal signals' report runs on a stack of its own, so that a stack overflow here is reported
  // too (crash-report.h): an alternate stack is the thread's own, never inherited.
  static thread_local std::vector<char> alternate(std::max<size_t>(SIGSTKSZ, 64 * 1024));
  stack_t stack{};
  stack.ss_sp = alternate.data();
  stack.ss_size = alternate.size();
  sigaltstack(&stack, nullptr);
  static_cast<JsThread*>(self)->serve();
  return nullptr;
}

bool JsThread::isCurrent() const { return tJsThread; }

void JsThread::runOnQueue(std::function<void()>&& task) {
  Cause cause = tJsThread ? tCause : tQueuedFor ? *tQueuedFor : cause_ ? cause_() : Cause{};
  bool staged = UiThread::isCurrent();
  {
    std::lock_guard lock(mutex_);
    (staged ? staged_ : tasks_).push_back(Task{std::move(task), std::move(cause)});
  }
  if (!staged) wake_.notify_one();
}

void JsThread::runOnQueueFirst(std::function<void()>&& task) {
  Cause cause = tQueuedFor ? *tQueuedFor : cause_ ? cause_() : Cause{};
  {
    std::lock_guard lock(mutex_);
    tasks_.push_front(Task{std::move(task), std::move(cause)});
  }
  wake_.notify_one();
}

bool JsThread::handOver() {
  {
    std::lock_guard lock(mutex_);
    if (staged_.empty() || running_ || !tasks_.empty()) return false;
    handOverLocked();
  }
  wake_.notify_one();
  return true;
}

void JsThread::handOverLocked() {
  if (staged_.empty()) return;
  for (auto& task : staged_) tasks_.push_back(std::move(task));
  staged_.clear();
  handed_ = batch_.fetch_add(1);
}

void JsThread::runOnQueueSync(std::function<void()>&& task) {
  if (isCurrent()) {
    task();
    return;
  }
  std::mutex done;
  std::condition_variable doneChanged;
  bool finished = false;
  std::exception_ptr failed;
  {
    // Run whatever the thread is doing: it is waited for, the loop's work queued before it included.
    std::lock_guard lock(mutex_);
    handOverLocked();
    tasks_.push_back(Task{[&] {
                            try {
                              task();
                            } catch (...) {
                              failed = std::current_exception();
                            }
                            std::lock_guard lock(done);
                            finished = true;
                            doneChanged.notify_all();
                          },
                          Cause{}});
  }
  wake_.notify_one();
  {
    UiThread::Unlocked unlocked;
    std::unique_lock lock(done);
    doneChanged.wait(lock, [&] { return finished; });
  }
  if (failed) std::rethrow_exception(failed);
}

bool JsThread::idle() const {
  std::lock_guard lock(mutex_);
  // What the loop queued and has not handed over yet is the loop's: JavaScript is idle without it.
  if (running_ || !tasks_.empty()) return false;
  auto due = timers_ ? timers_->next() : std::nullopt;
  return !due || *due > Clock::now();
}

int JsThread::stagedRank() const {
  std::lock_guard lock(mutex_);
  int rank = -1;
  for (const auto& task : staged_) rank = std::max(rank, task.cause.rank);
  return rank;
}

bool JsThread::waitIdle(Clock::time_point deadline) {
  UiThread::Unlocked unlocked;
  std::unique_lock lock(mutex_);
  // What the loop queued runs too.
  handOverLocked();
  wake_.notify_one();
  return settled_.wait_until(lock, deadline, [this] { return quit_ || (!running_ && tasks_.empty()); });
}

bool JsThread::next(Task& task, std::unique_lock<std::mutex>& lock) {
  while (true) {
    if (quit_) return false;
    if (!tasks_.empty()) {
      task = std::move(tasks_.front());
      tasks_.pop_front();
      return true;
    }
    // A timer due fires: its callback is queued (TimerManager::callTimer, through React Native's
    // scheduler, which queues here), so the lock is let go meanwhile.
    if (timers_ && timerManager_) {
      auto due = timers_->due(Clock::now());
      if (!due.empty()) {
        // Their callbacks are this stretch's when it runs one already, else a timer's.
        if (!running_) tCause = Cause{"a timer", 2};
        lock.unlock();
        for (auto id : due) timerManager_->callTimer(static_cast<facebook::react::TimerHandle>(id));
        lock.lock();
        continue;
      }
    }
    if (running_) return false;  // the stretch ends
    auto wakeAt = timers_ ? timers_->next() : std::nullopt;
    if (wakeAt) {
      wake_.wait_until(lock, *wakeAt);
    } else {
      wake_.wait(lock);
    }
  }
}

void JsThread::serve() {
  std::unique_lock lock(mutex_);
  while (true) {
    Task task;
    if (!next(task, lock)) {
      if (quit_) return;
      continue;
    }
    // A stretch of work begins: it lasts until the thread is idle again.
    running_ = true;
    Stretch stretch;
    if (task.cause.rank > tCause.rank || tCause.what.empty()) tCause = task.cause;
    lock.unlock();
    auto started = ThreadSample::now();
    while (true) {
      task.run();
      task.run = nullptr;
      ++stretch.tasks;
      lock.lock();
      bool more = next(task, lock);
      if (!more && !quit_ && idleHook_) {
        // About to be idle: what the commits dispatched meanwhile (Fabric's event beat) may queue
        // more, which this stretch runs.
        lock.unlock();
        idleHook_();
        lock.lock();
        more = next(task, lock);
      }
      if (!more) break;
      if (task.cause.rank > tCause.rank) tCause = task.cause;
      lock.unlock();
    }
    lock.unlock();
    stretch.span = ThreadSpan::between(started, ThreadSample::now());
    stretch.cause = std::exchange(tCause, Cause{});
    if (worked_) worked_(stretch);
    lock.lock();
    running_ = false;
    // Idle: every batch handed over has been run, with all it set off.
    if (tasks_.empty()) done_ = handed_.load();
    settled_.notify_all();
    if (quit_) return;
    if (settledHook_ && tasks_.empty()) {
      bool waiting = !staged_.empty();
      lock.unlock();
      settledHook_(waiting);
      lock.lock();
    }
  }
}

void JsThread::quitSynchronous() {
  {
    std::lock_guard lock(mutex_);
    if (joined_) return;
    quit_ = true;
    joined_ = true;
  }
  wake_.notify_all();
  settled_.notify_all();
  if (isCurrent()) {
    // Asked from a task: the thread ends as that task returns, and is never joined by itself.
    pthread_detach(thread_);
    return;
  }
  UiThread::Unlocked unlocked;
  pthread_join(thread_, nullptr);
}

std::unique_ptr<JsThread::Timers> JsThread::makeTimers() {
  auto timers = std::make_unique<Timers>(*this);
  std::lock_guard lock(mutex_);
  timers_ = timers.get();
  return timers;
}

void JsThread::fireTimersThrough(facebook::react::TimerManager& manager) {
  std::lock_guard lock(mutex_);
  timerManager_ = &manager;
}

void JsThread::Timers::add(uint32_t id, double delayMs, bool repeat) {
  auto every = std::chrono::duration_cast<Clock::duration>(std::chrono::duration<double, std::milli>(delayMs));
  {
    std::lock_guard lock(mutex_);
    timers_[id] = Timer{Clock::now() + every, every, repeat};
  }
  // Set on the thread itself, by JavaScript: its wait is reckoned again as the task ends. From
  // anywhere else, it is woken to reckon it.
  if (!thread_.isCurrent()) thread_.wake_.notify_one();
}

void JsThread::Timers::deleteTimer(uint32_t id) {
  std::lock_guard lock(mutex_);
  timers_.erase(id);
}

std::vector<uint32_t> JsThread::Timers::due(Clock::time_point now) {
  std::lock_guard lock(mutex_);
  std::vector<uint32_t> due;
  for (auto it = timers_.begin(); it != timers_.end();) {
    if (it->second.at > now) {
      ++it;
      continue;
    }
    due.push_back(it->first);
    if (it->second.repeat) {
      it->second.at = now + it->second.every;
      ++it;
    } else {
      it = timers_.erase(it);
    }
  }
  return due;
}

std::optional<JsThread::Clock::time_point> JsThread::Timers::next() const {
  std::lock_guard lock(mutex_);
  std::optional<Clock::time_point> next;
  for (const auto& [id, timer] : timers_) {
    if (!next || timer.at < *next) next = timer.at;
  }
  return next;
}

bool JsThread::Timers::oneShotDueBefore(Clock::time_point deadline) const {
  std::lock_guard lock(mutex_);
  for (const auto& [id, timer] : timers_) {
    if (!timer.repeat && timer.at < deadline) return true;
  }
  return false;
}

}  // namespace oxyos
