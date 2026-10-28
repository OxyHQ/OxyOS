// React Native's Android-only core components whose shadow nodes are portable C++. React Native's
// JavaScript runs its Android implementation on OxyOS, so it creates them.
#pragma once

#include <react/renderer/componentregistry/ComponentDescriptorProviderRegistry.h>

namespace oxyos {

// Registers them as Android's CoreComponentsRegistry does.
void addAndroidComponents(facebook::react::ComponentDescriptorProviderRegistry& providers);

}  // namespace oxyos
