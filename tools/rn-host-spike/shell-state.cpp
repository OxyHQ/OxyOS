#include "shell-state.h"

#include <cstring>

namespace oxyos {

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
    case OXYOS_EVENT_WORKSPACES:
      workspaces_ = event.detail;
      active_ = event.code;
      return true;
    default:
      return false;
  }
}

folly::dynamic ShellState::state() const {
  folly::dynamic windows = folly::dynamic::array();
  for (const auto& [id, window] : windows_) {
    windows.push_back(folly::dynamic::object("id", id)("appId", window.appId)("title", window.title)(
        "workspace", window.workspace)("minimized", window.minimized)("focused", window.focused));
  }
  return folly::dynamic::object("windows", std::move(windows))("workspaces", workspaces_)("active", active_);
}

}  // namespace oxyos
