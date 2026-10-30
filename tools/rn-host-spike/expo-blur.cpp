#include "expo-blur.h"

#include <react/renderer/core/propsConversions.h>

#include <cmath>
#include <map>

using namespace facebook::react;

namespace facebook::react {

const char ExpoBlurViewComponentName[] = "ViewManagerAdapter_ExpoBlur_ExpoBlurView";
const char ExpoBlurTargetViewComponentName[] = "ViewManagerAdapter_ExpoBlur_ExpoBlurTargetView";

ExpoBlurViewProps::ExpoBlurViewProps(const PropsParserContext& context, const ExpoBlurViewProps& sourceProps,
                                     const RawProps& rawProps)
    : ViewProps(context, sourceProps, rawProps),
      tint(convertRawProp(context, rawProps, "tint", sourceProps.tint, std::string("default"))),
      intensity(convertRawProp(context, rawProps, "intensity", sourceProps.intensity, Float{50})),
      blurTargetId(convertRawProp(context, rawProps, "blurTargetId", sourceProps.blurTargetId, 0)),
      blurMethod(convertRawProp(context, rawProps, "blurMethod", sourceProps.blurMethod, std::string("none"))),
      blurReductionFactor(
          convertRawProp(context, rawProps, "blurReductionFactor", sourceProps.blurReductionFactor, Float{4})) {}

}  // namespace facebook::react

namespace oxyos {

bool isExpoBlurView(const std::string& name) { return name == ExpoBlurViewComponentName; }

std::optional<ExpoBlurTarget> expoBlurTarget(const ShadowView& view) {
  if (!view.componentName || !isExpoBlurView(view.componentName)) return std::nullopt;
  auto props = std::static_pointer_cast<const ExpoBlurViewProps>(view.props);
  // As ExpoBlurView.configureBlurView: no target, or the method "none", blurs nothing; Dimezis'
  // view is given the intensity divided by the reduction factor as its radius.
  if (props->blurTargetId <= 0 || props->blurMethod == "none" || props->blurReductionFactor <= 0) {
    return std::nullopt;
  }
  float radius = props->intensity / props->blurReductionFactor;
  if (radius <= 0) return std::nullopt;
  return ExpoBlurTarget{props->blurTargetId, radius};
}

std::optional<uint32_t> expoBlurTint(const ShadowView& view) {
  if (!view.componentName || !isExpoBlurView(view.componentName)) return std::nullopt;
  auto props = std::static_pointer_cast<const ExpoBlurViewProps>(view.props);
  // TintStyle.toBlurEffect: each tint's grey and how opaque it is at full intensity.
  struct Tint {
    uint8_t grey;
    double opacity;
  };
  static const std::map<std::string, Tint> tints = {
      {"dark", {25, 0.69}},
      {"systemMaterialDark", {25, 0.69}},
      {"light", {249, 0.78}},
      {"extraLight", {249, 0.78}},
      {"systemMaterialLight", {249, 0.78}},
      {"systemUltraThinMaterialLight", {249, 0.78}},
      {"systemThickMaterialLight", {249, 0.78}},
      {"regular", {179, 0.82}},
      {"systemThinMaterialLight", {199, 0.78}},
      {"systemThinMaterial", {199, 0.97}},
      {"systemChromeMaterial", {255, 0.75}},
      {"systemChromeMaterialLight", {255, 0.97}},
      {"systemUltraThinMaterial", {191, 0.44}},
      {"systemThickMaterial", {153, 0.97}},
      {"systemThickMaterialDark", {37, 0.9}},
      {"systemThinMaterialDark", {37, 0.7}},
      {"systemUltraThinMaterialDark", {37, 0.55}},
      {"systemChromeMaterialDark", {0, 0.75}},
  };
  // Default, prominent and system material (and any other) are white at 0.44.
  auto found = tints.find(props->tint);
  Tint tint = found == tints.end() ? Tint{255, 0.44} : found->second;
  double intensity = props->intensity / 100.0;
  auto alpha = static_cast<uint32_t>(static_cast<int>(255 * intensity * tint.opacity) & 0xff);
  return (alpha << 24) | (uint32_t{tint.grey} << 16) | (uint32_t{tint.grey} << 8) | tint.grey;
}

}  // namespace oxyos
