// The host's surfaces on the Oxy compositor, through the protocol client (crates/oxy-client-ffi):
// its toplevel window and the popups it opens. The compositor configures each surface's size; the
// host draws into a buffer and presents it, one frame in flight per surface.
#pragma once

#include "renderer.h"

#include <oxy_client.h>

#include <cstdint>
#include <functional>
#include <map>
#include <string>

namespace oxy {

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
  bool dispatch(const std::function<void(const OxyEvent&)>& input);

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
  // ARGB (rounded corners); the toplevel is opaque.
  void present(uint32_t id, const Canvas& canvas);

  // Requests a popup of `parent`, placed by `positioner`, and its grab when `serial` is non-zero
  // (the input event that opened it). Throws on failure.
  void openPopup(uint32_t id, uint32_t parent, const OxyPositioner& positioner, uint64_t serial);
  // Places an open popup again (its anchor moved or its content resized).
  void reposition(uint32_t id, const OxyPositioner& positioner);
  void destroy(uint32_t id);

  uint64_t presented() const { return presented_; }

 private:
  OxyClient* client_ = nullptr;
  std::map<uint32_t, Surface> surfaces_;
  uint64_t presented_ = 0;
};

}  // namespace oxy
