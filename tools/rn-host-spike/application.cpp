#include "application.h"

#include "oxyos_client.h"

#include <cstdlib>
#include <iostream>
#include <optional>

namespace oxyos {

namespace {

// An application's directory (oxyos_application_directory's `which`), or "" with the reason said.
std::string directory(const std::string& id, uint8_t which) {
  std::string out(4096, '\0');
  int64_t length = oxyos_application_directory(id.c_str(), which, out.data(), out.size());
  if (length >= 0 && static_cast<size_t>(length) >= out.size()) {
    out.assign(static_cast<size_t>(length) + 1, '\0');
    length = oxyos_application_directory(id.c_str(), which, out.data(), out.size());
  }
  if (length < 0 || static_cast<size_t>(length) >= out.size()) {
    std::cerr << "[host] " << id << " has no directory: " << oxyos_last_error() << "\n";
    return "";
  }
  out.resize(static_cast<size_t>(length));
  return out;
}

}  // namespace

Application Application::launched() {
  Application application;
  const char* id = std::getenv("OXYOS_APP_ID");
  application.id = id ? id : "";
  application.recorded = !application.id.empty() && !std::getenv("OXYOS_APP_UNRECORDED");
  if (application.recorded) {
    application.dataDirectory = directory(application.id, 0);
    application.cacheDirectory = directory(application.id, 1);
  } else if (!application.id.empty()) {
    std::cerr << "[host] " << application.id
              << " was not started by the session: its id is only a claim, so it has no data directory\n";
  }
  return application;
}

}  // namespace oxyos
