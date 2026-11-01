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

# Skia, from its own repository, at the revision Chrome 156.0.8078.79 (stable) ships. To bump it:
# take the Skia revision of the Chrome stable release from chromiumdash.appspot.com (or
# `skia_revision` in Chromium's DEPS at that release's tag), set it here and run build.sh again;
# the dependencies Skia's build needs are taken at the revisions that Skia's DEPS pins.
SKIA_REVISION=aa789d9bd566b9865add4070dc6777b3266c64e9

# Debian's multiarch directory for this machine's libraries (/usr/lib/<triplet>): OxyOS builds for
# amd64 and arm64 (docs/adr/0032-arm.mdx).
case $(dpkg --print-architecture) in
  amd64) MULTIARCH=x86_64-linux-gnu SKIA_CPU=x64 ;;
  arm64) MULTIARCH=aarch64-linux-gnu SKIA_CPU=arm64 ;;
  *) echo "build.sh: $(dpkg --print-architecture) is not an OxyOS architecture (amd64, arm64)" >&2; exit 1 ;;
esac

RN_VERSION=$(node -p "require('$RN_DIR/package.json').version")
echo "react-native $RN_VERSION"

cd "$WORK"
[ -x tools/bin/cmake ] || { python3 -m venv tools && tools/bin/pip install -q cmake ninja; }
PATH="$WORK/tools/bin:$PATH"

# LLD links Hermes as the Android NDK links it (build-hermes.sh).
if [ ! -x clang/usr/bin/clang++-19 ] || [ ! -x clang/usr/lib/llvm-19/bin/ld.lld ]; then
  mkdir -p debs && (cd debs && apt-get download clang-19 libclang-cpp19 libllvm19 \
    libclang-common-19-dev libclang1-19 llvm-19-linker-tools libz3-4 lld-19)
  for deb in debs/*.deb; do dpkg -x "$deb" clang; done
fi
CC="$WORK/clang/usr/bin/clang-19"
CXX="$WORK/clang/usr/bin/clang++-19"

# FreeType and HarfBuzz headers (the libraries themselves are part of the Debian base).
if [ ! -e text/usr/include/harfbuzz/hb.h ]; then
  mkdir -p debs/text && (cd debs/text && apt-get download libfreetype-dev libharfbuzz-dev)
  for deb in debs/text/*.deb; do dpkg -x "$deb" text; done
  for lib in freetype harfbuzz; do
    so=$(ls /usr/lib/$MULTIARCH/lib$lib.so.[0-9]* | head -1)
    rm -f "text/usr/lib/$MULTIARCH/lib$lib.so" && ln -s "$so" "text/usr/lib/$MULTIARCH/lib$lib.so"
  done
fi

# ICU headers (the libraries are part of the Debian base), for Hermes' Unicode support and the
# host's word boundaries.
if [ ! -e icu/usr/include/unicode/uchar.h ]; then
  mkdir -p debs/icu && (cd debs/icu && apt-get download libicu-dev)
  for deb in debs/icu/*.deb; do dpkg -x "$deb" icu; done
  for lib in icuuc icui18n icudata; do
    so=$(ls /usr/lib/$MULTIARCH/lib$lib.so.[0-9]* | head -1)
    rm -f "icu/usr/lib/$MULTIARCH/lib$lib.so" && ln -s "$so" "icu/usr/lib/$MULTIARCH/lib$lib.so"
  done
fi

# Skia (SKIA_REVISION above): static libraries for the host, Ganesh on Vulkan, built against the
# system's FreeType, HarfBuzz and ICU (the ones the host and Hermes link, so the process has one of
# each). Of Skia's DEPS only what these arguments compile is fetched, at the revision DEPS pins: the
# Vulkan Memory Allocator. Not tools/git-sync-deps, which clones gigabytes of repositories for
# configurations not built here. Vulkan's headers are Skia's own copy; the Vulkan loader is opened
# at run time (Skia takes vkGetInstanceProcAddr), never linked.
git_at() { # <dir> <url> <revision>: a shallow checkout of exactly that revision
  if [ "$(git -C "$1" rev-parse HEAD 2>/dev/null)" != "$3" ]; then
    [ -d "$1/.git" ] || git init -q "$1"
    git -C "$1" fetch -q --depth 1 "$2" "$3"
    git -C "$1" checkout -q --force FETCH_HEAD
  fi
}
skia_head=$(git -C skia rev-parse HEAD 2>/dev/null || true)
git_at skia https://skia.googlesource.com/skia "$SKIA_REVISION"
# Skia's own gn, at the revision its bin/fetch-gn names.
[ -x skia/bin/gn ] && [ "$skia_head" = "$SKIA_REVISION" ] || python3 skia/bin/fetch-gn
deps_entry() { sed -n "s|^ *\"third_party/externals/$1\" *: *\"\(.*\)\",.*|\1|p" skia/DEPS; }
vma=$(deps_entry vulkanmemoryallocator)
git_at skia/third_party/externals/vulkanmemoryallocator "${vma%@*}" "${vma##*@}"

# libpng, zlib and Expat headers for Skia's PNG codec and its SVG module's XML parser (the libraries
# are part of the Debian base). Skia's public headers include none of them.
if [ ! -e skia-sysroot/usr/include/expat.h ]; then
  mkdir -p debs/skia && (cd debs/skia && apt-get download libpng-dev zlib1g-dev libexpat1-dev)
  for deb in debs/skia/*.deb; do dpkg -x "$deb" skia-sysroot; done
fi

# Where an argument is not Skia's default for an official build, the reason:
# - Ganesh on Vulkan. No OpenGL (OxyOS has no EGL or GLX) and no Graphite.
skia_args="is_official_build=true is_debug=false target_cpu=\"$SKIA_CPU\" cc=\"$CC\" cxx=\"$CXX\"
  skia_enable_ganesh=true skia_enable_graphite=false skia_use_vulkan=true skia_use_gl=false"
# - The system's FreeType, HarfBuzz and ICU, from the sysroots above, never Skia's bundled copies.
#   HarfBuzz's subsetter is only for PDF fonts.
skia_args="$skia_args skia_use_system_freetype2=true
  skia_system_freetype2_include_path=\"$WORK/text/usr/include/freetype2\"
  skia_use_system_harfbuzz=true skia_pdf_subset_harfbuzz=false skia_use_system_icu=true"
# - No fontconfig: the host registers its font files itself, through the empty custom font manager
#   (makeFromFile, makeFromData, variation axes); no directory scan or embedded fonts.
skia_args="$skia_args skia_use_fontconfig=false skia_enable_fontmgr_custom_empty=true
  skia_enable_fontmgr_custom_directory=false skia_enable_fontmgr_custom_embedded=false
  skia_enable_fontmgr_android=false"
# - Codecs: PNG through the system's libpng and zlib (Skia's PNG encoder; the host decodes its images
#   in crates/oxyos-image). JPEG is off: libjpeg62-turbo is not in the OxyOS image. No WebP, AVIF,
#   GIF (wuffs), RAW (piex, DNG SDK).
skia_args="$skia_args skia_use_system_libpng=true skia_use_system_zlib=true
  skia_use_libjpeg_turbo_decode=false skia_use_libjpeg_turbo_encode=false
  skia_use_libwebp_decode=false skia_use_libwebp_encode=false skia_use_libavif=false
  skia_use_wuffs=false skia_use_piex=false skia_use_dng_sdk=false"
# - Text: SkShaper (HarfBuzz) and SkUnicode (ICU), SkParagraph; the SVG module over the system's
#   Expat. No PDF, Skottie or tools.
skia_args="$skia_args skia_enable_skshaper=true skia_enable_skunicode=true skia_enable_skparagraph=true
  skia_use_expat=true skia_use_system_expat=true skia_enable_svg=true
  skia_enable_pdf=false skia_enable_skottie=false skia_enable_tools=false"
# - The system's allocator, as the host's (no PartitionAlloc); no Perfetto (tracing is off in an
#   official build anyway); no X11 (OxyOS has none) or XPS.
skia_args="$skia_args skia_use_partition_alloc=false skia_use_perfetto=false skia_use_x11=false
  skia_use_xps=false"
# - The headers of the system libraries. RTTI on: the host is built with it, and a host class
#   deriving from a Skia class with an out-of-line virtual function (SkDrawable, for one) needs the
#   base's typeinfo, which Skia built without RTTI does not emit.
# - Text's gamma as Chromium sets it on Linux (its skia/BUILD.gn and SkUserConfig.h): gray glyph
#   masks corrected too (SK_GAMMA_APPLY_TO_A8), with an exponent of 1.2 and a contrast of 0.2.
#   Skia's defaults leave gray masks linear, and text then draws thinner than Bloom's does in
#   Chromium. (Android's HWUI builds Skia with 1.4 and 0.0.)
skia_args="$skia_args extra_cflags=[\"-I$WORK/text/usr/include/harfbuzz\", \"-I$WORK/icu/usr/include\",
  \"-I$WORK/skia-sysroot/usr/include\", \"-DSK_GAMMA_APPLY_TO_A8\", \"-DSK_GAMMA_EXPONENT=1.2\",
  \"-DSK_GAMMA_CONTRAST=0.2\"] extra_cflags_cc=[\"-frtti\"]"
skia_libs="libskia.a libskshaper.a libskunicode_core.a libskunicode_icu.a libskparagraph.a libsvg.a
  libskresources.a"
skia_stamp=$(printf '%s\n' "$SKIA_REVISION" "$skia_args")
if [ "$(cat skia-build/oxyos-skia-stamp 2>/dev/null)" != "$skia_stamp" ]; then
  (cd skia && bin/gn gen "$WORK/skia-build" --args="$skia_args")
  # The preprocessor definitions Skia's public headers must see, as its public configurations give
  # them to a dependent (CMakeLists.txt reads them). gn desc fails for what a target or configuration
  # does not have (a static library's public_configs, a configuration with no defines).
  (cd skia && configs=$(for target in //:skia //modules/skshaper //modules/skunicode:skunicode_core \
      //modules/skunicode:skunicode_icu //modules/skparagraph //modules/svg; do
    for what in public_configs all_dependent_configs; do
      bin/gn desc "$WORK/skia-build" "$target" "$what" || true
    done
  done | grep '^//' | sort -u) &&
    for config in $configs; do bin/gn desc "$WORK/skia-build" "$config" defines || true; done |
    grep -v ' ' | sort -u > "$WORK/skia-build/oxyos-skia-defines.txt")
  echo "$skia_stamp" > skia-build/oxyos-skia-stamp
fi
# shellcheck disable=SC2086 # the list is words
ninja -C skia-build -j "$JOBS" $skia_libs

# Hermes V1, the release React Native ships, built as it ships it, with the system ICU for case
# mapping and normalisation (build-hermes.sh).
JOBS="$JOBS" "$HERE/build-hermes.sh" "$RN_DIR" "$WORK" hermes-build

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

# react-native-gesture-handler's shared runtime bindings, built from the Bloom checkout's copy.
GESTURE_HANDLER_DIR=$(cd "$BLOOM_DIR/node_modules/react-native-gesture-handler" && pwd)

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
  -DSVG_DIR="$SVG_DIR" -DSVG_CODEGEN_DIR="$WORK/codegen-svg/out/jni" -DGESTURE_HANDLER_DIR="$GESTURE_HANDLER_DIR" \
  -DTEXTINPUT_DIR="$WORK/textinput" -DSKIA_DIR="$WORK/skia" -DSKIA_BUILD="$WORK/skia-build" >/dev/null
ninja -C host-build -j "$JOBS" oxyos-rn-spike oxyos-bloom-host oxyos-skia-check oxyos-turbo-module-cache-test

# The runtime's TurboModules: one per name, a method kept past its module's object still working.
host-build/oxyos-turbo-module-cache-test

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
