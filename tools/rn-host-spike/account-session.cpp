#include "account-session.h"

#include <folly/json.h>

#include <array>
#include <cstring>
#include <memory>
#include <iostream>
#include <poll.h>
#include <sys/eventfd.h>
#include <unistd.h>

namespace oxyos {

void forget(std::string& secret) {
  if (!secret.empty()) explicit_bzero(secret.data(), secret.size());
  secret.clear();
}

namespace {

// A message's room: no answer of the service is longer.
constexpr size_t kMessage = OXYOS_ACCOUNT_MESSAGE_MAX;

// What a client-library call wrote into a buffer of a message's room, the buffer overwritten after.
AccountSession::Answer answered(int64_t length, std::string& buffer) {
  AccountSession::Answer answer;
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

AccountSession::AccountSession() : wake_(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK)) {
  worker_ = std::thread([this] { work(); });
}

AccountSession::~AccountSession() {
  {
    std::lock_guard lock(mutex_);
    stopping_ = true;
  }
  pending_.notify_one();
  worker_.join();
  oxyos_account_enrollment_free(enrollment_);
  oxyos_account_session_subscription_free(subscription_);
  if (wake_ >= 0) close(wake_);
}

folly::dynamic AccountSession::snapshot() {
  std::string buffer(kMessage, '\0');
  auto answer = answered(oxyos_account_session_snapshot(buffer.data(), buffer.size()), buffer);
  if (!answer.json) {
    std::cerr << "[host] the Oxy session: " << answer.error << "\n";
    return nullptr;
  }
  return folly::parseJson(*answer.json);
}

std::optional<std::string> AccountSession::platform() {
  std::string buffer(kMessage, '\0');
  auto answer = answered(oxyos_account_platform(buffer.data(), buffer.size()), buffer);
  if (!answer.json) std::cerr << "[host] the accounts' platform: " << answer.error << "\n";
  return answer.json;
}

bool AccountSession::subscribe(std::function<void(const folly::dynamic&)> changed) {
  if (subscription_) {
    changed_ = std::move(changed);
    return true;
  }
  subscription_ = oxyos_account_session_subscribe();
  if (!subscription_) {
    std::cerr << "[host] the Oxy session: " << oxyos_last_error() << "\n";
    return false;
  }
  changed_ = std::move(changed);
  return true;
}

int AccountSession::subscriptionFd() const {
  return subscription_ ? oxyos_account_session_subscription_fd(subscription_) : -1;
}

void AccountSession::run(std::function<Answer()> job, Done done) {
  {
    std::lock_guard lock(mutex_);
    jobs_.emplace_back(std::move(job), std::move(done));
  }
  pending_.notify_one();
}

void AccountSession::token(std::string reason, Done done) {
  run(
      [reason = std::move(reason)] {
        std::string buffer(kMessage, '\0');
        return answered(oxyos_account_session_token(reason.c_str(), buffer.data(), buffer.size()), buffer);
      },
      std::move(done));
}

void AccountSession::remove(Done done) {
  run(
      [] {
        Answer answer;
        if (oxyos_account_session_remove() == 0) {
          answer.json = "true";
        } else {
          answer.error = oxyos_last_error();
        }
        return answer;
      },
      std::move(done));
}

void AccountSession::holdCredential(std::string deviceId, std::string deviceSecret, std::string accountId) {
  if (held_) {
    for (auto& secret : *held_) forget(secret);
  }
  held_ = std::make_shared<std::array<std::string, 3>>(
      std::array<std::string, 3>{std::move(deviceId), std::move(deviceSecret), std::move(accountId)});
}

void AccountSession::enroll(std::string password, Done done) {
  auto credential = held_;
  auto secret = std::make_shared<std::string>(std::move(password));
  run(
      [this, credential, secret] {
        Answer answer;
        oxyos_account_enrollment_free(enrollment_);
        enrollment_ = nullptr;
        if (!credential) {
          answer.error = "no credential was handed over";
        } else if (!(enrollment_ = oxyos_account_enrollment_new())) {
          answer.error = oxyos_last_error();
        } else {
          std::string buffer(kMessage, '\0');
          auto& [deviceId, deviceSecret, accountId] = *credential;
          answer = answered(oxyos_account_enrollment_enroll(enrollment_, deviceId.c_str(), deviceSecret.c_str(),
                                                            accountId.c_str(), secret->c_str(), buffer.data(),
                                                            buffer.size()),
                            buffer);
        }
        forget(*secret);
        return answer;
      },
      std::move(done));
}

void AccountSession::secondFactor(std::string code, Done done) {
  auto secret = std::make_shared<std::string>(std::move(code));
  run(
      [this, secret] {
        Answer answer;
        if (!enrollment_) {
          answer.error = "no setup is under way";
        } else {
          std::string buffer(kMessage, '\0');
          answer = answered(
              oxyos_account_enrollment_second_factor(enrollment_, secret->c_str(), buffer.data(), buffer.size()),
              buffer);
        }
        forget(*secret);
        return answer;
      },
      std::move(done));
}

void AccountSession::endEnrollment() {
  if (held_) {
    for (auto& secret : *held_) forget(secret);
    held_.reset();
  }
  run(
      [this] {
        oxyos_account_enrollment_free(enrollment_);
        enrollment_ = nullptr;
        return Answer{};
      },
      nullptr);
}

void AccountSession::work() {
  while (true) {
    std::pair<std::function<Answer()>, Done> next;
    {
      std::unique_lock lock(mutex_);
      pending_.wait(lock, [this] { return stopping_ || !jobs_.empty(); });
      if (stopping_) return;
      next = std::move(jobs_.front());
      jobs_.pop_front();
    }
    Answer answer = next.first();
    if (!next.second) continue;
    {
      std::lock_guard lock(mutex_);
      answers_.emplace_back(std::move(answer), std::move(next.second));
    }
    uint64_t one = 1;
    if (write(wake_, &one, sizeof one) < 0) std::cerr << "[host] the Oxy session could not wake the host\n";
  }
}

bool AccountSession::deliver() {
  bool busy = false;
  std::deque<std::pair<Answer, Done>> answers;
  {
    std::lock_guard lock(mutex_);
    answers.swap(answers_);
  }
  if (!answers.empty()) {
    uint64_t count;
    while (read(wake_, &count, sizeof count) > 0) {
    }
    for (auto& [answer, done] : answers) {
      done(answer);
      if (answer.json) forget(*answer.json);
    }
    busy = true;
  }
  if (subscription_) {
    std::string buffer(kMessage, '\0');
    for (;;) {
      int64_t length = oxyos_account_session_subscription_next(subscription_, buffer.data(), buffer.size());
      if (length == 0) break;
      busy = true;
      if (length < 0 || static_cast<size_t>(length) >= buffer.size()) {
        std::cerr << "[host] the Oxy session's subscription ended: "
                  << (length < 0 ? oxyos_last_error() : "an answer longer than a message") << "\n";
        oxyos_account_session_subscription_free(subscription_);
        subscription_ = nullptr;
        if (changed_) changed_(nullptr);
        break;
      }
      if (changed_) changed_(folly::parseJson(std::string_view(buffer.data(), static_cast<size_t>(length))));
    }
  }
  return busy;
}

}  // namespace oxyos
