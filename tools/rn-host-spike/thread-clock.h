// The calling thread's own time, beside the wall clock: what the host's loop (where JavaScript, Yoga
// and the text it measures run) spent on a CPU, and what it spent off one, told apart as the kernel
// tells it (frame-diagnostics.h, report.h; docs/performance.mdx, "Wall time and CPU time").
//
// Between two samples of the same thread, the wall clock's time is exactly
//
//   cpu        the thread running (CLOCK_THREAD_CPUTIME_ID): its own work, however loaded the
//              machine; a CPU shared with a sibling hyperthread or a slower core still runs it slower;
//   preempted  the thread runnable, waiting for a CPU another thread had (its schedstat's
//              run_delay): the machine's load, not the thread's work;
//   blocked    the rest: the thread asleep until something woke it, another thread it waits for
//              (a lock, a painter, a garbage collector's), a file, the compositor's answer.
//
// so work another thread does while this one waits for it is never lost: it is this thread's
// `blocked`, in the part that waited. Where /proc has no schedstat (a kernel without
// CONFIG_SCHED_INFO), nothing is said to be preempted and all time off the CPU is `blocked`.
//
// A sample costs two system calls (the thread's CPU clock and a read of its schedstat), well under a
// microsecond each.
#pragma once

#include <chrono>
#include <cstdint>

namespace oxyos {

struct ThreadSample {
  std::chrono::steady_clock::time_point wall{};
  int64_t cpuNs = 0;
  int64_t runDelayNs = -1;  // -1: unknown (no schedstat)

  static ThreadSample now();
};

// What a thread did between two of its samples, in milliseconds: wallMs = cpuMs + preemptedMs + blockedMs.
struct ThreadSpan {
  double wallMs = 0, cpuMs = 0, preemptedMs = 0, blockedMs = 0;

  static ThreadSpan between(const ThreadSample& from, const ThreadSample& to);
  ThreadSpan& operator+=(const ThreadSpan& other) {
    wallMs += other.wallMs;
    cpuMs += other.cpuMs;
    preemptedMs += other.preemptedMs;
    blockedMs += other.blockedMs;
    return *this;
  }
};

}  // namespace oxyos
