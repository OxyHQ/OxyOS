#include "clipboard.h"

#include <fcntl.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <cstring>
#include <iostream>

namespace oxyos {

namespace {

constexpr const char* kText = "text/plain;charset=utf-8";

}  // namespace

void Clipboard::observe(const OxyOSEvent& event) {
  if (event.kind == OXYOS_EVENT_CLIPBOARD_TYPES) {
    selection_ = static_cast<uint32_t>(event.serial);
    types_.clear();
    expected_ = event.detail;
    if (expected_ == 0) settled();
  } else if (event.kind == OXYOS_EVENT_CLIPBOARD_TYPE && selection_ == static_cast<uint32_t>(event.serial)) {
    types_.emplace_back(event.text, strnlen(event.text, sizeof(event.text)));
    if (types_.size() == expected_) settled();
  } else if (event.kind == OXYOS_EVENT_CLIPBOARD_EMPTY) {
    selection_.reset();
    types_.clear();
    expected_ = 0;
    settled();
  }
}

void Clipboard::settled() {
  bool first = !known_;
  known_ = true;
  if (selection_ == heard_) return;
  heard_ = selection_;
  if (!first && changed_) changed_();
}

bool Clipboard::hasText() const {
  return selection_ && std::find(types_.begin(), types_.end(), kText) != types_.end();
}

bool Clipboard::copy(OxyOSClient* client, uint64_t serial, const std::string& text) {
  if (oxyos_client_copy_text(client, serial, text.data(), text.size()) == 0) return true;
  std::cerr << "[host] copy not sent: " << oxyos_last_error() << "\n";
  return false;
}

void Clipboard::paste(OxyOSClient* client, uint64_t serial, std::function<void(std::string)> done) {
  if (!hasText()) return done("");
  int fd = oxyos_client_paste(client, *selection_, kText, std::strlen(kText), serial);
  if (fd < 0) {
    std::cerr << "[host] paste not sent: " << oxyos_last_error() << "\n";
    return done("");
  }
  // Read from the host's loop as the content arrives; a slow owner never blocks the app.
  fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
  pastes_.push_back(Paste{fd, {}, std::chrono::steady_clock::now(), std::move(done)});
}

std::vector<int> Clipboard::fds() const {
  std::vector<int> out;
  for (const auto& paste : pastes_) out.push_back(paste.fd);
  return out;
}

std::optional<std::chrono::steady_clock::time_point> Clipboard::nextDeadline() const {
  std::optional<std::chrono::steady_clock::time_point> first;
  for (const auto& paste : pastes_) {
    if (!first || paste.started + kTimeout < *first) first = paste.started + kTimeout;
  }
  return first;
}

bool Clipboard::deliver() {
  bool finished = false;
  for (auto it = pastes_.begin(); it != pastes_.end();) {
    char buffer[64 * 1024];
    ssize_t got = 0;
    while ((got = read(it->fd, buffer, sizeof(buffer))) > 0 && it->bytes.size() <= kMaxBytes) {
      it->bytes.append(buffer, static_cast<size_t>(got));
    }
    const char* failure = nullptr;
    if (got < 0 && errno != EAGAIN && errno != EWOULDBLOCK) failure = std::strerror(errno);
    if (it->bytes.size() > kMaxBytes) failure = "more than 16 MiB";
    if (!failure && got != 0 && std::chrono::steady_clock::now() - it->started > kTimeout) failure = "no end in time";
    if (!failure && got != 0) {
      ++it;
      continue;
    }
    close(it->fd);
    if (failure) {
      std::cerr << "[host] paste failed: " << failure << "\n";
      it->done("");
    } else {
      it->done(std::move(it->bytes));
    }
    it = pastes_.erase(it);
    finished = true;
  }
  return finished;
}

}  // namespace oxyos
