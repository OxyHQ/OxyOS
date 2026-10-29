// react-native-svg on OxyOS: its components are built from its own C++ shadow nodes over codegen's
// props, as on Android, and an SVG view's subtree is drawn as one picture: its elements flattened
// into shapes (transforms composed, inherited fill and stroke resolved, currentColor taken from
// the nearest `color`, the view box fitted to the view) and rasterized by crates/oxyos-vector
// through the C ABI. What it does not draw yet (gradients and patterns, clip paths and masks,
// markers, filters, text, images, `use`) is reported once, never faked.
#pragma once

#include "renderer.h"

#include <react/renderer/componentregistry/ComponentDescriptorProviderRegistry.h>

#include <memory>

namespace oxyos {

// Registers react-native-svg's components.
void addSvgComponents(facebook::react::ComponentDescriptorProviderRegistry& providers);

// Whether `name` is an SVG view, whose subtree drawView draws through svgPicture.
bool isSvgView(const std::string& name);

// The SVG view `tag` drawn at `scale` (device pixels per logical pixel), its content box's size;
// kept until the view's subtree, size or scale changes. Null if there is nothing to draw.
std::shared_ptr<const Picture> svgPicture(const ViewTree& tree, facebook::react::Tag tag, float scale);

}  // namespace oxyos
