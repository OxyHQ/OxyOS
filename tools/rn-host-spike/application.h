// The application the host runs (docs/adr/0030-installing-applications.mdx, decision 7; ADR 0013),
// and what the platform keeps and answers for it (docs/react-native-android-adaptation.mdx):
//
// - **Which application.** Its id is the registry entry's (`so.oxy.stash`), which the session
//   gives it in OXYOS_APP_ID when it launches it (crates/oxyos-apps' `command`), and its module
//   the entry's `X-OxyOS-Bloom-Module`. An application the session did not start
//   (OXYOS_APP_UNRECORDED) has its id only as a claim; one started with no id has none.
// - **Its directories**, by that id (crates/oxyos-apps): its data directory
//   ($XDG_DATA_HOME/oxyos/apps/ID), what it keeps, and its cache directory
//   ($XDG_CACHE_HOME/oxyos/apps/ID), which expo-file-system gives as its document and cache
//   directories (files.h, docs/adr/0028-file-access.mdx). Only an application the session started
//   has them: a claimed id never reaches another application's data. Nothing is kept for one
//   without them.
// - **Its embedded app config**: the public Expo config its build embeds next to the bundle
//   (`applications/MODULE/app.config`, sample/bundle.mjs), as an Android build embeds `app.config`
//   in the APK's assets, for expo-constants.
// - **Its storage** (React Native's AsyncStorage, crates/oxyos-app-storage), in its data
//   directory, on a thread of its own: each request in order, as Android's AsyncStorage runs them
//   on a serial executor, and its answer back on the JavaScript thread.
// - **The machine's connectivity** (NetInfo, crates/oxyos-net), watched through rtnetlink:
//   `netInfo.networkStatusDidChange` when it changes.
#pragma once

#include "oxyos_client.h"

#include <folly/dynamic.h>

#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

namespace oxyos {

struct Application {
  // "" when it was started with none.
  std::string id;
  // Whether the session started it (its id is then the session's, not a claim).
  bool recorded = false;
  // "" without an id the session gave it.
  std::string dataDirectory, cacheDirectory;
  // Its bundle's module (`X-OxyOS-Bloom-Module`).
  std::string module;
  // Its public Expo config, as JSON; none when its build embedded none.
  std::optional<std::string> manifest;

  // As the launcher started it: its environment, and the bundle it runs (its config beside it).
  static Application launched(const std::string& bundlePath, const std::string& module);
};

class AsyncStorage {
 public:
  using Done = std::function<void(folly::dynamic answer)>;

  // Kept in `directory`, the application's data directory.
  explicit AsyncStorage(std::string directory);
  ~AsyncStorage();
  AsyncStorage(const AsyncStorage&) = delete;
  AsyncStorage& operator=(const AsyncStorage&) = delete;

  // From the JavaScript thread: runs `request` (oxyos_app_storage's) after those before it;
  // `done` gets its answer ({"ok": ...} or {"error": ...}) on the JavaScript thread (`deliver`).
  void request(folly::dynamic request, Done done);
  // Each turn: hands answers to their requests. True if it did.
  bool deliver();
  // Readable while answers wait.
  int fd() const { return wake_; }

 private:
  void work();

  std::string directory_;
  int wake_ = -1;
  std::mutex mutex_;
  std::condition_variable pending_;
  std::deque<std::pair<std::string, Done>> requests_;
  std::deque<std::pair<std::string, Done>> answers_;
  bool stopping_ = false;
  std::thread worker_;
};

class Connectivity {
 public:
  // `changed(state)` on the JavaScript thread, when it changes.
  explicit Connectivity(std::function<void(const folly::dynamic& state)> changed);
  ~Connectivity();
  Connectivity(const Connectivity&) = delete;
  Connectivity& operator=(const Connectivity&) = delete;

  // NetInfo's state, as its Android module gives it: {type, isConnected, details} (no
  // isInternetReachable: NetInfo checks it itself, over HTTP).
  folly::dynamic state() const;
  bool deliver();
  // -1 where changes cannot be watched.
  int fd() const;

 private:
  OxyOSConnectivity* connectivity_;
  std::function<void(const folly::dynamic&)> changed_;
};

}  // namespace oxyos
