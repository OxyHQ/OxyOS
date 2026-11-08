// The machine's Oxy session for the Bloom host (docs/adr/0020-oxy-account-login.mdx), over the
// client library's `oxyos_account_*` (crates/oxyos-client-ffi, account_session.rs):
//
// - **An application's** (decision 6): the session's snapshot, followed on one connection the
//   service keeps (its descriptor wakes the host's loop, as the network's does), and short tokens
//   for the application, minted online, so asked on a thread of its own and answered on the
//   JavaScript thread. Settings removes its own account the same way, and erases the computer
//   (docs/adr/0054-recovery-and-erasing.mdx): what erasing would do, then the erase with the
//   administrator's password, forgotten here once sent.
// - **The login screen's setup** (decision 4): the platform the SDK's sign-in runs with, and the
//   `enroll` conversation, a second factor answered on the same connection.
//
// Secrets (a token, a password, a device secret, a code) pass through here only on their way:
// every buffer that held one is overwritten once it is handed on, and nothing of them is logged.
#pragma once

#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <array>
#include <condition_variable>
#include <memory>
#include <deque>
#include <functional>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace oxyos {

// Overwrites a string that held a secret, so its bytes are not left in freed memory.
void forget(std::string& secret);

class AccountSession {
 public:
  // A call's answer: its JSON (a token's is secret: the receiver forgets it), or why it failed.
  struct Answer {
    std::optional<std::string> json;
    std::string error;
  };
  using Done = std::function<void(Answer& answer)>;
  // A setup's stage as the service starts it ("securing", "verifying", ...), on the JavaScript
  // thread, before the call's answer.
  using Stage = std::function<void(const std::string& stage)>;

  AccountSession();
  ~AccountSession();
  AccountSession(const AccountSession&) = delete;
  AccountSession& operator=(const AccountSession&) = delete;

  // The snapshot, asked once, at once (the service answers from memory): null when it cannot be
  // asked or refused.
  static folly::dynamic snapshot();
  // Follows the snapshot: `changed` on the JavaScript thread with each one, and once with null
  // when the subscription ends (the service went away or refused). False if it could not start.
  bool subscribe(std::function<void(const folly::dynamic& snapshot)> changed);

  // On the service's thread, answered on the JavaScript thread (`deliver`), in order.
  void token(std::string reason, Done done);
  void remove(Done done);
  void erasePreview(Done done);
  void eraseAll(std::string password, Done done);
  // The account's local sign-in (its PIN, whether its password signs in): Settings and the shell's
  // lock screen read it; Settings changes it ("setPin", "removePin", "passwordOn",
  // "passwordOff"), proved by the password or the current PIN (an empty one is none).
  void localSignIn(Done done);
  void changeLocal(std::string change, std::string pin, std::string password, std::string currentPin, Done done);
  // The login screen's setup. The device credential the SDK's sign-in handed over is held here,
  // not in JavaScript, until the person gives the account's password once more and chooses a PIN
  // for this computer (a credential held before replaces it); `enroll` sends them (an empty
  // password or PIN is none), and every secret is forgotten once sent.
  void holdCredential(std::string deviceId, std::string deviceSecret, std::string accountId);
  bool holdsCredential() const { return held_ != nullptr; }
  // `appearance` ("light", "dark" or "auto"; empty: none) is the new account's first.
  void enroll(std::string password, std::string pin, std::string appearance, Stage stage, Done done);
  void secondFactor(std::string code, Stage stage, Done done);
  // The setup again after it failed with the sign-in kept (`failed`): what the service kept.
  void retry(Stage stage, Done done);
  void endEnrollment();
  static std::optional<std::string> platform();

  // Each turn: hands answers and snapshots to JavaScript. True if it did.
  bool deliver();
  // Readable while answers wait; the subscription's (-1 without one).
  int fd() const { return wake_; }
  int subscriptionFd() const;

 private:
  void work();
  void run(std::function<Answer()> job, Done done);
  // The worker's: `stage` told to JavaScript, as an answer is, while the call it reports blocks.
  void reportStages(Stage stage);
  static void reported(void* context, const char* stage);
  void post(Answer answer, Done done);

  int wake_ = -1;
  std::mutex mutex_;
  std::condition_variable pending_;
  std::deque<std::pair<std::function<Answer()>, Done>> jobs_;
  std::deque<std::pair<Answer, Done>> answers_;
  bool stopping_ = false;
  std::thread worker_;
  // The setup under way: touched only by the worker.
  OxyOSAccountEnrollment* enrollment_ = nullptr;
  // The snapshot followed: touched only on the JavaScript thread.
  OxyOSAccountSubscription* subscription_ = nullptr;
  // The credential held for the setup (device id, secret, account id): JavaScript thread only.
  std::shared_ptr<std::array<std::string, 3>> held_;
  std::function<void(const folly::dynamic&)> changed_;
};

}  // namespace oxyos
