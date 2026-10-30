// React Native's MaskedView (@react-native-masked-view/masked-view's RNCMaskedView) on OxyOS, as
// Android draws it: its first child is the mask, its other children are drawn only where the mask
// is opaque (each of their pixels kept by the mask's alpha there, PorterDuff DST_IN), and the mask
// itself is never seen.
#pragma once

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <string>

namespace facebook::react {

extern const char RNCMaskedViewComponentName[];

using RNCMaskedViewShadowNode = ConcreteViewShadowNode<RNCMaskedViewComponentName, ViewProps>;
using RNCMaskedViewComponentDescriptor = ConcreteComponentDescriptor<RNCMaskedViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

bool isMaskedView(const std::string& name);

}  // namespace oxyos
