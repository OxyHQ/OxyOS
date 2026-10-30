#include "report.h"

#include <react/renderer/components/view/ViewProps.h>

#include <folly/json.h>

#include <cstdlib>
#include <ctime>

using namespace facebook::react;

namespace oxyos {

namespace {

int64_t monotonicNs() {
  timespec now{};
  clock_gettime(CLOCK_MONOTONIC, &now);
  return static_cast<int64_t>(now.tv_sec) * 1'000'000'000 + now.tv_nsec;
}

}  // namespace

Report& Report::get() {
  static Report report;
  return report;
}

Report::Report() {
  if (const char* path = std::getenv("OXYOS_HOST_REPORT")) file_ = std::fopen(path, "w");
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
      }
      props["transform"] = std::move(transform);
    }
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
  // The work so far in this turn is the frame's.
  if (turning_) {
    auto now = std::chrono::steady_clock::now();
    busyMs_ += std::chrono::duration<double, std::milli>(now - turn_).count();
    turn_ = now;
  }
  write(folly::dynamic::object("kind", "frame")("surface", surface)("width", width)("height", height)(
      "render_ms", renderMs)("busy_ms", busyMs_)("js_ms", jsMs_)("damaged", damaged)("texts", std::move(list))(
      "focus", focus.empty() ? folly::dynamic(nullptr) : folly::dynamic(focus))("presented", presented));
  busyMs_ = 0;
  jsMs_ = 0;
}

void Report::stage(const char* name) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "stage")("name", name));
}

void Report::beginTurn() {
  if (!file_) return;
  turn_ = std::chrono::steady_clock::now();
  turning_ = true;
}

void Report::endTurn() {
  if (!file_ || !turning_) return;
  busyMs_ += std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - turn_).count();
  turning_ = false;
}

void Report::javascript(double ms) {
  if (file_) jsMs_ += ms;
}

void Report::layout(float width, float height, double ms) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "layout")("width", width)("height", height)("ms", ms));
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

void Report::renderCheck(uint32_t surface, int largest, size_t beyondThree, size_t pixels) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "render_check")("surface", surface)("largest", largest)(
      "beyond_three", beyondThree)("pixels", pixels));
}

void Report::input(const char* event, uint32_t surface, const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "input";
  line["event"] = event;
  line["surface"] = surface;
  write(std::move(line));
}

void Report::textScroll(const ShadowView& view, float x, float y) {
  if (!file_) return;
  write(folly::dynamic::object("kind", "text_scroll")("target", name(view))("x", x)("y", y));
}

void Report::deliver(const std::string& event, uint32_t surface, const ShadowView& view, const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "deliver";
  line["event"] = event;
  line["surface"] = surface;
  line["target"] = name(view);
  write(std::move(line));
}

void Report::deliver(const std::string& event, uint32_t surface, const std::string& target,
                     const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "deliver";
  line["event"] = event;
  line["surface"] = surface;
  line["target"] = target;
  write(std::move(line));
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
