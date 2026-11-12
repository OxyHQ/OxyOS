// The UI thread (docs/adr/0071-the-ui-thread.mdx): the host's loop, on the process's main thread. It
// owns the frame clock, input, scroll physics and gesture tracking, Animated's native driver and
// worklets' UI runtime, the view tree it mounts, drawing and presenting, accessibility and the
// window; JavaScript runs on a thread of its own (js-thread.h) and never holds it up.
//
// The host's native state (the view tree, the window and its surfaces, popups, accessibility, text
// editing, the services JavaScript's modules reach) is the UI thread's. The loop holds `Lock` while it
// works and lets it go while it sleeps; code on another thread that touches that state, a module's
// method JavaScript calls (host-modules.h), holds it meanwhile, so the two never touch it at once.
// JavaScript running holds nothing: only its calls into the host do, each as long as the call.
//
// Another thread that gives the loop work (a commit to mount, an accessibility event, a worklet's job,
// a frame asked for) wakes it (`wake`): the loop polls an eventfd beside its other descriptors.
#pragma once

namespace oxyos {

class UiThread {
 public:
  // The calling thread is the UI thread: once, by the loop, before any other thread starts.
  static void adopt();
  static bool isCurrent();

  // The UI thread's state, held. On a thread that holds it already it is held again (a module's
  // method calling JavaScript that calls another), and released by the outermost.
  class Lock {
   public:
    Lock();
    ~Lock();
    Lock(const Lock&) = delete;
    Lock& operator=(const Lock&) = delete;
  };
  // Released for a while by a thread that holds it (the loop, around its sleep): the holder's
  // recursion is kept and taken again as this ends.
  class Unlocked {
   public:
    Unlocked();
    ~Unlocked();
    Unlocked(const Unlocked&) = delete;
    Unlocked& operator=(const Unlocked&) = delete;

   private:
    int depth_;
  };

  // The host's log (stderr) a whole line at a time, whichever thread writes it: `std::cerr` keeps what
  // each thread writes until its line ends, then writes the line at once, so the two threads' lines
  // never interleave in the session's log (frame-diagnostics.h's lines among them). Once, at start.
  static void wholeLogLines();

  // Wakes the loop from its sleep, from any thread.
  static void wake();
  // The descriptor the loop polls, and what it reads when it turns.
  static int wakeFd();
  static void woken();
};

}  // namespace oxyos
