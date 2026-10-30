#include "files.h"

#include <folly/json.h>

#include <iostream>

namespace oxyos {

namespace {

Files::Reply reply(OxyOSFilesReply* reply) {
  size_t length = 0;
  const char* json = oxyos_files_reply_json(reply, &length);
  size_t count = 0;
  const uint8_t* bytes = oxyos_files_reply_bytes(reply, &count);
  Files::Reply out{std::string(json, length), std::make_shared<std::vector<uint8_t>>(bytes, bytes + count)};
  oxyos_files_reply_free(reply);
  return out;
}

}  // namespace

Files::~Files() { oxyos_files_free(files_); }

bool Files::started() {
  if (!tried_) {
    tried_ = true;
    files_ = oxyos_files_new(appId_.empty() ? nullptr : appId_.data(), appId_.size(),
                             bundleDirectory_.empty() ? nullptr : bundleDirectory_.data(), bundleDirectory_.size());
    if (!files_) {
      failure_ = std::string("the file service could not start: ") + oxyos_last_error();
      std::cerr << "[host] " << failure_ << "\n";
    }
  }
  return files_ != nullptr;
}

std::string Files::failed(const std::string& message) const {
  return folly::toJson(folly::dynamic::object("ok", false)("error", "unavailable")("message", message));
}

Files::Reply Files::call(const std::string& json, const uint8_t* bytes, size_t length) {
  if (!started()) return {failed(failure_), std::make_shared<std::vector<uint8_t>>()};
  OxyOSFilesReply* answered = oxyos_files_call(files_, json.data(), json.size(), bytes, length);
  if (!answered) return {failed(oxyos_last_error()), std::make_shared<std::vector<uint8_t>>()};
  return reply(answered);
}

void Files::request(uint64_t id, const std::string& json, std::vector<uint8_t> bytes, Done done) {
  pending_[id] = std::move(done);
  if (!started()) {
    refused_.emplace_back(id, failure_);
  } else if (oxyos_files_request(files_, id, json.data(), json.size(), bytes.data(), bytes.size()) != 0) {
    refused_.emplace_back(id, oxyos_last_error());
  }
}

int Files::openRead(const std::string& uri, uint64_t& size, std::string& error) {
  if (!started()) {
    error = failure_;
    return -1;
  }
  int fd = oxyos_files_open_read(files_, uri.data(), uri.size(), &size);
  if (fd < 0) error = oxyos_last_error();
  return fd;
}

bool Files::deliver() {
  bool delivered = false;
  auto answer = [this](uint64_t id, Reply reply) {
    auto done = pending_.find(id);
    if (done == pending_.end()) return;
    auto handler = std::move(done->second);
    pending_.erase(done);
    handler(std::move(reply));
  };
  while (!refused_.empty()) {
    auto [id, message] = std::move(refused_.front());
    refused_.pop_front();
    answer(id, {failed(message), std::make_shared<std::vector<uint8_t>>()});
    delivered = true;
  }
  if (!files_) return delivered;
  while (OxyOSFilesReply* taken = oxyos_files_take(files_)) {
    uint64_t id = oxyos_files_reply_id(taken);
    Reply given = reply(taken);
    // An event (a watch's change, a transfer's progress) reaches JavaScript as the service's own
    // JSON, parsed there; an answer, its request's promise.
    if (id == 0) {
      emit_("OxyOSFilesEvent", given.json);
    } else {
      answer(id, std::move(given));
    }
    delivered = true;
  }
  return delivered;
}

}  // namespace oxyos
