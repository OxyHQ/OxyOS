// Native components the host has no implementation of (docs/react-native-android-adaptation.mdx,
// "What OxyOS does not provide"): React Native's Android `AndroidProgressBar` (ActivityIndicator),
// `AndroidSwitch`, `AndroidSwipeRefreshLayout` (RefreshControl) and `AndroidDrawerLayout`, and any
// package's view the host lacks. Each is mounted as a plain view under its own name, as React
// Native's iOS mounts a component it has no view for (RCTComponentViewFactory's last fallback, its
// UnimplementedView with the component's name), and said, never drawn silently: once, as the
// first is created, in the log (`[host] unimplemented native component: NAME, drawn as a plain
// view`) and in the report (`unimplemented`), which regress.py fails unless the scenario declares
// it.
#pragma once

#include <react/renderer/componentregistry/ComponentDescriptorProviderRegistry.h>

#include <string>

namespace oxyos {

// Has `providers` register each component it is asked for and does not have, as said above.
void addUnimplementedComponents(facebook::react::ComponentDescriptorProviderRegistry& providers);

// Whether `name` is a component this process has no implementation of, once one was created.
bool isUnimplementedComponent(const std::string& name);

}  // namespace oxyos
