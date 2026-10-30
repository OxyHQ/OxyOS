// What the session's shell knows of the compositor's windows and workspaces (ADR 0009), gathered
// from its events: each window by the number the compositor gave it, with its application id,
// title, workspace and state, and the workspaces. JavaScript reads it through the OxyOSShell
// module (host-modules.cpp) and hears `OxyOSShellChanged` when it changes. Also whether the session
// is locked (docs/adr/0010-lock-screen.mdx), the session's user, whom the lock screen names, and
// whether Caps Lock is on, which its password field shows; and the wallpaper's frame that shows the
// sun now (docs/adr/0012-solar-wallpaper.mdx), from the solar wallpaper OXYOS_WALLPAPER names. And
// the outputs (docs/adr/0018-several-displays.mdx): the shell runs on one, `output`, and has the dock
// only on the primary.
#pragma once

#include "notifications.h"

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
  // The shell of the output `output`.
  explicit ShellState(uint32_t output);
  // { windows: [{ id, appId, title, workspace, minimized, focused, launched, width, height, used, hidden }],
  //   workspaces, active, locked,
  //   menuBar: { window, serial, items: [{ parent, kind, enabled, checkable, checked, label, shortcut }] }
  //   (the active window's menus, docs/adr/0021-menu-bar.mdx: window 0 when none is active, no items
  //   when it declared none; kind "menu" | "item" | "submenu" | "separator", parent an index or -1),
  //   user: { name, fullName }, capsLock, wallpaper: "file://..." or null,
  //   applications: [{ id, name, icon: { uri } or { svg }, pinned: place or null }],
  //   output, outputs: [{ id, x, y, width, height, scale, primary }],
  //   oxyApi: Oxy's API, where adding an account at the login screen signs in (OXYOS_OXY_API),
  //   controls: { output, input, display, keyboard: { level (0 to 1), muted, available } or null,
  //   power: { battery: "absent" | "discharging" | "charging" | "full" | "not-charging", percent,
  //   onAC, minutes } or null, network: { wifi: "unavailable" | "off" | "disconnected" |
  //   "connecting" | "connected", signal, ssid, wired } or null, bluetooth: { state: "unavailable" |
  //   "off" | "on", connected } or null } (docs/adr/0025-hardware-keys-and-control-center.mdx: null
  //   until the session has told it),
  //   notifications: [{ id, appId, title, subtitle, body, actions: [{ label, reply: placeholder or
  //   null }], postedAt: ms since the epoch, banner, withheld }] (docs/adr/0026-notifications.mdx:
  //   oldest first; `banner` as the compositor last told it, `withheld` its texts kept from the
  //   locked shell), notificationPolicy: { doNotDisturb, previews } }
  // The applications are the session's registry (docs/adr/0013-launching-applications.mdx); the
  // outputs the compositor's, in the global space's logical pixels, empty until it has told them.
  folly::dynamic state() const;
  // Each turn: true if the wallpaper's frame changed (the sun moved on to another).
  bool tick();

 private:
  struct Window {
    std::string appId, title;
    uint32_t workspace = 0;
    bool minimized = false, focused = false, launched = false;
    // Its size, logical pixels (0 until told): the proportions the overview shows it in.
    uint32_t width = 0, height = 0;
    // How recently it was the active window (a rising count, 0 never), and whether it is hidden
    // with its application (docs/adr/0024-system-shortcuts.mdx).
    uint32_t used = 0;
    bool hidden = false;
  };
  std::map<uint32_t, Window> windows_;
  // A title or application id being received, by window.
  std::map<uint32_t, std::string> title_, appId_;
  uint32_t workspaces_ = 1, active_ = 0;
  // The active window's menus as last committed, and those being received.
  struct MenuItem {
    int64_t parent = -1;
    uint32_t kind = 0;
    uint8_t flags = 0;
    std::string label, shortcut;
  };
  struct MenuBar {
    uint32_t window = 0, serial = 0, count = 0;
    std::vector<MenuItem> items;
  };
  MenuBar menuBar_, receivingMenuBar_;
  bool locked_ = false;
  // The hardware controls as the session told them (ADR 0025), by OXYOS_CONTROL_*.
  struct Level {
    uint32_t level = 0;
    bool muted = false, available = false;
  };
  std::map<uint32_t, Level> levels_;
  std::optional<OxyOSEvent> power_, network_, bluetooth_;
  folly::dynamic controls() const;
  struct Output {
    int32_t x = 0, y = 0;
    uint32_t width = 0, height = 0, scale120 = 120;
    bool primary = false;
  };
  uint32_t output_;
  std::map<uint32_t, Output> outputs_;
  // Whether Caps Lock is on, as told to the shell's focused surface (off when it gains focus,
  // unless told).
  bool capsLock_ = false;
  // The notifications the compositor keeps (docs/adr/0026-notifications.mdx), by number, and
  // their texts being received.
  struct Notification {
    std::string appId, title, subtitle, body;
    std::vector<std::pair<std::string, std::optional<std::string>>> actions;
    int64_t postedAt = 0;
    bool banner = false, withheld = false;
  };
  std::map<uint32_t, Notification> notifications_;
  NotificationTexts notificationTexts_;
  bool doNotDisturb_ = false, previews_ = false;
  // The solar wallpaper: its folder, frames (by index), anchors, the frame shown when there is
  // no sun (the time zone has no place), the frame chosen and when it next changes.
  std::string wallpaperDir_;
  std::map<uint32_t, std::string> wallpaperFrames_;
  std::vector<OxyOSSolarAnchor> anchors_;
  uint32_t previewFrame_ = 0;
  std::optional<uint32_t> frame_;
  int64_t nextChange_ = 0;
  void chooseFrame();
  folly::dynamic applications_ = folly::dynamic::array();
  void readApplications();
  folly::dynamic wallpaperUri() const;
  static std::string oxyApi();
  // The session's user, from the account database: the login name, and the full name of its
  // GECOS field (empty if it has none).
  std::string user_, fullName_;
};

}  // namespace oxyos
