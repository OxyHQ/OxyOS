#!/bin/sh
# Feasibility spike for #33: build Hermes V1 and React Native's ReactCommon (Fabric, Yoga, cxx
# platform) for Linux on the machine it runs on (x86-64 or AArch64), plus the C++ cores of
# react-native-worklets and react-native-reanimated, and run a host that mounts a tree through
# nativeFabricUIManager.
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

# The toolchain (toolchain.sh): Clang, cmake, ninja, the text and ICU headers, Skia, and what
# follows this React Native (Hermes, its third-party sources, the codegen, the text input), kept in
# <work-dir> between runs and named in cmake-vars.txt.
"$HERE/toolchain.sh" "$WORK" "$BLOOM_DIR" "$WORK/cmake-vars.txt" "$RN_DIR"
PATH="$WORK/tools/bin:$PATH"
CC="$WORK/clang/usr/bin/clang-19"
CXX="$WORK/clang/usr/bin/clang++-19"
cd "$WORK"

# The display protocol client the host links (the repository's Cargo workspace).
REPO=$(cd "$HERE/../.." && pwd)
cargo build --release --locked --manifest-path "$REPO/Cargo.toml" -p oxyos-client-ffi
OXYOS_CLIENT_LIB="$REPO/target/release/liboxyos_client_ffi.a"

CC="$CC" CXX="$CXX" cmake -S "$HERE" -B host-build -G Ninja $(sed 's/^/-D/' "$WORK/cmake-vars.txt") \
  -DOXYOS_CLIENT_LIB="$OXYOS_CLIENT_LIB" >/dev/null
ninja -C host-build -j "$JOBS" oxyos-rn-spike oxyos-bloom-host oxyos-skia-check oxyos-turbo-module-cache-test oxyos-pipeline-cache-test

# The runtime's TurboModules: one per name, a method kept past its module's object still working.
host-build/oxyos-turbo-module-cache-test
# Ganesh's persistent cache on disk (pipeline-cache.h).
host-build/oxyos-pipeline-cache-test

# Skia on its own: a raster surface and, where a Vulkan device is present, Ganesh on it, the two
# compared (skia-check.cpp).
mkdir -p "$WORK/skia-check"
host-build/oxyos-skia-check "$BLOOM_DIR/node_modules/@oxy.so/bloom/src/fonts/assets/InterVariable.ttf" \
  "$WORK/skia-check"

# Fabric alone: a tree built through nativeFabricUIManager.
host-build/oxyos-rn-spike "$HERE/tree.js"

# React Native's JS and actual Bloom, bundled for the oxyos platform, drawn into a PNG. The sample
# compiles its stylesheet (Tailwind and Bloom's theme) as an Oxy app does, so Bloom's classNames
# resolve; its one dependency is Tailwind's PostCSS plugin (sample/package.json).
# The host is laid out as runtime 1 (docs/adr/0030-installing-applications.mdx, decision 2), with its
# framework, and each application is a package of its own, compiled with that runtime's hermesc; the
# probes are one development bundle (sample/packages.mjs).
rm -rf "$WORK/runtimes" && cmake --install host-build --prefix "$WORK/runtimes/1" >/dev/null
cargo build --release --locked --manifest-path "$REPO/Cargo.toml" -p oxyos-pack
# The runtime's framework (React Native's JavaScript, React, Bloom; ADR 0030, decision 17), then the
# applications built against it.
(cd "$HERE/sample" && npm ci --no-audit --no-fund >/dev/null && BLOOM_DIR="$BLOOM_DIR" node framework.mjs "$WORK/runtimes/1" \
  && BLOOM_DIR="$BLOOM_DIR" OXYOS_PACK="$REPO/target/release/oxyos-pack" OXYOS_RUNTIME="$WORK/runtimes/1" node packages.mjs "$WORK/packages")
OXYOS_SCRIPT_URL="file://$WORK/packages/so.oxy.inbox/bundle/index.js" \
  "$WORK/runtimes/1/oxyos-bloom-host" "$WORK/packages/so.oxy.inbox/index.hbc" OxyBloom "$WORK/bloom.png" 1024 640

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
  --runtimes "$WORK/runtimes" --packages "$WORK/packages" --out "$WORK/regress"
# A secure field's text is drawn as bullets and never reported or published to an assistive
# technology (docs/adr/0003-accessibility-protocol.mdx).
python3 "$HERE/regress.py" "$HERE/scenarios/secure-field.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-secure"
# React Native's Android APIs that OxyOS does not provide fail visibly; the rest answer truthfully
# (docs/react-native-android-adaptation.mdx).
python3 "$HERE/regress.py" "$HERE/scenarios/android-adaptation.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-adaptation"
# Editing keys in single-line and multiline fields: where each leaves the text and the selection.
python3 "$HERE/regress.py" "$HERE/scenarios/text-editing.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-editing"
# An input method's transactions (ADR 0008): what each leaves in a field, the events React receives,
# and what each field tells the compositor.
python3 "$HERE/regress.py" "$HERE/scenarios/text-input.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-text-input"
# An input method the session plays: keys through it, its compositions, a change for an older state,
# and a password field it never sees.
python3 "$HERE/regress.py" "$HERE/scenarios/input-method.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-input-method"
# The OxyOS shell (apps/shell) in one window: it renders once the safe area is reported.
python3 "$HERE/regress.py" "$HERE/scenarios/shell.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-shell"
# The same shell as the session's shell (ADR 0009): its panels and overlay, an application's window
# between them.
python3 "$HERE/regress.py" "$HERE/scenarios/session-shell.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-session-shell"
# The compositor's windows in the shell: an application joins the dock, and the dock restores it.
python3 "$HERE/regress.py" "$HERE/scenarios/shell-windows.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-shell-windows"
# fetch and XMLHttpRequest through the network in Rust (crates/oxyos-net), against a local server the
# regression runs.
python3 "$HERE/regress.py" "$HERE/scenarios/network.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-network"
# Images: a wallpaper frame and the shell's app artwork, their load events and the output's pixels.
python3 "$HERE/regress.py" "$HERE/scenarios/images.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-images"
# expo-clipboard on the OS clipboard, with Ctrl+C and Ctrl+V in a Bloom field.
python3 "$HERE/regress.py" "$HERE/scenarios/clipboard.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-clipboard"
# Bloom's bottom sheet in React Native's Modal, closed by its pan (gesture handler) and by Escape.
python3 "$HERE/regress.py" "$HERE/scenarios/sheet.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-sheet"
# The shell's overview: a window's card dragged onto another workspace.
python3 "$HERE/regress.py" "$HERE/scenarios/shell-overview.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-shell-overview"
# Transforms: views moved, scaled and turned, drawn and pressed where they are drawn.
python3 "$HERE/regress.py" "$HERE/scenarios/transform.json" --check-damage \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-transform"
# Modal's animationType: its window sliding and fading in and out, frame by frame.
python3 "$HERE/regress.py" "$HERE/scenarios/modal.json" --check-damage \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-modal"
# A touchpad pinch: two fingers to React Native and to gesture handlers' pinch and rotation.
python3 "$HERE/regress.py" "$HERE/scenarios/pinch.json" --check-damage \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-pinch"
# Animated's native driver: opacity, a transform and a colour stepped on the host's frames, and Bloom's Switch.
python3 "$HERE/regress.py" "$HERE/scenarios/native-animated.json" --check-damage \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-native-animated"
# Intl: dates and numbers for several locales and time zones, each as Node prints it with the same CLDR.
python3 "$HERE/regress.py" "$HERE/scenarios/intl.json" \
  --offscreen "$REPO/target/release/oxyos-offscreen" --runtimes "$WORK/runtimes" \
  --packages "$WORK/packages" --out "$WORK/regress-intl"
