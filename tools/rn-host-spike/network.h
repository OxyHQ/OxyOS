// HTTP for React Native's networking on OxyOS (docs/react-native-android-adaptation.mdx): the
// native half of XMLHttpRequest, and so of fetch and axios, as React Native's Android
// implementation exposes it (Networking, BlobModule, FileReaderModule).
//
// The requests themselves are Rust's (crates/oxyos-net, through oxyos_client.h): TLS, redirects,
// compression, a cookie jar for the process, timeouts, each request on a thread of its own. This
// side turns what they answer into React Native's events on the JavaScript thread (`deliver`).
// What OxyOS does not support yet fails the request with a message, never silently: file uploads
// in multipart bodies.
#pragma once

#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <cstdint>
#include <deque>
#include <functional>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace oxyos {

std::string base64Encode(const std::string& bytes);
std::optional<std::string> base64Decode(const std::string& text);

// Blobs React Native's JavaScript refers to by id (BlobModule): response bodies, and blobs built
// from parts.
class BlobStore {
 public:
  std::string put(std::string bytes);
  void put(const std::string& id, std::string bytes);
  // The bytes of [offset, offset + size) of a blob, if it exists.
  std::optional<std::string> slice(const std::string& id, size_t offset, size_t size) const;
  void release(const std::string& id);

 private:
  std::map<std::string, std::string> blobs_;
  uint64_t next_ = 0;
};

class Networking {
 public:
  // `emit(event, args)` sends an event to RCTDeviceEventEmitter on the JavaScript thread.
  using Emit = std::function<void(const std::string& event, folly::dynamic args)>;

  // Throws std::runtime_error if the network cannot be created.
  Networking(BlobStore& blobs, Emit emit);
  ~Networking();
  Networking(const Networking&) = delete;
  Networking& operator=(const Networking&) = delete;

  struct Request {
    int64_t id = 0;
    std::string method, url;
    std::vector<std::pair<std::string, std::string>> headers;
    std::string body;
    bool hasBody = false;
    std::string responseType;  // "text", "base64" or "blob"
    bool incremental = false;
    long timeoutMs = 0;
    bool withCredentials = true;
  };

  // From the JavaScript thread, as everything here: starts a request, or fails it on the next
  // turn if it cannot be sent.
  void send(Request request);
  // A request that cannot be sent: it fails, on the next turn, with `error`.
  void reject(int64_t id, const std::string& error);
  void abort(int64_t id);
  void clearCookies();

  // Each turn: emits what the network has answered. True if it did.
  bool deliver();

 private:
  // A request between its start and its completion.
  struct Live {
    std::string responseType;
    bool incremental = false;
    std::string partial;  // an incremental text body's trailing bytes of an unfinished character
    uint64_t progress = 0;
    int64_t total = -1;
  };

  void complete(int64_t id, const std::string& error, bool timedOut);

  BlobStore& blobs_;
  Emit emit_;
  OxyOSNet* net_;
  std::map<int64_t, Live> live_;
  std::deque<std::pair<int64_t, std::string>> rejected_;
};

}  // namespace oxyos
