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
  // Places an open popup again (its anchor moved or its content resized).
  void reposition(uint32_t id, const OxyOSPositioner& positioner);
  void destroy(uint32_t id);
  // Publishes a surface's accessibility snapshot (ADR 0003). Throws on failure.
  void publishAccessibility(uint32_t id, const std::vector<OxyOSAccessibleNode>& nodes);

  uint64_t presented() const { return presented_; }

  // The clipboard (clipboard.h): what it holds comes with the compositor's events.
  bool copyText(uint64_t serial, const std::string& text) { return clipboard_.copy(client_, serial, text); }
  void pasteText(uint64_t serial, std::function<void(std::string)> done) {
    clipboard_.paste(client_, serial, std::move(done));
  }
  // Each turn: finishes pastes whose content arrived. True if any did.
  bool deliverPastes() { return clipboard_.deliver(); }
  bool clipboardHasText() const { return clipboard_.hasText(); }
  // Runs `changed` whenever what the clipboard holds is known anew.
  void onClipboardChanged(std::function<void()> changed) { clipboard_.onChanged(std::move(changed)); }

 private:
  OxyOSClient* client_ = nullptr;
  Clipboard clipboard_;
  std::map<uint32_t, Surface> surfaces_;
  uint64_t presented_ = 0;
};

}  // namespace oxyos
