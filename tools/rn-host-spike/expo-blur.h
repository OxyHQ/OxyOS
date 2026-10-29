// expo-blur on OxyOS: its views as host components (Expo's `ViewManagerAdapter_` names). OxyOS
// has no blur target, so a blur view draws as Android's does without one (blur method "none"): its
// tint, at an alpha its intensity gives, over what is behind it. The target view lays out as a
// plain view.
#pragma once

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>
#include <react/renderer/mounting/ShadowView.h>

#include <cstdint>
#include <optional>
#include <string>

namespace facebook::react {

extern const char ExpoBlurViewComponentName[];
extern const char ExpoBlurTargetViewComponentName[];

class ExpoBlurViewProps final : public ViewProps {
 public:
  ExpoBlurViewProps() = default;
  ExpoBlurViewProps(const PropsParserContext& context, const ExpoBlurViewProps& sourceProps, const RawProps& rawProps);

  std::string tint{"default"};
  Float intensity{50};
};

using ExpoBlurViewShadowNode = ConcreteViewShadowNode<ExpoBlurViewComponentName, ExpoBlurViewProps>;
using ExpoBlurViewComponentDescriptor = ConcreteComponentDescriptor<ExpoBlurViewShadowNode>;
using ExpoBlurTargetViewShadowNode = ConcreteViewShadowNode<ExpoBlurTargetViewComponentName, ViewProps>;
using ExpoBlurTargetViewComponentDescriptor = ConcreteComponentDescriptor<ExpoBlurTargetViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

// Whether a component is expo-blur's blur view.
bool isExpoBlurView(const std::string& name);
// A blur view's tint as 0xAARRGGBB, as Android's TintStyle.toBlurEffect gives it; none for
// another view.
std::optional<uint32_t> expoBlurTint(const facebook::react::ShadowView& view);

}  // namespace oxyos
