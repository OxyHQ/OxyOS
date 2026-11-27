// React Native's Android ProgressBar on OxyOS (`AndroidProgressBar`, what ActivityIndicator renders
// on Android; docs/react-native-android-adaptation.mdx): an explicit adaptation of AOSP's
// `ProgressBar` and React Native's `ReactProgressBarViewManager`, drawn by the host.
//
// - Its size, without a width or height, is its style's (`styleAttr`): Small 16, Normal 48 and
//   Large 76 dp, as Android's ProgressBar styles measure (ActivityIndicator sets 20 or 36).
// - Indeterminate, it is AOSP's `progress_indeterminate_material` ring: a 48-unit viewport's
//   circle of radius 19 stroked 4 wide (Small: a 12-unit one's of radius 5 stroked 1), scaled to
//   the box, its arc trimmed over 1333 ms (the end ahead in the cycle's first half, the start
//   catching up in its second, both eased as AOSP's trim path interpolators) and offset by a quarter
//   turn a cycle, the whole ring turning 720 degrees in 4444 ms. Determinate (`indeterminate`
//   false), a still arc of `progress` from 12 o'clock. Not `animating`, it draws nothing and the
//   assistive technology does not see it, as React Native's ProgressBarContainerView hides it.
// - Its colour is `color`, else the session's accent (Android's colorControlActivated).
// - Reduce Motion: the ring goes on turning, as a Mac's spinners go on spinning while something is
//   under way (its motion is the state it shows), but its arc is held at three quarters of a turn:
//   the sweep of the arc growing and shrinking, motion that only decorates, is what Reduce Motion
//   takes away (docs/adr/0019-settings.mdx).
// - Spinning, it is drawn again each frame of its surface's display, its own rectangle alone
//   damaged (damage.h), on the UI thread: JavaScript neither drives nor holds it up. A frame that
//   changes nothing else says so (its report's `present` line, `progress_only`): what a step's
//   settling leaves aside (tools/oxyos-offscreen/src/progress.rs). A ring nothing shows (scrolled
//   away) does not turn.
// - The assistive technology sees a progress indicator (ADR 0003: `ProgressBar`), named by its
//   label: busy while indeterminate, its value the percentage while determinate.
#pragma once

#include <react/renderer/components/FBReactNativeSpec/EventEmitters.h>
#include <react/renderer/components/FBReactNativeSpec/Props.h>
#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>
#include <react/renderer/mounting/ShadowView.h>

#include <cstdint>
#include <string>

// Skia's canvas, which only the drawing below takes (the component's registration builds without
// Skia).
class SkCanvas;

namespace oxyos {

extern const char kAndroidProgressBar[];

class ProgressBarShadowNode final
    : public facebook::react::ConcreteViewShadowNode<kAndroidProgressBar, facebook::react::AndroidProgressBarProps,
                                                     facebook::react::AndroidProgressBarEventEmitter> {
 public:
  using ConcreteViewShadowNode::ConcreteViewShadowNode;
  static facebook::react::ShadowNodeTraits BaseTraits() {
    auto traits = ConcreteViewShadowNode::BaseTraits();
    traits.set(facebook::react::ShadowNodeTraits::Trait::LeafYogaNode);
    traits.set(facebook::react::ShadowNodeTraits::Trait::MeasurableYogaNode);
    return traits;
  }
  facebook::react::Size measureContent(const facebook::react::LayoutContext& layoutContext,
                                       const facebook::react::LayoutConstraints& layoutConstraints) const override;
};

using ProgressBarComponentDescriptor = facebook::react::ConcreteComponentDescriptor<ProgressBarShadowNode>;

// Its side for a style (`styleAttr`), in dp.
float progressBarSize(const std::string& style);
// Whether it is drawn (`animating`), and whether it spins (drawn and indeterminate).
bool progressBarShown(const facebook::react::AndroidProgressBarProps& props);
bool progressBarSpins(const facebook::react::AndroidProgressBarProps& props);
// Whether a view is a progress bar that spins.
bool spins(const facebook::react::ShadowView& view);

// The ring's arc, in turns clockwise from 12 o'clock: from `from` to `to` (to >= from).
struct ProgressArc {
  double from = 0, to = 0;
};
// An indeterminate ring's arc `ms` into its animation.
ProgressArc indeterminateArc(double ms, bool reducedMotion);
// A determinate one's, `progress` (0 to 1) of a turn.
ProgressArc determinateArc(double progress);

// Draws it in the box x, y, w, h (logical pixels) of `canvas`: in `color` (0xRRGGBB) unless its
// props give one, at `opacity`, `ms` into its animation.
void drawProgressBar(SkCanvas& canvas, const facebook::react::AndroidProgressBarProps& props, float x, float y, float w,
                     float h, uint32_t accent, float opacity, double ms, bool reducedMotion);

// The clock its animation runs on: milliseconds of the monotonic clock, or a scenario's phase
// (OXYOS_HOST_PROGRESS_PHASE_MS, regress.py's `progress_phase_ms`): the ring still drawn again each
// frame of its display, every frame at that point of its animation, so that a scenario's pixels can
// say where its arc is.
double progressClockMs();
// Reduce Motion, the session's (set by the host as it is told it).
void setProgressReducedMotion(bool reduced);
bool progressReducedMotion();

}  // namespace oxyos
