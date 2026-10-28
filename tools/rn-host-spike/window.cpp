#include "window.h"

#include <cstdlib>
#include <iostream>
#include <stdexcept>

namespace oxy {

namespace {

constexpr uint32_t kSurface = 1;

[[noreturn]] void failed(const std::string& what) {
  throw std::runtime_error(what + ": " + oxy_last_error());
}

}  // namespace

Window::Window(const std::string& socket, uint32_t width, uint32_t height) {
  client_ = oxy_client_connect(socket.c_str());
  if (!client_) failed("cannot connect to the compositor at " + socket);
  if (oxy_client_create_toplevel(client_, kSurface, width, height) != 0) {
    oxy_client_close(client_);
    failed("cannot create the window");
  }
}

Window::~Window() { oxy_client_close(client_); }

bool Window::dispatch(const std::function<void(const OxyEvent&)>& input) {
  OxyEvent event{};
  while (true) {
    switch (oxy_client_poll(client_, &event)) {
      case OXY_POLL_EMPTY:
        return true;
      case OXY_POLL_CLOSED:
        return false;
      case OXY_POLL_EVENT:
        if (std::getenv("OXY_HOST_TRACE")) {
          std::cerr << "[window] event " << event.kind << " surface " << event.surface << " serial " << event.serial
                    << " rect " << event.x << "," << event.y << " " << event.width << "x" << event.height
                    << " detail " << event.detail << "\n";
        }
        break;
      default:
        failed("compositor connection");
    }
    if (event.kind == OXY_EVENT_CONFIGURE && event.surface == kSurface) {
      float scale = event.scale_120 / 120.0f;
      if (event.width != width_ || event.height != height_ || scale != scale_) resized_ = true;
      width_ = static_cast<float>(event.width);
      height_ = static_cast<float>(event.height);
      scale_ = scale;
      configure_ = event.serial;
    } else if (event.kind == OXY_EVENT_FEEDBACK && event.surface == kSurface) {
      if (event.serial == inFlight_) inFlight_ = 0;
      if (event.detail == OXY_OUTCOME_PRESENTED) ++presented_;
    } else if (event.kind != OXY_EVENT_RELEASED) {
      input(event);
    }
  }
}

bool Window::resized() {
  bool resized = resized_;
  resized_ = false;
  return resized;
}

void Window::present(const Canvas& canvas) {
  // Canvas pixels are 0xAARRGGBB words: in memory (little-endian) B, G, R, A, which is XRGB8888.
  auto sequence = oxy_client_present(
      client_, kSurface, configure_, static_cast<uint32_t>(canvas.width), static_cast<uint32_t>(canvas.height),
      reinterpret_cast<const uint8_t*>(canvas.pixels.data()), canvas.pixels.size() * sizeof(uint32_t));
  if (sequence < 0) failed("cannot present");
  if (std::getenv("OXY_HOST_TRACE")) {
    std::cerr << "[window] present " << sequence << " " << canvas.width << "x" << canvas.height << " for configure "
              << configure_ << "\n";
  }
  inFlight_ = static_cast<uint64_t>(sequence);
}

}  // namespace oxy
