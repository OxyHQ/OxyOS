// The clipboard for the Bloom host (docs/adr/0007-clipboard.mdx): text copied with the serial
// of the key that asked for it, and pastes read from the pipe their content arrives through.
// The client library (oxyos_client.h) holds what was copied and answers pastes of it itself;
// this side only starts pastes and reads them, from the host's loop, without blocking.
#pragma once

#include <oxyos_client.h>

#include <chrono>
#include <cstdint>
#include <functional>
#include <optional>
#include <string>
#include <vector>

namespace oxyos {

class Clipboard {
 public:
  // What the clipboard holds, from the compositor's events: the content's selection and its
  // types, or nothing. `changed` runs when that content changes once it was known, as Android's
  // listener hears a new clip: not when the client first learns it, nor when it is told again
  // what it already knew (focus coming back).
  void observe(const OxyOSEvent& event);
  void onChanged(std::function<void()> changed) { changed_ = std::move(changed); }
  // Whether it holds text this client can paste.
  bool hasText() const;

  // Copies `text` with the serial of the input that asked for it. False if it could not be
  // sent (the compositor may still refuse it: a stale serial, or focus lost).
  bool copy(OxyOSClient* client, uint64_t serial, const std::string& text);
  // Pastes the clipboard's text with the serial of the input that asked for it: `done` receives
  // it when it has arrived whole, and an empty string if the clipboard holds no text or the
  // paste is refused or fails (the failure is logged).
  void paste(OxyOSClient* client, uint64_t serial, std::function<void(std::string)> done);

  // Each turn: reads what pastes have received; finished ones call their `done`. True if any
  // finished.
  bool deliver();

  // A paste's limits: content beyond them is dropped and the paste fails.
  static constexpr size_t kMaxBytes = 16 * 1024 * 1024;
  static constexpr std::chrono::seconds kTimeout{5};

 private:
  struct Paste {
    int fd;
    std::string bytes;
    std::chrono::steady_clock::time_point started;
    std::function<void(std::string)> done;
  };

  std::optional<uint32_t> selection_;
  std::vector<std::string> types_;
  size_t expected_ = 0;  // types announced for the selection
  bool known_ = false;    // whether the client was told anything yet
  std::optional<uint32_t> heard_;  // the content `changed` last reported, none if empty
  void settled();
  std::function<void()> changed_;
  std::vector<Paste> pastes_;
};

}  // namespace oxyos
