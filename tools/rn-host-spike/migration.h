// Migrate from a Backup for the Bloom host run as the login screen (docs/adr/0059-backups.mdx,
// decision 7), over the client library's `oxyos_migration_*` (crates/oxyos-client-ffi,
// migration.rs): one conversation with the migration service, on a thread of its own (every call
// waits on the service: a disk mounted, restic opening each computer's backups, a restore), its
// answers and a migration's progress handed to the JavaScript thread as they arrive.
//
// The backup password and a share's password pass through here only on their way: each buffer
// that held one is overwritten once it is sent, and nothing of them is logged.
#pragma once

#include "oxyos_client.h"

#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace oxyos {

class Migration {
 public:
  // An answer's JSON, or why the conversation could not go on.
  struct Answer {
    std::optional<std::string> json;
    std::string error;
  };
  using Done = std::function<void(Answer& answer)>;

  Migration();
  ~Migration();
  Migration(const Migration&) = delete;
  Migration& operator=(const Migration&) = delete;

  // Each answered on the JavaScript thread (`deliver`), in order. The conversation is opened by
  // the first, and again by the next after it ended.
  void disks(Done done);
  void backups(std::string source, std::string password, std::optional<std::string> sharePassword, Done done);
  // `progress` with each {"migrating"} message, then `done` with the answer.
  void migrate(std::string repository, std::string snapshot, Done progress, Done done);
  // Ends the conversation (the service unmounts a disk it mounted for it).
  void end();

  // Each turn: hands answers to JavaScript. True if it did.
  bool deliver();
  // Readable while answers wait.
  int fd() const { return wake_; }

 private:
  void run(std::function<void()> job);
  void post(Answer answer, Done done);
  // Opens the conversation if it is not: false (answered with why) when it cannot be.
  bool open(Done& done);
  void work();

  int wake_ = -1;
  std::mutex mutex_;
  std::condition_variable pending_;
  std::deque<std::function<void()>> jobs_;
  std::deque<std::pair<Answer, Done>> answers_;
  bool stopping_ = false;
  std::thread worker_;
  // The conversation: touched only by the worker.
  OxyOSMigration* migration_ = nullptr;
};

}  // namespace oxyos
