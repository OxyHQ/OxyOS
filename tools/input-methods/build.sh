#!/bin/sh
# Builds OxyOS's input-method engine (docs/adr/0041-input-methods.mdx): fcitx5's core with its D-Bus
# frontend, the emoji and QuickPhrase modules, and its engines for Chinese (pinyin, libime),
# Japanese (Anthy) and Korean (libhangul), from Debian Trixie's own source tarballs (the versions the
# image's base ships), with nothing that draws: X11 and Wayland off, so no classic UI, XIM,
# Wayland frontend, xcb or wayland modules, no Enchant, no Qt configuration tools, no cloud pinyin.
#
# Debian's binary packages cannot be used as they are: `fcitx5` depends on `fcitx5-modules`, which
# links Cairo, Pango, GdkPixbuf, the X11 client libraries and libwayland-client, and every engine
# package depends on `fcitx5`. What the engine needs at run time and Debian ships toolkit-free comes
# from apt (RUNTIME_PACKAGES below); this script checks the result links nothing else.
#
# Usage: tools/input-methods/build.sh <work-dir> <prefix>
#   <work-dir> holds the sources, the build-time packages (apt-get download + dpkg -x, as
#   tools/rn-host-spike/build.sh fetches its own) and the build directories; it is kept between
#   runs. <prefix> is where it is installed. The engine is relocatable: nothing in it names where
#   it was built (no RPATH), and the session tells fcitx5 and libime where their libraries,
#   addons and data are (crates/oxyos-input-methods), so the tree built and tested here is the one
#   copied to /usr/lib/oxyos/input-methods on a machine (oxyos-session and oxyos-offscreen take
#   another place as OXYOS_INPUT_METHODS). Not part of the Cargo workspace or CI.
# PRINT_RUNTIME_PACKAGES=1 tools/input-methods/build.sh prints the Debian packages it needs at run
# time, and nothing else.
set -eu

# What the engine needs on the machine that runs it: toolkit-free Debian packages, installed with
# apt there (and in the image's manifest once the session is in it). Printed for the deploy.
RUNTIME_PACKAGES="dbus-daemon libjson-c5 libxkbcommon0 libuuid1 libexpat1 libsystemd0 libfmt10 libboost-iostreams1.83.0 libzstd1 libhangul1 libhangul-data libanthy1t64 anthy-common xkb-data iso-codes"
if [ "${PRINT_RUNTIME_PACKAGES:-}" = 1 ]; then echo "$RUNTIME_PACKAGES"; exit 0; fi

[ $# -eq 2 ] || { echo "usage: $0 <work-dir> <prefix>" >&2; exit 2; }
WORK=$(mkdir -p "$1" && cd "$1" && pwd)
PREFIX=$2
JOBS=${JOBS:-12}
case $(dpkg --print-architecture) in
  amd64) MULTIARCH=x86_64-linux-gnu ;;
  arm64) MULTIARCH=aarch64-linux-gnu ;;
  *) echo "unsupported architecture" >&2; exit 1 ;;
esac
POOL=https://deb.debian.org/debian/pool/main

# Debian Trixie's versions and the SHA-256 of their .orig tarballs (as each .dsc lists it).
SOURCES="
f/fcitx5/fcitx5_5.1.12.orig.tar.xz 09d7e98a6498136839983caa4b8c1df752deb6933a984e6390557602cab357a0 fcitx5-5.1.12
libi/libime/libime_1.1.10+dict20250327.orig.tar.xz 91cbe6fc62a51825a74377b77b12bddc429fcb4f3b6f8a8f42fc96836fee3aa0 libime-1.1.10+dict20250327
f/fcitx5-chinese-addons/fcitx5-chinese-addons_5.1.8.orig.tar.xz 9501e529b7379aa298ce52ea5d71bcc5b24c73b416584cc7c07353396f34bb5c fcitx5-chinese-addons-5.1.8
f/fcitx5-hangul/fcitx5-hangul_5.1.6.orig.tar.xz 2f044d19b3946a0a6036d4ad736edd442751c101b4ab9aa70914ac5a8cdf229e fcitx5-hangul-5.1.6
f/fcitx5-anthy/fcitx5-anthy_5.1.6.orig.tar.xz 0d8687c01f657aa412f54231313a433dd54c95a55a3597f5cef7956a680c1a34 fcitx5-anthy-5.1.6
"
mkdir -p "$WORK/src"
echo "$SOURCES" | while read -r path sum dir; do
  [ -n "$path" ] || continue
  file=$WORK/src/${path##*/}
  if [ ! -d "$WORK/src/$dir" ]; then
    [ -f "$file" ] || curl -fsSL --retry 5 -o "$file" "$POOL/$path"
    echo "$sum  $file" | sha256sum -c --quiet
    tar -xJf "$file" -C "$WORK/src"
  fi
done

# Build-time packages, extracted into a sysroot: headers, CMake modules, msgfmt, and the
# libraries the -dev packages' CMake files name.
SYSROOT=$WORK/sysroot
if [ ! -e "$SYSROOT/.complete" ]; then
  rm -rf "$WORK/debs" "$SYSROOT" && mkdir -p "$WORK/debs" "$SYSROOT"
  (cd "$WORK/debs" && apt-get download -q extra-cmake-modules libjson-c-dev libxkbcommon-dev uuid-dev \
    libexpat1-dev libsystemd-dev zlib1g-dev libfmt-dev libboost1.83-dev libboost-iostreams1.83-dev \
    libzstd-dev libhangul-dev libhangul1 libanthy-dev libanthy1t64 libanthyinput0t64 \
    gettext gettext-base libgettextpo0 \
    libjson-c5 libxkbcommon0 libuuid1 libexpat1 libsystemd0 zlib1g libfmt10 libboost-iostreams1.83.0 libzstd1 > /dev/null)
  for deb in "$WORK"/debs/*.deb; do dpkg -x "$deb" "$SYSROOT"; done
  # The -dev packages' .so links name their runtime library, absolute or beside them: the
  # sysroot's when it was extracted, else the machine's (a Trixie base has it).
  find "$SYSROOT/usr/lib/$MULTIARCH" -maxdepth 1 -type l | while read -r link; do
    target=$(readlink "$link")
    case $target in
      /*) [ -e "$SYSROOT$target" ] && ln -sf "$SYSROOT$target" "$link" ;;
      *) [ -e "$SYSROOT/usr/lib/$MULTIARCH/$target" ] || ln -sf "/usr/lib/$MULTIARCH/$target" "$link" ;;
    esac
  done
  # Their pkg-config files name /usr: the sysroot's.
  find "$SYSROOT" -name "*.pc" -exec sed -i "s|^prefix=/usr|prefix=$SYSROOT/usr|; s|=/usr/include|=$SYSROOT/usr/include|; s|=/usr/lib|=$SYSROOT/usr/lib|" {} +
  touch "$SYSROOT/.complete"
fi

export PATH="$SYSROOT/usr/bin:$PATH"
export LD_LIBRARY_PATH="$PREFIX/lib:$SYSROOT/usr/lib/$MULTIARCH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig:$SYSROOT/usr/lib/$MULTIARCH/pkgconfig:$SYSROOT/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR=
export GETTEXTDATADIR="$SYSROOT/usr/share/gettext"
common="-G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$PREFIX -DCMAKE_INSTALL_LIBDIR=lib
  -DCMAKE_PREFIX_PATH=$PREFIX;$SYSROOT/usr -DENABLE_TEST=Off
  -DCMAKE_C_FLAGS=-I$SYSROOT/usr/include -DCMAKE_CXX_FLAGS=-I$SYSROOT/usr/include
  -DCMAKE_EXE_LINKER_FLAGS=-L$SYSROOT/usr/lib/$MULTIARCH -DCMAKE_SHARED_LINKER_FLAGS=-L$SYSROOT/usr/lib/$MULTIARCH
  -DCMAKE_MODULE_LINKER_FLAGS=-L$SYSROOT/usr/lib/$MULTIARCH"

build() { # name source-dir [cmake args...]
  name=$1; src=$WORK/src/$2; shift 2
  echo "building $name"
  rm -rf "$WORK/build/$name"
  # No RPATH installed (fcitx5's CMake would write the prefix and the build's sysroot): the
  # session gives fcitx5 the engine's lib/ (crates/oxyos-input-methods).
  # shellcheck disable=SC2086
  cmake -S "$src" -B "$WORK/build/$name" $common -DCMAKE_SKIP_INSTALL_RPATH=ON \
    "$@" > "$WORK/build-$name.log" 2>&1 \
    || { tail -30 "$WORK/build-$name.log"; exit 1; }
  ninja -C "$WORK/build/$name" -j "$JOBS" >> "$WORK/build-$name.log" 2>&1 \
    || { grep -E "error|FAILED" -A3 "$WORK/build-$name.log" | head -30; exit 1; }
  ninja -C "$WORK/build/$name" install >> "$WORK/build-$name.log" 2>&1 || { tail -20 "$WORK/build-$name.log"; exit 1; }
}

rm -rf "$PREFIX"
# fcitx5: the daemon, its libraries, the D-Bus module and frontend, the keyboard engine, emoji,
# QuickPhrase, punctuation's dependencies. Nothing that draws or speaks X11 or Wayland.
build fcitx5 fcitx5-5.1.12 -DENABLE_X11=Off -DENABLE_WAYLAND=Off -DENABLE_ENCHANT=Off \
  -DENABLE_DOC=Off -DENABLE_XDGAUTOSTART=Off -DBUILD_SPELL_DICT=Off -DENABLE_TESTING_ADDONS=On \
  -DUSE_SYSTEMD=On -DENABLE_EMOJI=On -DENABLE_KEYBOARD=On -DENABLE_DBUS=On
# libime: pinyin's language model and dictionaries (in Debian's tarball), built by its own tools.
build libime libime-1.1.10+dict20250327 -DENABLE_DOC=Off
build chinese-addons fcitx5-chinese-addons-5.1.8 -DENABLE_GUI=Off -DENABLE_BROWSER=Off \
  -DENABLE_CLOUDPINYIN=Off -DENABLE_OPENCC=Off -DUSE_QT6=Off
build hangul fcitx5-hangul-5.1.6
build anthy fcitx5-anthy-5.1.6

# What is not needed to run: headers, CMake and pkg-config files, static archives, the testing
# addons (which only fcitx5's own tests load).
rm -rf "$PREFIX/include" "$PREFIX/lib/cmake" "$PREFIX/lib/pkgconfig" "$PREFIX/share/pkgconfig"
find "$PREFIX" -name "*.a" -delete
for addon in testfrontend testim testui; do
  rm -f "$PREFIX/lib/fcitx5/lib$addon.so" "$PREFIX/lib/fcitx5/$addon.so" "$PREFIX/share/fcitx5/addon/$addon.conf"
done

# QuickPhrase is emoji's search here (CLDR's annotations, through the keyboard engine): not its
# lists of kaomoji and LaTeX phrases, which would come before the emoji.
rm -rf "$PREFIX/share/fcitx5/data/quickphrase.d"
# Only fcitx5 itself among its programs: no configuration tool, diagnosis script or dictionary
# tool runs on a machine.
find "$PREFIX/bin" -mindepth 1 ! -name fcitx5 -delete
# Nothing it installed may link a toolkit, X11 or Wayland.
bad=$(find "$PREFIX" -type f \( -name "*.so*" -o -perm -u+x \) -exec sh -c 'readelf -d "$1" 2>/dev/null | sed -n "s/.*(NEEDED).*\[\(.*\)\]/\1/p"' _ {} \; \
  | sort -u | grep -E "^lib(X|x(cb|kbcommon-x11)|wayland|gtk|gdk|cairo|pango|Qt|glib|gobject|gio|enchant)" || true)
[ -z "$bad" ] || { echo "the engine links what the image forbids:"; echo "$bad"; exit 1; }
# Nor name where it was built.
named=$(find "$PREFIX" -type f \( -name "*.so*" -o -perm -u+x \) -exec sh -c 'readelf -d "$1" 2>/dev/null | grep -qE "\((RPATH|RUNPATH)\)" && echo "$1"' _ {} \;)
[ -z "$named" ] || { echo "these name a library path:"; echo "$named"; exit 1; }
echo "input methods built into $PREFIX ($(du -sh "$PREFIX" | cut -f1)); run-time packages: $RUNTIME_PACKAGES"
