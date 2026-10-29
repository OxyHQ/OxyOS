// What the session's shell knows of the compositor's windows and workspaces (ADR 0009), gathered
// from its events: each window by the number the compositor gave it, with its application id,
// title, workspace and state, and the workspaces. JavaScript reads it through the OxyOSShell
// module (host-modules.cpp) and hears `OxyOSShellChanged` when it changes.
#pragma once

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <map>
#include <string>

namespace oxyos {

class ShellState {
 public:
  // Takes in a window or workspace event; true if what the shell knows changed.
  bool observe(const OxyOSEvent& event);
  // { windows: [{ id, appId, title, workspace, minimized, focused }], workspaces, active }
  folly::dynamic state() const;

 private:
  struct Window {
    std::string appId, title;
    uint32_t workspace = 0;
    bool minimized = false, focused = false;
  };
  std::map<uint32_t, Window> windows_;
  // A title or application id being received, by window.
  std::map<uint32_t, std::string> title_, appId_;
  uint32_t workspaces_ = 1, active_ = 0;
};

}  // namespace oxyos
