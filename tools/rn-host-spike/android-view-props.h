// What a View draws with beyond the portable C++ props: React Native's Android-only view props,
// which only Android's platform props parse (HostPlatformViewProps): `elevation` (the shadow Android
// casts under a view raised off its parent) and `needsOffscreenAlphaCompositing` (whether its
// opacity is a layer, ReactViewGroup.hasOverlappingRendering) and `focusable` (whether the keyboard
// focuses it, View.setFocusable, keyboard-focus.h). The host's View descriptor keeps
// them beside each props as Fabric makes them, as the ScrollView's keeps `overScrollMode`
// (overscroll.h).
#pragma once

#include <react/renderer/components/view/ViewComponentDescriptor.h>

namespace oxyos {

struct AndroidViewProps {
  float elevation = 0;
  bool needsOffscreenAlphaCompositing = false;
  bool focusable = false;
  bool operator==(const AndroidViewProps&) const = default;
};

// What a View's props were made with: the defaults for props this descriptor did not make.
AndroidViewProps androidViewPropsOf(const facebook::react::Props& props);

// React Native's View descriptor, which also keeps each props' Android-only props (a prop it does
// not set inherits its source's, as Fabric's props do).
class ViewComponentDescriptor final : public facebook::react::ViewComponentDescriptor {
 public:
  using facebook::react::ViewComponentDescriptor::ViewComponentDescriptor;

  facebook::react::Props::Shared cloneProps(const facebook::react::PropsParserContext& context,
                                            const facebook::react::Props::Shared& props,
                                            facebook::react::RawProps rawProps) const override;
};

}  // namespace oxyos
