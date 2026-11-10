#include "report.h"

#include <react/renderer/components/view/ViewProps.h>

#include <folly/json.h>
#include <jsi/JSIDynamic.h>
#include <jsi/jsi.h>

#include <cctype>
#include <cstdlib>
#include <ctime>
#include <fcntl.h>
#include <filesystem>
#include <sys/file.h>
#include <unistd.h>

using namespace facebook::react;

namespace oxyos {

namespace {

int64_t monotonicNs() {
  timespec now{};
  clock_gettime(CLOCK_MONOTONIC, &now);
  return static_cast<int64_t>(now.tv_sec) * 1'000'000'000 + now.tv_nsec;
}

}  // namespace

std::string perProcessPath(std::string target, const std::string& application, const char* extension) {
  std::error_code error;
  if (target.empty() || (target.back() != '/' && !std::filesystem::is_directory(target, error))) return target;
  // A directory: this process's own file in it, named by its application and pid.
  std::filesystem::create_directories(target, error);
  std::string name;
  for (char c : application) name += std::isalnum(static_cast<unsigned char>(c)) || c == '.' || c == '-' ? c : '_';
  if (target.back() != '/') target += '/';
  return target + (name.empty() ? std::string("host") : name) + "-" + std::to_string(getpid()) + extension;
}

Report& Report::get() {
  static Report report;
  return report;
}

Report::Report() = default;

void Report::open(const std::string& application) {
  const char* path = std::getenv("OXYOS_HOST_REPORT");
  if (file_ || !path || !*path) return;
  std::string target = perProcessPath(path, application, ".jsonl");
  // Held for the host's life: a second host given the same file while the first runs (two
  // applications the session launched, both told `.launched`) writes beside it, named by its
  // pid, rather than truncating it under the first; a file no running host holds is written anew.
  int fd = ::open(target.c_str(), O_WRONLY | O_CREAT | O_CLOEXEC, 0644);
  if (fd >= 0 && ::flock(fd, LOCK_EX | LOCK_NB) != 0) {
    ::close(fd);
    target += "." + std::to_string(getpid());
    fd = ::open(target.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
  } else if (fd >= 0 && ::ftruncate(fd, 0) != 0) {
    ::close(fd);
    fd = -1;
  }
  if (fd >= 0) file_ = ::fdopen(fd, "w");
  if (!file_) {
    if (fd >= 0) ::close(fd);
    std::fprintf(stderr, "[host] cannot write the report to %s\n", target.c_str());
  }
}

void Report::diagnostic(const char* kind, folly::dynamic&& line) {
  if (!file_) return;
  line["kind"] = kind;
  write(std::move(line));
}

std::string Report::name(const ShadowView& view) {
  if (auto props = std::dynamic_pointer_cast<const ViewProps>(view.props)) {
    if (!props->testId.empty()) return props->testId;
    if (!props->accessibilityLabel.empty()) return props->accessibilityLabel;
  }
  return std::string(view.componentName ? view.componentName : "?") + "#" + std::to_string(view.tag);
}

void Report::accessibilityEvent(const std::string& type, const facebook::react::ShadowView& view,
                                const std::string& within) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "a11y_event")("event", type)("target", name(view))("within", within));
}

void Report::accessibility(uint32_t surface, const folly::dynamic& nodes) {
  if (!file_) return;
  auto it = accessibility_.find(surface);
  if (it != accessibility_.end() && it->second == nodes) return;
  accessibility_[surface] = nodes;
  write(folly::dynamic::object("kind", "a11y")("surface", surface)("nodes", nodes));
}

void Report::animated(const facebook::react::ShadowView& view, const folly::dynamic& props) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "animated")("target", name(view))("props", props));
}

void Report::committed(const ShadowViewMutationList& mutations) {
  if (!file_) return;
  for (const auto& mutation : mutations) {
    if (mutation.type != ShadowViewMutation::Update) continue;
    auto before = std::dynamic_pointer_cast<const ViewProps>(mutation.oldChildShadowView.props);
    auto after = std::dynamic_pointer_cast<const ViewProps>(mutation.newChildShadowView.props);
    if (!before || !after || after->testId.empty()) continue;
    folly::dynamic props = folly::dynamic::object();
    if (before->opacity != after->opacity) props["opacity"] = after->opacity;
    if (before->transform != after->transform) {
      folly::dynamic transform = folly::dynamic::array;
      for (const auto& operation : after->transform.operations) {
        if (operation.type == TransformOperationType::Translate) {
          transform.push_back(folly::dynamic::object("translateX", operation.x.value));
          transform.push_back(folly::dynamic::object("translateY", operation.y.value));
        }
        // A uniform scale as one value (the dock's Trash under a drag, docs/adr/0044-system-sounds.mdx).
        if (operation.type == TransformOperationType::Scale) {
          if (operation.x.value == operation.y.value) {
            transform.push_back(folly::dynamic::object("scale", operation.x.value));
          } else {
            transform.push_back(folly::dynamic::object("scaleX", operation.x.value));
            transform.push_back(folly::dynamic::object("scaleY", operation.y.value));
          }
        }
      }
      props["transform"] = std::move(transform);
    }
    // The top corners' radii as drawn (the dock's shelf, docs/adr/0034-the-dock.mdx).
    auto radii = [](const ShadowView& view, const ViewProps& props) {
      return props.resolveBorderMetrics(view.layoutMetrics).borderRadii;
    };
    auto was = radii(mutation.oldChildShadowView, *before);
    auto now = radii(mutation.newChildShadowView, *after);
    if (was.topLeft.horizontal != now.topLeft.horizontal) props["borderTopLeftRadius"] = now.topLeft.horizontal;
    if (was.topRight.horizontal != now.topRight.horizontal) props["borderTopRightRadius"] = now.topRight.horizontal;
    // Its width as laid out (an application's place in the dock opening and closing as icons are
    // dragged along it, docs/adr/0034-the-dock.mdx, decision 10).
    auto wasWidth = mutation.oldChildShadowView.layoutMetrics.frame.size.width;
    auto nowWidth = mutation.newChildShadowView.layoutMetrics.frame.size.width;
    if (wasWidth != nowWidth) props["width"] = nowWidth;
    if (!props.empty()) {
      write(folly::dynamic::object("kind", "committed")("target", after->testId)("props", std::move(props)));
    }
  }
}

void Report::animationFrame(double timestamp, size_t callbacks) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "animation_frame")("timestamp", timestamp)("callbacks", callbacks));
}

void Report::write(folly::dynamic&& line) {
  if (!file_) return;
  std::lock_guard lock(mutex_);
  line["t"] = monotonicNs();
  std::string text = folly::toJson(line);
  std::fwrite(text.data(), 1, text.size(), file_);
  std::fputc('\n', file_);
  std::fflush(file_);
}

void Report::frame(uint32_t surface, int width, int height, double renderMs, const std::vector<std::string>& texts,
                   const std::string& focus, double damaged, bool presented) {
  if (!file_) return;
  folly::dynamic list = folly::dynamic::array;
  for (const auto& text : texts) list.push_back(text);
  std::lock_guard lock(mutex_);
  // The work so far in this turn is the frame's.
  if (turning_) {
    auto now = std::chrono::steady_clock::now();
    busyMs_ += std::chrono::duration<double, std::milli>(now - turn_).count();
    turn_ = now;
  }
  // The loop's work and JavaScript's on its thread (counted once: the loop's own JavaScript, worklets',
  // is in its turns).
  write(folly::dynamic::object("kind", "frame")("surface", surface)("width", width)("height", height)(
      "render_ms", renderMs)("busy_ms", busyMs_ + jsThreadMs_)("js_ms", js_.wallMs)("js_cpu_ms", js_.cpuMs)(
      "js_blocked_ms", js_.blockedMs)("react_commits", reactCommits_)("damaged", damaged)(
      "texts", std::move(list))(
      "focus", focus.empty() ? folly::dynamic(nullptr) : folly::dynamic(focus))("presented", presented));
  busyMs_ = jsThreadMs_ = 0;
  js_ = {};
  reactCommits_ = 0;
}

void Report::stage(const char* name) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "stage")("name", name));
}

void Report::beginTurn() {
  if (!file_) return;
  std::lock_guard lock(mutex_);
  turn_ = std::chrono::steady_clock::now();
  turning_ = true;
}

void Report::endTurn() {
  if (!file_) return;
  std::lock_guard lock(mutex_);
  if (!turning_) return;
  busyMs_ += std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - turn_).count();
  turning_ = false;
}

void Report::javascript(const ThreadSpan& span, bool onItsThread) {
  if (!file_) return;
  std::lock_guard lock(mutex_);
  js_ += span;
  if (onItsThread) jsThreadMs_ += span.wallMs;
}

void Report::watchReactCommits(facebook::jsi::Runtime& runtime) {
  using namespace facebook;
  runtime.global().setProperty(
      runtime, "__oxyosReactCommitted",
      jsi::Function::createFromHostFunction(
          runtime, jsi::PropNameID::forAscii(runtime, "__oxyosReactCommitted"), 1,
          [this](jsi::Runtime& rt, const jsi::Value&, const jsi::Value* args, size_t count) {
            {
              std::lock_guard lock(mutex_);
              ++reactCommits_;
            }
            // [[name, count, {name: count, …}], …]: where each re-render began, how many components
            // it rendered and which.
            if (count == 0 || !args[0].isObject()) return jsi::Value::undefined();
            auto renders = jsi::dynamicFromValue(rt, args[0]);
            if (renders.empty()) return jsi::Value::undefined();
            write(folly::dynamic::object("kind", "react_render")("renders", std::move(renders)));
            return jsi::Value::undefined();
          }));
  // The smallest hook React's renderers accept (react-reconciler's injectInternals): they inject
  // themselves, then call onCommitFiberRoot after every commit, with the root whose `current` is
  // the tree just committed and its `alternate` the one before. Its walk is React DevTools' own
  // (renderer.js, updateFiberRecursively and didFiberRender): a subtree whose child is the one the
  // tree before had was not reconciled, and nothing in it rendered; a fiber that is new mounted,
  // and one of the tree before rendered when its PerformedWork flag (1) is set, which React
  // clears on every fiber it begins again. Components are function (0), class (1), forwardRef
  // (11) and simple memo (15) fibers; a memo of another kind (14) is counted by the component
  // inside it. A component is named by its displayName, else its function's name.
  static const char* hook =
      "(() => {"
      "  const component = (f) => f.tag === 0 || f.tag === 1 || f.tag === 11 || f.tag === 15;"
      "  const named = (f) => {"
      "    const t = f.type;"
      "    if (!t) return 'Anonymous';"
      "    if (f.tag === 11) return t.displayName || (t.render && (t.render.displayName || t.render.name)) || 'ForwardRef';"
      "    return t.displayName || t.name || 'Anonymous';"
      "  };"
      "  const walk = (first, within, tops) => {"
      "    for (let f = first; f !== null; f = f.sibling) {"
      "      const before = f.alternate;"
      "      let inside = within;"
      "      if (component(f)) {"
      "        if (before === null || (f.flags & 1) !== 0) {"
      "          const name = named(f);"
      "          if (within === null) { inside = [name, 0, {}]; tops.push(inside); } else inside = within;"
      "          inside[1] += 1;"
      "          inside[2][name] = (inside[2][name] || 0) + 1;"
      "        } else inside = null;"
      "      }"
      "      if (f.child !== null && (before === null || f.child !== before.child)) walk(f.child, inside, tops);"
      "    }"
      "  };"
      "  globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {"
      "    supportsFiber: true, renderers: new Map(),"
      "    inject(internals) { const id = this.renderers.size + 1; this.renderers.set(id, internals); return id; },"
      "    onCommitFiberRoot(id, root) {"
      "      const tops = [];"
      "      const current = root && root.current;"
      "      if (current && current.child !== null && (current.alternate === null || current.child !== current.alternate.child))"
      "        walk(current.child, null, tops);"
      "      __oxyosReactCommitted(tops);"
      "    },"
      "    onCommitFiberUnmount() {}, onPostCommitFiberRoot() {}, checkDCE() {},"
      "  };"
      "})();";
  // Without OXYOS_HOST_REPORT_RENDERS=1 (regress.py sets it for a scenario that holds `renders`),
  // commits are counted and nothing more: the walk's allocations move where Hermes' collections
  // fall, so a run that measures speed (perf.py) never walks, nor does a session.
  static const char* count =
      "globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {"
      "  supportsFiber: true, renderers: new Map(),"
      "  inject(internals) { const id = this.renderers.size + 1; this.renderers.set(id, internals); return id; },"
      "  onCommitFiberRoot() { __oxyosReactCommitted(); },"
      "  onCommitFiberUnmount() {}, onPostCommitFiberRoot() {}, checkDCE() {},"
      "};";
  const char* renders = std::getenv("OXYOS_HOST_REPORT_RENDERS");
  bool walk = renders != nullptr && std::string(renders) == "1";
  runtime.evaluateJavaScript(std::make_shared<jsi::StringBuffer>(walk ? hook : count), "oxyos-report-react-commits");
}

void Report::layout(float width, float height, double ms) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "layout")("width", width)("height", height)("ms", ms));
}

void Report::image(const char* event, const std::string& uri) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "image")("event", event)("uri", uri));
}

void Report::imageLayers(const ShadowView& view, const folly::dynamic& layers) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "image_layers")("target", name(view))("layers", layers));
}

void Report::worklet(const std::string& url, bool compiledAhead) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "worklet")("url", url)("ahead", compiledAhead));
}

void Report::refresh(uint32_t mhz) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "refresh")("mhz", mhz));
}

void Report::settled(double ms, int turns, bool timedOut) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "settle")("ms", ms)("turns", turns)("timed_out", timedOut));
}

void Report::damageMismatch(uint32_t surface, size_t pixels) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "damage_mismatch")("surface", surface)("pixels", pixels));
}

void Report::renderCheck(uint32_t surface, const FrameDifference& difference) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "render_check")("surface", surface)("largest", difference.largest)(
      "beyond_three", difference.beyondThree)("pixels", difference.pixels)("blocks", difference.blocks));
}

void Report::present(uint32_t surface, int width, int height, bool gpu, const folly::dynamic& parts) {
  if (!file_) return;
  folly::dynamic line = parts;
  line["kind"] = "present";
  line["surface"] = surface;
  line["width"] = width;
  line["height"] = height;
  line["gpu"] = gpu;
  write(std::move(line));
}

void Report::feedback(uint32_t surface, uint64_t sequence, bool presented) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "feedback")("surface", surface)("sequence", sequence)("presented", presented));
}

void Report::input(const char* event, uint32_t surface, const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "input";
  line["event"] = event;
  line["surface"] = surface;
  write(std::move(line));
}

void Report::fling(const ShadowView& view, float vx, float vy, double dx, double dy, double ms, float friction) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "fling")("target", name(view))("vx", vx)("vy", vy)("dx", dx)("dy", dy)("ms", ms)(
      "friction", friction));
}

void Report::stretch(const ShadowView& view, float x, float y, float distanceX, float distanceY) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "stretch")("target", name(view))("x", x)("y", y)("pulled_x", distanceX)(
      "pulled_y", distanceY));
}

void Report::scrollbar(const char* event, const ShadowView& view, folly::dynamic axes, folly::dynamic thumbs,
                       float alpha) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "scrollbar")("event", event)("target", name(view))("axes", std::move(axes))(
      "thumbs", std::move(thumbs))("alpha", alpha));
}

void Report::textScroll(const ShadowView& view, float x, float y) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "text_scroll")("target", name(view))("x", x)("y", y));
}

void Report::composition(const ShadowView& view, const folly::dynamic& state) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "composition")("target", name(view))("state", state));
}

void Report::deliver(const std::string& event, uint32_t surface, const ShadowView& view, const folly::dynamic& extra) {
  if (!file_) return;
  deliver(event, surface, name(view), extra);
}

void Report::deliver(const std::string& event, uint32_t surface, const std::string& target,
                     const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "deliver";
  line["event"] = event;
  line["surface"] = surface;
  line["target"] = target;
  if (handlingKind_) {
    line["input_serial"] = handlingSerial_;
    line["input_seq"] = handlingSeq_;
    line["cause"] = handlingKind_;
    line["cause_surface"] = handlingSurface_;
  }
  write(std::move(line));
}

const char* Report::inputKind(const OxyOSEvent& event) {
  static const char* pointer[] = {nullptr,         "pointer.enter",   "pointer.leave", "pointer.motion",
                                  "pointer.press", "pointer.release", "pointer.cancel"};
  static const char* touch[] = {nullptr, "touch.down", "touch.motion", "touch.up", "touch.cancel"};
  static const char* pinch[] = {nullptr, "pinch.begin", "pinch.update", "pinch.end", "pinch.cancel"};
  static const char* hold[] = {nullptr, "hold.begin", "hold.update", "hold.end", "hold.cancel"};
  switch (event.kind) {
    case OXYOS_EVENT_POINTER: return event.detail <= 6 ? pointer[event.detail] : nullptr;
    case OXYOS_EVENT_KEY: return event.flag ? "key.press" : "key.release";
    case OXYOS_EVENT_TOUCH: return event.detail <= 4 ? touch[event.detail] : nullptr;
    case OXYOS_EVENT_SCROLL: return "scroll";
    case OXYOS_EVENT_TEXT: return "text";
    case OXYOS_EVENT_PREEDIT: return "preedit";
    case OXYOS_EVENT_TEXT_INPUT: return "text_input";
    case OXYOS_EVENT_ACCESSIBILITY_ACTION: return "a11y.action";
    case OXYOS_EVENT_PINCH: return event.detail <= 4 ? pinch[event.detail] : nullptr;
    case OXYOS_EVENT_HOLD: return event.detail <= 4 ? hold[event.detail] : nullptr;
    case OXYOS_EVENT_FOCUS: return event.flag ? "focus.in" : "focus.out";
    case OXYOS_EVENT_MENU_ITEM_ACTIVATED: return "menu.activated";
    case OXYOS_EVENT_SYSTEM_SHORTCUT: return "shortcut";
    case OXYOS_EVENT_DRAG_ENTER: return "drag.enter";
    case OXYOS_EVENT_DRAG_MOTION: return "drag.motion";
    case OXYOS_EVENT_DRAG_LEAVE: return "drag.leave";
    case OXYOS_EVENT_DRAG_DROP: return "drag.drop";
    default: return nullptr;
  }
}

void Report::received(const OxyOSEvent& event) {
  if (!file_) return;
  const char* kind = inputKind(event);
  if (!kind) return;
  // A system shortcut names its output, not a surface (oxyos-compositor's delivery_kind alike).
  uint32_t surface = event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT ? event.code : event.surface;
  write(folly::dynamic::object("kind", "received")("event", kind)("surface", surface)("serial", event.serial)(
      "pid", static_cast<int64_t>(getpid()))("seq", ++received_));
}

Report::Handling::Handling(const OxyOSEvent& event) : Handling(event, Report::get().received_) {}

Report::Handling::Handling(const OxyOSEvent& event, uint64_t seq) {
  auto& report = Report::get();
  kind_ = report.handlingKind_;
  serial_ = report.handlingSerial_;
  surface_ = report.handlingSurface_;
  seq_ = report.handlingSeq_;
  if (const char* kind = inputKind(event)) {
    report.handlingKind_ = kind;
    // The event `received` wrote last, when it is this one (Window::dispatch writes it first), or
    // the number it was received as (an event handled after others received since).
    report.handlingSeq_ = seq;
    report.handlingSerial_ = event.serial;
    report.handlingSurface_ = event.kind == OXYOS_EVENT_SYSTEM_SHORTCUT ? event.code : event.surface;
  }
}

Report::Handling::~Handling() {
  auto& report = Report::get();
  report.handlingKind_ = kind_;
  report.handlingSerial_ = serial_;
  report.handlingSurface_ = surface_;
  report.handlingSeq_ = seq_;
}

void Report::gesture(const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "gesture";
  write(std::move(line));
}

void Report::modalAnimation(const char* event, const char* kind, const ShadowView& view, const std::string& within) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "modal_animation")("event", event)("animation", kind)("target", name(view))(
      "within", within));
}

void Report::modalPresented(const char* event, const std::string& within, float offset, float alpha, float height) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "modal_presented")("event", event)("within", within)("offset", offset)(
      "alpha", alpha)("height", height));
}

void Report::popup(const char* event, uint32_t surface, const ShadowView* view, const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "popup";
  line["event"] = event;
  line["surface"] = surface;
  if (view) line["view"] = name(*view);
  write(std::move(line));
}

}  // namespace oxyos
