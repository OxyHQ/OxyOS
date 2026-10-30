#include "shell-state.h"

#include <folly/json.h>
#include <pwd.h>
#include <unistd.h>

#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <iostream>
#include <sstream>
#include <vector>

namespace oxyos {

ShellState::ShellState(uint32_t output) : output_(output) {
  std::vector<char> buffer(16 * 1024);
  passwd entry{};
  passwd* found = nullptr;
  if (getpwuid_r(getuid(), &entry, buffer.data(), buffer.size(), &found) == 0 && found) {
    if (entry.pw_name) user_ = entry.pw_name;
    // GECOS: the full name, then office, phones and other, separated by commas.
    if (entry.pw_gecos) {
      fullName_ = entry.pw_gecos;
      fullName_ = fullName_.substr(0, fullName_.find(','));
    }
  }
  readApplications();
  // The solar wallpaper's manifest: its frames and the sun each shows.
  if (const char* dir = std::getenv("OXYOS_WALLPAPER")) {
    try {
      std::ifstream file(std::string(dir) + "/manifest.json");
      std::stringstream text;
      text << file.rdbuf();
      auto manifest = folly::parseJson(text.str());
      for (const auto& frame : manifest["frames"]) {
        wallpaperFrames_[static_cast<uint32_t>(frame["index"].asInt())] = frame["file"].asString();
      }
      for (const auto& anchor : manifest["solarAnchors"]) {
        anchors_.push_back(OxyOSSolarAnchor{static_cast<uint32_t>(anchor["frame"].asInt()),
                                            anchor["elevationDegrees"].asDouble(),
                                            anchor["azimuthDegrees"].asDouble()});
      }
      previewFrame_ = static_cast<uint32_t>(manifest.getDefault("previewFrame", 0).asInt());
      wallpaperDir_ = dir;
      chooseFrame();
    } catch (const std::exception& error) {
      std::cerr << "[host] the wallpaper at " << dir << ": " << error.what() << "\n";
    }
  }
}

// The clock the wallpaper follows: the system's, or a scenario's fixed moment (OXYOS_SOLAR_NOW, unix
// seconds), which never moves on.
static int64_t solarNow() {
  static const char* fixed = std::getenv("OXYOS_SOLAR_NOW");
  return fixed ? std::atoll(fixed) : static_cast<int64_t>(std::time(nullptr));
}

void ShellState::readApplications() {
  int64_t length = oxyos_applications(nullptr, 0);
  if (length < 0) {
    std::cerr << "[host] the applications: " << oxyos_last_error() << "\n";
    return;
  }
  std::string json(static_cast<size_t>(length) + 1, '\0');
  oxyos_applications(json.data(), json.size());
  json.resize(static_cast<size_t>(length));
  folly::dynamic applications = folly::dynamic::array();
  for (auto app : folly::parseJson(json)) {
    // A picture's file, or an SVG's drawing (react-native-svg draws it from its text).
    folly::dynamic icon = nullptr;
    if (app["icon"].isString()) {
      const auto& path = app["icon"].getString();
      if (path.size() > 4 && path.compare(path.size() - 4, 4, ".svg") == 0) {
        std::ifstream file(path);
        std::stringstream text;
        text << file.rdbuf();
        if (file) icon = folly::dynamic::object("svg", text.str());
      } else {
        icon = folly::dynamic::object("uri", "file://" + path);
      }
    }
    applications.push_back(folly::dynamic::object("id", app["id"])("name", app["name"])("icon", icon)(
        "pinned", app["pinned"]));
  }
  applications_ = std::move(applications);
}

void ShellState::chooseFrame() {
  uint32_t frame = previewFrame_;
  int64_t next = 0;
  int64_t now = solarNow();
  switch (oxyos_solar_frame(anchors_.data(), anchors_.size(), now, &frame, &next)) {
    case 0:
      break;
    case 1:
      // No sun where the clock is set (UTC): the wallpaper's own frame, looked at again hourly.
      frame = previewFrame_;
      next = now + 3600;
      break;
    default:
      std::cerr << "[host] the wallpaper: " << oxyos_last_error() << "\n";
      frame = previewFrame_;
      next = now + 3600;
  }
  frame_ = frame;
  nextChange_ = next;
}

bool ShellState::tick() {
  if (wallpaperDir_.empty() || solarNow() < nextChange_) return false;
  auto before = frame_;
  chooseFrame();
  return frame_ != before;
}

bool ShellState::observe(const OxyOSEvent& event) {
  switch (event.kind) {
    case OXYOS_EVENT_WINDOW_STATE: {
      auto& window = windows_[event.code];
      window.workspace = event.detail;
      window.minimized = (event.flag & OXYOS_WINDOW_MINIMIZED) != 0;
      window.focused = (event.flag & OXYOS_WINDOW_FOCUSED) != 0;
      window.launched = (event.flag & OXYOS_WINDOW_LAUNCHED) != 0;
      return true;
    }
    case OXYOS_EVENT_WINDOW_TITLE:
    case OXYOS_EVENT_WINDOW_APP_ID: {
      bool title = event.kind == OXYOS_EVENT_WINDOW_TITLE;
      auto& receiving = (title ? title_ : appId_)[event.code];
      if (event.detail == 0) receiving.clear();
      receiving.append(event.text, strnlen(event.text, sizeof(event.text)));
      if (!event.flag) return false;
      auto& window = windows_[event.code];
      (title ? window.title : window.appId) = std::move(receiving);
      (title ? title_ : appId_).erase(event.code);
      return true;
    }
    case OXYOS_EVENT_WINDOW_CLOSED:
      title_.erase(event.code);
      appId_.erase(event.code);
      return windows_.erase(event.code) > 0;
    case OXYOS_EVENT_FOCUS:
      // A surface gains focus with Caps Lock off unless told: CAPS_LOCK follows when it is on.
      if (!event.flag || !capsLock_) return false;
      capsLock_ = false;
      return true;
    case OXYOS_EVENT_CAPS_LOCK:
      if (capsLock_ == (event.flag != 0)) return false;
      capsLock_ = event.flag != 0;
      return true;
    case OXYOS_EVENT_LOCK_STATE:
      if (locked_ == (event.flag != 0)) return false;
      locked_ = event.flag != 0;
      return true;
    // The active window's menus (docs/adr/0021-menu-bar.mdx), shown once whole.
    case OXYOS_EVENT_MENU_BAR:
      receivingMenuBar_ = MenuBar{.window = event.code, .serial = event.detail, .count = event.width};
      return false;
    case OXYOS_EVENT_MENU_BAR_ITEM: {
      if (event.code != receivingMenuBar_.window || event.detail != receivingMenuBar_.items.size()) return false;
      MenuItem item;
      item.parent = event.width == OXYOS_NO_MENU_PARENT ? -1 : static_cast<int64_t>(event.width);
      item.kind = event.height;
      item.flags = event.flag;
      receivingMenuBar_.items.push_back(std::move(item));
      return false;
    }
    case OXYOS_EVENT_MENU_BAR_TEXT: {
      if (event.code != receivingMenuBar_.window || event.detail >= receivingMenuBar_.items.size()) return false;
      auto& item = receivingMenuBar_.items[event.detail];
      (event.height == OXYOS_MENU_SHORTCUT ? item.shortcut : item.label)
          .append(event.text, strnlen(event.text, sizeof(event.text)));
      return false;
    }
    case OXYOS_EVENT_MENU_BAR_COMMITTED:
      if (event.code != receivingMenuBar_.window || receivingMenuBar_.items.size() != receivingMenuBar_.count) {
        return false;
      }
      menuBar_ = std::move(receivingMenuBar_);
      receivingMenuBar_ = MenuBar{};
      return true;
    case OXYOS_EVENT_WORKSPACES:
      workspaces_ = event.detail;
      active_ = event.code;
      return true;
    case OXYOS_EVENT_OUTPUT:
      outputs_[event.code] = Output{event.x, event.y, event.width, event.height, event.scale_120, event.flag != 0};
      return true;
    case OXYOS_EVENT_OUTPUT_REMOVED:
      return outputs_.erase(event.code) > 0;
    case OXYOS_EVENT_WINDOW_SIZE: {
      // The window's proportions, for the overview's cards (docs/adr/0022-window-views.mdx).
      auto& window = windows_[event.code];
      window.width = event.width;
      window.height = event.height;
      return true;
    }
    default:
      return false;
  }
}

folly::dynamic ShellState::wallpaperUri() const {
  if (!frame_) return nullptr;
  auto file = wallpaperFrames_.find(*frame_);
  if (file == wallpaperFrames_.end()) return nullptr;
  return "file://" + wallpaperDir_ + "/" + file->second;
}

folly::dynamic ShellState::state() const {
  folly::dynamic windows = folly::dynamic::array();
  for (const auto& [id, window] : windows_) {
    windows.push_back(folly::dynamic::object("id", id)("appId", window.appId)("title", window.title)(
        "workspace", window.workspace)("minimized", window.minimized)("focused", window.focused)("launched", window.launched)("width", window.width)("height", window.height));
  }
  folly::dynamic outputs = folly::dynamic::array();
  for (const auto& [id, output] : outputs_) {
    outputs.push_back(folly::dynamic::object("id", id)("x", output.x)("y", output.y)("width", output.width)(
        "height", output.height)("scale", output.scale120 / 120.0)("primary", output.primary));
  }
  static const char* kinds[] = {"", "menu", "item", "submenu", "separator"};
  folly::dynamic items = folly::dynamic::array();
  for (const auto& item : menuBar_.items) {
    items.push_back(folly::dynamic::object("parent", item.parent)("kind", item.kind <= 4 ? kinds[item.kind] : "")(
        "enabled", (item.flags & OXYOS_MENU_DISABLED) == 0)("checkable", (item.flags & OXYOS_MENU_CHECKABLE) != 0)(
        "checked", (item.flags & OXYOS_MENU_CHECKED) != 0)("label", item.label)("shortcut", item.shortcut));
  }
  folly::dynamic menuBar =
      folly::dynamic::object("window", menuBar_.window)("serial", menuBar_.serial)("items", std::move(items));
  return folly::dynamic::object("windows", std::move(windows))("menuBar", std::move(menuBar))("workspaces", workspaces_)("active", active_)(
      "locked", locked_)("user", folly::dynamic::object("name", user_)("fullName", fullName_))("capsLock", capsLock_)("wallpaper", wallpaperUri())(
      "applications", applications_)("output", output_)("outputs", std::move(outputs));
}

}  // namespace oxyos
