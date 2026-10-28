// What the host showed and delivered, for regression checks (tools/rn-host-spike/regress.py):
// with OXY_HOST_REPORT=<path>, one JSON object per line.
//
//   {"t":…,"kind":"frame","surface":1,"width":…,"height":…,"render_ms":…,"texts":[…],"focus":"reply"}
//   {"t":…,"kind":"input","event":"pointer.press","surface":2}        (from the compositor)
//   {"t":…,"kind":"deliver","event":"touchStart","surface":1,"target":"open-menu"}  (to React)
//   {"t":…,"kind":"popup","event":"open","surface":2,"view":"menu"}
//
// `t` is CLOCK_MONOTONIC in nanoseconds, the clock oxy-offscreen stamps its steps with, so the
// two logs line up. Views are named by testID, else by accessibility label, else component#tag.
#pragma once

#include <react/renderer/core/ReactPrimitives.h>
#include <react/renderer/mounting/ShadowView.h>

#include <folly/dynamic.h>

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

namespace oxy {

class ViewTree;

class Report {
 public:
  static Report& get();
  bool enabled() const { return file_ != nullptr; }

  // A presented frame (its size in pixels) and the text it shows, in drawing order.
  void frame(uint32_t surface, int width, int height, double renderMs, const std::vector<std::string>& texts,
             const std::string& focus);
  // A compositor event as received.
  void input(const char* event, uint32_t surface, const folly::dynamic& extra = folly::dynamic::object());
  // An event delivered to React (touch, text, scroll, …) on `view`.
  void deliver(const std::string& event, uint32_t surface, const facebook::react::ShadowView& view,
               const folly::dynamic& extra = folly::dynamic::object());
  // A popup's lifecycle.
  void popup(const char* event, uint32_t surface, const facebook::react::ShadowView* view,
             const folly::dynamic& extra = folly::dynamic::object());

  static std::string name(const facebook::react::ShadowView& view);

 private:
  Report();
  void write(folly::dynamic&& line);
  std::FILE* file_ = nullptr;
};

}  // namespace oxy
