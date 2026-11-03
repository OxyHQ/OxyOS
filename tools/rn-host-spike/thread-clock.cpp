#include "thread-clock.h"

#include <algorithm>
#include <cstdlib>
#include <ctime>
#include <fcntl.h>
#include <unistd.h>

namespace oxyos {

namespace {

// This thread's schedstat ("sum_exec_runtime run_delay timeslices", nanoseconds), opened once per
// thread: -1 when the kernel has none.
int schedstat() {
  thread_local int fd = ::open("/proc/thread-self/schedstat", O_RDONLY | O_CLOEXEC);
  return fd;
}

}  // namespace

ThreadSample ThreadSample::now() {
  ThreadSample sample;
  sample.wall = std::chrono::steady_clock::now();
  timespec cpu{};
  clock_gettime(CLOCK_THREAD_CPUTIME_ID, &cpu);
  sample.cpuNs = static_cast<int64_t>(cpu.tv_sec) * 1'000'000'000 + cpu.tv_nsec;
  // run_delay counts a wait for a CPU once the thread has one again, so read from the running
  // thread it holds every wait before this sample.
  if (int fd = schedstat(); fd >= 0) {
    char text[96];
    ssize_t n = ::pread(fd, text, sizeof text - 1, 0);
    if (n > 0) {
      text[n] = '\0';
      char* end = nullptr;
      std::strtoll(text, &end, 10);  // sum_exec_runtime: the CPU clock above is exact, this is not
      if (end != text) sample.runDelayNs = std::strtoll(end, nullptr, 10);
    }
  }
  return sample;
}

ThreadSpan ThreadSpan::between(const ThreadSample& from, const ThreadSample& to) {
  ThreadSpan span;
  span.wallMs = std::max(0.0, std::chrono::duration<double, std::milli>(to.wall - from.wall).count());
  // The clocks are read a few hundred nanoseconds apart: the CPU's time is kept within the wall's,
  // and the preemption within what was off the CPU, so that the three parts add up to the wall's.
  span.cpuMs = std::clamp(static_cast<double>(to.cpuNs - from.cpuNs) / 1e6, 0.0, span.wallMs);
  double off = span.wallMs - span.cpuMs;
  if (from.runDelayNs >= 0 && to.runDelayNs >= 0) {
    span.preemptedMs = std::clamp(static_cast<double>(to.runDelayNs - from.runDelayNs) / 1e6, 0.0, off);
  }
  span.blockedMs = off - span.preemptedMs;
  return span;
}

}  // namespace oxyos
