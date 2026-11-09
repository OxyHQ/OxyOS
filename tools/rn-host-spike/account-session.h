// The machine's Oxy session for the Bloom host (docs/adr/0020-oxy-account-login.mdx), over the
// client library's `oxyos_account_*` (crates/oxyos-client-ffi, account_session.rs):
//
// - **An application's** (decision 6): the session's snapshot, followed on one connection the
//   service keeps from the host's start (its descriptor wakes the host's loop, as the network's
//   does), the last one held here, so JavaScript reads it without asking the service
//   (docs/adr/0058-the-platform-api.mdx, decision 8), and short tokens
//   for the application, minted online, so asked on a thread of its own and answered on the
//   JavaScript thread. Settings removes its own account the same way, and erases the computer
//   (docs/adr/0054-recovery-and-erasing.mdx): what erasing would do, then the erase with the
//   administrator's password, forgotten here once sent.
// - **The login screen's setup** (decision 4): the platform the SDK's sign-in runs with, and the
//   `enroll` conversation, a second factor answered on the same connection.
// - **The keychain's "ask to save"** (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 5):
//   an application asks to save a login it has just used; the service asks the person in the
//   session's sheet and saves it only on their yes.
// - **Passkeys** (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): an application's
//   WebAuthn ceremonies, in WebAuthn's JSON shapes, which wait for the person on the system's
//   sheet: each on a thread of its own, so a token is never kept waiting behind one, and the host
//   never waits for one to end (the thread holds only its own outbox, never the answer's callback).
//
// Secrets (a token, a password, a device secret, a code) pass through here only on their way:
// every buffer that held one is overwritten once it is handed on, and nothing of them is logged.
#pragma once

#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <array>
#include <chrono>
#include <condition_variable>
#include <memory>
#include <deque>
#include <map>
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
  // Oxy asked the service to wait `seconds` before asking again (a 429 on the redemption, oxy ADR
  // 0058 R8), on the JavaScript thread; the call it interrupts goes on by itself after it.
  using Waiting = std::function<void(uint64_t seconds)>;

  AccountSession();
  ~AccountSession();
  AccountSession(const AccountSession&) = delete;
  AccountSession& operator=(const AccountSession&) = delete;

  // Follows the snapshot on one connection the service keeps, which tells it at once and at each
  // change: started with the host, before JavaScript runs, so its first is held by the time it is
  // read, and followed again by itself as `subscribe` says. False if it could not start now.
  bool follow();
  // The last snapshot the service told, null before the first and once JavaScript was told the
  // subscription ended: what arrived since the last turn read without waiting, never a question
  // to the service on the JavaScript thread.
  folly::dynamic current();
  // Follows the snapshot: `changed` on the JavaScript thread with each one. When a subscription
  // that was followed ends (the accounts' service restarted, then refuses this process until the
  // session registers it again), or the service is not there, it is followed again by itself, after
  // 0.5 s, then waits doubling to 30 s, for as long as the application runs; `changed` is told null
  // once only if that has lasted more than a few seconds, so a service restarting under it is never
  // seen. Refused from the start (a process the service does not know), `changed` is told null at
  // once, and nothing is tried again by itself. False if it could not start.
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
  // One of Settings' keychain requests, its JSON (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
  // decision 8): it may hold a password or an item, and the answer an item, a recovery key or a
  // code, so both are forgotten once handed on.
  void keychain(std::string request, Done done);
  // The password is forgotten here once sent; `domain` may be empty (none).
  void savePassword(std::string username, std::string password, std::string domain, Done done);
  // A passkey ceremony ("create" or "get") with WebAuthn's options JSON: {"credential"} or
  // {"error": {"name", "reason"}}.
  void passkey(std::string ceremony, std::string options, Done done);
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
  // Told of every setup call's waits from now on (set once, when the module is made).
  void onWaiting(Waiting waiting) { waiting_ = std::move(waiting); }
  static std::optional<std::string> platform();

  // Each turn: hands answers and snapshots to JavaScript. True if it did.
  bool deliver();
  // Readable while answers wait; the subscription's (-1 without one).
  int fd() const { return wake_; }
  int subscriptionFd() const;
  // When the loop must turn again to follow the snapshot again, or to say it ended: none while it
  // is followed.
  std::optional<std::chrono::steady_clock::time_point> nextDeadline() const;

 private:
  void work();
  void run(std::function<Answer()> job, Done done);
  // The worker's: `stage` told to JavaScript, as an answer is, while the call it reports blocks.
  void reportStages(Stage stage);
  static void reported(void* context, const char* stage);
  static void waited(void* context, uint64_t seconds);
  void post(Answer answer, Done done);
  // Reads what the subscription told since, without waiting, following it again once it is time:
  // the last held, and told. True if anything arrived, it was followed again or it ended.
  bool drain();

  int wake_ = -1;
  Waiting waiting_;
  std::mutex mutex_;
  std::condition_variable pending_;
  std::deque<std::pair<std::function<Answer()>, Done>> jobs_;
  std::deque<std::pair<Answer, Done>> answers_;
  bool stopping_ = false;
  std::thread worker_;
  // Passkeys' answers, from their threads: shared with them, so a thread still waiting for the
  // person when the host ends writes into this and not into a session gone. Their callbacks stay
  // here, on the JavaScript thread, by ticket.
  struct PasskeyOutbox {
    std::mutex mutex;
    std::deque<std::pair<uint64_t, Answer>> answers;
    int wake = -1;
    ~PasskeyOutbox();
  };
  std::shared_ptr<PasskeyOutbox> passkeys_;
  std::map<uint64_t, Done> passkeyDone_;
  uint64_t passkeyTickets_ = 0;
  // The setup under way: touched only by the worker.
  OxyOSAccountEnrollment* enrollment_ = nullptr;
  // The snapshot followed: touched only on the JavaScript thread.
  OxyOSAccountSubscription* subscription_ = nullptr;
  // Whether the host follows it (`follow`), and the last snapshot told: JavaScript thread only.
  bool following_ = false;
  folly::dynamic latest_ = nullptr;
  // The credential held for the setup (device id, secret, account id): JavaScript thread only.
  std::shared_ptr<std::array<std::string, 3>> held_;
  std::function<void(const folly::dynamic&)> changed_;
  // Following again after the subscription ended (JavaScript thread only): when next, how many
  // tries since it was last followed, since when it is not, and whether null was told. Only a
  // subscription that was followed once, or tried while the service was away, is followed again:
  // one refused from the start is a process the service does not know.
  void ended();
  bool followed_ = false;
  bool unreachable_ = false;
  std::optional<std::chrono::steady_clock::time_point> resubscribeAt_;
  int resubscribeTries_ = 0;
  std::optional<std::chrono::steady_clock::time_point> endedAt_;
  bool toldEnded_ = false;
};

}  // namespace oxyos
