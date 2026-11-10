#include "ui-thread.h"

#include <cerrno>
#include <cstdint>
#include <mutex>

#include <sys/eventfd.h>
#include <unistd.h>

namespace oxyos {

namespace {

// The state's lock, and how deep the calling thread holds it.
std::mutex gState;
thread_local int tDepth = 0;
thread_local bool tUiThread = false;

int wakeDescriptor() {
  static const int fd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
  return fd;
}

}  // namespace

void UiThread::adopt() {
  tUiThread = true;
  wakeDescriptor();
}

bool UiThread::isCurrent() { return tUiThread; }

UiThread::Lock::Lock() {
  if (tDepth++ == 0) gState.lock();
}

UiThread::Lock::~Lock() {
  if (--tDepth == 0) gState.unlock();
}

UiThread::Unlocked::Unlocked() : depth_(tDepth) {
  if (depth_ > 0) {
    tDepth = 0;
    gState.unlock();
  }
}

UiThread::Unlocked::~Unlocked() {
  if (depth_ > 0) {
    gState.lock();
    tDepth = depth_;
  }
}

void UiThread::wake() {
  uint64_t one = 1;
  // A full counter already wakes the loop: nothing is lost when this fails.
  [[maybe_unused]] ssize_t written = ::write(wakeDescriptor(), &one, sizeof one);
}

int UiThread::wakeFd() { return wakeDescriptor(); }

void UiThread::woken() {
  uint64_t count = 0;
  while (::read(wakeDescriptor(), &count, sizeof count) == sizeof count) {
  }
}

}  // namespace oxyos
