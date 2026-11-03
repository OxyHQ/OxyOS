// react-native-svg on OxyOS: its components are built from its own C++ shadow nodes over codegen's
// props, as on Android, and an SVG view's subtree is drawn as react-native-svg's Android views draw
// it, through Skia: its elements' paths (transforms on the canvas, inherited fill and stroke
// resolved, currentColor taken from the nearest `color`, the view box fitted to the view) recorded
// as one picture the view draws at any scale. Fills and strokes take colours and `url(#id)`
// references to linear and radial gradients (Skia's gradient shaders) and patterns (a picture
// shader, tiled at the output's resolution); elements are clipped by `<ClipPath>` and masked by
// `<Mask>` (luminance or alpha, in a layer), as its Android RenderableView does; `<Image href>`
// draws the picture images.h loads for it, by preserveAspectRatio. What it does not draw yet
// (markers, filters, text, `use`) is reported once, never faked.
#pragma once

#include "renderer.h"

#include <react/renderer/componentregistry/ComponentDescriptorProviderRegistry.h>

#include <include/core/SkPicture.h>

namespace oxyos {

// Registers react-native-svg's components.
void addSvgComponents(facebook::react::ComponentDescriptorProviderRegistry& providers);

// Whether `name` is an SVG view, whose subtree drawView draws through svgPicture.
bool isSvgView(const std::string& name);

// The SVG view `tag`'s elements, in its content box's logical pixels; kept until the view's subtree
// or size changes. Null if there is nothing to draw.
sk_sp<SkPicture> svgPicture(const ViewTree& tree, facebook::react::Tag tag);

}  // namespace oxyos
