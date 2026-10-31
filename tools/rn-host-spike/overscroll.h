// What a ScrollView's overscroll needs beyond the portable C++ props: React Native's Android
// `overScrollMode`, which only Android's platform props parse (HostPlatformScrollViewProps), kept
// beside each ScrollView's props by the host's ScrollView descriptor as Fabric makes them.
#pragma once

#include <react/renderer/components/scrollview/ScrollViewComponentDescriptor.h>

namespace oxyos {

// As ReactScrollViewHelper.parseOverScrollMode reads it: "auto" (or none, or anything else) is
// Android's OVER_SCROLL_IF_CONTENT_SCROLLS, "always" OVER_SCROLL_ALWAYS, "never" OVER_SCROLL_NEVER.
enum class OverScrollMode { IfContentScrolls, Always, Never };

// The mode a ScrollView's props were made with: IfContentScrolls for props this descriptor did not
// make.
OverScrollMode overScrollModeOf(const facebook::react::Props& props);

// React Native's ScrollView descriptor, which also keeps each props' `overScrollMode` (a prop it
// does not set inherits its source's, as Fabric's props do).
class ScrollViewComponentDescriptor final : public facebook::react::ScrollViewComponentDescriptor {
 public:
  using facebook::react::ScrollViewComponentDescriptor::ScrollViewComponentDescriptor;

  facebook::react::Props::Shared cloneProps(const facebook::react::PropsParserContext& context,
                                            const facebook::react::Props::Shared& props,
                                            facebook::react::RawProps rawProps) const override;
};

}  // namespace oxyos
