// What the host showed and delivered, for regression checks (tools/rn-host-spike/regress.py):
// with OXYOS_HOST_REPORT=<path>, one JSON object per line.
//
//   {"t":…,"kind":"frame","surface":1,"width":…,"height":…,"render_ms":…,"texts":[…],"focus":"reply"}
//   {"t":…,"kind":"input","event":"pointer.press","surface":2}        (from the compositor)
//   {"t":…,"kind":"deliver","event":"touchStart","surface":1,"target":"open-menu"}  (to React)
//   {"t":…,"kind":"popup","event":"open","surface":2,"view":"menu"}
//   {"t":…,"kind":"a11y_event","event":"focus","target":"open-menu","within":"open-menu"}
//   {"t":…,"kind":"text_scroll","target":"notes","x":0,"y":21}  (an input's text scrolled)
//   {"t":…,"kind":"a11y","surface":2,"nodes":[{"role":"menuitem","name":"Reply",…},…]}  (on change)
//
// `t` is CLOCK_MONOTONIC in nanoseconds, the clock oxyos-offscreen stamps its steps with, so the
// two logs line up. Views are named by testID, else by accessibility label, else component#tag.
#pragma once

#include <react/renderer/core/ReactPrimitives.h>
#include <react/renderer/mounting/ShadowView.h>

#include <folly/dynamic.h>

#include <cstdint>
#include <cstdio>
#include <map>
#include <string>
#include <vector>

namespace oxyos {

class ViewTree;

class Report {
 public:
  static Report& get();
  bool enabled() const { return file_ != nullptr; }

  // A presented frame (its size in pixels) and the text it shows, in drawing order.
  // `damaged` is the fraction of the frame drawn again (1 when drawn whole). A frame not
  // `presented` is what the surface still shows with a change that drew nothing (a new focus).
  void frame(uint32_t surface, int width, int height, double renderMs, const std::vector<std::string>& texts,
             const std::string& focus, double damaged = 1.0, bool presented = true);
  // A frame drawn from its damage that differs from the frame drawn whole (OXYOS_HOST_DAMAGE_CHECK):
  // `pixels` of them. Any is a defect in the damage.
  void damageMismatch(uint32_t surface, size_t pixels);
  // A compositor event as received.
  void input(const char* event, uint32_t surface, const folly::dynamic& extra = folly::dynamic::object());
  // An event delivered to React (touch, text, scroll, …) on `view`.
  void deliver(const std::string& event, uint32_t surface, const facebook::react::ShadowView& view,
               const folly::dynamic& extra = folly::dynamic::object());
  // How far a text input's text is now scrolled within it (textinput.h).
  void textScroll(const facebook::react::ShadowView& view, float x, float y);
  // A popup's lifecycle.
  void popup(const char* event, uint32_t surface, const facebook::react::ShadowView* view,
             const folly::dynamic& extra = folly::dynamic::object());

  // An accessibility event JavaScript sent for `view` (AccessibilityInfo.sendAccessibilityEvent).
  // `within` names the nearest view with a testID, the view itself included.
  void accessibilityEvent(const std::string& type, const facebook::react::ShadowView& view, const std::string& within);
  // A surface's accessibility tree (accessibility.h), written when it changed.
  void accessibility(uint32_t surface, const folly::dynamic& nodes);

  static std::string name(const facebook::react::ShadowView& view);

 private:
  Report();
  void write(folly::dynamic&& line);
  std::FILE* file_ = nullptr;
  std::map<uint32_t, folly::dynamic> accessibility_;  // the tree last written per surface
};

}  // namespace oxyos
