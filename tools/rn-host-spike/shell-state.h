// What the session's shell knows of the compositor's windows and workspaces (ADR 0009), gathered
// from its events: each window by the number the compositor gave it, with its application id,
// title, workspace and state, and the workspaces. JavaScript reads it through the OxyOSShell
// module (host-modules.cpp) and hears `OxyOSShellChanged` when it changes. Also whether the session
// is locked (docs/adr/0010-lock-screen.mdx), the session's user, whom the lock screen names, and
// whether Caps Lock is on, which its password field shows; and the wallpaper's frame that shows the
// sun now (docs/adr/0012-solar-wallpaper.mdx), from the solar wallpaper OXYOS_WALLPAPER names.
#pragma once

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <map>
#include <optional>
#include <string>
#include <vector>

namespace oxyos {

class ShellState {
 public:
  // Takes in a window or workspace event; true if what the shell knows changed.
  bool observe(const OxyOSEvent& event);
  ShellState();
  // { windows: [{ id, appId, title, workspace, minimized, focused }], workspaces, active, locked,
  //   user: { name, fullName }, capsLock, wallpaper: "file://..." or null }
  folly::dynamic state() const;
  // Each turn: true if the wallpaper's frame changed (the sun moved on to another).
  bool tick();

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
  // The solar wallpaper: its folder, frames (by index), anchors, the frame shown when there is
  // no sun (the time zone has no place), the frame chosen and when it next changes.
  std::string wallpaperDir_;
  std::map<uint32_t, std::string> wallpaperFrames_;
  std::vector<OxyOSSolarAnchor> anchors_;
  uint32_t previewFrame_ = 0;
  std::optional<uint32_t> frame_;
  int64_t nextChange_ = 0;
  void chooseFrame();
  folly::dynamic wallpaperUri() const;
  // The session's user, from the account database: the login name, and the full name of its
  // GECOS field (empty if it has none).
  std::string user_, fullName_;
};

}  // namespace oxyos
