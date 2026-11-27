#include "ui-thread.h"

#include <cerrno>
#include <cstdint>
#include <iostream>
#include <mutex>
#include <streambuf>
#include <string>

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

namespace {

// std::cerr's buffer: each thread's characters kept until its line ends, written with one write(2).
class WholeLines : public std::streambuf {
 protected:
  int_type overflow(int_type c) override {
    if (c == traits_type::eof()) return traits_type::not_eof(c);
    auto& line = pending();
    line.push_back(static_cast<char>(c));
    if (c == '\n') emit(line);
    return c;
  }
  std::streamsize xsputn(const char* text, std::streamsize count) override {
    auto& line = pending();
    for (std::streamsize i = 0; i < count; ++i) {
      line.push_back(text[i]);
      if (text[i] == '\n') emit(line);
    }
    return count;
  }
  // std::cerr flushes after each output: a line is written only once it ends.
  int sync() override { return 0; }

 private:
  static std::string& pending() {
    thread_local std::string line;
    return line;
  }
  static void emit(std::string& line) {
    for (size_t done = 0; done < line.size();) {
      ssize_t n = ::write(STDERR_FILENO, line.data() + done, line.size() - done);
      if (n <= 0) break;
      done += static_cast<size_t>(n);
    }
    line.clear();
  }
};

}  // namespace

void UiThread::wholeLogLines() {
  static WholeLines buffer;
  std::cerr.rdbuf(&buffer);
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
