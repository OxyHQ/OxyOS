#include "android-components.h"

#include "progress-bar.h"

#include <react/renderer/components/scrollview/AndroidHorizontalScrollContentViewComponentDescriptor.h>
#include <react/renderer/componentregistry/ComponentDescriptorProvider.h>

namespace oxyos {

void addAndroidComponents(facebook::react::ComponentDescriptorProviderRegistry& providers) {
  using namespace facebook::react;
  // The content of a horizontal ScrollView (`AndroidHorizontalScrollView` itself is the
  // ScrollView component); a view whose layout stays within the scroll range in RTL.
  providers.add(concreteComponentDescriptorProvider<AndroidHorizontalScrollContentViewComponentDescriptor>());
  // ProgressBar, what ActivityIndicator renders (progress-bar.h): measured by its style and drawn
  // by the host, where Android's measures and draws it in Java.
  providers.add(concreteComponentDescriptorProvider<ProgressBarComponentDescriptor>());
}

}  // namespace oxyos
