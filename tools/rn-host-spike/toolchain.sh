#!/bin/sh
# The Bloom host's toolchain, prepared in a work directory and kept there between runs, and the
# CMake variables that name it (written to <vars-out>, one NAME=value a line) for the host's
# CMakeLists.txt: build.sh, the scenario gate (tools/gate/run-scenarios.sh) and a deploy build with it.
#   <work-dir>/tools, clang, text, icu, skia, skia-build, skia-sysroot
#                             what follows no React Native: cmake and ninja (a venv), Clang and LLD
#                             (apt-get download + dpkg -x), FreeType's, HarfBuzz's and ICU's headers,
#                             Skia at SKIA_REVISION built as below
#   <work-dir>/rn-<version>/  what follows the Bloom checkout's React Native, once per version: the
#                             Hermes it pairs with (build-hermes.sh), its third-party sources laid out
#                             (prepare.py), its core codegen and its patched text input
#   <work-dir>/rn-<version>/reanimated-<v>/, svg-<v>/
#                             Reanimated's and react-native-svg's component codegen
# A lock keeps two runs from preparing one work directory at once.
#
# Usage: tools/rn-host-spike/toolchain.sh <work-dir> <bloom-checkout> <vars-out> [<react-native-dir>]
#   <react-native-dir> defaults to the Bloom checkout's node_modules/react-native.
#
# Debian trixie host. Nothing is installed system-wide.
set -eu

WORK=$(mkdir -p "$1" && cd "$1" && pwd)
BLOOM_DIR=$(cd "$2" && pwd)
OUT=$3
RN_DIR=$(cd "${4:-$BLOOM_DIR/node_modules/react-native}" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
JOBS=${JOBS:-12}

mkdir -p "$WORK/.locks"
exec 9>"$WORK/.locks/toolchain"
flock 9

# Skia, from its own repository, at the revision Chrome 156.0.8078.79 (stable) ships. To bump it:
# take the Skia revision of the Chrome stable release from chromiumdash.appspot.com (or
# `skia_revision` in Chromium's DEPS at that release's tag), set it here and run build.sh (or the scenario gate) again;
# the dependencies Skia's build needs are taken at the revisions that Skia's DEPS pins.
SKIA_REVISION=aa789d9bd566b9865add4070dc6777b3266c64e9

# Debian's multiarch directory for this machine's libraries (/usr/lib/<triplet>): OxyOS builds for
# amd64 and arm64 (docs/adr/0032-arm.mdx).
case $(dpkg --print-architecture) in
  amd64) MULTIARCH=x86_64-linux-gnu SKIA_CPU=x64 ;;
  arm64) MULTIARCH=aarch64-linux-gnu SKIA_CPU=arm64 ;;
  *) echo "build.sh: $(dpkg --print-architecture) is not an OxyOS architecture (amd64, arm64)" >&2; exit 1 ;;
esac



cd "$WORK"
# cmake and ninja, at the versions and wheels toolchain-requirements.txt pins.
if ! cmp -s "$HERE/toolchain-requirements.txt" tools/.requirements 2>/dev/null; then
  [ -x tools/bin/pip ] || python3 -m venv tools
  tools/bin/pip install -q --only-binary=:all: --require-hashes -r "$HERE/toolchain-requirements.txt"
  cp "$HERE/toolchain-requirements.txt" tools/.requirements
fi
PATH="$WORK/tools/bin:$PATH"

# LLD links Hermes as the Android NDK links it, and LLVM's ar archives its ThinLTO objects
# (build-hermes.sh); compiler-rt has the sanitizers' runtimes (docs/bloom-host-sanitizers.mdx).
if [ ! -x clang/usr/bin/clang++-19 ] || [ ! -x clang/usr/lib/llvm-19/bin/ld.lld ] || [ ! -x clang/usr/bin/llvm-ar-19 ]; then
  mkdir -p debs && (cd debs && apt-get download clang-19 libclang-cpp19 libllvm19 \
    libclang-common-19-dev libclang1-19 llvm-19-linker-tools libz3-4 lld-19 llvm-19 libclang-rt-19-dev)
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
# - Text: SkShaper (HarfBuzz) and SkUnicode (ICU), SkParagraph; the SVG module (SkSVGDOM, libsvg.a)
#   over the system's Expat, which decodes SVG documents for Image (svg-image.h) and carries the path
#   operations react-native-svg's clip paths unite with (svg.cpp). No PDF, Skottie or tools.
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

# What follows React Native: in a directory of its version's.
version() { node -p "require('$1/package.json').version"; }
RN_VERSION=$(version "$RN_DIR")
NM=$BLOOM_DIR/node_modules
WORKLETS_DIR=$(cd "$NM/react-native-worklets" && pwd)
REANIMATED_DIR=$(cd "$NM/react-native-reanimated" && pwd)
SVG_DIR=$(cd "$NM/react-native-svg" && pwd)
GESTURE_HANDLER_DIR=$(cd "$NM/react-native-gesture-handler" && pwd)
RN=$WORK/rn-$RN_VERSION
REA=$RN/reanimated-$(version "$REANIMATED_DIR")
SVG=$RN/svg-$(version "$SVG_DIR")
codegen() { # <out> <specs> <library> <java-package>: specs generated the way ReactAndroid's Gradle tasks do
  mkdir -p "$1"
  (cd "$RN_DIR" &&
    node ../@react-native/codegen/lib/cli/combine/combine-js-to-schema-cli.js --platform android "$1/schema.json" "$2" &&
    node scripts/generate-specs-cli.js --platform android --schemaPath "$1/schema.json" --outputDir "$1/out" \
      --libraryName "$3" --javaPackageName "$4") >/dev/null
}
if [ ! -f "$RN/.ready" ]; then
  echo "react-native $RN_VERSION: preparing $RN"
  mkdir -p "$RN"
  # Hermes V1, the release React Native ships, built as it ships it, with the system ICU for case
  # mapping and normalisation (build-hermes.sh).
  HERMES_SRC="$RN/hermes" JOBS="$JOBS" "$HERE/build-hermes.sh" "$RN_DIR" "$WORK" "rn-$RN_VERSION/hermes-build"
  # React Native's pinned third-party sources (versions from its gradle/libs.versions.toml).
  versions="$RN_DIR/gradle/libs.versions.toml"
  v() { sed -n "s/^$1 *= *\"\(.*\)\"/\1/p" "$versions"; }
  mkdir -p "$WORK/third-party-src"
  # Each checked against third-party.sha256 before it is unpacked.
  fetch() {
    [ -d "$WORK/third-party-src/$2" ] && return
    sum=$(awk -v url="$1" '$2 == url { print $1 }' "$HERE/third-party.sha256")
    [ -n "$sum" ] || { echo "toolchain.sh: $1 is not in third-party.sha256: add its SHA-256" >&2; exit 1; }
    curl -fsSL --retry 5 -o "$WORK/third-party-src/download.tar.gz" "$1"
    echo "$sum  $WORK/third-party-src/download.tar.gz" | sha256sum -c --quiet
    tar xzf "$WORK/third-party-src/download.tar.gz" -C "$WORK/third-party-src"
    rm -f "$WORK/third-party-src/download.tar.gz"
  }
  fetch "https://archives.boost.io/release/$(v boost | tr _ .)/source/boost_$(v boost).tar.gz" "boost_$(v boost)"
  fetch "https://github.com/google/double-conversion/archive/v$(v doubleconversion).tar.gz" "double-conversion-$(v doubleconversion)"
  fetch "https://github.com/facebook/folly/archive/v$(v folly).tar.gz" "folly-$(v folly)"
  fetch "https://github.com/fastfloat/fast_float/archive/v$(v fastFloat).tar.gz" "fast_float-$(v fastFloat)"
  fetch "https://github.com/fmtlib/fmt/archive/$(v fmt).tar.gz" "fmt-$(v fmt)"
  fetch "https://github.com/google/glog/archive/v$(v glog).tar.gz" "glog-$(v glog)"
  python3 "$HERE/prepare.py" "$WORK/third-party-src" "$RN_DIR" "$RN/third-party"
  # Core component and module specs.
  codegen "$RN/codegen" src FBReactNativeSpec com.facebook.fbreact.specs
  # React Native's text input component, patched to build off Android (see the patch's header). A
  # private copy: the React Native package is never modified.
  rm -rf "$RN/textinput"
  cp -R "$RN_DIR/ReactCommon/react/renderer/components/textinput" "$RN/textinput"
  patch -s -d "$RN/textinput" -p1 < "$HERE/patches/react-native-androidtextinput-props.patch"
  touch "$RN/.ready"
fi
# Reanimated's and react-native-svg's component specs, from the Bloom checkout's copies.
[ -f "$REA/.ready" ] || { codegen "$REA" "$REANIMATED_DIR/src/specs" rnreanimated com.swmansion.reanimated && touch "$REA/.ready"; }
[ -f "$SVG/.ready" ] || { codegen "$SVG" "$SVG_DIR/src/fabric" rnsvg com.horcrux.svg && touch "$SVG/.ready"; }

cat > "$OUT" <<EOF
CMAKE_BUILD_TYPE=Release
RN_DIR=$RN_DIR
HERMES_SRC=$RN/hermes
HERMES_BUILD=$RN/hermes-build
CODEGEN_DIR=$RN/codegen/out/jni
THIRD_PARTY_DIR=$RN/third-party
TEXTINPUT_DIR=$RN/textinput
REANIMATED_CODEGEN_DIR=$REA/out/jni
SVG_CODEGEN_DIR=$SVG/out/jni
WORKLETS_DIR=$WORKLETS_DIR
REANIMATED_DIR=$REANIMATED_DIR
SVG_DIR=$SVG_DIR
GESTURE_HANDLER_DIR=$GESTURE_HANDLER_DIR
TEXT_SYSROOT=$WORK/text/usr
ICU_SYSROOT=$WORK/icu/usr
SKIA_DIR=$WORK/skia
SKIA_BUILD=$WORK/skia-build
EOF
echo "toolchain: react-native $RN_VERSION, $(git -C "$RN/hermes" describe --tags), Skia $(git -C "$WORK/skia" rev-parse --short HEAD)"
