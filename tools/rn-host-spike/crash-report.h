// Why the host ends, when it ends for a fault (docs/adr/0061-crashes-and-resetting-an-application.mdx,
// decision 2): the session starts each application, and its shell, with a file of its own made
// empty and private for it (OXYOS_CRASH_REPORT), which the host opens as it starts, never makes,
// and writes into only as it ends for a fatal signal or a fatal JavaScript error. The session reads
// it once the process is gone, into the report the person sees, and removes it.
#pragma once

#include <string>

namespace oxyos {

// Opens the session's report file, if it gave one: before anything can fail.
void openCrashReport();

// Its descriptor, -1 for none: the fatal signal's handler writes into it, async-signal-safely.
int crashReportFd();

// The exit status of a host ended by a fatal JavaScript error (sysexits' EX_SOFTWARE): the session
// sees a status that is not success, and the report says why.
constexpr int kFatalJavaScriptStatus = 70;

// A JavaScript error nothing caught (React Native's fatal one): said in the log and in the report,
// then the host ends, as a release build of React Native ends on Android and iOS. A window whose
// React tree an error unmounted is never left on the screen, drawn by nothing.
[[noreturn]] void endForFatalJavaScriptError(const std::string& report);

}  // namespace oxyos
