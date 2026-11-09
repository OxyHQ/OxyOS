// The host's modules on the platform API's specs (docs/adr/0058-the-platform-api.mdx, decision 5).
// Each module behind a framework of @oxyos/platform has a spec in TypeScript
// (sample/oxyos/platform/specs/NativeOxyOS*.ts), and React Native's codegen makes from it the C++
// base the module implements (NativeOxyOS*CxxSpec, CMakeLists.txt): a method the spec names and the
// module lacks, or one whose types differ, does not compile. The other modules are still
// HostModule's method tables (host-modules.cpp), and give way to these framework by framework.
#pragma once

#include "host-modules.h"

#include <memory>
#include <string>

namespace oxyos {

// The module `name` on its spec's generated base, or null for one that is not (yet) such a module.
std::shared_ptr<facebook::react::TurboModule> createPlatformModule(
    const std::string& name,
    const std::shared_ptr<facebook::react::CallInvoker>& invoker,
    HostServices& services);

}  // namespace oxyos
