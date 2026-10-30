#include "application.h"

#include <folly/json.h>

#include <poll.h>
#include <sys/eventfd.h>
#include <unistd.h>

#include <cstdlib>
#include <filesystem>
#include <string_view>
#include <fstream>
#include <iostream>
#include <sstream>

namespace oxyos {

namespace {

// What a C ABI call writing JSON as oxyos_applications does gives, with room enough.
template <typename Call>
std::optional<std::string> written(Call call, size_t capacity = 4096) {
  std::string out(capacity, '\0');
  int64_t length = call(out.data(), out.size());
  if (length < 0) return std::nullopt;
  if (static_cast<size_t>(length) >= out.size()) {
    out.assign(static_cast<size_t>(length) + 1, '\0');
    length = call(out.data(), out.size());
    if (length < 0 || static_cast<size_t>(length) >= out.size()) return std::nullopt;
  }
  out.resize(static_cast<size_t>(length));
  return out;
}

// An application's directory (oxyos_application_directory's `which`), or "" with the reason said.
std::string directory(const std::string& id, uint8_t which) {
  auto path = written([&](char* out, size_t capacity) {
    return oxyos_application_directory(id.c_str(), which, out, capacity);
  });
  if (!path) std::cerr << "[host] " << id << " has no directory: " << oxyos_last_error() << "\n";
  return path.value_or("");
}

}  // namespace

Application Application::launched(const std::string& bundlePath, const std::string& module) {
  Application application;
  const char* id = std::getenv("OXYOS_APP_ID");
  application.id = id ? id : "";
  application.recorded = !application.id.empty() && !std::getenv("OXYOS_APP_UNRECORDED");
  application.module = module;
  if (application.recorded) {
    application.dataDirectory = directory(application.id, 0);
    application.cacheDirectory = directory(application.id, 1);
  } else if (!application.id.empty()) {
    std::cerr << "[host] " << application.id
              << " was not started by the session: its id is only a claim, so it has no data directory\n";
  }
  // Its embedded app config is next to its source bundle, which its script URL names: a package's
  // compiled index.hbc is beside its bundle/ (docs/adr/0030-installing-applications.mdx).
  std::filesystem::path base = std::filesystem::path(bundlePath).parent_path();
  if (const char* url = std::getenv("OXYOS_SCRIPT_URL"); url && std::string_view(url).starts_with("file://")) {
    base = std::filesystem::path(url + 7).parent_path();
  }
  auto config = base / "applications" / module / "app.config";
  if (std::ifstream in{config}) {
    std::stringstream text;
    text << in.rdbuf();
    application.manifest = text.str();
  }
  return application;
}

AsyncStorage::AsyncStorage(std::string directory)
    : directory_(std::move(directory)), wake_(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK)) {
  worker_ = std::thread([this] { work(); });
}

AsyncStorage::~AsyncStorage() {
  {
    std::lock_guard lock(mutex_);
    stopping_ = true;
  }
  pending_.notify_one();
  worker_.join();
  if (wake_ >= 0) close(wake_);
}

void AsyncStorage::request(folly::dynamic request, Done done) {
  {
    std::lock_guard lock(mutex_);
    requests_.emplace_back(folly::toJson(request), std::move(done));
  }
  pending_.notify_one();
}

void AsyncStorage::work() {
  while (true) {
    std::pair<std::string, Done> next;
    {
      std::unique_lock lock(mutex_);
      pending_.wait(lock, [this] { return stopping_ || !requests_.empty(); });
      // What was asked before the host stops is still done: a write is never dropped.
      if (requests_.empty()) return;
      next = std::move(requests_.front());
      requests_.pop_front();
    }
    auto answer = written([&](char* out, size_t capacity) {
      return oxyos_app_storage(directory_.c_str(), next.first.c_str(), out, capacity);
    }, 64 * 1024);
    {
      std::lock_guard lock(mutex_);
      answers_.emplace_back(answer.value_or(folly::toJson(folly::dynamic::object(
                                "error", std::string("the application's storage failed: ") + oxyos_last_error()))),
                            std::move(next.second));
    }
    uint64_t one = 1;
    if (write(wake_, &one, sizeof one) < 0) std::cerr << "[host] AsyncStorage could not wake the host\n";
  }
}

bool AsyncStorage::deliver() {
  std::deque<std::pair<std::string, Done>> answers;
  {
    std::lock_guard lock(mutex_);
    answers.swap(answers_);
  }
  if (answers.empty()) return false;
  uint64_t count;
  while (read(wake_, &count, sizeof count) > 0) {
  }
  for (auto& [answer, done] : answers) done(folly::parseJson(answer));
  return true;
}

Connectivity::Connectivity(std::function<void(const folly::dynamic&)> changed)
    : connectivity_(oxyos_connectivity_new()), changed_(std::move(changed)) {}

Connectivity::~Connectivity() { oxyos_connectivity_free(connectivity_); }

int Connectivity::fd() const { return oxyos_connectivity_fd(connectivity_); }

folly::dynamic Connectivity::state() const {
  auto json = written([&](char* out, size_t capacity) { return oxyos_connectivity_state(connectivity_, out, capacity); });
  folly::dynamic kernel = folly::parseJson(json.value_or(R"({"type":"unknown","isConnected":false})"));
  bool connected = kernel["isConnected"].asBool();
  // Android's details for a connection; whether it is metered is not known on OxyOS yet.
  folly::dynamic details = connected ? folly::dynamic::object("isConnectionExpensive", false) : folly::dynamic(nullptr);
  return folly::dynamic::object("type", kernel["type"])("isConnected", connected)("details", details);
}

bool Connectivity::deliver() {
  int watched = fd();
  if (watched < 0) return false;
  pollfd readable{watched, POLLIN, 0};
  if (poll(&readable, 1, 0) <= 0) return false;
  if (oxyos_connectivity_refresh(connectivity_) == 1) changed_(state());
  return true;
}

}  // namespace oxyos
