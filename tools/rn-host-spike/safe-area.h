// react-native-safe-area-context on OxyOS: its provider and view as host components. An OxyOS
// window has no system insets (the compositor keeps panels outside the work area windows are
// placed in), so the provider reports zero insets with its frame in the window, as Android's
// SafeAreaProvider reports its insets and frame, and the view lays out as a plain view.
#pragma once

#include "renderer.h"

#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <map>

namespace facebook::react {

extern const char RNCSafeAreaProviderComponentName[];
extern const char RNCSafeAreaViewComponentName[];
using RNCSafeAreaProviderShadowNode = ConcreteViewShadowNode<RNCSafeAreaProviderComponentName, ViewProps>;
using RNCSafeAreaProviderComponentDescriptor = ConcreteComponentDescriptor<RNCSafeAreaProviderShadowNode>;
// Its `mode` and `edges` would apply the insets as padding or margin; with none, they change
// nothing, so its props are a view's.
using RNCSafeAreaViewShadowNode = ConcreteViewShadowNode<RNCSafeAreaViewComponentName, ViewProps>;
using RNCSafeAreaViewComponentDescriptor = ConcreteComponentDescriptor<RNCSafeAreaViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

class SafeAreas {
 public:
  // After a mount: tells each provider that appeared, or whose frame in the window changed, its
  // insets and frame (`onInsetsChange`). The JavaScript provider renders nothing before.
  void sync(const ViewTree& tree);

 private:
  std::map<facebook::react::Tag, facebook::react::Rect> told_;
};

}  // namespace oxyos
