#include "frame-diagnostics.h"

#include "report.h"

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <algorithm>
#include <cmath>
#include <cstdarg>
#include <cstdio>
#include <ctime>
#include <unistd.h>

namespace oxyos {

namespace {

using Clock = FrameDiagnostics::Clock;

constexpr auto kSummaryEvery = std::chrono::seconds(10);
constexpr auto kSlowLinesEvery = std::chrono::seconds(1);
// A live resize ends when no frame answered a configure for this long.
constexpr auto kResizeRunEnds = std::chrono::milliseconds(500);
// A window operation not answered by then is said to be.
constexpr auto kOperationAnswer = std::chrono::seconds(3);

const char* const kPartNames[] = {"other", "events", "javascript", "animation", "mount", "draw", "present"};

double ms(Clock::duration duration) { return std::chrono::duration<double, std::milli>(duration).count(); }

uint64_t monotonicNs() {
  timespec now{};
  clock_gettime(CLOCK_MONOTONIC, &now);
  return static_cast<uint64_t>(now.tv_sec) * 1'000'000'000 + static_cast<uint64_t>(now.tv_nsec);
}

std::string format(const char* pattern, ...) __attribute__((format(printf, 1, 2)));
std::string format(const char* pattern, ...) {
  char buffer[1024];
  va_list arguments;
  va_start(arguments, pattern);
  int length = std::vsnprintf(buffer, sizeof buffer, pattern, arguments);
  va_end(arguments);
  return std::string(buffer, static_cast<size_t>(std::clamp(length, 0, static_cast<int>(sizeof buffer) - 1)));
}

double round2(double value) { return std::round(value * 100) / 100; }

}  // namespace

FrameDiagnostics& FrameDiagnostics::get() {
  static FrameDiagnostics diagnostics;
  return diagnostics;
}

double FrameDiagnostics::percentile(const std::vector<double>& sorted, int p) {
  if (sorted.empty()) return 0;
  return sorted[std::min(sorted.size() * static_cast<size_t>(p) / 100, sorted.size() - 1)];
}

void FrameDiagnostics::start(const std::string& application) {
  started_ = true;
  application_ = application;
  since_ = turnEnded_ = Clock::now();
  work_.reserve(2048);
  latency_.reserve(512);
}

void FrameDiagnostics::charge(Clock::time_point now) {
  if (turning_) parts_[static_cast<size_t>(current_)] += ms(now - since_);
  since_ = now;
}

FrameDiagnostics::Part FrameDiagnostics::enter(Part part) {
  if (!started_) return Part::Other;
  charge(Clock::now());
  Part previous = current_;
  current_ = part;
  return previous;
}

void FrameDiagnostics::leave(Part previous) {
  if (!started_) return;
  charge(Clock::now());
  current_ = previous;
}

void FrameDiagnostics::beginTurn() {
  if (!started_) return;
  auto now = Clock::now();
  // What the loop slept since its last turn was the compositor's when a frame waited for it.
  if (waitedForCompositor_) waitMs_ += ms(now - turnEnded_);
  waitedForCompositor_ = false;
  current_ = Part::Other;
  since_ = now;
  turning_ = true;
  presentedThisTurn_ = false;
  ++turns_;
}

void FrameDiagnostics::endTurn(bool waitingForCompositor, bool sleeping) {
  if (!started_ || !turning_) return;
  auto now = Clock::now();
  charge(now);
  turning_ = false;
  turnEnded_ = now;
  waitedForCompositor_ = waitingForCompositor;
  if (presentedThisTurn_) {
    finish(true, now);
  } else if (sleeping && !waitingForCompositor) {
    finish(false, now);
  }
  due(now);
}

void FrameDiagnostics::trigger(const char* what, int rank) {
  if (!started_ || rank <= triggerRank_) return;
  trigger_ = what;
  triggerRank_ = rank;
}

void FrameDiagnostics::input(uint32_t kind, uint32_t detail, uint64_t timeNs) {
  if (!started_) return;
  const char* name = nullptr;
  switch (kind) {
    case OXYOS_EVENT_POINTER: {
      static const char* const details[] = {"pointer",        "pointer.enter",   "pointer.leave", "pointer.motion",
                                            "pointer.press", "pointer.release", "pointer.cancel"};
      name = detail < sizeof details / sizeof *details ? details[detail] : "pointer";
      break;
    }
    case OXYOS_EVENT_KEY: name = "key"; break;
    case OXYOS_EVENT_TOUCH: name = "touch"; break;
    case OXYOS_EVENT_SCROLL: name = "scroll"; break;
    case OXYOS_EVENT_PINCH: name = "pinch"; break;
    case OXYOS_EVENT_HOLD: name = "hold"; break;
    case OXYOS_EVENT_TEXT: name = "text"; break;
    case OXYOS_EVENT_TEXT_INPUT: name = "text input"; break;
    case OXYOS_EVENT_ACCESSIBILITY_ACTION: name = "accessibility action"; break;
    case OXYOS_EVENT_MENU_ITEM_ACTIVATED: name = "menu command"; break;
    case OXYOS_EVENT_SYSTEM_SHORTCUT: name = "system shortcut"; break;
    case OXYOS_EVENT_ACTIVATED: trigger("activation", 1); return;
    default: trigger("a compositor event", 1); return;
  }
  bool moving = kind == OXYOS_EVENT_POINTER && (detail == OXYOS_POINTER_MOTION || detail == OXYOS_POINTER_ENTER ||
                                                detail == OXYOS_POINTER_LEAVE);
  trigger(name, moving ? 1 : 2);
  // The input's time, as its device registered it; when the read itself without one.
  uint64_t at = timeNs != 0 ? timeNs : monotonicNs();
  if (inputNs_ == 0 || at < inputNs_) inputNs_ = at;
}

void FrameDiagnostics::committed(double commitMs, double layoutMs, double textMs) {
  if (!started_) return;
  commitMs_ += commitMs;
  layoutMs_ += layoutMs;
  textMs_ += textMs;
}

void FrameDiagnostics::presented(bool gpu, double syncMs) {
  if (!started_) return;
  presentedThisTurn_ = true;
  ++presents_;
  gpu_ |= gpu;
  syncMs_ += syncMs;
}

std::string FrameDiagnostics::partsText() const {
  auto part = [&](Part p) { return parts_[static_cast<size_t>(p)]; };
  std::string painter = presents_ == 0 ? "" : gpu_ ? format(" on the GPU (sync %.1f)", syncMs_) : " from the CPU";
  return format("events %.1f, JavaScript %.1f (commits %.1f, of which Yoga layout %.1f and text %.1f), animation %.1f, "
                "mount %.1f, drawing %.1f, presenting %.1f%s, other %.1f; waiting for the compositor %.1f; %d turn%s",
                part(Part::Events), part(Part::JavaScript), commitMs_, layoutMs_, textMs_, part(Part::Animation),
                part(Part::Mount), part(Part::Draw), part(Part::Present), painter.c_str(), part(Part::Other), waitMs_,
                turns_, turns_ == 1 ? "" : "s");
}

void FrameDiagnostics::log(const std::string& line) {
  std::string text = "[host " + application_ + "] " + line;
  // One write, so that the line arrives whole among what the session's other processes write to
  // the same log (a pipe takes a write this short at once).
  std::string written = text + "\n";
  for (size_t done = 0; done < written.size();) {
    ssize_t n = ::write(STDERR_FILENO, written.data() + done, written.size() - done);
    if (n <= 0) break;
    done += static_cast<size_t>(n);
  }
  auto& report = Report::get();
  if (report.enabled()) report.diagnostic("log", folly::dynamic::object("text", text));
}

void FrameDiagnostics::finish(bool presented, Clock::time_point now) {
  double work = 0;
  for (double part : parts_) work += part;
  // A slow frame: its work, and what the compositor kept it waiting beyond one refresh (a frame in
  // flight is answered within one), past a refresh.
  double late = work + std::max(0.0, waitMs_ - periodMs_);
  bool slow = late > periodMs_;
  int dropped = slow ? static_cast<int>(std::ceil(late / periodMs_)) - 1 : 0;
  std::optional<double> latency;
  if (presented && inputNs_ != 0) latency = static_cast<double>(monotonicNs() - inputNs_) / 1e6;
  std::string what = first_ && presented ? "the first frame" : trigger_.empty() ? "JavaScript" : trigger_;

  if (presented || slow) {
    auto& report = Report::get();
    if (report.enabled()) {
      folly::dynamic parts = folly::dynamic::object;
      for (size_t i = 0; i < parts_.size(); ++i) parts[kPartNames[i]] = round2(parts_[i]);
      report.diagnostic("timing", folly::dynamic::object("presented", presented)("work_ms", work)("wait_ms", waitMs_)(
                                      "late_ms", late)("slow", slow)("dropped", dropped)(
                                      "latency_ms", latency ? folly::dynamic(*latency) : folly::dynamic(nullptr))(
                                      "trigger", what)("parts", std::move(parts))("commit_ms", commitMs_)(
                                      "layout_ms", layoutMs_)("text_ms", textMs_)("gpu", gpu_)("sync_ms", syncMs_)(
                                      "turns", turns_)("refresh_ms", periodMs_));
    }
  }
  if (presented) {
    if (!windowFrom_) windowFrom_ = now;
    work_.push_back(work);
    if (latency) latency_.push_back(*latency);
    if (slow) {
      ++slow_;
      dropped_ += dropped;
    }
  }
  if (slow) {
    if (!slowLogged_ || now - *slowLogged_ >= kSlowLinesEvery) {
      std::string subject = presented ? (what == "the first frame" ? what : "a frame after " + what)
                                      : "handling " + what + " (no frame)";
      std::string line = format("%s took %.1f ms, %d dropped (a refresh is %.1f ms): ", subject.c_str(), work,
                                dropped, periodMs_) +
                         partsText();
      if (latency) line += format("; input to present %.1f ms", *latency);
      if (slowSkipped_ > 0) line += format("; %d slow not shown since the last", slowSkipped_);
      log(line);
      slowLogged_ = now;
      slowSkipped_ = 0;
    } else {
      ++slowSkipped_;
    }
  }
  if (presented && resizePending_) {
    double presentedMs = ms(now - *resizePending_);
    if (!resizeRun_) {
      log(format("a resize to %gx%g: its frame presented %.1f ms after the configure (laying it out %.1f, "
                 "settling %.1f%s; the frame's work %.1f, waiting for the compositor %.1f)",
                 resizeWidth_, resizeHeight_, presentedMs, resizeLayoutMs_, resizeSettleMs_,
                 resizeCut_ ? ", cut short at a refresh" : "", work, waitMs_));
      resizeRun_ = ResizeRun{.from = *resizePending_, .last = now};
    }
    resizeRun_->last = now;
    resizeRun_->presentedMs.push_back(presentedMs);
    resizeRun_->layoutMs.push_back(resizeLayoutMs_);
    resizeRun_->cut += resizeCut_ ? 1 : 0;
    resizeRun_->configures += configures_;
    resizePending_.reset();
    configures_ = 0;
  }
  if (presented && operation_ && operation_->name == "zoom" && operation_->answered) {
    log(format("zoom (%s): its configure %.1f ms after it was asked, its frame presented %.1f ms after",
               operation_->maximize ? "to fill the work area" : "back to its size",
               ms(*operation_->answered - operation_->asked), ms(now - operation_->asked)));
    operation_.reset();
  }
  if (presented) first_ = false;
  parts_.fill(0);
  waitMs_ = commitMs_ = layoutMs_ = textMs_ = syncMs_ = 0;
  turns_ = presents_ = 0;
  gpu_ = false;
  trigger_.clear();
  triggerRank_ = -1;
  inputNs_ = 0;
}

void FrameDiagnostics::resized(Clock::time_point received, float width, float height) {
  if (!started_) return;
  trigger("a resize");
  if (!resizePending_) {
    resizePending_ = received == Clock::time_point{} ? Clock::now() : received;
    resizeLayoutMs_ = resizeSettleMs_ = 0;
    resizeCut_ = false;
  }
  ++configures_;
  resizeWidth_ = width;
  resizeHeight_ = height;
}

void FrameDiagnostics::laidOut(double ms) {
  if (started_ && resizePending_) resizeLayoutMs_ += ms;
}

void FrameDiagnostics::settled(double ms, bool cut) {
  if (!started_ || !resizePending_) return;
  resizeSettleMs_ += ms;
  resizeCut_ |= cut;
}

void FrameDiagnostics::endResizeRun() {
  auto run = std::move(*resizeRun_);
  resizeRun_.reset();
  if (run.presentedMs.size() < 2) return;  // one resize, said as it was presented
  auto presented = run.presentedMs, layout = run.layoutMs;
  std::sort(presented.begin(), presented.end());
  std::sort(layout.begin(), layout.end());
  log(format("a live resize: %zu frames for %d configures over %.0f ms; configure to frame %.1f ms median, %.1f at "
             "most; laying out %.1f ms median, %.1f at most; %d settles cut short at a refresh",
             presented.size(), run.configures, ms(run.last - run.from), percentile(presented, 50), presented.back(),
             percentile(layout, 50), layout.back(), run.cut));
}

void FrameDiagnostics::zoomAsked(bool maximize) {
  if (started_) operation_ = Operation{.name = "zoom", .asked = Clock::now(), .maximize = maximize};
}

void FrameDiagnostics::minimizeAsked() {
  if (started_) operation_ = Operation{.name = "minimize", .asked = Clock::now()};
}

void FrameDiagnostics::closeAsked(const char* by) {
  if (!started_ || (operation_ && operation_->name.starts_with("close"))) return;
  std::string name = std::string("close (asked by ") + by;
  if (inputNs_ != 0) name += format(" %.1f ms after the input", static_cast<double>(monotonicNs() - inputNs_) / 1e6);
  operation_ = Operation{.name = name + ")", .asked = Clock::now()};
}

void FrameDiagnostics::windowState(bool maximized, bool active) {
  if (!started_) return;
  if (operation_ && !operation_->answered) {
    auto now = Clock::now();
    if (operation_->name == "zoom" && maximized == operation_->maximize && maximized != maximized_) {
      operation_->answered = now;
    } else if (operation_->name == "minimize" && !active && active_) {
      log(format("minimize: the window inactive %.1f ms after it was asked", ms(now - operation_->asked)));
      operation_.reset();
    }
  }
  maximized_ = maximized;
  active_ = active;
}

void FrameDiagnostics::leaving() {
  if (started_) leftAt_ = Clock::now();
}

void FrameDiagnostics::ended() {
  if (!started_) return;
  auto now = Clock::now();
  if (operation_ && operation_->name.starts_with("close")) {
    log(format("%s: the loop left %.1f ms after it was asked, the window and the runtime gone %.1f ms after",
               operation_->name.c_str(), leftAt_ ? ms(*leftAt_ - operation_->asked) : 0.0, ms(now - operation_->asked)));
    operation_.reset();
  }
  if (resizeRun_) endResizeRun();
  summarize(now);
  started_ = false;
}

void FrameDiagnostics::due(Clock::time_point now) {
  if (windowFrom_ && now - *windowFrom_ >= kSummaryEvery) summarize(now);
  if (resizeRun_ && now - resizeRun_->last >= kResizeRunEnds) endResizeRun();
  if (operation_ && !operation_->name.starts_with("close") && now - operation_->asked >= kOperationAnswer) {
    const char* missing = operation_->name == "minimize" ? "the window is still active"
                          : operation_->answered        ? "its frame is not presented"
                                                        : "no configure answered it";
    log(format("%s asked %.0f ms ago: %s", operation_->name.c_str(), ms(now - operation_->asked), missing));
    operation_.reset();
  }
}

std::optional<Clock::time_point> FrameDiagnostics::nextDeadline() const {
  if (!started_) return std::nullopt;
  std::optional<Clock::time_point> next;
  auto consider = [&](Clock::time_point at) {
    if (!next || at < *next) next = at;
  };
  if (windowFrom_) consider(*windowFrom_ + kSummaryEvery);
  if (resizeRun_) consider(resizeRun_->last + kResizeRunEnds);
  if (operation_ && !operation_->name.starts_with("close")) consider(operation_->asked + kOperationAnswer);
  return next;
}

void FrameDiagnostics::summarize(Clock::time_point now) {
  if (!windowFrom_) return;
  double seconds = ms(now - *windowFrom_) / 1000;
  windowFrom_.reset();
  if (work_.empty()) return;
  std::sort(work_.begin(), work_.end());
  std::sort(latency_.begin(), latency_.end());
  std::string inputs = latency_.empty()
                           ? std::string("no input")
                           : format("input to present %.1f ms median, %.1f at the 95th percentile (%zu inputs)",
                                    percentile(latency_, 50), percentile(latency_, 95), latency_.size());
  log(format("in the last %.1f s: %zu frames, their work %.1f ms median, %.1f at the 95th percentile, %.1f at most; %s; "
             "%d slow, %d dropped (a refresh is %.1f ms)",
             seconds, work_.size(), percentile(work_, 50), percentile(work_, 95), work_.back(), inputs.c_str(), slow_,
             dropped_, periodMs_));
  auto& report = Report::get();
  if (report.enabled()) {
    auto optional = [&](double value) { return latency_.empty() ? folly::dynamic(nullptr) : folly::dynamic(value); };
    report.diagnostic("timing_summary",
                      folly::dynamic::object("seconds", seconds)("frames", work_.size())(
                          "work_p50", percentile(work_, 50))("work_p95", percentile(work_, 95))("work_max", work_.back())(
                          "latency_p50", optional(percentile(latency_, 50)))(
                          "latency_p95", optional(percentile(latency_, 95)))("inputs", latency_.size())("slow", slow_)(
                          "dropped", dropped_)("refresh_ms", periodMs_));
  }
  work_.clear();
  latency_.clear();
  slow_ = dropped_ = 0;
}

}  // namespace oxyos
