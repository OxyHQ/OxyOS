// react-native-keyboard-controller's provider view (`KeyboardControllerView`, which its
// KeyboardProvider renders around an application) on OxyOS: a view and nothing more. On Android it
// reports the software keyboard's moves and the focused input's layout; OxyOS has no software
// keyboard, so none of its events is ever sent (its module, host-modules.cpp, says the keyboard is
// hidden), and its `enabled` and translucency props change nothing.
#pragma once

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

namespace facebook::react {

extern const char KeyboardControllerViewComponentName[];

using KeyboardControllerViewShadowNode = ConcreteViewShadowNode<KeyboardControllerViewComponentName, ViewProps>;
using KeyboardControllerViewComponentDescriptor = ConcreteComponentDescriptor<KeyboardControllerViewShadowNode>;

}  // namespace facebook::react
