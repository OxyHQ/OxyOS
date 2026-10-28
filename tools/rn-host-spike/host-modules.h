// TurboModules the OxyOS host implements for React Native's start-up and Bloom.
#pragma once

#include "renderer.h"

#include <ReactCommon/CallInvoker.h>
#include <ReactCommon/TurboModule.h>

#include <memory>
#include <string>

namespace oxy {

std::shared_ptr<facebook::react::TurboModule> createHostModule(
    const std::string& name,
    const std::shared_ptr<facebook::react::CallInvoker>& invoker,
    const Display& display);

}  // namespace oxy
