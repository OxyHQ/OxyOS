#include "android-components.h"

#include <react/renderer/components/scrollview/AndroidHorizontalScrollContentViewComponentDescriptor.h>
#include <react/renderer/componentregistry/ComponentDescriptorProvider.h>

namespace oxy {

void addAndroidComponents(facebook::react::ComponentDescriptorProviderRegistry& providers) {
  using namespace facebook::react;
  // The content of a horizontal ScrollView (`AndroidHorizontalScrollView` itself is the
  // ScrollView component); a view whose layout stays within the scroll range in RTL.
  providers.add(concreteComponentDescriptorProvider<AndroidHorizontalScrollContentViewComponentDescriptor>());
}

}  // namespace oxy
