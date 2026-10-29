// What the session's shell knows of the compositor's windows and workspaces (ADR 0009), gathered
// from its events: each window by the number the compositor gave it, with its application id,
// title, workspace and state, and the workspaces. JavaScript reads it through the OxyOSShell
// module (host-modules.cpp) and hears `OxyOSShellChanged` when it changes. Also whether the session
// is locked (docs/adr/0010-lock-screen.mdx), the session's user, whom the lock screen names, and
// whether Caps Lock is on, which its password field shows.
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
  ShellState();
  // { windows: [{ id, appId, title, workspace, minimized, focused }], workspaces, active, locked,
  //   user: { name, fullName }, capsLock }
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
  bool locked_ = false;
  // Whether Caps Lock is on, as told to the shell's focused surface (off when it gains focus,
  // unless told).
  bool capsLock_ = false;
  // The session's user, from the account database: the login name, and the full name of its
  // GECOS field (empty if it has none).
  std::string user_, fullName_;
};

}  // namespace oxyos
