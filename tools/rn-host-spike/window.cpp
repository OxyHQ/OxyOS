#include "window.h"

#include <cstdlib>
#include <iostream>
#include <stdexcept>

namespace oxyos {

namespace {

[[noreturn]] void failed(const std::string& what) {
  throw std::runtime_error(what + ": " + oxyos_last_error());
}

bool tracing() {
  static const bool trace = std::getenv("OXYOS_HOST_TRACE") != nullptr;
  return trace;
}

}  // namespace

Window::Window(const std::string& socket, uint32_t width, uint32_t height) {
  client_ = oxyos_client_connect(socket.c_str());
  if (!client_) failed("cannot connect to the compositor at " + socket);
  if (oxyos_client_create_toplevel(client_, kToplevel, width, height) != 0) {
    oxyos_client_close(client_);
    failed("cannot create the window");
  }
  surfaces_[kToplevel] = Surface{};
}

Window::~Window() { oxyos_client_close(client_); }

Window::Surface* Window::surface(uint32_t id) {
  auto it = surfaces_.find(id);
  return it == surfaces_.end() ? nullptr : &it->second;
}

bool Window::dispatch(const std::function<void(const OxyOSEvent&)>& input) {
  OxyOSEvent event{};
  while (true) {
    switch (oxyos_client_poll(client_, &event)) {
      case OXYOS_POLL_EMPTY:
        return true;
      case OXYOS_POLL_CLOSED:
        return false;
      case OXYOS_POLL_EVENT:
        if (tracing()) {
          std::cerr << "[window] event " << event.kind << " surface " << event.surface << " serial " << event.serial
                    << " rect " << event.x << "," << event.y << " " << event.width << "x" << event.height
                    << " detail " << event.detail << "\n";
        }
        break;
      default:
        failed("compositor connection");
    }
    Surface* target = surface(event.surface);
    if (event.kind == OXYOS_EVENT_CONFIGURE && target) {
      float scale = event.scale_120 / 120.0f;
      if (event.width != target->width || event.height != target->height || scale != target->scale) {
        target->resized = true;
      }
      target->x = static_cast<float>(event.x);
      target->y = static_cast<float>(event.y);
      target->width = static_cast<float>(event.width);
      target->height = static_cast<float>(event.height);
      target->scale = scale;
      target->configure = event.serial;
    } else if (event.kind == OXYOS_EVENT_FEEDBACK) {
      if (target && event.serial == target->inFlight) target->inFlight = 0;
      if (event.detail == OXYOS_OUTCOME_PRESENTED && event.surface == kToplevel) ++presented_;
    } else if (event.kind == OXYOS_EVENT_CLIPBOARD_TYPES || event.kind == OXYOS_EVENT_CLIPBOARD_TYPE ||
               event.kind == OXYOS_EVENT_CLIPBOARD_EMPTY) {
      clipboard_.observe(event);
    } else if (event.kind != OXYOS_EVENT_RELEASED) {
      input(event);
    }
  }
}

bool Window::resized() {
  auto& toplevel = surfaces_.at(kToplevel);
  bool resized = toplevel.resized;
  toplevel.resized = false;
  return resized;
}

bool Window::ready(uint32_t id) const {
  auto it = surfaces_.find(id);
  return it != surfaces_.end() && it->second.configure != 0 && it->second.inFlight == 0;
}

void Window::present(uint32_t id, const Canvas& canvas, const std::vector<OxyOSRect>& damage) {
  Surface& target = surfaces_.at(id);
  // Canvas pixels are 0xAARRGGBB words: in memory (little-endian) B, G, R, A, which is the
  // protocol's XRGB8888 / premultiplied ARGB8888 layout.
  uint32_t format = id == kToplevel ? OXYOS_FORMAT_XRGB8888 : OXYOS_FORMAT_ARGB8888;
  auto sequence = oxyos_client_present_damaged(client_, id, target.configure, static_cast<uint32_t>(canvas.width),
                                             static_cast<uint32_t>(canvas.height), format,
                                             reinterpret_cast<const uint8_t*>(canvas.pixels.data()),
                                             canvas.pixels.size() * sizeof(uint32_t), damage.data(), damage.size());
  if (sequence < 0) failed("cannot present");
  if (tracing()) {
    std::cerr << "[window] present " << id << " #" << sequence << " " << canvas.width << "x" << canvas.height
              << " for configure " << target.configure << "\n";
  }
  target.inFlight = static_cast<uint64_t>(sequence);
}

void Window::openPopup(uint32_t id, uint32_t parent, const OxyOSPositioner& positioner, uint64_t serial) {
  if (oxyos_client_create_popup(client_, id, parent, &positioner) != 0) failed("cannot open a popup");
  surfaces_[id] = Surface{};
  if (serial != 0 && oxyos_client_grab_popup(client_, id, serial) != 0) failed("cannot grab the popup");
}

void Window::reposition(uint32_t id, const OxyOSPositioner& positioner) {
  if (oxyos_client_reposition(client_, id, &positioner) != 0) failed("cannot reposition a popup");
}

void Window::publishAccessibility(uint32_t id, const std::vector<OxyOSAccessibleNode>& nodes) {
  if (oxyos_client_publish_accessibility(client_, id, nodes.data(), nodes.size()) != 0) {
    failed("cannot publish accessibility");
  }
}

void Window::destroy(uint32_t id) {
  if (surfaces_.erase(id) == 0) return;
  if (oxyos_client_destroy_surface(client_, id) != 0) failed("cannot destroy a popup");
}

}  // namespace oxyos
