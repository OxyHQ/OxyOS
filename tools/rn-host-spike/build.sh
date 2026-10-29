#!/bin/sh
# Feasibility spike for #33: build Hermes V1 and React Native's ReactCommon (Fabric, Yoga, cxx
# platform) for Linux x86_64, plus the C++ cores of react-native-worklets and
# react-native-reanimated, and run a host that mounts a tree through nativeFabricUIManager.
#
# Usage: tools/rn-host-spike/build.sh <react-native-package-dir> <work-dir> [<bloom-checkout>]
#   e.g. tools/rn-host-spike/build.sh ../Bloom/node_modules/react-native /tmp/rn-spike
# The Bloom checkout defaults to the one that owns the React Native package.
#
# Debian trixie host. Nothing is installed system-wide: cmake/ninja come from a venv and clang
# from `apt-get download` + `dpkg -x` into <work-dir>. Not part of the Cargo workspace or CI.
set -eu

RN_DIR=$(cd "$1" && pwd)
WORK=$(mkdir -p "$2" && cd "$2" && pwd)
BLOOM_DIR=$(cd "${3:-$RN_DIR/../..}" && pwd)
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

# FreeType and HarfBuzz headers (the libraries themselves are part of the Debian base).
if [ ! -e text/usr/include/harfbuzz/hb.h ]; then
  mkdir -p debs/text && (cd debs/text && apt-get download libfreetype-dev libharfbuzz-dev)
  for deb in debs/text/*.deb; do dpkg -x "$deb" text; done
  for lib in freetype harfbuzz; do
    so=$(ls /usr/lib/x86_64-linux-gnu/lib$lib.so.[0-9]* | head -1)
    rm -f "text/usr/lib/x86_64-linux-gnu/lib$lib.so" && ln -s "$so" "text/usr/lib/x86_64-linux-gnu/lib$lib.so"
  done
fi

# ICU headers (the libraries are part of the Debian base), for Hermes' Unicode support and the
# host's word boundaries.
if [ ! -e icu/usr/include/unicode/uchar.h ]; then
  mkdir -p debs/icu && (cd debs/icu && apt-get download libicu-dev)
  for deb in debs/icu/*.deb; do dpkg -x "$deb" icu; done
  for lib in icuuc icui18n icudata; do
    so=$(ls /usr/lib/x86_64-linux-gnu/lib$lib.so.[0-9]* | head -1)
    rm -f "icu/usr/lib/x86_64-linux-gnu/lib$lib.so" && ln -s "$so" "icu/usr/lib/x86_64-linux-gnu/lib$lib.so"
  done
fi

# Hermes V1 at the revision React Native pins, with ICU for case mapping and normalisation.
# Unicode-lite (Android's build) leaves String.prototype.toLocaleLowerCase and normalize() as
# identity functions without the platform's Java ICU behind them; OxyOS has no Java, so Hermes
# links the system ICU instead. Intl stays off: Hermes' Linux Intl is a stub.
[ -d hermes ] || git clone -q --depth 1 --branch "$HERMES_TAG" https://github.com/facebook/hermes.git hermes
cmake -S hermes -B hermes-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DHERMES_ENABLE_INTL=OFF \
  -DHERMES_UNICODE_LITE=OFF -DICU_ROOT="$WORK/icu/usr" -DHERMES_ENABLE_TEST_SUITE=OFF >/dev/null
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

# Reanimated's component specs, generated the same way from the Bloom checkout's Reanimated.
WORKLETS_DIR=$(cd "$BLOOM_DIR/node_modules/react-native-worklets" && pwd)
REANIMATED_DIR=$(cd "$BLOOM_DIR/node_modules/react-native-reanimated" && pwd)
mkdir -p "$WORK/codegen-reanimated"
(cd "$RN_DIR" &&
  node ../@react-native/codegen/lib/cli/combine/combine-js-to-schema-cli.js --platform android \
    "$WORK/codegen-reanimated/schema.json" "$REANIMATED_DIR/src/specs" &&
  node scripts/generate-specs-cli.js --platform android \
    --schemaPath "$WORK/codegen-reanimated/schema.json" --outputDir "$WORK/codegen-reanimated/out" \
    --libraryName rnreanimated --javaPackageName com.swmansion.reanimated)

# react-native-svg's component specs, generated the same way from the Bloom checkout's copy.
SVG_DIR=$(cd "$BLOOM_DIR/node_modules/react-native-svg" && pwd)
mkdir -p "$WORK/codegen-svg"
(cd "$RN_DIR" &&
  node ../@react-native/codegen/lib/cli/combine/combine-js-to-schema-cli.js --platform android \
    "$WORK/codegen-svg/schema.json" "$SVG_DIR/src/fabric" &&
  node scripts/generate-specs-cli.js --platform android \
    --schemaPath "$WORK/codegen-svg/schema.json" --outputDir "$WORK/codegen-svg/out" \
    --libraryName rnsvg --javaPackageName com.horcrux.svg)

# React Native's text input component, patched to build off Android (see the patch's header). A
# private copy: the React Native package is never modified.
rm -rf "$WORK/textinput"
cp -R "$RN_DIR/ReactCommon/react/renderer/components/textinput" "$WORK/textinput"
patch -s -d "$WORK/textinput" -p1 < "$HERE/patches/react-native-androidtextinput-props.patch"

# The display protocol client the host links (the repository's Cargo workspace).
REPO=$(cd "$HERE/../.." && pwd)
cargo build --release --locked --manifest-path "$REPO/Cargo.toml" -p oxyos-client-ffi
OXYOS_CLIENT_LIB="$REPO/target/release/liboxyos_client_ffi.a"

CC="$CC" CXX="$CXX" cmake -S "$HERE" -B host-build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DRN_DIR="$RN_DIR" -DHERMES_SRC="$WORK/hermes" -DHERMES_BUILD="$WORK/hermes-build" \
  -DCODEGEN_DIR="$WORK/codegen/out/jni" -DTHIRD_PARTY_DIR="$WORK/third-party" \
  -DTEXT_SYSROOT="$WORK/text/usr" -DICU_SYSROOT="$WORK/icu/usr" -DWORKLETS_DIR="$WORKLETS_DIR" -DREANIMATED_DIR="$REANIMATED_DIR" \
  -DREANIMATED_CODEGEN_DIR="$WORK/codegen-reanimated/out/jni" -DOXYOS_CLIENT_LIB="$OXYOS_CLIENT_LIB" \
  -DSVG_DIR="$SVG_DIR" -DSVG_CODEGEN_DIR="$WORK/codegen-svg/out/jni" \
  -DTEXTINPUT_DIR="$WORK/textinput" >/dev/null
ninja -C host-build -j "$JOBS" oxyos-rn-spike oxyos-bloom-host

# Fabric alone: a tree built through nativeFabricUIManager.
host-build/oxyos-rn-spike "$HERE/tree.js"

# React Native's JS and actual Bloom, bundled for the oxyos platform, drawn into a PNG. The sample
# compiles its stylesheet (Tailwind and Bloom's theme) as an Oxy app does, so Bloom's classNames
# resolve; its one dependency is Tailwind's PostCSS plugin (sample/package.json).
(cd "$HERE/sample" && npm ci --no-audit --no-fund >/dev/null && BLOOM_DIR="$BLOOM_DIR" node bundle.mjs index.js "$WORK/sample")
OXYOS_SCRIPT_URL="file://$WORK/sample/index.js" \
  host-build/oxyos-bloom-host "$WORK/sample/index.js" OxyBloom "$WORK/bloom.png" 1024 640

# The same app in a window, as a regression: the offscreen compositor launches the host as its own
# process, which presents through the display protocol, and drives scenarios/sample.json. Each
# step's expected result is checked: the text shown, the focused input and surface, the exact
# events React received (no duplicate or leaked one) and a latency budget (regress.py). The menu
# steps need Bloom 15.61.0 or later (the platform adapter, OxyHQ/Bloom#507, and the text field's
# reading order, #511): Bloom's main has both; point BLOOM_DIR at such a checkout.
cargo build --release --locked --manifest-path "$REPO/Cargo.toml" -p oxyos-offscreen
# With OXYOS_GPU=1 every frame is also composed on a Vulkan device and must match the CPU
# composition byte for byte (docs/adr/0004-gpu-composition.mdx).
python3 "$HERE/regress.py" "$HERE/scenarios/sample.json" ${OXYOS_GPU:+--gpu} --offscreen "$REPO/target/release/oxyos-offscreen" \
  --host "$WORK/host-build/oxyos-bloom-host" --bundle "$WORK/sample" --out "$WORK/regress"
# A secure field's text is drawn as bullets and never reported or published to an assistive
# technology (docs/adr/0003-accessibility-protocol.mdx).
python3 "$HERE/regress.py" "$HERE/scenarios/secure-field.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-secure"
# React Native's Android APIs that OxyOS does not provide fail visibly; the rest answer truthfully
# (docs/react-native-android-adaptation.mdx).
python3 "$HERE/regress.py" "$HERE/scenarios/android-adaptation.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-adaptation"
# Editing keys in single-line and multiline fields: where each leaves the text and the selection.
python3 "$HERE/regress.py" "$HERE/scenarios/text-editing.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-editing"
# An input method's transactions (ADR 0008): what each leaves in a field, the events React receives,
# and what each field tells the compositor.
python3 "$HERE/regress.py" "$HERE/scenarios/text-input.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-text-input"
# An input method the session plays: keys through it, its compositions, a change for an older state,
# and a password field it never sees.
python3 "$HERE/regress.py" "$HERE/scenarios/input-method.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-input-method"
# The OxyOS shell (apps/shell) in one window: it renders once the safe area is reported.
python3 "$HERE/regress.py" "$HERE/scenarios/shell.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-shell"
# fetch and XMLHttpRequest through the network in Rust (crates/oxyos-net), against a local server the
# regression runs.
python3 "$HERE/regress.py" "$HERE/scenarios/network.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-network"
# Images: a wallpaper frame and the shell's app artwork, their load events and the output's pixels.
python3 "$HERE/regress.py" "$HERE/scenarios/images.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-images"
# expo-clipboard on the OS clipboard, with Ctrl+C and Ctrl+V in a Bloom field.
python3 "$HERE/regress.py" "$HERE/scenarios/clipboard.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --host "$WORK/host-build/oxyos-bloom-host" \
  --bundle "$WORK/sample" --out "$WORK/regress-clipboard"
