// Minimal <android/log.h> for third-party cores built with Android semantics on OxyOS: the log
// priorities and __android_log_print, writing to stderr.
#pragma once

#include <cstdarg>
#include <cstdio>

enum android_LogPriority {
  ANDROID_LOG_UNKNOWN = 0, ANDROID_LOG_DEFAULT, ANDROID_LOG_VERBOSE, ANDROID_LOG_DEBUG,
  ANDROID_LOG_INFO, ANDROID_LOG_WARN, ANDROID_LOG_ERROR, ANDROID_LOG_FATAL, ANDROID_LOG_SILENT,
};

inline int __android_log_print(int, const char* tag, const char* format, ...) {
  std::fprintf(stderr, "[%s] ", tag);
  va_list args;
  va_start(args, format);
  int written = std::vfprintf(stderr, format, args);
  va_end(args);
  std::fputc('\n', stderr);
  return written;
}
