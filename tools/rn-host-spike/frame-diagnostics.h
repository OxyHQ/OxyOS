// Where a window's frames spend their time, written to the session's log (stderr) always, as the
// compositor's own lines are ("[compositor] a turn worked …", "the GPU composed N frames …"):
// cheap enough to leave on in every session, so that a machine that feels slow says why without
// anyone running a profiler on it (docs/performance.mdx, "Reading the session's log").
//
// The host's loop (bloom-host.cpp) charges every moment of a turn to one part (Part): handling the
// compositor's events, JavaScript (React's handlers, render and commits with their Yoga layout, the
// timers' and requestAnimationFrame's callbacks), animation (Animated's native driver, worklets and
// Reanimated, a fling, gesture handlers), mounting, drawing, presenting, or anything else; and the time it slept while a frame waited for the compositor's
// answer to the one before. A frame's work is the parts of the turns since the frame before (a turn
// that sleeps with nothing to draw ends the work without a frame). Each frame:
//
//   - is slow when its work, plus any wait for the compositor beyond one refresh, takes longer than a
//     refresh of its display (display-refresh.h); the refreshes it missed are its dropped frames.
//     One line says where its time went and what started it (an input, a resize, an animation
//     frame, a timer, data arriving, JavaScript's own work), at most one a second, with how many
//     slow frames went unsaid since the last;
//   - goes into a summary written every 10 s while the window draws (nothing while it is idle): its
//     frames, their work's median, 95th percentile and most, the latency from an input (its
//     device's time) to the frame presented after it, and the slow and dropped frames.
//
// Resizes (a configure to the frame answering it, its layout and settle) and the window's own
// operations (zoom, minimize, close, from the request to what answers it) have lines of their own.
// With OXYOS_HOST_REPORT, every frame's timing, every line and every summary also go to the report
// (report.h: `timing`, `log`, `timing_summary`).
#pragma once

#include <array>
#include <chrono>
#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace oxyos {

class FrameDiagnostics {
 public:
  using Clock = std::chrono::steady_clock;

  // Where a moment of the loop went: each is charged to exactly one part.
  enum class Part : uint8_t { Other, Events, JavaScript, Animation, Mount, Draw, Present, Count };

  static FrameDiagnostics& get();

  // Starts the diagnostics for a window of `application` (its id, else its module): until then,
  // and for a render to a file, every call does nothing.
  void start(const std::string& application);
  bool started() const { return started_; }
  // A refresh of the window's display, in milliseconds.
  void setRefresh(double periodMs) { periodMs_ = periodMs; }

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
  void charge(Clock::time_point now);
  void finish(bool presented, Clock::time_point now);
  void due(Clock::time_point now);
  void summarize(Clock::time_point now);
  void endResizeRun();
  void log(const std::string& line);
  std::string partsText() const;

  bool started_ = false;
  std::string application_;
  double periodMs_ = 1000.0 / 60;

  // The current turn and the work since the last frame.
  Part current_ = Part::Other;
  Clock::time_point since_{};
  bool turning_ = false;
  bool waitedForCompositor_ = false;  // the sleep before this turn was the compositor's
  Clock::time_point turnEnded_{};
  std::array<double, static_cast<size_t>(Part::Count)> parts_{};
  double waitMs_ = 0, commitMs_ = 0, layoutMs_ = 0, textMs_ = 0, syncMs_ = 0;
  int turns_ = 0, presents_ = 0;
  bool gpu_ = false, presentedThisTurn_ = false, first_ = true;
  std::string trigger_;
  int triggerRank_ = -1;
  uint64_t inputNs_ = 0;  // the earliest input's device time since the last frame

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
