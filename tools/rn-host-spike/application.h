// The application the host runs (docs/adr/0030-installing-applications.mdx, decision 7; ADR 0013):
//
// - **Which application.** Its id is the registry entry's (`so.oxy.stash`), which the session
//   gives it in OXYOS_APP_ID when it launches it (crates/oxyos-apps' `command`). An application the
//   session did not start (OXYOS_APP_UNRECORDED) has its id only as a claim; one started with no id
//   has none.
// - **Its directories**, by that id (crates/oxyos-apps): its data directory
//   ($XDG_DATA_HOME/oxyos/apps/ID), what it keeps, and its cache directory
//   ($XDG_CACHE_HOME/oxyos/apps/ID), which expo-file-system gives as its document and cache
//   directories (files.h, docs/adr/0028-file-access.mdx). Only an application the session started
//   has them: a claimed id never reaches another application's data. Nothing is kept for one
//   without them.
#pragma once

#include <string>

namespace oxyos {

struct Application {
  // "" when it was started with none.
  std::string id;
  // Whether the session started it (its id is then the session's, not a claim).
  bool recorded = false;
  // "" without an id the session gave it.
  std::string dataDirectory, cacheDirectory;

  // As the launcher started it: from its environment.
  static Application launched();
};

}  // namespace oxyos
