// OxyOS platform declarations for react-native-reanimated's C++ core, which only declares some
// platform hooks for Android or Apple. Synchronous UI-prop updates are not enabled on OxyOS (their
// static flags are false), so every animated update goes through Fabric's shadow-tree commit.
#pragma once

#include "linux-compat.h"

#include <folly/dynamic.h>

#include <functional>

namespace reanimated {
using SynchronouslyUpdateUIPropsFunction = std::function<void(const int, const folly::dynamic&)>;
}  // namespace reanimated
