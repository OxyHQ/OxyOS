// Where a window's frames spend their time, written to the session's log (stderr) always, as the
// compositor's own lines are ("[compositor] a turn worked …", "the GPU composed N frames …"):
// cheap enough to leave on in every session, so that a machine that feels slow says why without
// anyone running a profiler on it (docs/performance.mdx, "Reading the session's log").
//
// The host has two threads (docs/adr/0067-the-ui-thread.mdx), each measured on its own clocks
// (thread-clock.h): by the wall clock, what a person waits for, and by the thread's own CPU clock,
// what the work cost however loaded the machine, with the rest of its wall time told apart as the
// thread blocked (waiting for another thread, a lock, a file: never hidden) or preempted (runnable
// while another thread had its CPU).
//
// The UI thread, the host's loop (bloom-host.cpp), makes the window's frames. It charges every moment
// of a turn to one part (Part): handling the compositor's events, laying the window out (a commit
// made on the UI thread: a new size), animation (Animated's native driver, worklets and Reanimated, a
// fling, gesture handlers, requestAnimationFrame's frame begun), mounting what JavaScript committed,
// drawing, presenting, or anything else; and the time it slept while a frame waited for the
// compositor's answer to the one before. A frame's work is the parts of the turns since the frame
// before (a turn that sleeps with nothing to draw ends the work without a frame).
//
// JavaScript's thread (js-thread.h) runs React's handlers, renders and commits with their Yoga layout,
// the timers' and requestAnimationFrame's callbacks, in stretches: from taking work up idle to being
// idle again. Its work never holds a frame up; what it costs a person is the time from an input to
// the frame showing JavaScript's answer, which each frame says (`latency`), and each stretch longer
// than a refresh is said as JavaScript's (`javascriptWorked`). Each frame:
//
//   - is slow when its work (by the wall clock), plus any wait for the compositor beyond one refresh, takes longer than a
//     refresh of its display (display-refresh.h); the refreshes it missed are its dropped frames.
//     One line says where its time went and what started it (an input, a resize, an animation
//     frame, a timer, data arriving, JavaScript's own work), at most one a second, with how many
//     slow frames went unsaid since the last;
//   - goes into a summary written every 10 s while the window draws (nothing while it is idle): its
//     frames, their work's median, 95th percentile and most, the latency from an input (its
//     device's time) to the frame showing its answer, the slow and dropped frames, and JavaScript's
//     work on its thread (its stretches, the slow ones, the longest).
//
// An input's answer is the first frame presented once JavaScript ran what the input's turn queued for
// it (its events, through Fabric's beat) and the loop applied what that committed; an input whose turn
// queued nothing for JavaScript (a scroll the loop moved itself) is answered by the next frame.
//
// A frame the compositor holds is not a slow frame. The compositor answers a shown window's frame
// at its display's next refresh; it withholds the answer while the window is not shown or the
// displays are off (the compositor's `set_displays_on` completes the frame in flight unshown, then
// composes nothing, so the next frame waits for the displays to come back). Waiting for that answer
// longer than kHeldAfter (a second: dozens of refreshes at any rate a display runs at, and longer
// than the compositor's own slowest turns, a modeset, which it reports itself) is a hold, as
// Android's FrameMetrics and JankStats count only the frames of a window that draws: from then on,
// the turns' work and the wait go to the hold, not to the frame, and the inputs that have waited
// longer than kHeldAfter are timed by no frame (no one saw a frame answer them late; an input that
// wakes the displays is newer, and its wait for the frame after the hold is timed). When the answer
// comes, the hold has a line of its own ("held N s by the compositor …": how long, and the work
// done meanwhile, which is the application's and never dropped), and the frame after it is timed
// from the turn the answer came in.
//
// Resizes (a configure to the frame answering it, its layout and settle) and the window's own
// operations (zoom, minimize, close, from the request to what answers it) have lines of their own.
// With OXYOS_HOST_REPORT, every frame's timing, every line and every summary also go to the report
// (report.h: `timing`, `log`, `timing_summary`).
#pragma once

#include "thread-clock.h"

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace oxyos {

class FrameDiagnostics {
 public:
  using Clock = std::chrono::steady_clock;

  // Where a moment of the loop went: each is charged to exactly one part.
  enum class Part : uint8_t { Other, Events, Layout, Animation, Mount, Draw, Present, Count };

  static FrameDiagnostics& get();

  // Starts the diagnostics for a window of `application` (its id, else its module): until then,
  // and for a render to a file, every call does nothing.
  void start(const std::string& application);
  bool started() const { return started_; }
  // A refresh of the window's display, in milliseconds.
  void setRefresh(double periodMs) {
    periodMs_ = periodMs;
    jsPeriodMs_ = periodMs;
  }

  // A turn of the host's loop. `waitingForCompositor`: a frame is ready to draw but the one before
  // is not answered yet, so what the loop sleeps next is the compositor's. `sleeping`: the loop
  // sleeps next.
  void beginTurn();
  void endTurn(bool waitingForCompositor, bool sleeping);

  // Charges what came before to the current part and moves to `part`; `leave` moves back.
  Part enter(Part part);
  void leave(Part previous);
  class Scope {
   public:
    explicit Scope(Part part) : previous_(get().enter(part)) {}
    ~Scope() { get().leave(previous_); }
    Scope(const Scope&) = delete;
    Scope& operator=(const Scope&) = delete;

   private:
    Part previous_;
  };

  // What started the work: the first of a frame's names it, unless something of a higher `rank`
  // follows: 0 for JavaScript running (an input or a timer may be behind it), 1 for the pointer
  // moving and other compositor events (a press comes after the motion to its place), 2 for
  // anything else. An input also gives when its device registered it (CLOCK_MONOTONIC nanoseconds,
  // 0 when unknown): the frame's latency is from the earliest.
  void trigger(const char* what, int rank = 2);
  void input(uint32_t kind, uint32_t detail, uint64_t timeNs);
  // What started the work of the frame now being made, for what the loop queues for JavaScript
  // (js-thread.h's Cause): "" before anything did.
  const std::string& currentTrigger() const { return trigger_; }
  int currentRank() const { return triggerRank_; }
  // The inputs of the turn are handed to JavaScript, if `handed` (an event dispatched to a view, a
  // device event), its thread having been given `queued` tasks by then: they are answered by the
  // first frame presented once JavaScript has run those and the loop applied what they committed
  // (javascriptApplied). Inputs that handed it nothing are answered by the next frame.
  void handedToJavaScript(bool handed, uint64_t queued);
  // The loop is about to apply what JavaScript handed over: the tasks it had run by then, `ran`.
  void javascriptApplied(uint64_t ran);
  // Whether an input waits for JavaScript to run what it was handed: the loop is then woken as
  // JavaScript's thread settles, to time it, or to forget it if its answer drew nothing.
  bool awaitingJavaScript() const;

  // A stretch of JavaScript's work on its thread (js-thread.h), called there: begun at `started`,
  // started by `cause`, `tasks` of React Native's tasks. One longer than a refresh is said (at most
  // one line a second, as the frames'), with the frames the window presented meanwhile; each is in the
  // report (`js_timing`) and in the next frame's line and the summary.
  void javascriptWorked(const ThreadSpan& span, Clock::time_point started, const std::string& cause, int tasks);

  // A transaction mounted: its commit's time, its layout's (Yoga) and its text measuring's.
  void committed(double commitMs, double layoutMs, double textMs);
  // A frame of a surface presented: on the GPU with its synchronization's time, or in shared memory.
  void presented(bool gpu, double syncMs);

  // The window resized by a configure received at `received`, laid out in `ms`, and the time it
  // then waited for JavaScript's layout to settle (`cut` when a refresh ended the wait).
  void resized(Clock::time_point received, float width, float height);
  void laidOut(double ms);
  void settled(double ms, bool cut);

  // The window's own operations (docs/adr/0016-windows.mdx): asked for, and the state that answers.
  void zoomAsked(bool maximize);
  void minimizeAsked();
  void closeAsked(const char* by);
  void windowState(bool maximized, bool active);
  // The loop left, and everything after it (the window, the runtime) is gone.
  void leaving();
  void ended();

  // When the next summary, or the end of a live resize, is due: the loop wakes for it.
  std::optional<Clock::time_point> nextDeadline() const;

  // The percentile `p` (0 to 100) of sorted `values`, as the compositor's summary picks it.
  static double percentile(const std::vector<double>& sorted, int p);

 private:
  struct Frame {
    double workMs = 0, waitMs = 0;
    std::optional<double> latencyMs;
  };
  // A frame held by the compositor (above): since when the loop waited for its answer, until the
  // last turn charged to it, and the work, the wait and the inputs not timed meanwhile.
  struct Held {
    Clock::time_point since, until;
    std::array<ThreadSpan, static_cast<size_t>(Part::Count)> parts{};
    double waitMs = 0, commitMs = 0, layoutMs = 0, textMs = 0;
    int turns = 0, untimedInputs = 0;
  };
  void charge(const ThreadSample& now);
  void hold(Clock::time_point now);
  void endHold();
  void finish(bool presented, Clock::time_point now);
  void due(Clock::time_point now);
  void summarize(Clock::time_point now);
  void endResizeRun();
  void log(const std::string& line);
  std::string partsText(const ThreadSpan& javascript) const;

  // Set once by the loop; read by JavaScript's thread too (javascriptWorked), as is the refresh.
  std::atomic<bool> started_ = false;
  std::string application_;
  double periodMs_ = 1000.0 / 60;
  std::atomic<double> jsPeriodMs_ = 1000.0 / 60;

  // An input not yet answered by a frame: its device's time (CLOCK_MONOTONIC nanoseconds), its name,
  // and the JavaScript tasks that must have run before the frame answering it (0 for none; unset
  // until its turn handed it over).
  struct Input {
    uint64_t ns = 0;
    std::string name;
    std::optional<uint64_t> javascript;
  };
  // JavaScript's work on its thread, shared with it (jsMutex_): since the last frame, for the
  // summary, the frames presented lately (when, and how many refreshes each dropped) for a stretch's
  // line, and the slow lines' pace.
  struct JavaScript {
    ThreadSpan sinceFrame;
    int stretchesSinceFrame = 0;
    std::vector<double> work;
    int slow = 0;
    std::vector<std::pair<Clock::time_point, int>> frames;
    std::optional<Clock::time_point> slowLogged;
    int slowSkipped = 0;
  };
  mutable std::mutex jsMutex_;
  JavaScript js_;
  uint64_t javascriptApplied_ = 0;

  // The current turn and the work since the last frame.
  Part current_ = Part::Other;
  ThreadSample since_{};
  bool turning_ = false;
  bool waitedForCompositor_ = false;  // the sleep before this turn was the compositor's
  Clock::time_point turnEnded_{};
  // Each part's time: by the wall clock, and of it on the CPU, preempted and blocked (thread-clock.h).
  std::array<ThreadSpan, static_cast<size_t>(Part::Count)> parts_{};
  double waitMs_ = 0, commitMs_ = 0, layoutMs_ = 0, textMs_ = 0, syncMs_ = 0;
  int turns_ = 0, presents_ = 0;
  bool gpu_ = false, presentedThisTurn_ = false, first_ = true;
  std::string trigger_;
  int triggerRank_ = -1;
  // The inputs not yet answered: a frame's latency is from the earliest it answers.
  std::vector<Input> inputs_;
  // Since when the loop waits for the compositor's answer to the window's frame, without a turn
  // between that did not wait; and the hold that wait became, if it did.
  std::optional<Clock::time_point> waitingSince_;
  std::optional<Held> held_;

  // The slow lines' pace: one a second, with how many were not written since.
  std::optional<Clock::time_point> slowLogged_;
  int slowSkipped_ = 0;

  // The current summary's frames.
  std::optional<Clock::time_point> windowFrom_;
  std::vector<double> work_, latency_;
  int slow_ = 0, dropped_ = 0;

  // A resize waiting for its frame, and the live resize it is part of.
  std::optional<Clock::time_point> resizePending_;
  int configures_ = 0;  // configures that resized since the last frame answering one
  float resizeWidth_ = 0, resizeHeight_ = 0;
  double resizeLayoutMs_ = 0, resizeSettleMs_ = 0;
  bool resizeCut_ = false;
  struct ResizeRun {
    Clock::time_point from, last;
    std::vector<double> presentedMs, layoutMs;
    int cut = 0, configures = 0;
  };
  std::optional<ResizeRun> resizeRun_;

  // The window's operation asked for and not yet answered.
  struct Operation {
    std::string name;
    Clock::time_point asked;
    bool maximize = false;
    std::optional<Clock::time_point> answered;  // its state reached (a zoom's configure)
  };
  std::optional<Operation> operation_;
  bool maximized_ = false, active_ = false;
  std::optional<Clock::time_point> leftAt_;
};

}  // namespace oxyos
