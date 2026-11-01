#!/bin/sh
# Builds the Hermes the Bloom host runs (libhermesvm.so) and its compiler (hermesc) the way React
# Native ships Hermes: the release that React Native's version pairs with, built as that release's
# Android library is (facebook/hermes android/build.gradle.kts, its `release` variant, the one an
# application's release build packages), with the toolchain's Clang.
#
# Usage: tools/rn-host-spike/build-hermes.sh <react-native-package-dir> <work-dir> [<build-dir> [<cmake-arg>...]]
#   <work-dir> is build.sh's (its Clang, cmake and ninja, ICU); the checkout is <work-dir>/hermes
#   (HERMES_SRC overrides it) and the build <work-dir>/<build-dir>, hermes-build unless given.
#   Further arguments go to CMake after these, and win: a sanitized build is this one with
#   -DHERMES_ENABLE_ADDRESS_SANITIZER=ON (docs/bloom-host-sanitizers.mdx). HERMES_EXTRA_FLAGS are
#   added to the compiler's flags (-fsanitize=undefined, for one).
set -eu

RN_DIR=$(cd "$1" && pwd)
WORK=$(cd "$2" && pwd)
BUILD="$WORK/${3:-hermes-build}"
[ $# -gt 3 ] && shift 3 || set --
SRC=${HERMES_SRC:-$WORK/hermes}
JOBS=${JOBS:-12}
PATH="$WORK/tools/bin:$PATH"
case $(dpkg --print-architecture) in
  amd64) MULTIARCH=x86_64-linux-gnu ;;
  arm64) MULTIARCH=aarch64-linux-gnu ;;
esac

# The Hermes React Native's version ships: both its Android library (com.facebook.hermes:hermes-android)
# and its iOS pod (hermes-engine) take their version from sdks/hermes-engine/version.properties,
# released by facebook/hermes as the tag hermes-v<version>. (sdks/.hermesv1version names the tag a
# build of React Native from source would take, which can be an older release of the same line.)
HERMES_VERSION=$(sed -n 's/^HERMES_VERSION_NAME=//p' "$RN_DIR/sdks/hermes-engine/version.properties")
HERMES_TAG="hermes-v$HERMES_VERSION"
if [ "$(git -C "$SRC" describe --tags --exact-match HEAD 2>/dev/null)" != "$HERMES_TAG" ]; then
  [ -d "$SRC/.git" ] || git init -q "$SRC"
  git -C "$SRC" fetch -q --depth 1 https://github.com/facebook/hermes.git tag "$HERMES_TAG"
  git -C "$SRC" checkout -q --force "$HERMES_TAG"
fi
echo "hermes $HERMES_TAG ($(git -C "$SRC" rev-parse --short HEAD)) into $BUILD"

# As the release variant of hermes-android is configured, where Linux allows the same:
# - The Android NDK's Clang (r27) there, the toolchain's Clang 19 here, linking with LLD as the NDK
#   does; MinSizeRel (-Os) with link-time optimisation (ThinLTO, as CMake asks Clang for it).
# - HEAP_HV_PREFER32: on a 64-bit machine, 32-bit compressed pointers and boxed doubles in a
#   contiguous heap, the object layout every React Native application on a 64-bit phone runs.
# - The debugger off, as in a release build.
# - JSI from React Native's own sources, as a shared library that libhermesvm.so needs: the process
#   has one JSI, the host's libjsi.so (the hermes-android package leaves its own libjsi.so out for
#   React Native's).
# - The NDK's flags for every build type (its legacy toolchain file, the default): sections for
#   --gc-sections, unwind tables, the stack protector, _FORTIFY_SOURCE, and -g, its debug
#   information kept in the build (addr2line reads lines from this libhermesvm.so) and stripped
#   from the runtime's copy as the Android Gradle plugin strips an application's libraries.
# - Hermes adds -fno-strict-aliasing and -fno-strict-overflow itself, with either compiler.
# Where Linux differs: Unicode from the system ICU (Android's comes from Java, which OxyOS has
# not; Hermes' unicode-lite would leave toLocaleLowerCase and normalize() as identity functions),
# Intl off (Android's is Java; the host provides ECMA-402, intl.h), and not HERMES_IS_ANDROID, whose
# remaining effect, a mobile build, takes JSX, Flow and TypeScript out of the parser, which hermesc
# needs as React Native builds it (for the host machine, not as a mobile build).
CC="$WORK/clang/usr/bin/clang-19" CXX="$WORK/clang/usr/bin/clang++-19" cmake -S "$SRC" -B "$BUILD" -G Ninja \
  --log-level=ERROR -Wno-dev \
  -DCMAKE_BUILD_TYPE=MinSizeRel -DCMAKE_INTERPROCEDURAL_OPTIMIZATION=ON -DCMAKE_LINKER_TYPE=LLD \
  -DHERMESVM_HEAP_HV_MODE=HEAP_HV_PREFER32 -DHERMES_ENABLE_DEBUGGER=OFF \
  -DJSI_DIR="$RN_DIR/ReactCommon/jsi" -DHERMES_BUILD_SHARED_JSI=ON \
  -DCMAKE_C_FLAGS="-g -fdata-sections -ffunction-sections -funwind-tables -fstack-protector-strong -D_FORTIFY_SOURCE=2 ${HERMES_EXTRA_FLAGS:-}" \
  -DCMAKE_CXX_FLAGS="-g -fdata-sections -ffunction-sections -funwind-tables -fstack-protector-strong -D_FORTIFY_SOURCE=2 ${HERMES_EXTRA_FLAGS:-}" \
  -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--gc-sections -Wl,--build-id=sha1" \
  -DCMAKE_EXE_LINKER_FLAGS="-Wl,--gc-sections -Wl,--build-id=sha1" \
  -DHERMES_ENABLE_INTL=OFF -DHERMES_UNICODE_LITE=OFF -DICU_ROOT="$WORK/icu/usr" \
  -DICU_INCLUDE_DIR="$WORK/icu/usr/include" \
  -DICU_UC_LIBRARY_RELEASE="$WORK/icu/usr/lib/$MULTIARCH/libicuuc.so" \
  -DICU_I18N_LIBRARY_RELEASE="$WORK/icu/usr/lib/$MULTIARCH/libicui18n.so" \
  -DICU_DATA_LIBRARY_RELEASE="$WORK/icu/usr/lib/$MULTIARCH/libicudata.so" \
  -DHERMES_ENABLE_TEST_SUITE=OFF "$@" >/dev/null
ninja -C "$BUILD" -j "$JOBS" lib/libhermesvm.so bin/hermes bin/hermesc
