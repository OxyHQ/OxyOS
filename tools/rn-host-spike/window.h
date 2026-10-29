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
  bool authenticate(uint32_t id, const std::string& secret);
  // Where a surface takes input: a surface-local logical rectangle.
  void setInputRegion(uint32_t id, const OxyOSRect& region);
  // The pointer's shape while it is over a surface (OXYOS_POINTER_*).
  void setPointerShape(uint32_t id, uint8_t shape);
  // Places an open popup again (its anchor moved or its content resized).
  void reposition(uint32_t id, const OxyOSPositioner& positioner);
  void destroy(uint32_t id);
  // Publishes a surface's accessibility snapshot (ADR 0003). Throws on failure.
  void publishAccessibility(uint32_t id, const std::vector<OxyOSAccessibleNode>& nodes);
  // Announces text to assistive technologies from a surface (OXYOS_POLITE, OXYOS_ASSERTIVE).
  // Throws on failure.
  void announce(uint32_t id, uint8_t politeness, const std::string& text);

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
  uint64_t presented_ = 0;
  uint32_t nextSurface_ = kToplevel + 1;
  std::optional<Shell> shell_;
};

}  // namespace oxyos
