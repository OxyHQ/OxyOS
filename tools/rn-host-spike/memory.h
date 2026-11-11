// What a Bloom host holds in memory (docs/performance.mdx, *Memory*): the process set up to hold
// what it uses and no more, what it keeps for work given back once its window has been idle a
// while, and its own account of each part, written to its report.
//
// - The process (configureProcessMemory, before anything else runs): no transparent huge pages,
//   which on a kernel set to `always` (Debian's) back a whole 2 MiB of a sparse heap for one page
//   touched in it, and khugepaged later fills in the rest (the session's shell held 160 MiB of them
//   after five days, of 175 MiB anonymous); and malloc's threshold for mapping an allocation of its
//   own fixed at 4 MiB, so that a large buffer (a picture decoded, a canvas's pixels) is returned
//   to the system when it is freed, where glibc's sliding threshold would raise itself to that
//   buffer's size and keep every later one in its heap.
// - Idle (Memory::tick, kSettledAfter after the loop's last work, once): the painter's GPU buffers
//   the compositor released but the one each surface shows (window.h), Skia's scratch textures
//   (offscreen layers, made again when a frame needs one) and the textures not drawn for
//   kUnusedFor, malloc's free pages (malloc_trim), and Ganesh's Vulkan pipeline cache written to
//   its persistent cache (skia-gpu.h) when it learned something.
// - The account (a `memory` line in the report, after each trim): Hermes' heaps (the application's
//   runtime and worklets' UI runtime: their segments, what is allocated in them, the malloc'd
//   memory their objects hold), malloc's (what is in use and what it holds free), Skia's GPU
//   resource cache and its CPU caches (pictures, glyphs), the painter's GPU buffers, in MiB.
#pragma once

#include <folly/dynamic.h>

#include <chrono>
#include <functional>
#include <optional>
#include <vector>

namespace facebook::jsi {
class Runtime;
}

namespace oxyos {

// The process's memory settings: called first thing in main, before any thread starts.
void configureProcessMemory();

class Memory {
 public:
  using Clock = std::chrono::steady_clock;
  // The loop idle this long after work (no frame, no JavaScript, no event): its window is not
  // animating, and the buffers and scratch space kept for frames in a row are given back.
  static constexpr std::chrono::milliseconds kSettledAfter{1000};
  // Skia's textures not drawn for this long go at a trim (a picture scrolled away long ago).
  static constexpr std::chrono::seconds kUnusedFor{30};

  static Memory& get();

  // The JavaScript runtimes the account names: each a function that gives the runtime (or null,
  // one not made yet), called on the thread that runs it. `onItsThread` runs a task there (React
  // Native's runtime, on JavaScript's thread, js-thread.h); none: the loop's thread, which runs it
  // (worklets' UI runtime).
  void addRuntime(const char* name, std::function<facebook::jsi::Runtime*()> runtime,
                  std::function<void(std::function<void()>)> onItsThread = nullptr);
  // What gives back the painter's spare buffers (Window::trimBuffers): the bytes freed.
  void setBufferTrim(std::function<uint64_t()> trim) { trimBuffers_ = std::move(trim); }

  // The loop worked at `now`: the trim waits for it to be idle again.
  void worked(Clock::time_point now);
  // When the next trim is due, if one is: kSettledAfter after the last work, once.
  std::optional<Clock::time_point> nextDeadline() const;
  // Trims when due (the loop calls it every turn); true when it did.
  bool tick(Clock::time_point now);

  // The account, in MiB (rounded to a tenth): what the report's `memory` line holds. A runtime run on
  // another thread is not in it: tick adds it there (heapOf).
  folly::dynamic account() const;

 private:
  struct Runtime {
    const char* name;
    std::function<facebook::jsi::Runtime*()> runtime;
    std::function<void(std::function<void()>)> onItsThread;
  };
  // Adds what runtime `runtime` holds to `account`, on its thread.
  static void heapOf(const Runtime& runtime, folly::dynamic& account);
  std::vector<Runtime> runtimes_;
  std::function<uint64_t()> trimBuffers_;
  std::optional<Clock::time_point> lastWork_;
  bool trimmed_ = true;  // nothing to give back before the first work
};

}  // namespace oxyos
