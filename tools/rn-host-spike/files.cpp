#include "files.h"

#include <folly/json.h>

#include <iostream>
#include <vector>

namespace oxyos {

Files::~Files() { oxyos_files_free(files_); }

void Files::request(const std::string& json) {
  if (!tried_) {
    tried_ = true;
    files_ = oxyos_files_new();
    if (!files_) {
      failure_ = std::string("the file service could not start: ") + oxyos_last_error();
      std::cerr << "[host] " << failure_ << "\n";
    }
  }
  if (!files_ || oxyos_files_request(files_, json.data(), json.size()) != 0) {
    refused_.push_back(json);
  }
}

bool Files::deliver() {
  bool delivered = false;
  while (!refused_.empty()) {
    folly::dynamic id = nullptr;
    try {
      auto request = folly::parseJson(refused_.front());
      if (request.isObject() && request.count("id")) id = request["id"];
    } catch (const std::exception&) {
    }
    refused_.pop_front();
    std::string message = failure_.empty() ? "the request is not UTF-8" : failure_;
    emit_("OxyOSFilesAnswer",
          folly::toJson(folly::dynamic::object("id", id)("ok", false)("error", "unavailable")("message", message)));
    delivered = true;
  }
  if (!files_) return delivered;
  std::vector<char> buffer(64 * 1024);
  for (;;) {
    int64_t length = oxyos_files_answer(files_, buffer.data(), buffer.size());
    if (length <= 0) break;
    if (static_cast<size_t>(length) >= buffer.size()) {
      buffer.resize(static_cast<size_t>(length) + 1);
      continue;
    }
    std::string answer(buffer.data(), static_cast<size_t>(length));
    // A change names the watch; an answer its request. Each reaches JavaScript as the service's
    // own JSON, parsed there.
    bool change = answer.rfind("{\"changed\":", 0) == 0;
    emit_(change ? "OxyOSFilesChanged" : "OxyOSFilesAnswer", answer);
    delivered = true;
  }
  return delivered;
}

}  // namespace oxyos
