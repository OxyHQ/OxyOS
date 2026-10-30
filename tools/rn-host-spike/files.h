// The file service an application is given (`OxyOSFiles`, docs/adr/0027-files.mdx): the user's
// home and the volumes mounted for them, through crates/oxyos-files behind oxyos_client.h.
//
// JavaScript sends requests as JSON text (`request`); the service runs them in order on a thread
// of its own and this side hands each answer, and each watched change, to JavaScript on its own
// thread (`deliver`), as `OxyOSFilesAnswer` and `OxyOSFilesChanged` events carrying the service's
// JSON. The service starts with the first request, so an application that never asks costs
// nothing. A service that cannot start answers every request with its reason: never silence.
#pragma once

#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <deque>
#include <functional>
#include <string>

namespace oxyos {

class Files {
 public:
  // `emit(event, payload)` sends an event to RCTDeviceEventEmitter on the JavaScript thread.
  using Emit = std::function<void(const std::string& event, folly::dynamic payload)>;

  explicit Files(Emit emit) : emit_(std::move(emit)) {}
  ~Files();
  Files(const Files&) = delete;
  Files& operator=(const Files&) = delete;

  // Queues a request (`{"id": N, "op": ...}`); it is answered on a later turn.
  void request(const std::string& json);
  // Each turn: emits what the service answered. True if it did.
  bool deliver();
  // Readable while answers wait to be emitted; -1 before the first request (poll ignores it).
  int fd() const { return files_ ? oxyos_files_fd(files_) : -1; }

 private:
  Emit emit_;
  OxyOSFiles* files_ = nullptr;
  bool tried_ = false;
  std::string failure_;
  // Requests a service that could not start answers, on the next turn.
  std::deque<std::string> refused_;
};

}  // namespace oxyos
