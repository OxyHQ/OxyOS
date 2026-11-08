// The host's trace: what it does, written to stderr while OXYOS_HOST_TRACE is set (popups, text
// fields, the window's events, flings and scrolls, Reanimated's events, the start's stages).
#pragma once

#include <cstdlib>

namespace oxyos {

// Whether OXYOS_HOST_TRACE is set: read once, as it is asked on every event.
inline bool tracing() {
  static const bool on = std::getenv("OXYOS_HOST_TRACE") != nullptr;
  return on;
}

}  // namespace oxyos
