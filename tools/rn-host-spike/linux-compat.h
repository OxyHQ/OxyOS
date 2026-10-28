// Linux compatibility for C++ written against libc++ on Android/Apple, force-included into
// third-party cores built unchanged (react-native-worklets, react-native-reanimated).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <iterator>
#include <sstream>
#include <pthread.h>
#include <string>

// Apple's one-argument form names the calling thread; glibc's takes the thread and a name of at
// most 15 characters.
inline int pthread_setname_np(const char* name) {
  std::string shortName(name);
  shortName.resize(std::min<size_t>(shortName.size(), 15));
  return pthread_setname_np(pthread_self(), shortName.c_str());
}
