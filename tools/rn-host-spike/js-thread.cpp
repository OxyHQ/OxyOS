#include "js-thread.h"

#include <algorithm>
#include <atomic>
#include <csignal>
#include <iostream>
#include <stdexcept>
#include <utility>
#include <vector>

#include <sys/prctl.h>
#include <unistd.h>

namespace oxyos {

namespace {

// As large as the main thread's (its RLIMIT_STACK on Debian): React's recursion through a deep tree
// and Hermes' own frames need what JavaScript had on the main thread.
constexpr size_t kStackSize = 8 * 1024 * 1024;

thread_local bool tJsThread = false;
// The JavaScript thread's id, for a signal handler: lock-free, so async-signal-safe.
std::atomic<pid_t> gJsThreadId{0};

}  // namespace

JsThread::JsThread() {
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
  gJsThreadId.store(static_cast<pid_t>(gettid()), std::memory_order_relaxed);
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

pid_t jsThreadId() { return gJsThreadId.load(std::memory_order_relaxed); }

void JsThread::serve() {
  std::unique_lock lock(mutex_);
  while (true) {
    asked_.wait(lock, [this] { return quit_ || job_; });
    if (quit_) return;
    auto job = std::move(job_);
    job_ = nullptr;
    lock.unlock();
    std::exception_ptr failed;
    try {
      job();
    } catch (...) {
      failed = std::current_exception();
    }
    lock.lock();
    jobFailed_ = failed;
    jobDone_ = true;
    done_.notify_all();
  }
}

void JsThread::perform(std::function<void()> job) {
  std::unique_lock lock(mutex_);
  if (quit_) return;
  job_ = std::move(job);
  jobDone_ = false;
  asked_.notify_one();
  done_.wait(lock, [this] { return jobDone_; });
  if (auto failed = std::exchange(jobFailed_, nullptr)) std::rethrow_exception(failed);
}

void JsThread::runOnQueue(std::function<void()>&& task) {
  std::lock_guard lock(mutex_);
  tasks_.push_back(std::move(task));
}

void JsThread::runOnQueueSync(std::function<void()>&& task) {
  if (isCurrent()) {
    task();
    return;
  }
  perform(std::move(task));
}

JsThread::Work JsThread::drain() {
  Work work;
  perform([this, &work] {
    auto started = ThreadSample::now();
    while (true) {
      std::function<void()> task;
      {
        std::lock_guard lock(mutex_);
        if (tasks_.empty()) break;
        task = std::move(tasks_.front());
        tasks_.pop_front();
      }
      task();
      work.ran = true;
    }
    work.span = ThreadSpan::between(started, ThreadSample::now());
  });
  return work;
}

void JsThread::quitSynchronous() {
  {
    std::lock_guard lock(mutex_);
    if (joined_) return;
    quit_ = true;
    joined_ = true;
    asked_.notify_all();
  }
  if (isCurrent()) {
    // Asked from a task: the thread ends as that task returns, and is never joined by itself.
    pthread_detach(thread_);
    return;
  }
  pthread_join(thread_, nullptr);
}

void JsThread::Timers::add(uint32_t id, double delayMs, bool repeat) {
  auto every = std::chrono::duration_cast<Clock::duration>(std::chrono::duration<double, std::milli>(delayMs));
  std::lock_guard lock(mutex_);
  timers_[id] = Timer{Clock::now() + every, every, repeat};
}

void JsThread::Timers::deleteTimer(uint32_t id) {
  std::lock_guard lock(mutex_);
  timers_.erase(id);
}

bool JsThread::Timers::fire(facebook::react::TimerManager& manager) {
  std::vector<uint32_t> due;
  {
    std::lock_guard lock(mutex_);
    auto now = Clock::now();
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
  }
  // Each queues its callback (TimerManager::callTimer), which runs with the queue's next drain.
  for (auto id : due) manager.callTimer(static_cast<facebook::react::TimerHandle>(id));
  return !due.empty();
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
