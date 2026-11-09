#include "migration.h"

#include "account-session.h"

#include <folly/json.h>

#include <cstring>
#include <iostream>
#include <memory>
#include <sys/eventfd.h>
#include <unistd.h>

namespace oxyos {

namespace {

// What a client-library call wrote into a buffer of a message's room, the buffer overwritten after.
Migration::Answer answered(int64_t length, std::string& buffer) {
  Migration::Answer answer;
  if (length < 0) {
    answer.error = oxyos_last_error();
  } else if (static_cast<size_t>(length) >= buffer.size()) {
    answer.error = "an answer longer than a message";
  } else {
    answer.json = std::string(buffer.data(), static_cast<size_t>(length));
  }
  explicit_bzero(buffer.data(), buffer.size());
  return answer;
}

}  // namespace

Migration::Migration() : wake_(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK)) {
  worker_ = std::thread([this] { work(); });
}

Migration::~Migration() {
  {
    std::lock_guard lock(mutex_);
    stopping_ = true;
  }
  pending_.notify_one();
  worker_.join();
  oxyos_migration_free(migration_);
  if (wake_ >= 0) close(wake_);
}

void Migration::run(std::function<void()> job) {
  {
    std::lock_guard lock(mutex_);
    jobs_.push_back(std::move(job));
  }
  pending_.notify_one();
}

void Migration::post(Answer answer, Done done) {
  if (!done) return;
  {
    std::lock_guard lock(mutex_);
    answers_.emplace_back(std::move(answer), std::move(done));
  }
  uint64_t one = 1;
  if (write(wake_, &one, sizeof one) < 0) std::cerr << "[host] the migration could not wake the host\n";
}

bool Migration::open(Done& done) {
  if (migration_) return true;
  migration_ = oxyos_migration_new();
  if (migration_) return true;
  post(Answer{std::nullopt, oxyos_last_error()}, std::move(done));
  return false;
}

void Migration::disks(Done done) {
  run([this, done = std::move(done)]() mutable {
    if (!open(done)) return;
    std::string buffer(OXYOS_MIGRATION_MESSAGE_MAX, '\0');
    auto answer = answered(oxyos_migration_disks(migration_, buffer.data(), buffer.size()), buffer);
    if (!answer.json) {
      oxyos_migration_free(migration_);
      migration_ = nullptr;
    }
    post(std::move(answer), std::move(done));
  });
}

void Migration::backups(std::string source, std::string password, std::optional<std::string> sharePassword,
                        Done done) {
  auto secrets = std::make_shared<std::pair<std::string, std::optional<std::string>>>(std::move(password),
                                                                                        std::move(sharePassword));
  run([this, source = std::move(source), secrets, done = std::move(done)]() mutable {
    if (open(done)) {
      std::string buffer(OXYOS_MIGRATION_MESSAGE_MAX, '\0');
      auto answer = answered(oxyos_migration_backups(migration_, source.c_str(), secrets->first.c_str(),
                                                     secrets->second ? secrets->second->c_str() : nullptr,
                                                     buffer.data(), buffer.size()),
                             buffer);
      if (!answer.json) {
        oxyos_migration_free(migration_);
        migration_ = nullptr;
      }
      post(std::move(answer), std::move(done));
    }
    forget(secrets->first);
    if (secrets->second) forget(*secrets->second);
  });
}

void Migration::migrate(std::string repository, std::string snapshot, Done progress, Done done) {
  run([this, repository = std::move(repository), snapshot = std::move(snapshot), progress = std::move(progress),
       done = std::move(done)]() mutable {
    // What JavaScript gave is let go on its own thread: the progress callback goes there with the
    // answer, never dropped here.
    auto answer = [this, &progress, &done](Answer told) {
      post(std::move(told), std::move(done));
      if (progress) post(Answer{}, [held = std::move(progress)](Answer&) {});
    };
    if (!migration_) {
      answer(Answer{std::nullopt, "no backups were opened"});
      return;
    }
    if (oxyos_migration_migrate(migration_, repository.c_str(), snapshot.c_str()) != 0) {
      answer(Answer{std::nullopt, oxyos_last_error()});
      return;
    }
    std::string buffer(OXYOS_MIGRATION_MESSAGE_MAX, '\0');
    for (;;) {
      auto told = answered(oxyos_migration_next(migration_, buffer.data(), buffer.size()), buffer);
      if (told.json && folly::parseJson(*told.json).count("migrating") > 0) {
        post(std::move(told), progress);
        continue;
      }
      if (!told.json) {
        oxyos_migration_free(migration_);
        migration_ = nullptr;
      }
      answer(std::move(told));
      return;
    }
  });
}

void Migration::end() {
  run([this] {
    oxyos_migration_free(migration_);
    migration_ = nullptr;
  });
}

void Migration::work() {
  while (true) {
    std::function<void()> next;
    {
      std::unique_lock lock(mutex_);
      pending_.wait(lock, [this] { return stopping_ || !jobs_.empty(); });
      if (stopping_) return;
      next = std::move(jobs_.front());
      jobs_.pop_front();
    }
    next();
  }
}

bool Migration::deliver() {
  std::deque<std::pair<Answer, Done>> answers;
  {
    std::lock_guard lock(mutex_);
    answers.swap(answers_);
  }
  if (answers.empty()) return false;
  uint64_t count;
  while (read(wake_, &count, sizeof count) > 0) {
  }
  for (auto& [answer, done] : answers) done(answer);
  return true;
}

}  // namespace oxyos
