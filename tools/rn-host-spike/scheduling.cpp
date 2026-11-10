#include "scheduling.h"

#include <sys/syscall.h>
#include <unistd.h>

#include <cerrno>
#include <cstdint>
#include <cstring>
#include <iostream>

namespace oxyos {

namespace {

// `struct sched_attr` as far as SCHED_ATTR_SIZE_VER0 (linux/sched/types.h), declared here as
// oxyos-priority declares it: glibc's own wrapper is too recent for every system the host builds on.
struct SchedAttr {
  uint32_t size;
  uint32_t policy;
  uint64_t flags;
  int32_t nice;
  uint32_t priority;
  uint64_t runtime;
  uint64_t deadline;
  uint64_t period;
};

constexpr uint32_t kSchedAttrSize = 48;
static_assert(sizeof(SchedAttr) == kSchedAttrSize);
constexpr uint64_t kResetOnFork = 0x01;
constexpr uint64_t kSliceNs = 1000000;
// SCHED_OTHER, SCHED_BATCH, SCHED_IDLE: the fair scheduler's policies, the ones with a slice.
constexpr uint32_t kOther = 0, kBatch = 3, kIdle = 5;

}  // namespace

void requestInteractiveSlice() {
  SchedAttr current{};
  if (syscall(SYS_sched_getattr, 0, &current, kSchedAttrSize, 0) != 0) {
    std::cerr << "[host] a thread keeps its slice: " << std::strerror(errno) << "\n";
    return;
  }
  if (current.policy != kOther && current.policy != kBatch && current.policy != kIdle) return;
  SchedAttr wanted{};
  wanted.size = kSchedAttrSize;
  wanted.policy = current.policy;
  wanted.flags = (current.flags & kResetOnFork) | kResetOnFork;
  wanted.nice = current.nice;
  wanted.runtime = kSliceNs;
  if (syscall(SYS_sched_setattr, 0, &wanted, 0) != 0) {
    std::cerr << "[host] a thread keeps its slice: " << std::strerror(errno) << "\n";
  }
}

}  // namespace oxyos
