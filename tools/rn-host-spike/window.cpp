#include "window.h"

#include <algorithm>
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

Window::Window(const std::string& socket, Shell shell) : shell_(shell) {
  client_ = oxyos_client_connect(socket.c_str());
  if (!client_) failed("cannot connect to the compositor at " + socket);
  // A background takes the whole output; its requested size is not used.
  if (oxyos_client_create_shell_surface(client_, kToplevel, OXYOS_SHELL_BACKGROUND, OXYOS_EDGE_NONE, shell.output, 1,
                                        1, 0) != 0) {
    oxyos_client_close(client_);
    failed("cannot create the shell's background (does this client have the shell grant?)");
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
      target->states = event.flag;
    } else if (event.kind == OXYOS_EVENT_REFRESH) {
      if (target) target->refreshMhz = event.detail;
    } else if (event.kind == OXYOS_EVENT_FEEDBACK) {
      if (target && event.serial == target->inFlight) {
        target->inFlight = 0;
        if (event.detail == OXYOS_OUTCOME_PRESENTED) {
          target->shownWidth = target->inFlightWidth;
          target->shownHeight = target->inFlightHeight;
        }
      }
      if (event.detail == OXYOS_OUTCOME_PRESENTED && target) target->shown = true;
      if (event.detail == OXYOS_OUTCOME_PRESENTED && event.surface == kToplevel) ++presented_;
    } else if (event.kind == OXYOS_EVENT_CLIPBOARD_TYPES || event.kind == OXYOS_EVENT_CLIPBOARD_TYPE ||
               event.kind == OXYOS_EVENT_CLIPBOARD_EMPTY) {
      clipboard_.observe(event);
    } else if (event.kind == OXYOS_EVENT_PREFERENCES) {
      drawing_ = Drawing{.dark = (event.flag & OXYOS_PREFERENCES_DARK) != 0,
                         .reduceMotion = (event.flag & OXYOS_PREFERENCES_REDUCE_MOTION) != 0,
                         .reduceTransparency = (event.flag & OXYOS_PREFERENCES_REDUCE_TRANSPARENCY) != 0,
                         .accent = event.text};
      input(event);
    } else if (event.kind == OXYOS_EVENT_PREFERENCE) {
      if (event.detail == 0) receiving_.clear();
      receiving_ += event.text;
      if (event.flag) {
        auto equals = receiving_.find('=');
        if (equals != std::string::npos) preferences_[receiving_.substr(0, equals)] = receiving_.substr(equals + 1);
        receiving_.clear();
        input(event);
      }
    } else if (event.kind == OXYOS_EVENT_DISPLAY_NAME) {
      auto& telling = telling_[event.code];
      if (event.detail == 0 || !telling.name) telling.name = "";
      *telling.name += event.text;
    } else if (event.kind == OXYOS_EVENT_DISPLAY_MODE) {
      auto& telling = telling_[event.code];
      if (event.detail == 0 || !telling.modes) telling.modes.emplace();
      telling.modes->push_back(DisplayMode{.width = event.width,
                                           .height = event.height,
                                           .refreshMhz = static_cast<uint32_t>(event.x),
                                           .preferred = event.flag != 0});
    } else if (event.kind == OXYOS_EVENT_DISPLAY_SCALE) {
      auto& telling = telling_[event.code];
      if (event.detail == 0 || !telling.scales) telling.scales.emplace();
      telling.scales->push_back(event.scale_120);
      if (event.flag) telling.recommended = event.scale_120;
    } else if (event.kind == OXYOS_EVENT_DISPLAY) {
      // What was told before it, and what it says.
      auto& display = displays_[event.code];
      auto telling = std::move(telling_[event.code]);
      telling_.erase(event.code);
      if (telling.name) display.name = *telling.name;
      if (telling.modes) display.modes = *telling.modes;
      if (telling.scales) {
        display.scales = *telling.scales;
        display.recommended = telling.recommended;
      }
      display.x = event.x;
      display.y = event.y;
      display.width = event.width;
      display.height = event.height;
      display.scale120 = event.scale_120;
      display.mode = event.detail;
      display.widthMm = event.surface;
      display.heightMm = static_cast<uint32_t>(event.serial);
      display.primary = (event.flag & OXYOS_DISPLAY_PRIMARY) != 0;
      display.builtin = (event.flag & OXYOS_DISPLAY_BUILTIN) != 0;
      input(event);
    } else if (event.kind == OXYOS_EVENT_DISPLAY_REMOVED) {
      displays_.erase(event.code);
      telling_.erase(event.code);
      input(event);
    } else if (event.kind == OXYOS_EVENT_DISPLAY_CONFIRMATION) {
      trial_ = event.detail ? Trial{.output = event.code, .seconds = event.detail} : Trial{};
      input(event);
    } else if (event.kind == OXYOS_EVENT_ACTIVATED && event.surface == kToplevel) {
      active_ = event.flag != 0;
      input(event);
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

bool Window::answered(uint32_t id) const {
  auto it = surfaces_.find(id);
  // Before its first frame there is nothing to answer with: that frame comes with its content.
  return it == surfaces_.end() || it->second.presentedFor == 0 || it->second.presentedFor == it->second.configure;
}

bool Window::ready(uint32_t id) const {
  auto it = surfaces_.find(id);
  return it != surfaces_.end() && it->second.configure != 0 && it->second.inFlight == 0;
}

void Window::choosePainter() {
  if (painterChosen_) return;
  painterChosen_ = true;
  const char* asked = std::getenv("OXYOS_HOST_RENDERER");
  std::string renderer = asked ? asked : "";
  if (renderer == "cpu") {
    std::cerr << "[window] drawing on the CPU (OXYOS_HOST_RENDERER=cpu)\n";
    return;
  }
  uint64_t device = 0;
  std::string why;
  if (oxyos_client_gpu_device(client_, &device) != 1) {
    why = "the compositor offers no GPU buffers";
  } else if (OxyOSPaint* paint = oxyos_paint_new(client_)) {
    if (oxyos_paint_exports(paint)) {
      setGpuPaint(paint);
      std::cerr << "[window] drawing on the GPU: " << oxyos_paint_device_name(paint) << "\n";
      return;
    }
    why = std::string(oxyos_paint_device_name(paint)) + " exports no DMA-BUF";
    oxyos_paint_free(paint);
  } else {
    why = oxyos_last_error();
  }
  if (renderer == "gpu") failed("OXYOS_HOST_RENDERER=gpu, but " + why);
  std::cerr << "[window] drawing on the CPU: " << why << "\n";
}

void Window::present(uint32_t id, const Canvas& canvas, const std::vector<OxyOSRect>& damage) {
  Surface& target = surfaces_.at(id);
  // Canvas pixels are 0xAARRGGBB words: in memory (little-endian) B, G, R, A, which is the
  // protocol's XRGB8888 / premultiplied ARGB8888 layout.
  uint32_t format = id == kToplevel ? OXYOS_FORMAT_XRGB8888 : OXYOS_FORMAT_ARGB8888;
  int64_t sequence = -1;
  if (canvas.gpu) {
    // Drawn on the GPU: presented in the surface's GPU buffers (docs/adr/0031-gpu-buffers.mdx).
    sequence = oxyos_paint_present(gpuPaint() ? gpuPaint() : nullptr, client_, id, target.configure,
                                   gpuCanvasId(canvas), format, damage.data(), damage.size());
    if (sequence < 0) {
      std::cerr << "[window] the GPU path stops (" << oxyos_last_error() << "): drawing on the CPU\n";
      setGpuPaint(nullptr);
    }
  }
  if (sequence < 0) {
    std::vector<uint32_t> pixels = canvasPixels(canvas);
    if (pixels.size() != static_cast<size_t>(canvas.width) * canvas.height) failed("cannot read a frame back");
    sequence = oxyos_client_present_damaged(client_, id, target.configure, static_cast<uint32_t>(canvas.width),
                                          static_cast<uint32_t>(canvas.height), format,
                                          reinterpret_cast<const uint8_t*>(pixels.data()),
                                          pixels.size() * sizeof(uint32_t), damage.data(), damage.size());
  }
  if (sequence < 0) failed("cannot present");
  // The next frames are drawn on the GPU where the compositor offers one (its table came after
  // Welcome, before this first configure was answered).
  choosePainter();
  if (tracing()) {
    std::cerr << "[window] present " << id << " #" << sequence << " " << canvas.width << "x" << canvas.height
              << " for configure " << target.configure << "\n";
  }
  target.inFlight = static_cast<uint64_t>(sequence);
  target.inFlightWidth = target.width;
  target.inFlightHeight = target.height;
  target.presentedFor = target.configure;
}

std::optional<std::pair<float, float>> Window::extent(uint32_t id) const {
  auto it = surfaces_.find(id);
  if (it == surfaces_.end()) return std::nullopt;
  const Surface& s = it->second;
  bool shown = s.shownWidth > 0 && s.shownHeight > 0, flying = s.inFlight != 0;
  if (shown && flying) {
    return std::pair{std::min(s.shownWidth, s.inFlightWidth), std::min(s.shownHeight, s.inFlightHeight)};
  }
  if (shown) return std::pair{s.shownWidth, s.shownHeight};
  if (flying) return std::pair{s.inFlightWidth, s.inFlightHeight};
  if (s.width > 0 && s.height > 0) return std::pair{s.width, s.height};
  return std::nullopt;
}

void Window::openPopup(uint32_t id, uint32_t parent, const OxyOSPositioner& positioner, uint64_t serial) {
  if (oxyos_client_create_popup(client_, id, parent, &positioner) != 0) failed("cannot open a popup");
  surfaces_[id] = Surface{};
  if (serial != 0 && oxyos_client_grab_popup(client_, id, serial) != 0) failed("cannot grab the popup");
}

void Window::openShellSurface(uint32_t id, uint8_t kind, uint8_t edge, uint32_t width, uint32_t height,
                              uint32_t exclusive) {
  if (!shell_) failed("only the shell has shell surfaces");
  if (oxyos_client_create_shell_surface(client_, id, kind, edge, shell_->output, width, height, exclusive) != 0) {
    failed("cannot open a shell surface");
  }
  surfaces_[id] = Surface{};
}

void Window::openLockSurface(uint32_t id) {
  if (!shell_) failed("only the shell has lock surfaces");
  if (oxyos_client_create_lock_surface(client_, id, shell_->output) != 0) failed("cannot open a lock surface");
  surfaces_[id] = Surface{};
}

bool Window::launch(uint32_t serial, const std::string& id) {
  return oxyos_client_launch(client_, serial, id.data(), id.size()) == 0;
}

void Window::lockSession() {
  if (oxyos_client_lock_session(client_) != 0) failed("cannot lock the session");
}

bool Window::setControlLevel(uint8_t control, uint32_t level) {
  return oxyos_client_set_control_level(client_, control, level) == 0;
}

bool Window::setControlMuted(uint8_t control, bool muted) {
  return oxyos_client_set_control_muted(client_, control, muted) == 0;
}

bool Window::setWifiEnabled(bool enabled) { return oxyos_client_set_wifi_enabled(client_, enabled) == 0; }

bool Window::setAppearance(uint32_t serial, bool dark) {
  return oxyos_client_set_appearance(client_, serial, dark) == 0;
}

bool Window::refreshControls() { return oxyos_client_refresh_controls(client_) == 0; }

bool Window::authenticate(uint32_t id, const std::string& secret, const std::string& account) {
  return oxyos_client_authenticate_account(client_, id, account.empty() ? nullptr : account.c_str(), secret.data(),
                                           secret.size()) == 0;
}

void Window::nameWindow(const std::string& title, const std::string& appId) {
  if (oxyos_client_name_window(client_, kToplevel, 0, title.data(), title.size()) != 0 ||
      oxyos_client_name_window(client_, kToplevel, 1, appId.data(), appId.size()) != 0) {
    failed("cannot name the window");
  }
}

void Window::focusShellSurface(uint32_t id, uint64_t serial) {
  if (oxyos_client_focus_shell_surface(client_, id, serial) != 0) failed("cannot focus a shell surface");
}

void Window::manage(uint8_t action, uint32_t window, uint32_t workspace) {
  if (oxyos_client_manage(client_, action, window, workspace) != 0) failed("cannot manage a window");
}

bool Window::setPreference(uint32_t serial, const std::string& key, const std::string& value) {
  return oxyos_client_set_preference(client_, serial, key.c_str(), value.c_str()) == 0;
}

bool Window::setDisplayMode(uint32_t serial, uint32_t output, uint32_t width, uint32_t height, uint32_t refreshMhz) {
  return oxyos_client_set_display_mode(client_, serial, output, width, height, refreshMhz) == 0;
}

bool Window::setDisplayScale(uint32_t serial, uint32_t output, uint32_t scale120) {
  return oxyos_client_set_display_scale(client_, serial, output, scale120) == 0;
}

bool Window::setDisplayOrigin(uint32_t serial, uint32_t output, int32_t x, int32_t y) {
  return oxyos_client_set_display_origin(client_, serial, output, x, y) == 0;
}

bool Window::setPrimaryDisplay(uint32_t serial, uint32_t output) {
  return oxyos_client_set_primary_display(client_, serial, output) == 0;
}

bool Window::confirmDisplays(uint32_t serial, bool keep) { return oxyos_client_confirm_displays(client_, serial, keep) == 0; }

void Window::beginMove(uint64_t serial) {
  if (oxyos_client_begin_move(client_, kToplevel, serial) != 0) failed("cannot move the window");
}

void Window::setMaximized(bool enabled) {
  if (oxyos_client_set_maximized(client_, kToplevel, enabled) != 0) failed("cannot zoom the window");
}

void Window::minimize() {
  if (oxyos_client_minimize(client_, kToplevel) != 0) failed("cannot minimize the window");
}

void Window::setSizeLimits(uint32_t minWidth, uint32_t minHeight, uint32_t maxWidth, uint32_t maxHeight) {
  if (oxyos_client_set_size_limits(client_, kToplevel, minWidth, minHeight, maxWidth, maxHeight) != 0) {
    failed("cannot set the window's sizes");
  }
}

void Window::setPointerShape(uint32_t id, uint8_t shape) {
  if (oxyos_client_set_pointer_shape(client_, id, shape) != 0) failed("cannot set the pointer's shape");
}

void Window::setInputRegion(uint32_t id, const OxyOSRect& region) {
  if (oxyos_client_set_input_region(client_, id, region.x, region.y, region.width, region.height) != 0) {
    failed("cannot set an input region");
  }
}

void Window::showWindows(uint32_t id, const std::vector<OxyOSWindowView>& views) {
  if (oxyos_client_show_windows(client_, id, views.data(), views.size()) != 0) {
    failed("cannot show windows live");
  }
}

void Window::reposition(uint32_t id, const OxyOSPositioner& positioner) {
  if (oxyos_client_reposition(client_, id, &positioner) != 0) failed("cannot reposition a popup");
}

void Window::publishAccessibility(uint32_t id, const std::vector<OxyOSAccessibleNode>& nodes) {
  if (oxyos_client_publish_accessibility(client_, id, nodes.data(), nodes.size()) != 0) {
    failed("cannot publish accessibility");
  }
}

void Window::setMenus(uint32_t serial, const std::vector<OxyOSMenuItem>& items) {
  if (oxyos_client_set_menus(client_, kToplevel, serial, items.data(), items.size()) != 0) {
    failed("cannot set the window's menus");
  }
}

void Window::activateMenuItem(uint32_t window, uint32_t serial, uint32_t index) {
  if (oxyos_client_activate_menu_item(client_, window, serial, index) != 0) failed("cannot choose a menu item");
}

void Window::announce(uint32_t id, uint8_t politeness, const std::string& text) {
  if (oxyos_client_announce(client_, id, politeness, text.data(), text.size()) != 0) failed("cannot announce");
}

bool Window::setDoNotDisturb(uint32_t serial, bool enabled) {
  return oxyos_client_set_do_not_disturb(client_, serial, enabled) == 0;
}

bool Window::postNotification(uint32_t serial, const OxyOSNotification& notification) {
  return oxyos_client_post_notification(client_, serial, &notification) == 0;
}

void Window::withdrawNotification(uint32_t notification) {
  if (oxyos_client_withdraw_notification(client_, notification) != 0) failed("cannot withdraw a notification");
}

void Window::activateNotification(uint32_t notification, uint8_t action, uint64_t serial, const std::string& reply) {
  if (oxyos_client_activate_notification(client_, notification, action, serial, reply.data(), reply.size()) != 0) {
    failed("cannot activate a notification");
  }
}

uint32_t Window::textInput(uint32_t surface, const OxyOSTextField* field, bool newField) {
  int64_t serial = oxyos_client_text_input(client_, surface, field, newField ? 1 : 0);
  if (serial < 0) failed("cannot tell the text field");
  return static_cast<uint32_t>(serial);
}

void Window::destroy(uint32_t id) {
  if (surfaces_.erase(id) == 0) return;
  if (gpuPaint()) oxyos_paint_forget_surface(gpuPaint(), client_, id);
  if (oxyos_client_destroy_surface(client_, id) != 0) failed("cannot destroy a popup");
}

}  // namespace oxyos
