// WebSockets for React Native on OxyOS (docs/react-native-android-adaptation.mdx): the native half
// of `WebSocket`, as React Native's Android implementation exposes it (`WebSocketModule`, and
// `BlobModule`'s handler and `sendOverSocket`).
//
// The sockets themselves are Rust's (crates/oxyos-net's websocket.rs, through oxyos_client.h):
// tungstenite's protocol over rustls verified against the system's certificates, each socket on a
// thread of its own that sleeps until its connection, the application or the machine's addresses
// wake it. This side turns what they hear into React Native's events on the JavaScript thread
// (`deliver`), in order: `websocketOpen`, `websocketMessage` (text, binary as base64, or a blob for
// a socket whose `binaryType` is "blob"), `websocketClosed` and `websocketFailed`, each with one map
// as Android emits it. The sockets close with 1001 (going away) when this is destroyed, with the
// application's runtime.
#pragma once

#include "network.h"
#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <cstdint>
#include <deque>
#include <functional>
#include <set>
#include <string>
#include <utility>
#include <vector>

namespace oxyos {

class WebSockets {
 public:
  // `emit(event, body)` sends an event to RCTDeviceEventEmitter on the JavaScript thread.
  using Emit = std::function<void(const std::string& event, folly::dynamic body)>;

  // Sockets sending `networking`'s cookies. Throws std::runtime_error if they cannot be made.
  WebSockets(Networking& networking, BlobStore& blobs, Emit emit);
  ~WebSockets();
  WebSockets(const WebSockets&) = delete;
  WebSockets& operator=(const WebSockets&) = delete;

  // From the JavaScript thread, as everything here. `connect` and `close` throw
  // std::invalid_argument for what `new WebSocket` and `close` refuse (a malformed URL or header, a
  // close code or reason no frame may carry), which changes nothing.
  void connect(int64_t id, const std::string& url, const std::vector<std::string>& protocols,
               const std::vector<std::pair<std::string, std::string>>& headers);
  void send(int64_t id, const std::string& text);
  void sendBinary(int64_t id, const std::string& bytes);
  void ping(int64_t id);
  void close(int64_t id, double code, const std::string& reason);
  // Binary messages of socket `id` arrive as blobs (BlobModule.addWebSocketHandler), or as base64.
  void receiveBlobs(int64_t id, bool blobs);

  // Each turn: emits what the sockets heard. True if it did.
  bool deliver();
  // Readable while events wait to be emitted.
  int fd() const { return oxyos_websockets_fd(sockets_); }

 private:
  BlobStore& blobs_;
  Emit emit_;
  OxyOSWebSockets* sockets_;
  std::set<int64_t> blobHandlers_;
  // Sockets refused before they start (an application not granted the network): failed on the
  // next turn, with why.
  std::deque<std::pair<int64_t, std::string>> refused_;
};

}  // namespace oxyos
