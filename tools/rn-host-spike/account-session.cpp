#include "account-session.h"

#include <folly/json.h>

#include <algorithm>
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
  changed_ = std::move(changed);
  subscription_ = oxyos_account_session_subscribe();
  if (!subscription_) {
    // The service is not there: followed by itself once it is; the caller knows it is not now.
    if (!endedAt_) std::cerr << "[host] the Oxy session: " << oxyos_last_error() << "; following it once it is there\n";
    unreachable_ = true;
    ended();
    toldEnded_ = true;
    return false;
  }
  return true;
}

int AccountSession::subscriptionFd() const {
  return subscription_ ? oxyos_account_session_subscription_fd(subscription_) : -1;
}

namespace {
// How long the snapshot may not be followed before JavaScript is told (a service restarting is
// back well within it).
constexpr std::chrono::seconds kEndedGrace{3};
constexpr std::chrono::milliseconds kResubscribeFirst{500};
constexpr std::chrono::seconds kResubscribeMost{30};
}  // namespace

void AccountSession::ended() {
  auto now = std::chrono::steady_clock::now();
  if (!endedAt_) endedAt_ = now;
  auto wait = std::chrono::duration_cast<std::chrono::steady_clock::duration>(
      kResubscribeFirst * (1LL << std::min(resubscribeTries_, 16)));
  resubscribeAt_ = now + std::min<std::chrono::steady_clock::duration>(wait, kResubscribeMost);
  resubscribeTries_ += 1;
}

std::optional<std::chrono::steady_clock::time_point> AccountSession::nextDeadline() const {
  if (subscription_ || !changed_) return std::nullopt;
  std::optional<std::chrono::steady_clock::time_point> next = resubscribeAt_;
  if (endedAt_ && !toldEnded_ && (!next || *endedAt_ + kEndedGrace < *next)) next = *endedAt_ + kEndedGrace;
  return next;
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

void AccountSession::erasePreview(Done done) {
  run(
      [] {
        std::string buffer(kMessage, '\0');
        return answered(oxyos_account_session_erase_preview(buffer.data(), buffer.size()), buffer);
      },
      std::move(done));
}

void AccountSession::eraseAll(std::string password, Done done) {
  auto secret = std::make_shared<std::string>(std::move(password));
  run(
      [secret] {
        std::string buffer(kMessage, '\0');
        auto answer = answered(oxyos_account_session_erase_all(secret->c_str(), buffer.data(), buffer.size()), buffer);
        forget(*secret);
        return answer;
      },
      std::move(done));
}

void AccountSession::keychain(std::string request, Done done) {
  auto secret = std::make_shared<std::string>(std::move(request));
  run(
      [secret] {
        std::string buffer(kMessage, '\0');
        auto answer = answered(oxyos_account_session_keychain(secret->c_str(), buffer.data(), buffer.size()), buffer);
        forget(*secret);
        return answer;
      },
      std::move(done));
}

void AccountSession::savePassword(std::string username, std::string password, std::string domain, Done done) {
  auto secret = std::make_shared<std::array<std::string, 3>>(
      std::array<std::string, 3>{std::move(username), std::move(password), std::move(domain)});
  run(
      [secret] {
        std::string buffer(kMessage, '\0');
        auto& [username, password, domain] = *secret;
        auto answer = answered(oxyos_account_session_save_password(username.c_str(), password.c_str(),
                                                                    domain.empty() ? nullptr : domain.c_str(),
                                                                    buffer.data(), buffer.size()),
                               buffer);
        for (auto& text : *secret) forget(text);
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

void AccountSession::localSignIn(Done done) {
  run(
      [] {
        std::string buffer(kMessage, '\0');
        return answered(oxyos_account_session_local_sign_in(buffer.data(), buffer.size()), buffer);
      },
      std::move(done));
}

void AccountSession::changeLocal(std::string change, std::string pin, std::string password, std::string currentPin,
                                 Done done) {
  auto secrets = std::make_shared<std::array<std::string, 3>>(
      std::array<std::string, 3>{std::move(pin), std::move(password), std::move(currentPin)});
  run(
      [change = std::move(change), secrets] {
        std::string buffer(kMessage, '\0');
        auto& [pin, password, currentPin] = *secrets;
        auto orNull = [](const std::string& text) { return text.empty() ? nullptr : text.c_str(); };
        auto answer = answered(oxyos_account_session_change_local(change.c_str(), orNull(pin), orNull(password),
                                                                  orNull(currentPin), buffer.data(), buffer.size()),
                               buffer);
        for (auto& secret : *secrets) forget(secret);
        return answer;
      },
      std::move(done));
}

void AccountSession::enroll(std::string password, std::string pin, std::string appearance, Stage stage, Done done) {
  auto credential = held_;
  auto secret = std::make_shared<std::string>(std::move(password));
  auto passcode = std::make_shared<std::string>(std::move(pin));
  run(
      [this, credential, secret, passcode, appearance = std::move(appearance), stage = std::move(stage)] {
        Answer answer;
        oxyos_account_enrollment_free(enrollment_);
        enrollment_ = nullptr;
        if (!credential) {
          answer.error = "no credential was handed over";
        } else if (!(enrollment_ = oxyos_account_enrollment_new())) {
          answer.error = oxyos_last_error();
        } else if (!appearance.empty() &&
                   oxyos_account_enrollment_set_appearance(enrollment_, appearance.c_str()) != 0) {
          answer.error = oxyos_last_error();
        } else {
          reportStages(stage);
          std::string buffer(kMessage, '\0');
          auto& [deviceId, deviceSecret, accountId] = *credential;
          answer = answered(oxyos_account_enrollment_enroll(
                                enrollment_, deviceId.c_str(), deviceSecret.c_str(), accountId.c_str(),
                                secret->empty() ? nullptr : secret->c_str(),
                                passcode->empty() ? nullptr : passcode->c_str(), buffer.data(), buffer.size()),
                            buffer);
        }
        forget(*secret);
        forget(*passcode);
        return answer;
      },
      std::move(done));
}

void AccountSession::secondFactor(std::string code, Stage stage, Done done) {
  auto secret = std::make_shared<std::string>(std::move(code));
  run(
      [this, secret, stage = std::move(stage)] {
        Answer answer;
        if (!enrollment_) {
          answer.error = "no setup is under way";
        } else {
          reportStages(stage);
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

void AccountSession::retry(Stage stage, Done done) {
  run(
      [this, stage = std::move(stage)] {
        Answer answer;
        if (!enrollment_) {
          answer.error = "no setup is under way";
        } else {
          reportStages(stage);
          std::string buffer(kMessage, '\0');
          answer = answered(oxyos_account_enrollment_retry(enrollment_, buffer.data(), buffer.size()), buffer);
        }
        return answer;
      },
      std::move(done));
}

namespace {
// What a setup's stages are told to: the session and the JavaScript function, for the call under
// way (the worker's alone, replaced by each call).
struct StageReport {
  AccountSession* session;
  AccountSession::Stage stage;
};
thread_local std::unique_ptr<StageReport> currentReport;
}  // namespace

void AccountSession::reportStages(Stage stage) {
  currentReport = stage ? std::make_unique<StageReport>(StageReport{this, std::move(stage)}) : nullptr;
  oxyos_account_enrollment_on_progress(enrollment_, currentReport ? &AccountSession::reported : nullptr,
                                       currentReport.get());
}

void AccountSession::reported(void* context, const char* stage) {
  auto* report = static_cast<StageReport*>(context);
  if (!report || !stage) return;
  std::string name(stage);
  auto told = report->stage;
  report->session->post(Answer{}, [told, name](Answer&) { told(name); });
}

void AccountSession::post(Answer answer, Done done) {
  {
    std::lock_guard lock(mutex_);
    answers_.emplace_back(std::move(answer), std::move(done));
  }
  uint64_t one = 1;
  if (write(wake_, &one, sizeof one) < 0) std::cerr << "[host] the Oxy session could not wake the host\n";
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
    post(std::move(answer), std::move(next.second));
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
  // Followed again once it is time: a refusal (the session has not registered this process again
  // yet) ends it at once, and it is tried again later.
  auto now = std::chrono::steady_clock::now();
  if (!subscription_ && changed_ && resubscribeAt_ && now >= *resubscribeAt_) {
    resubscribeAt_.reset();
    subscription_ = oxyos_account_session_subscribe();
    if (!subscription_) {
      unreachable_ = true;
      ended();
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
        oxyos_account_session_subscription_free(subscription_);
        subscription_ = nullptr;
        // Followed before, or the service was away: it restarted, and refuses this process only
        // until the session registers it again. Followed again; said once per outage.
        if (followed_ || unreachable_) {
          if (!endedAt_) {
            std::cerr << "[host] the Oxy session's subscription ended: "
                      << (length < 0 ? oxyos_last_error() : "an answer longer than a message")
                      << "; following it again\n";
          }
          ended();
          break;
        }
        // Refused from the start: the service does not know this process, and that does not change.
        std::cerr << "[host] the Oxy session's subscription ended: "
                  << (length < 0 ? oxyos_last_error() : "an answer longer than a message") << "\n";
        toldEnded_ = true;
        if (changed_) changed_(nullptr);
        break;
      }
      if (endedAt_ && toldEnded_) std::cerr << "[host] the Oxy session is followed again\n";
      followed_ = true;
      unreachable_ = false;
      endedAt_.reset();
      toldEnded_ = false;
      resubscribeTries_ = 0;
      resubscribeAt_.reset();
      if (changed_) changed_(folly::parseJson(std::string_view(buffer.data(), static_cast<size_t>(length))));
    }
  }
  // Not followed for longer than a restart takes: JavaScript is told, once.
  if (!subscription_ && endedAt_ && !toldEnded_ && now >= *endedAt_ + kEndedGrace) {
    toldEnded_ = true;
    busy = true;
    if (changed_) changed_(nullptr);
  }
  return busy;
}

}  // namespace oxyos
