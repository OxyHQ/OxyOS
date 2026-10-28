#!/bin/sh
# Feasibility spike for #33: build Hermes V1 and React Native's ReactCommon (Fabric, Yoga, cxx
# platform) for Linux x86_64 and run a host that mounts a tree through nativeFabricUIManager.
#
# Usage: tools/rn-host-spike/build.sh <react-native-package-dir> <work-dir>
#   e.g. tools/rn-host-spike/build.sh ../Bloom/node_modules/react-native /tmp/rn-spike
#
# Debian trixie host. Nothing is installed system-wide: cmake/ninja come from a venv and clang
# from `apt-get download` + `dpkg -x` into <work-dir>. Not part of the Cargo workspace or CI.
set -eu

RN_DIR=$(cd "$1" && pwd)
WORK=$(mkdir -p "$2" && cd "$2" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
JOBS=${JOBS:-12}

RN_VERSION=$(node -p "require('$RN_DIR/package.json').version")
HERMES_TAG=$(cat "$RN_DIR/sdks/.hermesv1version")
echo "react-native $RN_VERSION, hermes $HERMES_TAG"

cd "$WORK"
[ -x tools/bin/cmake ] || { python3 -m venv tools && tools/bin/pip install -q cmake ninja; }
PATH="$WORK/tools/bin:$PATH"

if [ ! -x clang/usr/bin/clang++-19 ]; then
  mkdir -p debs && (cd debs && apt-get download clang-19 libclang-cpp19 libllvm19 \
    libclang-common-19-dev libclang1-19 llvm-19-linker-tools libz3-4)
  for deb in debs/*.deb; do dpkg -x "$deb" clang; done
fi
CC="$WORK/clang/usr/bin/clang-19"
CXX="$WORK/clang/usr/bin/clang++-19"

# Hermes V1 at the revision React Native pins. Unicode-lite: no ICU, as on Android.
[ -d hermes ] || git clone -q --depth 1 --branch "$HERMES_TAG" https://github.com/facebook/hermes.git hermes
cmake -S hermes -B hermes-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DHERMES_ENABLE_INTL=OFF \
  -DHERMES_UNICODE_LITE=ON -DHERMES_ENABLE_TEST_SUITE=OFF >/dev/null
ninja -C hermes-build -j "$JOBS" lib/libhermesvm.so bin/hermes bin/hermesc

# React Native's pinned third-party sources (versions from its gradle/libs.versions.toml).
versions="$RN_DIR/gradle/libs.versions.toml"
v() { sed -n "s/^$1 *= *\"\(.*\)\"/\1/p" "$versions"; }
mkdir -p third-party-src && cd third-party-src
fetch() { [ -d "$2" ] || curl -sSL "$1" | tar xz; }
fetch "https://archives.boost.io/release/$(v boost | tr _ .)/source/boost_$(v boost).tar.gz" "boost_$(v boost)"
fetch "https://github.com/google/double-conversion/archive/v$(v doubleconversion).tar.gz" "double-conversion-$(v doubleconversion)"
fetch "https://github.com/facebook/folly/archive/v$(v folly).tar.gz" "folly-$(v folly)"
fetch "https://github.com/fastfloat/fast_float/archive/v$(v fastFloat).tar.gz" "fast_float-$(v fastFloat)"
fetch "https://github.com/fmtlib/fmt/archive/$(v fmt).tar.gz" "fmt-$(v fmt)"
fetch "https://github.com/google/glog/archive/v$(v glog).tar.gz" "glog-$(v glog)"
cd "$WORK"
python3 "$HERE/prepare.py" "$WORK/third-party-src" "$RN_DIR" "$WORK/third-party"

# Core component and module specs, generated the way ReactAndroid's Gradle tasks do.
mkdir -p "$WORK/codegen"
(cd "$RN_DIR" &&
  node ../@react-native/codegen/lib/cli/combine/combine-js-to-schema-cli.js --platform android \
    "$WORK/codegen/schema.json" src &&
  node scripts/generate-specs-cli.js --platform android --schemaPath "$WORK/codegen/schema.json" \
    --outputDir "$WORK/codegen/out" --libraryName FBReactNativeSpec \
    --javaPackageName com.facebook.fbreact.specs)

CC="$CC" CXX="$CXX" cmake -S "$HERE" -B host-build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DRN_DIR="$RN_DIR" -DHERMES_SRC="$WORK/hermes" -DHERMES_BUILD="$WORK/hermes-build" \
  -DCODEGEN_DIR="$WORK/codegen/out/jni" -DTHIRD_PARTY_DIR="$WORK/third-party" >/dev/null
ninja -C host-build -j "$JOBS" oxy-rn-spike

host-build/oxy-rn-spike "$HERE/tree.js"
