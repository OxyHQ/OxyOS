#include "network.h"

#include <cctype>
#include <iostream>
#include <stdexcept>
#include <utility>

namespace oxyos {

namespace {

constexpr char kBase64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

}  // namespace

std::string base64Encode(const std::string& bytes) {
  std::string out;
  out.reserve((bytes.size() + 2) / 3 * 4);
  size_t i = 0;
  for (; i + 2 < bytes.size(); i += 3) {
    uint32_t n = static_cast<uint8_t>(bytes[i]) << 16 | static_cast<uint8_t>(bytes[i + 1]) << 8 |
                 static_cast<uint8_t>(bytes[i + 2]);
    out += {kBase64[n >> 18], kBase64[(n >> 12) & 63], kBase64[(n >> 6) & 63], kBase64[n & 63]};
  }
  if (i < bytes.size()) {
    uint32_t n = static_cast<uint8_t>(bytes[i]) << 16;
    if (i + 1 < bytes.size()) n |= static_cast<uint8_t>(bytes[i + 1]) << 8;
    out += kBase64[n >> 18];
    out += kBase64[(n >> 12) & 63];
    out += i + 1 < bytes.size() ? kBase64[(n >> 6) & 63] : '=';
    out += '=';
  }
  return out;
}

std::optional<std::string> base64Decode(const std::string& text) {
  std::string out;
  uint32_t buffer = 0;
  int bits = 0;
  for (char c : text) {
    if (c == '=' || std::isspace(static_cast<unsigned char>(c))) continue;
    const char* at = std::char_traits<char>::find(kBase64, 64, c);
    if (!at) return std::nullopt;
    buffer = buffer << 6 | static_cast<uint32_t>(at - kBase64);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out += static_cast<char>((buffer >> bits) & 0xff);
    }
  }
  return out;
}

std::string BlobStore::put(std::string bytes) {
  std::string id = "oxyos-blob-" + std::to_string(++next_);
  blobs_[id] = std::move(bytes);
  return id;
}

void BlobStore::put(const std::string& id, std::string bytes) { blobs_[id] = std::move(bytes); }

std::optional<std::string> BlobStore::slice(const std::string& id, size_t offset, size_t size) const {
  auto it = blobs_.find(id);
  if (it == blobs_.end() || offset > it->second.size()) return std::nullopt;
  return it->second.substr(offset, size);
}

void BlobStore::release(const std::string& id) { blobs_.erase(id); }

namespace {

std::string view(const char* data, size_t length) { return data ? std::string(data, length) : std::string(); }

// The length of `bytes` without a trailing character that is not complete yet: a chunk may end in
// the middle of one, which the next completes.
size_t completeUtf8(const std::string& bytes) {
  size_t size = bytes.size();
  for (size_t back = 1; back <= 3 && back <= size; ++back) {
    auto byte = static_cast<uint8_t>(bytes[size - back]);
    if ((byte & 0xc0) == 0x80) continue;  // a continuation byte: look further back
    size_t needs = byte >= 0xf0 ? 4 : byte >= 0xe0 ? 3 : byte >= 0xc0 ? 2 : 1;
    return needs > back ? size - back : size;
  }
  return size;
}

}  // namespace

Networking::Networking(BlobStore& blobs, Emit emit) : blobs_(blobs), emit_(std::move(emit)), net_(oxyos_net_new()) {
  if (!net_) throw std::runtime_error(std::string("the network cannot start: ") + oxyos_last_error());
}

Networking::~Networking() { oxyos_net_free(net_); }

void Networking::send(Request request) {
  std::vector<OxyOSNetHeader> headers;
  for (const auto& [name, value] : request.headers) {
    headers.push_back({name.data(), name.size(), value.data(), value.size()});
  }
  // Only a text body is answered as it arrives, as on Android.
  bool incremental = request.incremental && request.responseType == "text";
  OxyOSNetRequest native{
      .id = request.id,
      .method = request.method.data(),
      .method_length = request.method.size(),
      .url = request.url.data(),
      .url_length = request.url.size(),
      .headers = headers.data(),
      .header_count = headers.size(),
      .body = reinterpret_cast<const uint8_t*>(request.body.data()),
      .body_length = request.body.size(),
      .timeout_ms = request.timeoutMs > 0 ? static_cast<uint64_t>(request.timeoutMs) : 0,
      .has_body = request.hasBody,
      .with_credentials = request.withCredentials,
      .incremental = incremental,
  };
  if (oxyos_net_send(net_, &native) != 0) return reject(request.id, oxyos_last_error());
  live_[request.id] = Live{.responseType = request.responseType, .incremental = incremental};
}

void Networking::reject(int64_t id, const std::string& error) {
  rejected_.emplace_back(id, "not sent: " + error);
}

void Networking::abort(int64_t id) {
  oxyos_net_abort(net_, id);
  live_.erase(id);
  std::erase_if(rejected_, [id](const auto& rejected) { return rejected.first == id; });
}

void Networking::clearCookies() { oxyos_net_clear_cookies(net_); }

void Networking::complete(int64_t id, const std::string& error, bool timedOut) {
  live_.erase(id);
  // fetch reports any failure as "Network request failed": the reason is told here.
  if (!error.empty()) std::cerr << "[host] network request " << id << " failed: " << error << "\n";
  emit_("didCompleteNetworkResponse",
        folly::dynamic::array(id, error.empty() ? folly::dynamic(nullptr) : folly::dynamic(error), timedOut));
}

bool Networking::deliver() {
  bool delivered = !rejected_.empty();
  for (auto rejected = std::exchange(rejected_, {}); auto& [id, error] : rejected) complete(id, error, false);
  OxyOSNetAnswer answer;
  while (oxyos_net_answer(net_, &answer) == OXYOS_NET_ANSWER) {
    delivered = true;
    auto live = live_.find(answer.id);
    if (live == live_.end()) continue;  // aborted while it was queued
    folly::dynamic id = answer.id;
    std::string bytes = view(reinterpret_cast<const char*>(answer.bytes), answer.length);
    switch (answer.kind) {
      case OXYOS_NET_RESPONSE: {
        folly::dynamic headers = folly::dynamic::object;
        for (size_t i = 0; i < answer.header_count; ++i) {
          const auto& header = answer.headers[i];
          headers[view(header.name, header.name_length)] = view(header.value, header.value_length);
        }
        emit_("didReceiveNetworkResponse", folly::dynamic::array(id, static_cast<int64_t>(answer.status), headers,
                                                                 view(answer.url, answer.url_length)));
        break;
      }
      case OXYOS_NET_CHUNK: {
        Live& request = live->second;
        std::string text = std::exchange(request.partial, {}) + bytes;
        size_t complete = completeUtf8(text);
        request.partial = text.substr(complete);
        text.resize(complete);
        request.progress = answer.progress;
        request.total = answer.total;
        emit_("didReceiveNetworkIncrementalData",
              folly::dynamic::array(id, text, static_cast<int64_t>(request.progress), request.total));
        break;
      }
      case OXYOS_NET_DATA: {
        const std::string& type = live->second.responseType;
        if (type == "base64") {
          emit_("didReceiveNetworkData", folly::dynamic::array(id, base64Encode(bytes)));
        } else if (type == "blob") {
          size_t size = bytes.size();
          std::string blob = blobs_.put(std::move(bytes));
          emit_("didReceiveNetworkData",
                folly::dynamic::array(id, folly::dynamic::object("blobId", blob)("offset", 0)(
                                              "size", static_cast<int64_t>(size))(
                                              "type", view(answer.content_type, answer.content_type_length))(
                                              "lastModified", 0)));
        } else {
          emit_("didReceiveNetworkData", folly::dynamic::array(id, bytes));
        }
        break;
      }
      case OXYOS_NET_COMPLETE: {
        Live& request = live->second;
        // A body that ended inside a character still delivers its bytes.
        if (!request.partial.empty()) {
          emit_("didReceiveNetworkIncrementalData",
                folly::dynamic::array(id, std::exchange(request.partial, {}), static_cast<int64_t>(request.progress),
                                      request.total));
        }
        complete(answer.id, view(answer.error, answer.error_length), answer.timed_out != 0);
        break;
      }
    }
  }
  return delivered;
}

}  // namespace oxyos
