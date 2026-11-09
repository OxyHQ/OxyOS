#include "unimplemented-components.h"

#include "report.h"

#include <react/renderer/components/unimplementedview/UnimplementedViewComponentDescriptor.h>
#include <react/renderer/componentregistry/ComponentDescriptorProvider.h>

#include <atomic>
#include <iostream>
#include <memory>
#include <mutex>
#include <set>

namespace oxyos {

namespace {

// The components registered as unimplemented, by name, for the process's life. The registry may ask
// from any thread; drawing asks of every view, so a process with none answers without the lock.
std::mutex namesLock;
std::atomic<bool> anyRegistered{false};
std::set<std::string>& names() {
  static std::set<std::string> registered;
  return registered;
}

}  // namespace

void addUnimplementedComponents(facebook::react::ComponentDescriptorProviderRegistry& providers) {
  using namespace facebook::react;
  // The registry asks for a name it has no descriptor for, every registry made from `providers`
  // included; the descriptor added here is then every registry's, so each name is asked once.
  providers.setComponentDescriptorProviderRequest([&providers](ComponentName requested) {
    std::string name = requested;
    {
      std::lock_guard<std::mutex> hold(namesLock);
      if (!names().insert(name).second) return;
      anyRegistered = true;
    }
    // The name is the descriptor's flavor, so its shadow nodes and views carry the component's own
    // name, never one shared by every unknown component.
    auto flavor = std::make_shared<const std::string>(name);
    ComponentName componentName = flavor->c_str();
    providers.add(ComponentDescriptorProvider{
        .handle = reinterpret_cast<ComponentHandle>(componentName),
        .name = componentName,
        .flavor = flavor,
        .constructor = &concreteComponentDescriptorConstructor<UnimplementedViewComponentDescriptor>});
    std::cerr << "[host] unimplemented native component: " << name << ", drawn as a plain view\n";
    Report::get().unimplemented(name);
  });
}

bool isUnimplementedComponent(const std::string& name) {
  if (!anyRegistered) return false;
  std::lock_guard<std::mutex> hold(namesLock);
  return names().count(name) > 0;
}

}  // namespace oxyos
