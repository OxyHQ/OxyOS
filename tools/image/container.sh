#!/bin/sh
# What tools/image/build.sh runs inside its privileged debian:trixie container. Not meant to be
# run on a host: it installs packages and builds as root in the current system.
#
#   container.sh build      build config/ (read-only at /src) in /work/build/<arch>, the image in
#                           /work/out
#   container.sh extract I  extract the dpkg database and the file list of image I (under /work)
#                           into /work/audit/<image name>/ for scripts/check-image-contents.py
#   container.sh clean      remove the build tree (root-owned), keeping the package cache
#
# Environment: OXYOS_ARCHITECTURE (amd64 or arm64: the image's, which must be this container's
# own), HOST_UID, HOST_GID (owner of what is left in /work/out and /work/audit),
# SOURCE_DATE_EPOCH, OXYOS_COMMIT, OXYOS_FRESH (1: drop the package and bootstrap caches first).
set -eu

LIVE_BUILD_VERSION="1:20250505+deb13u1"

say() {
    printf 'P: %s\n' "$*"
}

die() {
    printf 'E: %s\n' "$*" >&2
    exit 1
}

[ -f /.dockerenv ] || [ -n "${OXYOS_IN_CONTAINER:-}" ] || die "run through tools/image/build.sh, not on a host"

ARCHITECTURE=${OXYOS_ARCHITECTURE:-amd64}
BUILD=/work/build/$ARCHITECTURE

install_tools() {
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -q >/dev/null
    # shellcheck disable=SC2068 # one word per package
    apt-get install -y -q --no-install-recommends $@ >/dev/null
}

hand_back() {
    chown -R "${HOST_UID:-0}:${HOST_GID:-0}" "$@"
}

build() {
    [ -d /src/config ] || die "/src/config missing: mount the repository at /src"
    # A native build: live-build bootstraps and runs the chroot with this container's own
    # architecture, so the image's must be it (build.sh runs the arm64 container for arm64).
    [ "$(dpkg --print-architecture)" = "$ARCHITECTURE" ] ||
        die "an $ARCHITECTURE image in a $(dpkg --print-architecture) container: run through tools/image/build.sh --arch"
    # config/bootstrap takes the architecture from here, and live-build reads the
    # config/<file>.<architecture> overrides by it.
    export LB_ARCHITECTURE="$ARCHITECTURE"
    # The pinned live-build: another version may build a different image from the same config.
    install_tools "live-build=${LIVE_BUILD_VERSION}" apt-utils cpio file wget xz-utils bzip2 rsync \
        ca-certificates || die "live-build ${LIVE_BUILD_VERSION} is not installable; update LIVE_BUILD_VERSION in tools/image/container.sh deliberately"

    mkdir -p "$BUILD" /work/out
    cd "$BUILD"
    if [ "${OXYOS_FRESH:-0}" = 1 ]; then
        say "dropping the package and bootstrap caches"
        rm -rf cache
    fi
    # A clean tree each time: only cache/ (downloaded .debs, the bootstrap stage) survives.
    if [ -d config ]; then
        lb clean --chroot --binary --stage --source >/dev/null 2>&1 || true
    fi
    find . -mindepth 1 -maxdepth 1 ! -name cache -exec rm -rf --one-file-system {} +
    cp -a /src/config config
    # lb build runs lb config when .build/config is missing, which would rewrite config/ with
    # live-build's defaults: the configuration is complete as committed.
    mkdir -p .build
    touch .build/config

    log=/work/out/build-${ARCHITECTURE}.log
    say "lb build ${ARCHITECTURE} (SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-unset}, commit ${OXYOS_COMMIT:-unknown}); log in out/build-${ARCHITECTURE}.log"
    started=$(date +%s)
    if ! lb build >"$log" 2>&1; then
        tail -n 40 "$log" >&2
        hand_back /work/out
        die "lb build failed; full log in out/build-${ARCHITECTURE}.log"
    fi
    finished=$(date +%s)

    . config/oxyos.conf
    iso=$(ls ./*.hybrid.iso 2>/dev/null | head -n 1)
    [ -n "$iso" ] || die "lb build produced no .hybrid.iso"
    # "base": the console base system of config/, never the OxyOS desktop (docs/iso-build.mdx).
    name="oxyos-base-${OS_VERSION}-${OXYOS_COMMIT:-unknown}-${ARCHITECTURE}"
    mv "$iso" "/work/out/${name}.iso"
    cp chroot.packages.live "/work/out/${name}.packages"
    {
        printf 'commit=%s\n' "${OXYOS_COMMIT:-unknown}"
        printf 'architecture=%s\n' "$ARCHITECTURE"
        printf 'source_date_epoch=%s\n' "${SOURCE_DATE_EPOCH:-}"
        printf 'build_seconds=%s\n' "$((finished - started))"
        dpkg-query -W --showformat='container_${Package}=${Version}\n' live-build debootstrap
        printf 'container_image=%s\n' "${OXYOS_CONTAINER_IMAGE:-}"
    } >"/work/out/${name}.build-info"
    (cd /work/out && sha256sum "${name}.iso" >"${name}.iso.sha256")
    hand_back /work/out
    say "image: out/${name}.iso ($((finished - started)) s)"
}

extract() {
    image=$1
    [ -f "$image" ] || die "no image at $image"
    install_tools squashfs-tools xorriso initramfs-tools-core cpio zstd
    name=$(basename "$image" .iso)
    dir=/work/audit/$name
    rm -rf "$dir"
    mkdir -p "$dir"
    # The ISO's own file list (the bootloaders, the kernel, the installer), with sizes.
    xorriso -indev "$image" -find / -type f -exec lsdl 2>/dev/null >"$dir/iso-files.txt"
    xorriso -osirrox on -indev "$image" -extract /live/filesystem.squashfs "$dir/filesystem.squashfs" \
        >/dev/null 2>&1
    # Every path of the live root with its size, and the dpkg database the packages were
    # installed into: what the image is, rather than what the manifest asks for.
    unsquashfs -lls "$dir/filesystem.squashfs" >"$dir/files.txt"
    # The dpkg database, os-release, and every file config/ includes, to compare with the source.
    {
        printf '%s\n' var/lib/dpkg/status var/lib/dpkg/info var/lib/dpkg/diversions etc/os-release
        (cd /src/config/includes.chroot_after_packages && find . -type f | sed 's|^\./||')
    } >"$dir/extract-list"
    unsquashfs -no-progress -d "$dir/root" -ef "$dir/extract-list" "$dir/filesystem.squashfs" >/dev/null
    # The initramfs: the boot logo must be in it, as Plymouth starts there.
    # live-build names it /live/initrd.img for syslinux (amd64); GRUB alone (arm64) keeps only the
    # kernel's versioned name. The live medium has one kernel.
    initrd=$(sed -n "s|.*'\(/live/initrd\.img[^']*\)'\$|\1|p" "$dir/iso-files.txt" | sort | head -n 1)
    [ -n "$initrd" ] || die "no /live/initrd.img* on $image"
    xorriso -osirrox on -indev "$image" -extract "$initrd" "$dir/initrd.img" >/dev/null 2>&1
    unmkinitramfs "$dir/initrd.img" "$dir/initrd-tree"
    (cd "$dir/initrd-tree" && find . | sed 's|^\./||' | sort) >"$dir/initrd-files.txt"
    mkdir -p "$dir/initrd/etc"
    main=$(find "$dir/initrd-tree" -path '*/etc/plymouth' -type d | head -n 1)
    [ -z "$main" ] || cp -a "$main" "$dir/initrd/etc/plymouth"
    rm -rf "$dir/initrd-tree" "$dir/initrd.img"
    stat -c %s "$dir/filesystem.squashfs" >"$dir/squashfs-size"
    stat -c %s "$image" >"$dir/iso-size"
    rm -f "$dir/filesystem.squashfs"
    hand_back "$dir"
    say "extracted $name into audit/$name"
}

clean() {
    if [ -d "$BUILD" ]; then
        cd "$BUILD"
        lb clean --chroot --binary --stage --source >/dev/null 2>&1 || true
        find . -mindepth 1 -maxdepth 1 ! -name cache -exec rm -rf --one-file-system {} +
    fi
    say "build tree removed; the package cache is kept (OXYOS_FRESH=1 drops it on the next build)"
}

case "${1:-}" in
    build) build ;;
    extract) shift; extract "$@" ;;
    clean) clean ;;
    *) die "usage: container.sh build | extract IMAGE | clean" ;;
esac
