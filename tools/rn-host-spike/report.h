// What the host showed and delivered, for regression checks (tools/rn-host-spike/regress.py):
// with OXYOS_HOST_REPORT=<path>, one JSON object per line. A path that ends in `/` or names a
// directory is a directory every host writes its own report into, <application>-<pid>.jsonl (the
// application's id, else its module): a session's applications profiled at once, each its own.
//
//   {"t":…,"kind":"frame","surface":1,"width":…,"height":…,"render_ms":…,"busy_ms":…,"js_ms":…,"js_cpu_ms":…,"js_blocked_ms":…,"react_commits":0,"texts":[…],"focus":"reply"}
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
//   {"t":…,"kind":"timing","work_ms":…,"parts":{…},"trigger":"pointer.press",…}  (frame-diagnostics.h: each
//       frame, and work without one, "presented": false, that was slow or ran JavaScript)
//   {"t":…,"kind":"log","text":"[host so.oxy.dropsea] a frame after …"}  (a line written to the log)
//   {"t":…,"kind":"timing_summary","frames":…,"work_p50":…,…}  (the log's summary)
//   {"t":…,"kind":"memory","malloc_in_use":…,"hermes_heap":…,"skia_gpu":…,"buffers":…,…}  (memory.h, idle)
//   {"t":…,"kind":"image","event":"cancelled","uri":"https://…"}  (a remote image's load, images.h)
//   {"t":…,"kind":"react_render","renders":[["DockIcon",9,{"DockIcon":1,"View":4,…}],…]}  (a commit of
//       React's: where each re-render began, how many components rendered in it and which; watchReactCommits)
//
// `t` is CLOCK_MONOTONIC in nanoseconds, the clock oxyos-offscreen stamps its steps with, so the
// two logs line up. Views are named by testID, else by accessibility label, else component#tag.
#pragma once

#include "renderer.h"
#include "thread-clock.h"

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

namespace facebook::jsi {
class Runtime;
}

namespace oxyos {

class ViewTree;

// The file a host writes something of its own to, given a path from its environment: the path
// itself, or, when it ends in `/` or names a directory, `<application>-<pid><extension>` in it
// (created), so that every host of a session writes its own beside the others'.
std::string perProcessPath(std::string target, const std::string& application, const char* extension);

class Report {
 public:
  static Report& get();
  // Opens the report OXYOS_HOST_REPORT names, if any, for `application` (its id, else its module).
  void open(const std::string& application);
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
  // drawing, presenting, the compositor's events). Both by the wall clock; of `js_ms`, `js_cpu_ms`
  // is the loop thread's CPU time (thread-clock.h), the measure of JavaScript's work however
  // loaded the machine, and `js_blocked_ms` the time it was blocked (waiting for another thread, a
  // lock, a file), so that a wait is said, not hidden in the CPU's time.
  void beginTurn();
  void endTurn();
  void javascript(const ThreadSpan& span);
  // React's commits (each render React finished, whether or not it changed a view), written with
  // the next frame as `react_commits`, those since the frame before: React's renderers announce
  // their commits to the global hook its DevTools install, which this installs in `runtime`
  // before any renderer loads. A window resized lays its tree out natively, no commit of React's;
  // only what JavaScript re-rendered for the new size is.
  //
  // Each commit that rendered a component is also written as a `react_render` line: the
  // components where a render began (one whose nearest component above it did not render: a
  // state, a context or a store it reads changed), each with how many components rendered in its
  // subtree for it, itself included, and how many of each name, as React's fibers have it (a function or class component,
  // a forwardRef or a memo whose render ran, its fiber's PerformedWork, or one mounted). What a
  // commit re-rendered is counted, not timed: the same on any machine, however loaded. Only with
  // OXYOS_HOST_REPORT_RENDERS=1 as well (regress.py's, for a scenario holding `renders`): walking
  // allocates, which moves where Hermes' collections fall, so a run measuring speed, and a session,
  // count commits alone.
  void watchReactCommits(facebook::jsi::Runtime& runtime);
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
  // of any channel, how many of its pixels differ by more than 3 levels, and the largest
  // difference between their 4 × 4 blocks' means.
  void renderCheck(uint32_t surface, const FrameDifference& difference);
  // A frame presented: its size, whether on the GPU, and where its time went in milliseconds (on the
  // GPU, oxyos_paint_last_present's parts; in shared memory, the copy and send alone).
  void present(uint32_t surface, int width, int height, bool gpu, const folly::dynamic& parts);
  // The compositor's answer for a frame presented (its sequence): shown, or discarded unshown.
  void feedback(uint32_t surface, uint64_t sequence, bool presented);
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
  // A scroll view's stretch past its end (overscroll, input.h): what is drawn along x and y (a share
  // of its size, negative past its start) and how far it is pulled.
  void stretch(const facebook::react::ShadowView& view, float x, float y, float distanceX, float distanceY);
  // A scroll view's overlay scrollbars changing what they show (scrollbars.h): `event` is `shown`,
  // `wide-x` or `wide-y` (the pointer over one, grown wide) or `hidden`; with the axes that have one,
  // each thumb where it is in the window ([x, y, width, height]) and how opaque they are.
  void scrollbar(const char* event, const facebook::react::ShadowView& view, folly::dynamic axes,
                 folly::dynamic thumbs, float alpha);
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
  // Views with a testID whose opacity, translation or top corners' radii a mounted transaction
  // changed, as committed (React's commits, a JavaScript-driven Animated's and Reanimated's among
  // them): `props` as the native driver's lines have them, the radii as drawn.
  void committed(const facebook::react::ShadowViewMutationList& mutations);
  // A frame of requestAnimationFrame (animation-frames.h): its timestamp, as the callbacks get it,
  // and how many callbacks it ran.
  void animationFrame(double timestamp, size_t callbacks);
  // What the image loader did with a remote image (images.h): "fetch", a request started (or
  // answered from a cache), "cancelled", one no view waits for any more.
  void image(const char* event, const std::string& uri);
  // What an expo-image view draws now (images.h), each time it changes: its layers bottom to top,
  // each one's alpha, whether it is a placeholder and the frame shown, with the time.
  void imageLayers(const facebook::react::ShadowView& view, const folly::dynamic& layers);
  // A popup's lifecycle.
  void popup(const char* event, uint32_t surface, const facebook::react::ShadowView* view,
             const folly::dynamic& extra = folly::dynamic::object());

  // An accessibility event JavaScript sent for `view` (AccessibilityInfo.sendAccessibilityEvent).
  // `within` names the nearest view with a testID, the view itself included.
  void accessibilityEvent(const std::string& type, const facebook::react::ShadowView& view, const std::string& within);
  // A surface's accessibility tree (accessibility.h), written when it changed.
  void accessibility(uint32_t surface, const folly::dynamic& nodes);

  // What the frame diagnostics write (frame-diagnostics.h): a line of `kind`.
  void diagnostic(const char* kind, folly::dynamic&& line);
  // What the host holds in memory, after it gave back what an idle window does not need
  // (memory.h's account, in MiB).
  void memory(folly::dynamic&& account) { diagnostic("memory", std::move(account)); }

  static std::string name(const facebook::react::ShadowView& view);

 private:
  Report();
  void write(folly::dynamic&& line);
  std::FILE* file_ = nullptr;
  std::map<uint32_t, folly::dynamic> accessibility_;  // the tree last written per surface
  std::chrono::steady_clock::time_point turn_{};  // when the work not yet counted began
  bool turning_ = false;
  double busyMs_ = 0;  // work since the last frame written
  ThreadSpan js_;      // JavaScript since the last frame written
  int reactCommits_ = 0;          // React's commits since the last frame written
};

}  // namespace oxyos
