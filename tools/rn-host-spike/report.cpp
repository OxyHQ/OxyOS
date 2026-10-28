#include "report.h"

#include <react/renderer/components/view/ViewProps.h>

#include <folly/json.h>

#include <cstdlib>
#include <ctime>

using namespace facebook::react;

namespace oxy {

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
  if (const char* path = std::getenv("OXY_HOST_REPORT")) file_ = std::fopen(path, "w");
}

std::string Report::name(const ShadowView& view) {
  if (auto props = std::dynamic_pointer_cast<const ViewProps>(view.props)) {
    if (!props->testId.empty()) return props->testId;
    if (!props->accessibilityLabel.empty()) return props->accessibilityLabel;
  }
  return std::string(view.componentName ? view.componentName : "?") + "#" + std::to_string(view.tag);
}

void Report::accessibility(uint32_t surface, const folly::dynamic& nodes) {
  if (!file_) return;
  auto it = accessibility_.find(surface);
  if (it != accessibility_.end() && it->second == nodes) return;
  accessibility_[surface] = nodes;
  write(folly::dynamic::object("kind", "a11y")("surface", surface)("nodes", nodes));
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
                   const std::string& focus) {
  if (!file_) return;
  folly::dynamic list = folly::dynamic::array;
  for (const auto& text : texts) list.push_back(text);
  write(folly::dynamic::object("kind", "frame")("surface", surface)("width", width)("height", height)(
      "render_ms", renderMs)("texts", std::move(list))(
      "focus", focus.empty() ? folly::dynamic(nullptr) : folly::dynamic(focus)));
}

void Report::input(const char* event, uint32_t surface, const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "input";
  line["event"] = event;
  line["surface"] = surface;
  write(std::move(line));
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

void Report::popup(const char* event, uint32_t surface, const ShadowView* view, const folly::dynamic& extra) {
  if (!file_) return;
  folly::dynamic line = extra;
  line["kind"] = "popup";
  line["event"] = event;
  line["surface"] = surface;
  if (view) line["view"] = name(*view);
  write(std::move(line));
}

}  // namespace oxy
