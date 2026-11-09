#include "crash-report.h"

#include <cstdlib>
#include <fcntl.h>
#include <iostream>
#include <unistd.h>

namespace oxyos {

namespace {
int reportFd = -1;
}  // namespace

void openCrashReport() {
  const char* path = std::getenv("OXYOS_CRASH_REPORT");
  if (!path || !*path || reportFd >= 0) return;
  // The session made it: never made here, never a link followed.
  reportFd = open(path, O_WRONLY | O_APPEND | O_CLOEXEC | O_NOFOLLOW);
  if (reportFd < 0) std::cerr << "[host] the crash report " << path << " cannot be opened\n";
}

int crashReportFd() { return reportFd; }

void endForFatalJavaScriptError(const std::string& report) {
  std::cerr << "[host] a fatal JavaScript error ends the application:\n" << report << std::flush;
  if (reportFd >= 0) {
    const std::string text = "Fatal JavaScript error: " + report;
    for (size_t written = 0; written < text.size();) {
      ssize_t now = write(reportFd, text.data() + written, text.size() - written);
      if (now <= 0) break;
      written += static_cast<size_t>(now);
    }
  }
  _exit(kFatalJavaScriptStatus);
}

}  // namespace oxyos
