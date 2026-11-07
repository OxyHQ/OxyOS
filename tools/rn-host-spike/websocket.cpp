#include "websocket.h"
#include "system.h"

#include <cmath>
#include <iostream>
#include <stdexcept>

namespace oxyos {

namespace {

std::string view(const uint8_t* data, size_t length) {
  return data ? std::string(reinterpret_cast<const char*>(data), length) : std::string();
}

}  // namespace

WebSockets::WebSockets(Networking& networking, BlobStore& blobs, Emit emit)
    : blobs_(blobs), emit_(std::move(emit)), sockets_(oxyos_websockets_new(networking.handle())) {
  if (!sockets_) throw std::runtime_error(std::string("WebSockets cannot start: ") + oxyos_last_error());
}

WebSockets::~WebSockets() { oxyos_websockets_free(sockets_); }

void WebSockets::connect(int64_t id, const std::string& url, const std::vector<std::string>& protocols,
                         const std::vector<std::pair<std::string, std::string>>& headers) {
  // An installed version reaches the Internet only if its account granted `network`, as its
  // requests do (network.cpp): refused as a socket that failed, never a crash.
  if (!LaunchCapabilities::current().allows("network")) {
    refused_.emplace_back(id, kNotGrantedTheNetwork);
    return;
  }
  std::vector<OxyOSWebSocketProtocol> asked;
  for (const auto& protocol : protocols) asked.push_back({protocol.data(), protocol.size()});
  std::vector<OxyOSNetHeader> sent;
  for (const auto& [name, value] : headers) sent.push_back({name.data(), name.size(), value.data(), value.size()});
  OxyOSWebSocketRequest request{
      .id = id,
      .url = url.data(),
      .url_length = url.size(),
      .protocols = asked.data(),
      .protocol_count = asked.size(),
      .headers = sent.data(),
      .header_count = sent.size(),
  };
  if (oxyos_websockets_connect(sockets_, &request) != 0) throw std::invalid_argument(oxyos_last_error());
}

void WebSockets::send(int64_t id, const std::string& text) {
  if (oxyos_websockets_send(sockets_, id, reinterpret_cast<const uint8_t*>(text.data()), text.size(), 0) != 0) {
    throw std::invalid_argument(oxyos_last_error());
  }
}

void WebSockets::sendBinary(int64_t id, const std::string& bytes) {
  oxyos_websockets_send(sockets_, id, reinterpret_cast<const uint8_t*>(bytes.data()), bytes.size(), 1);
}

void WebSockets::ping(int64_t id) { oxyos_websockets_ping(sockets_, id); }

void WebSockets::close(int64_t id, double code, const std::string& reason) {
  if (!std::isfinite(code) || code != std::trunc(code) || code < 0 || code > 65535) {
    throw std::invalid_argument("the close code is not a number from 1000 to 4999");
  }
  if (oxyos_websockets_close(sockets_, id, static_cast<uint32_t>(code), reason.data(), reason.size()) != 0) {
    throw std::invalid_argument(oxyos_last_error());
  }
}

void WebSockets::receiveBlobs(int64_t id, bool blobs) {
  if (blobs) {
    blobHandlers_.insert(id);
  } else {
    blobHandlers_.erase(id);
  }
}

bool WebSockets::deliver() {
  bool delivered = !refused_.empty();
  for (auto refused = std::exchange(refused_, {}); auto& [id, message] : refused) {
    std::cerr << "[host] WebSocket " << id << " failed: " << message << "\n";
    emit_("websocketFailed", folly::dynamic::object("id", id)("message", message));
  }
  OxyOSWebSocketEvent event;
  while (oxyos_websockets_event(sockets_, &event) == OXYOS_WEBSOCKET_EVENT) {
    delivered = true;
    folly::dynamic id = event.id;
    std::string data = view(event.data, event.length);
    switch (event.kind) {
      case OXYOS_WEBSOCKET_OPEN:
        emit_("websocketOpen", folly::dynamic::object("id", id)("protocol", data));
        break;
      case OXYOS_WEBSOCKET_TEXT:
        emit_("websocketMessage", folly::dynamic::object("id", id)("type", "text")("data", data));
        break;
      case OXYOS_WEBSOCKET_BINARY:
        if (blobHandlers_.count(event.id)) {
          // As Android's BlobModule hands a binary message over: a blob of its bytes.
          auto size = static_cast<int64_t>(data.size());
          folly::dynamic blob = folly::dynamic::object("blobId", blobs_.put(std::move(data)))("offset", 0)("size", size);
          emit_("websocketMessage", folly::dynamic::object("id", id)("type", "blob")("data", std::move(blob)));
        } else {
          emit_("websocketMessage", folly::dynamic::object("id", id)("type", "binary")("data", base64Encode(data)));
        }
        break;
      case OXYOS_WEBSOCKET_CLOSED:
        blobHandlers_.erase(event.id);
        emit_("websocketClosed",
              folly::dynamic::object("id", id)("code", static_cast<int64_t>(event.code))("reason", data));
        break;
      case OXYOS_WEBSOCKET_FAILED:
        blobHandlers_.erase(event.id);
        // The application's error event says nothing of why: the reason is told here.
        std::cerr << "[host] WebSocket " << event.id << " failed: " << data << "\n";
        emit_("websocketFailed", folly::dynamic::object("id", id)("message", data));
        break;
    }
  }
  return delivered;
}

}  // namespace oxyos
