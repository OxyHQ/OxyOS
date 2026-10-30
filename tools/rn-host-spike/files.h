// The file service an application's window is given (`OxyOSFiles`, docs/adr/0028-file-access.mdx):
// the user's home, the volumes mounted for them and the application's own folders, through
// crates/oxyos-files behind oxyos_client.h. JavaScript reaches it through expo-file-system's API
// (sample/oxyos/expo-file-system.js) and the Bloom adapter's places and Trash
// (sample/oxyos/platform-surface.js), never directly.
//
// A request is JSON with bytes beside it. One is answered at once on the JavaScript thread
// (`call`: expo-file-system's synchronous methods, as on Android), or queued and answered in order on
// the service's thread (`request`: its asynchronous ones), the answer handed back on a later turn
// (`deliver`); what watches see and transfers' progress reach JavaScript as `OxyOSFilesEvent`
// events carrying the service's JSON. The service starts with the first request, so an application
// that never asks costs nothing; one that cannot start answers every request with its reason:
// never silence.
#pragma once

#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <cstdint>
#include <deque>
#include <functional>
#include <map>
#include <memory>
#include <string>
#include <vector>

namespace oxyos {

class Files {
 public:
  // `emit(event, payload)` sends an event to RCTDeviceEventEmitter on the JavaScript thread.
  using Emit = std::function<void(const std::string& event, folly::dynamic payload)>;
  // An answer: the service's JSON, and its bytes (what was read).
  struct Reply {
    std::string json;
    std::shared_ptr<std::vector<uint8_t>> bytes;
  };
  using Done = std::function<void(Reply)>;

  // For the application `appId` (its launch's OXYOS_APP_ID; empty: none, no folders of its own),
  // run from the bundle in the folder `bundleDirectory`.
  Files(Emit emit, std::string appId, std::string bundleDirectory)
      : emit_(std::move(emit)), appId_(std::move(appId)), bundleDirectory_(std::move(bundleDirectory)) {}
  ~Files();
  Files(const Files&) = delete;
  Files& operator=(const Files&) = delete;

  // Answers `json` (with `bytes`) at once.
  Reply call(const std::string& json, const uint8_t* bytes, size_t length);
  // Queues `json` (with `bytes`) as `id` (not 0); `done` gets its answer on a later turn.
  void request(uint64_t id, const std::string& json, std::vector<uint8_t> bytes, Done done);
  // Opens a file the scope allows for reading (a multipart form's file part): its descriptor, the
  // caller's, and its size; -1 with the reason in `error`.
  int openRead(const std::string& uri, uint64_t& size, std::string& error);
  // Each turn: hands out what the service answered and emits what it saw. True if it did.
  bool deliver();
  // Readable while replies wait; -1 before the first request (poll ignores it).
  int fd() const { return files_ ? oxyos_files_fd(files_) : -1; }

 private:
  // Starts the service if it has not tried yet; whether it runs.
  bool started();
  // An answer saying the service cannot run, or that a request was not UTF-8.
  std::string failed(const std::string& message) const;

  Emit emit_;
  std::string appId_, bundleDirectory_;
  OxyOSFiles* files_ = nullptr;
  bool tried_ = false;
  std::string failure_;
  std::map<uint64_t, Done> pending_;
  // Requests answered on the next turn without the service: its failure, or text not UTF-8.
  std::deque<std::pair<uint64_t, std::string>> refused_;
};

}  // namespace oxyos
