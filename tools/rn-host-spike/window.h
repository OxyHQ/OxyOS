// The host's surfaces on the OxyOS compositor, through the protocol client (crates/oxyos-client-ffi):
// its toplevel window and the popups it opens. The compositor configures each surface's size; the
// host draws into a buffer and presents it, one frame in flight per surface.
#pragma once

#include "clipboard.h"
#include "renderer.h"

#include <oxyos_client.h>

#include <cstdint>
#include <functional>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace oxyos {

class Window {
 public:
  // The toplevel's surface id; popups take the following ids.
  static constexpr uint32_t kToplevel = 1;

  // Connects to the compositor at `socket` and requests a toplevel of the given logical size.
  // Throws std::runtime_error on failure.
  Window(const std::string& socket, uint32_t width, uint32_t height);
  // The session's shell (docs/adr/0009-session-shell.mdx): the root is a background shell surface
  // over `output`, configured at the output's size, where an application has its window.
  struct Shell {
    uint32_t output = 0;
  };
  Window(const std::string& socket, Shell shell);
  ~Window();
  Window(const Window&) = delete;
  Window& operator=(const Window&) = delete;

  // Handles every queued compositor event. Configures and frame feedback update the surfaces;
  // input and popup dismissals go to `input`. Returns false once the compositor closed the
  // connection; throws on a protocol error.
  bool dispatch(const std::function<void(const OxyOSEvent&)>& input);

  struct Surface {
    uint64_t configure = 0;  // serial of the latest configure; 0 until the first
    float x = 0, y = 0;      // a popup's position relative to its parent
    float width = 0, height = 0, scale = 1;
    bool resized = false;    // set by a configure that changed size or scale
    uint64_t inFlight = 0;   // sequence of the frame awaiting feedback
    uint64_t presentedFor = 0;  // the configure the latest frame presented answers
    uint8_t states = 0;      // a toplevel's OXYOS_CONFIGURE_* bits, from its latest configure
    bool shown = false;      // a frame of it was presented (on the screen)
  };
  // The surface, or null if it does not exist (or no longer does).
  Surface* surface(uint32_t id);

  bool configured() const { return surfaces_.at(kToplevel).configure != 0; }
  float width() const { return surfaces_.at(kToplevel).width; }
  float height() const { return surfaces_.at(kToplevel).height; }
  float scale() const { return surfaces_.at(kToplevel).scale; }
  // Reports (once) a toplevel size the host must lay out for.
  bool resized();

  // A frame may be presented: the surface is configured and its previous frame was answered.
  bool ready(uint32_t id) const;
  // Whether the surface's latest configure is answered by a frame presented for it. One that is
  // not needs a frame, even with nothing changed in it: the compositor keeps the configures of a
  // resize to one in flight (docs/adr/0016-windows.mdx), and the next waits for that frame.
  bool answered(uint32_t id) const;
  // Presents a canvas drawn at the surface's configured size and scale. Popups use premultiplied
  // ARGB (rounded corners); the toplevel is opaque. `damage` (canvas pixels) is what changed
  // since the surface's previous frame; none: everything (docs/adr/0005-damage.mdx).
  void present(uint32_t id, const Canvas& canvas, const std::vector<OxyOSRect>& damage = {});

  // Requests a popup of `parent`, placed by `positioner`, and its grab when `serial` is non-zero
  // (the input event that opened it). Throws on failure.
  void openPopup(uint32_t id, uint32_t parent, const OxyOSPositioner& positioner, uint64_t serial);
  // Names the toplevel (ADR 0009): its title and application id, as the session's shell shows it.
  void nameWindow(const std::string& title, const std::string& appId);
  // The session's shell acting on a window or a workspace (OXYOS_MANAGE_*).
  void manage(uint8_t action, uint32_t window, uint32_t workspace);
  // The session's shell asks for the keyboard focus on one of its own regions' surfaces, citing
  // the input `serial` (docs/adr/0024-system-shortcuts.mdx).
  void focusShellSurface(uint32_t id, uint64_t serial);
  // The application's own window (docs/adr/0016-windows.mdx): moved with the pointer from the
  // press `serial` names, zoomed to the work area or restored, minimized, kept within sizes.
  void beginMove(uint64_t serial);
  void setMaximized(bool enabled);
  void minimize();
  void setSizeLimits(uint32_t minWidth, uint32_t minHeight, uint32_t maxWidth, uint32_t maxHeight);
  // Whether the window is zoomed or fills its output, from its latest configure, and whether it
  // is the active window, from the compositor's ACTIVATED.
  bool maximized() const { return surfaces_.at(kToplevel).states & OXYOS_CONFIGURE_MAXIMIZED; }
  bool fullscreen() const { return surfaces_.at(kToplevel).states & OXYOS_CONFIGURE_FULLSCREEN; }
  bool active() const { return active_; }
  // The session's preferences (docs/adr/0019-settings.mdx): those that change how every client
  // draws, told to it from the first event; and, to Settings alone (the `settings` grant), every
  // preference as kept, by key. `settingsGranted` once the session has told any.
  struct Drawing {
    bool dark = false, reduceMotion = false, reduceTransparency = false;
    std::string accent;
    bool operator==(const Drawing&) const = default;
  };
  const Drawing& drawing() const { return drawing_; }
  const std::map<std::string, std::string>& preferences() const { return preferences_; }
  bool settingsGranted() const { return !preferences_.empty(); }
  // Asks the session to change a preference (Settings only); a PREFERENCE_RESULT with `serial`
  // answers. False if it could not be sent.
  bool setPreference(uint32_t serial, const std::string& key, const std::string& value);
  // The session's displays (docs/adr/0018-several-displays.mdx), told to Settings alone: each as
  // the session last told it, by its output (its name, modes, scale choices from the most space to
  // the largest text, the one its density recommends, its rectangle in the global space, scale,
  // mode, physical size, whether it is the primary and built in), and a mode on trial.
  struct DisplayMode {
    uint32_t width = 0, height = 0, refreshMhz = 0;
    bool preferred = false;
    bool operator==(const DisplayMode&) const = default;
  };
  struct Display {
    std::string name;
    std::vector<DisplayMode> modes;
    std::vector<uint32_t> scales;
    uint32_t recommended = 0;
    int32_t x = 0, y = 0;
    uint32_t width = 0, height = 0, scale120 = 0, mode = 0, widthMm = 0, heightMm = 0;
    bool primary = false, builtin = false;
    bool operator==(const Display&) const = default;
  };
  // The display whose mode is on trial and the seconds it was given (0: none).
  struct Trial {
    uint32_t output = 0, seconds = 0;
    bool operator==(const Trial&) const = default;
  };
  const std::map<uint32_t, Display>& displays() const { return displays_; }
  const Trial& trial() const { return trial_; }
  // Settings changes a display (each answered by a PREFERENCE_RESULT with `serial`): one of its
  // modes, on trial until kept (`confirmDisplays`); one of its scales; where it is wanted in the
  // global space; the primary. False if it could not be sent.
  bool setDisplayMode(uint32_t serial, uint32_t output, uint32_t width, uint32_t height, uint32_t refreshMhz);
  bool setDisplayScale(uint32_t serial, uint32_t output, uint32_t scale120);
  bool setDisplayOrigin(uint32_t serial, uint32_t output, int32_t x, int32_t y);
  bool setPrimaryDisplay(uint32_t serial, uint32_t output);
  bool confirmDisplays(uint32_t serial, bool keep);
  // A surface id for a popup or a shell region, never used before on this connection.
  uint32_t allocate() { return nextSurface_++; }
  bool shell() const { return shell_.has_value(); }
  // A shell region's surface (shell-surface.h): a panel at `edge` or an overlay, on the shell's
  // output, of the given logical size, keeping `exclusive` pixels out of the work area. Throws on
  // failure.
  void openShellSurface(uint32_t id, uint8_t kind, uint8_t edge, uint32_t width, uint32_t height, uint32_t exclusive);
  // The lock screen (docs/adr/0010-lock-screen.mdx): the lock surface of the shell's output (all
  // that shows while the session is locked; throws on failure), locking the session, and the
  // password typed in a lock surface, for the session to check (false if it could not be sent:
  // longer than 512 bytes). The compositor's AUTHENTICATION_RESULT answers.
  void openLockSurface(uint32_t id);
  void lockSession();
  // The Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx): a level (OXYOS_CONTROL_*,
  // 0 to 65536), a volume's mute, Wi-Fi, the appearance (a PREFERENCE_RESULT with `serial`
  // answers) and the system looked at again. False if it could not be sent.
  bool setControlLevel(uint8_t control, uint32_t level);
  bool setControlMuted(uint8_t control, bool muted);
  bool setWifiEnabled(bool enabled);
  bool setAppearance(uint32_t serial, bool dark);
  bool refreshControls();
  // The session's shell asks for an application of the registry (ADR 0013); a LAUNCH_RESULT with
  // `serial` answers. False if it could not be sent.
  bool launch(uint32_t serial, const std::string& id);
  // The password for `account` (a login screen's; empty: the session's own user).
  bool authenticate(uint32_t id, const std::string& secret, const std::string& account = "");
  // The windows a shell surface shows live (docs/adr/0022-window-views.mdx), bottom to top,
  // shown with its next present. Throws on failure.
  void showWindows(uint32_t id, const std::vector<OxyOSWindowView>& views);
  // Where a surface takes input: a surface-local logical rectangle.
  void setInputRegion(uint32_t id, const OxyOSRect& region);
  // The pointer's shape while it is over a surface (OXYOS_POINTER_*).
  void setPointerShape(uint32_t id, uint8_t shape);
  // Places an open popup again (its anchor moved or its content resized).
  void reposition(uint32_t id, const OxyOSPositioner& positioner);
  void destroy(uint32_t id);
  // Publishes a surface's accessibility snapshot (ADR 0003). Throws on failure.
  void publishAccessibility(uint32_t id, const std::vector<OxyOSAccessibleNode>& nodes);
  // The window's menus (docs/adr/0021-menu-bar.mdx), entries in order, as the commit `serial`, above
  // the last; a MENU_ITEM_ACTIVATED names it and the chosen entry's index. Throws on failure.
  void setMenus(uint32_t serial, const std::vector<OxyOSMenuItem>& items);
  // The session's shell chooses the entry `index` of the commit `serial` of window `window`'s menus,
  // as a MENU_BAR told them.
  void activateMenuItem(uint32_t window, uint32_t serial, uint32_t index);
  // Announces text to assistive technologies from a surface (OXYOS_POLITE, OXYOS_ASSERTIVE).
  // Throws on failure.
  void announce(uint32_t id, uint8_t politeness, const std::string& text);
  // Notifications (docs/adr/0026-notifications.mdx). An application posts one as `serial` (false if
  // it could not be sent); a NOTIFICATION_RESULT answers. It withdraws its own (0: all of them); the
  // shell removes any (0: all), and activates one (`action` 0) or its action, citing `serial`, an
  // input it received, with the reply typed.
  bool postNotification(uint32_t serial, const OxyOSNotification& notification);
  void withdrawNotification(uint32_t notification);
  void activateNotification(uint32_t notification, uint8_t action, uint64_t serial, const std::string& reply);
  // The shell's Control Center turns Do Not Disturb on or off, as `serial` (a PREFERENCE_RESULT
  // answers, as setAppearance's); false if it could not be sent.
  bool setDoNotDisturb(uint32_t serial, bool enabled);

  uint64_t presented() const { return presented_; }

  // The clipboard (clipboard.h): what it holds comes with the compositor's events.
  bool copyText(uint64_t serial, const std::string& text) { return clipboard_.copy(client_, serial, text); }
  void pasteText(uint64_t serial, std::function<void(std::string)> done) {
    clipboard_.paste(client_, serial, std::move(done));
  }
  // Each turn: finishes pastes whose content arrived. True if any did.
  bool deliverPastes() { return clipboard_.deliver(); }
  // What the host's loop waits on for this window: the compositor's connection, readable when an
  // event has come, and the pipes of pastes on their way.
  std::vector<int> fds() const {
    std::vector<int> out = clipboard_.fds();
    out.push_back(oxyos_client_fd(client_));
    return out;
  }
  bool clipboardHasText() const { return clipboard_.hasText(); }
  // Runs `changed` whenever what the clipboard holds is known anew.
  void onClipboardChanged(std::function<void()> changed) { clipboard_.onChanged(std::move(changed)); }

  // Text input (ADR 0008): tells the compositor about the field focused in `surface`, or that
  // none is (`field` null); only what changed is sent. Returns the commit serial transactions
  // made for this state carry. Throws on failure.
  uint32_t textInput(uint32_t surface, const OxyOSTextField* field, bool newField);
  // Inside `dispatch`'s callback, for a TEXT_INPUT event: the transaction it completed.
  bool transaction(OxyOSTextTransaction& out) { return oxyos_client_text_transaction(client_, &out) == 1; }

 private:
  OxyOSClient* client_ = nullptr;
  Clipboard clipboard_;
  std::map<uint32_t, Surface> surfaces_;
  Drawing drawing_;
  std::map<std::string, std::string> preferences_;
  std::string receiving_;  // a preference's chunks so far
  // A display's name, modes and scales being told, applied by its DISPLAY.
  struct Telling {
    std::optional<std::string> name;
    std::optional<std::vector<DisplayMode>> modes;
    std::optional<std::vector<uint32_t>> scales;
    uint32_t recommended = 0;
  };
  std::map<uint32_t, Telling> telling_;
  std::map<uint32_t, Display> displays_;
  Trial trial_;
  bool active_ = false;
  uint64_t presented_ = 0;
  uint32_t nextSurface_ = kToplevel + 1;
  std::optional<Shell> shell_;
};

}  // namespace oxyos
