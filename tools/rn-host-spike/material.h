// The platform's material in the Bloom host (docs/adr/0069-the-platforms-material.mdx, decisions 1
// and 7): the `OxyOSMaterialView` host component, which sample/oxyos/platform-surface.js gives Bloom
// as the adapter's `Material` (Bloom mounts it under a floating surface's tint). It has no content.
//
// It is drawn in two parts, each by the one that owns the pixels:
// - The host blurs what the surface has drawn beneath it (renderer.cpp's drawView): a backdrop
//   filter over the canvas, a Gaussian of σ = 20 logical pixels (the canvas's matrix takes it to the
//   surface's pixels, as the compositor's σ is 20 logical pixels at the output's scale), then the
//   saturation × 1.8 by Rec. 709's luminance, within its rounded shape. Where nothing of the surface
//   lies beneath (a popup's transparent back), that is transparent.
// - The compositor blurs the other surfaces beneath it: the view's frame and corners are its
//   surface's material regions (`RenderStats::materials`), sent with each frame that changes them
//   (Window::setMaterialRegions) to a compositor of protocol version 45 or later.
//
// With the session's Reduce transparency on (expo-blur.h's reducedTransparency), it is drawn as
// nothing and is no region: Bloom does not mount it then, and this is the second guard.
#pragma once

#include <folly/dynamic.h>
#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <cstdint>
#include <vector>

namespace facebook::react {

extern const char OxyOSMaterialViewComponentName[];

using OxyOSMaterialViewShadowNode = ConcreteViewShadowNode<OxyOSMaterialViewComponentName, ViewProps>;
using OxyOSMaterialViewComponentDescriptor = ConcreteComponentDescriptor<OxyOSMaterialViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

// The material's blur, a Gaussian's deviation in logical pixels (ADR 0069, decision 2).
inline constexpr float kMaterialSigma = 20;
// What reaches into a material's pixels around it, in logical pixels: three deviations, the
// Gaussian's extent. A change there changes the material (damage.cpp).
inline constexpr float kMaterialReach = 3 * kMaterialSigma;
// Its saturation (ADR 0069, decision 2).
inline constexpr float kMaterialSaturation = 1.8f;

// Whether a component is the material view.
bool isMaterialView(const char* name);
// Material regions as the host's report gives them (`material.regions`): each [x, y, width,
// height, radius] in the surface's pixels.
folly::dynamic materialReport(const std::vector<int32_t>& regions);

}  // namespace oxyos
