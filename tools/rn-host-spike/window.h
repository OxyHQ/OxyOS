// The host's toplevel window on the Oxy compositor, through the protocol client
// (crates/oxy-client-ffi). The compositor configures the window's size; the host draws its view
// tree into a buffer and presents it, one frame in flight at a time.
#pragma once

#include "renderer.h"

#include <oxy_client.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <string>

namespace oxy {

class Window {
 public:
  // Connects to the compositor at `socket` and requests a toplevel of the given logical size.
  // Throws std::runtime_error on failure.
  Window(const std::string& socket, uint32_t width, uint32_t height);
  ~Window();
  Window(const Window&) = delete;
  Window& operator=(const Window&) = delete;

  // Handles every queued compositor event. Configures and frame feedback are the window's own;
  // input events go to `input`. Returns false once the compositor closed the connection; throws
  // on a protocol error.
  bool dispatch(const std::function<void(const OxyEvent&)>& input);

  // The configured logical size and scale. `resized()` reports (once) a size the host must lay
  // out for.
  bool configured() const { return configure_ != 0; }
  float width() const { return width_; }
  float height() const { return height_; }
  float scale() const { return scale_; }
  bool resized();

  // A frame may be presented: the window is configured and the previous frame was answered.
  bool ready() const { return configured() && inFlight_ == 0; }
  // Presents a canvas drawn at the configured size and scale.
  void present(const Canvas& canvas);

  uint64_t presented() const { return presented_; }

 private:
  OxyClient* client_ = nullptr;
  uint64_t configure_ = 0;
  float width_ = 0, height_ = 0, scale_ = 1;
  bool resized_ = false;
  uint64_t inFlight_ = 0;
  uint64_t presented_ = 0;
};

}  // namespace oxy
