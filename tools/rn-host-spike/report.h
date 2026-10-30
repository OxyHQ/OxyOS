// What the host showed and delivered, for regression checks (tools/rn-host-spike/regress.py):
// with OXYOS_HOST_REPORT=<path>, one JSON object per line.
//
//   {"t":…,"kind":"frame","surface":1,"width":…,"height":…,"render_ms":…,"busy_ms":…,"js_ms":…,"texts":[…],"focus":"reply"}
//   {"t":…,"kind":"layout","width":…,"height":…,"ms":…}         (the window laid out at a new size)
//   {"t":…,"kind":"stage","name":"load script"}                    (a stage of the start begins)
//   {"t":…,"kind":"input","event":"pointer.press","surface":2}        (from the compositor)
//   {"t":…,"kind":"deliver","event":"touchStart","surface":1,"target":"open-menu"}  (to React)
//   {"t":…,"kind":"popup","event":"open","surface":2,"view":"menu"}
//   {"t":…,"kind":"a11y_event","event":"focus","target":"open-menu","within":"open-menu"}
//   {"t":…,"kind":"text_scroll","target":"notes","x":0,"y":21}  (an input's text scrolled)
//   {"t":…,"kind":"gesture","event":"state","handler":"PanGestureHandler","view":"card","state":"ACTIVE",…}
//   {"t":…,"kind":"a11y","surface":2,"nodes":[{"role":"menuitem","name":"Reply",…},…]}  (on change)
//   {"t":…,"kind":"animated","target":"fade","props":{"opacity":0.42}}  (set by Animated's native driver)
//   {"t":…,"kind":"committed","target":"jsbox","props":{"transform":[{"translateX":12.5}]}}  (on mount)
//   {"t":…,"kind":"animation_frame","timestamp":1234.5,"callbacks":2}  (requestAnimationFrame's frame)
//
// `t` is CLOCK_MONOTONIC in nanoseconds, the clock oxyos-offscreen stamps its steps with, so the
// two logs line up. Views are named by testID, else by accessibility label, else component#tag.
#pragma once

#include <react/renderer/core/ReactPrimitives.h>
#include <react/renderer/mounting/ShadowView.h>
#include <react/renderer/mounting/ShadowViewMutation.h>

#include <folly/dynamic.h>

#include <chrono>
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
  // The host loop's work (bloom-host.cpp): what it did between beginTurn and endTurn, never its
  // waiting, is written with the next frame as `busy_ms`, the work since the frame before, and
  // `js_ms`, the part of it running JavaScript (React's runtime and the worklets' UI runtime:
  // rendering, Fabric's commit and Yoga's layout among it); the rest is native (mounting,
  // drawing, presenting, the compositor's events).
  void beginTurn();
  void endTurn();
  void javascript(double ms);
  // A stage of the host's start beginning (the Hermes instance, Fabric's scheduler, the bundle).
  void stage(const char* name);
  // The window laid out at a new size (logical pixels), in `ms` (Fabric's commit with the new
  // constraints, Yoga's layout and the mount).
  void layout(float width, float height, double ms);
  // The window's display refreshes at `mhz` (the compositor's REFRESH): the host's frames' pace.
  void refresh(uint32_t mhz);
  // After a new size, the window waited `ms` for JavaScript to lay it out (bloom-host's
  // settle bound, a frame of the display), over `turns` loop turns; `timedOut` when the bound
  // ended it.
  void settled(double ms, int turns, bool timedOut);
  // A frame drawn from its damage that differs from the frame drawn whole (OXYOS_HOST_DAMAGE_CHECK):
  // `pixels` of them. Any is a defect in the damage.
  void damageMismatch(uint32_t surface, size_t pixels);
  // A frame drawn on the GPU against the same frame drawn whole on the CPU
  // (OXYOS_HOST_RENDER_CHECK, docs/adr/0031-gpu-buffers.mdx, decision 13): the largest difference
  // of any channel, and how many of its pixels differ by more than 3 levels.
  void renderCheck(uint32_t surface, int largest, size_t beyondThree, size_t pixels);
  // A frame presented: its size, whether on the GPU, and where its time went in milliseconds (on the
  // GPU, oxyos_paint_last_present's parts; in shared memory, the copy and send alone).
  void present(uint32_t surface, int width, int height, bool gpu, const folly::dynamic& parts);
  // A compositor event as received.
  void input(const char* event, uint32_t surface, const folly::dynamic& extra = folly::dynamic::object());
  // An event delivered to React (touch, text, scroll, …) on `view`.
  void deliver(const std::string& event, uint32_t surface, const facebook::react::ShadowView& view,
               const folly::dynamic& extra = folly::dynamic::object());
  // An event delivered to JavaScript that no view is the target of (a choice in the shell's menu
  // bar), named `target`.
  void deliver(const std::string& event, uint32_t surface, const std::string& target,
               const folly::dynamic& extra = folly::dynamic::object());
  // What gesture handlers recognized (gesture-handler.h): a state change or an update, as sent to
  // JavaScript, or the gesture taken from React Native.
  void gesture(const folly::dynamic& line);
  // A scroll view flung by a touchpad's fingers (input.h): the velocity they left with (dp per
  // second, along the offset) and the fling's distance on each axis, how long it lasts and its
  // friction.
  void fling(const facebook::react::ShadowView& view, float vx, float vy, double dx, double dy, double ms,
             float friction);
  // How far a text input's text is now scrolled within it (textinput.h).
  void textScroll(const facebook::react::ShadowView& view, float x, float y);
  // A modal's window starting to animate in or out (modal.h): `event` is `in` or `out`, `kind`
  // its animationType; `within` names the modal by its testID.
  void modalAnimation(const char* event, const char* kind, const facebook::react::ShadowView& view,
                      const std::string& within);
  // How an animating modal's window is presented in a frame (modal.h): moved down by `offset`
  // logical pixels of the window's `height`, at `alpha`.
  void modalPresented(const char* event, const std::string& within, float offset, float alpha, float height);
  // Props Animated's native driver set on `view` between commits (native-animated.h), as computed
  // for a frame: what the view is drawn with next.
  void animated(const facebook::react::ShadowView& view, const folly::dynamic& props);
  // Views with a testID whose opacity or translation a mounted transaction changed, as committed
  // (React's commits, a JavaScript-driven Animated's among them): `props` as the native driver's
  // lines have them.
  void committed(const facebook::react::ShadowViewMutationList& mutations);
  // A frame of requestAnimationFrame (animation-frames.h): its timestamp, as the callbacks get it,
  // and how many callbacks it ran.
  void animationFrame(double timestamp, size_t callbacks);
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
  std::chrono::steady_clock::time_point turn_{};  // when the work not yet counted began
  bool turning_ = false;
  double busyMs_ = 0, jsMs_ = 0;  // work since the last frame written
};

}  // namespace oxyos
