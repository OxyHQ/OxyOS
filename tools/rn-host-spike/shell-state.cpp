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

ShellState::ShellState() {
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
    case OXYOS_EVENT_WORKSPACES:
      workspaces_ = event.detail;
      active_ = event.code;
      return true;
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
        "workspace", window.workspace)("minimized", window.minimized)("focused", window.focused));
  }
  return folly::dynamic::object("windows", std::move(windows))("workspaces", workspaces_)("active", active_)(
      "locked", locked_)("user", folly::dynamic::object("name", user_)("fullName", fullName_))("capsLock", capsLock_)("wallpaper", wallpaperUri());
}

}  // namespace oxyos
