// expo-blur on OxyOS: its views as host components (Expo's `ViewManagerAdapter_` names), drawn as
// Android draws them. Without a blur target, or with the blur method "none", a blur view is its
// tint, at an alpha its intensity gives, over what is behind it. With a target (a BlurTargetView,
// `blurTargetId`) and a blur method (Dimezis' BlurView on Android), it is the target's content
// under it blurred (a Gaussian whose radius is intensity / blurReductionFactor logical pixels),
// then its tint, clipped to it. The target view draws its children as a plain view.
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
  // The target's tag (findNodeHandle of the BlurTargetView), 0 for none.
  int blurTargetId{0};
  std::string blurMethod{"none"};
  Float blurReductionFactor{4};
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

// What a blur view with a target blurs: the target's tag and the Gaussian's radius (its standard
// deviation) in logical pixels; none for one that draws its tint alone.
struct ExpoBlurTarget {
  facebook::react::Tag target;
  float radius;
};
std::optional<ExpoBlurTarget> expoBlurTarget(const facebook::react::ShadowView& view);

}  // namespace oxyos
