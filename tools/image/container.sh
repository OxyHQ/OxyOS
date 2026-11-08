#!/bin/sh
# What tools/image/build.sh runs inside its privileged debian:trixie container. Not meant to be
# run on a host: it installs packages and builds as root in the current system.
#
#   container.sh build      build config/ (read-only at /src) in /work/build/<arch>, the image in
#                           /work/out
#   container.sh extract I  extract the dpkg database and the file list of image I (under /work)
#                           into /work/audit/<image name>/ for scripts/check-image-contents.py
#   container.sh clean      remove the build tree (root-owned), keeping the package cache
#   container.sh packages   build OxyOS's patched Debian packages (tools/image/packages/) and its
#                           own programs (tools/image/programs/) into /work/packages/<arch>
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

# Docker marks its containers with /.dockerenv, Podman with /run/.containerenv.
[ -f /.dockerenv ] || [ -f /run/.containerenv ] || [ -n "${OXYOS_IN_CONTAINER:-}" ] || die "run through tools/image/build.sh, not on a host"

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

# OxyOS's patched Debian packages (tools/image/packages/<source>/): each Debian source package,
# fetched from the archive and checked against its pinned SHA-256s, OxyOS's patches added to its
# own (quilt) series, rebuilt as OxyOS's version, and its binary packages the image takes left in
# /work/packages/<arch>/. A source whose inputs are unchanged since its last build is not built
# again (a stamp of the inputs beside its packages).
packages() {
    [ "$(dpkg --print-architecture)" = "$ARCHITECTURE" ] ||
        die "$ARCHITECTURE packages in a $(dpkg --print-architecture) container: run through tools/image/build.sh --arch"
    out=/work/packages/$ARCHITECTURE
    sources=/work/packages/sources
    mkdir -p "$out" "$sources"
    # What is left under /work/packages is the host user's, whether the build succeeds or not.
    trap 'hand_back /work/packages' EXIT
    install_tools dpkg-dev build-essential fakeroot quilt curl ca-certificates
    for dir in /src/tools/image/packages/*/; do
        [ -f "$dir/source" ] || continue
        # shellcheck disable=SC1091 # the package's own pin
        . "$dir/source"
        stamp=$(cat "$dir"/source "$dir"/*.patch | sha256sum | cut -d' ' -f1)
        # The archive must not have moved past the pinned Debian version: OxyOS's packages would
        # keep the image on the older one, its fixes lost.
        archive=$(apt-cache show "$SOURCE" 2>/dev/null | sed -n 's/^Version: //p' | sort -V | tail -n 1)
        if [ -n "$archive" ] && dpkg --compare-versions "$archive" gt "$DEBIAN_VERSION"; then
            die "Debian has $SOURCE $archive, newer than tools/image/packages/$SOURCE pins ($DEBIAN_VERSION): rebase the patches onto it"
        fi
        if [ -f "$out/$SOURCE.stamp" ] && [ "$(cat "$out/$SOURCE.stamp")" = "$stamp" ]; then
            say "$SOURCE $VERSION: up to date"
            continue
        fi
        pool="https://deb.debian.org/debian/pool/main/$(printf %.1s "$SOURCE")/$SOURCE"
        dsc="${SOURCE}_${DEBIAN_VERSION}.dsc"
        for pair in "$dsc $DSC_SHA256" "$ORIG $ORIG_SHA256" "$DEBIAN_TARBALL $DEBIAN_SHA256"; do
            set -- $pair
            [ -f "$sources/$1" ] || curl -fsSL --retry 5 -o "$sources/$1" "$pool/$1"
            echo "$2  $sources/$1" | sha256sum -c --quiet || die "$1 does not match its pinned SHA-256"
        done
        tree=/work/packages/build/$ARCHITECTURE/$SOURCE
        rm -rf "$tree" && mkdir -p "$tree"
        # Unpacked as <source>-<upstream version>, the directory its tarball has: Plymouth's build
        # takes its version from that name when it is not a git checkout.
        src=$tree/$SOURCE-${DEBIAN_VERSION%-*}
        dpkg-source -x --skip-patches "$sources/$dsc" "$src" >/dev/null
        for patch in "$dir"/*.patch; do
            name=oxyos-$(basename "$patch")
            cp "$patch" "$src/debian/patches/$name"
            echo "$name" >>"$src/debian/patches/series"
        done
        (cd "$src" && QUILT_PATCHES=debian/patches quilt push -a -q >/dev/null) ||
            die "$SOURCE: OxyOS's patches do not apply to Debian's $DEBIAN_VERSION"
        changelog=$src/debian/changelog
        {
            printf '%s (%s) trixie; urgency=medium\n\n' "$SOURCE" "$VERSION"
            for patch in "$dir"/*.patch; do
                printf '  * OxyOS: %s\n' "$(sed -n 's/^Subject: \(\[PATCH[^]]*\] \)\{0,1\}//p' "$patch" | head -n 1)"
            done
            printf '\n -- %s  %s\n\n' "$MAINTAINER" "$DATE"
            cat "$changelog"
        } >"$changelog.new"
        mv "$changelog.new" "$changelog"
        apt-get build-dep -y -q --no-install-recommends "$src" >/dev/null
        log=/work/packages/$SOURCE-$ARCHITECTURE.log
        say "building $SOURCE $VERSION; log in packages/$SOURCE-$ARCHITECTURE.log"
        # The files' times are the changelog entry's, not the image's commit: the stamp says the
        # same inputs, so the packages must be the same.
        if ! (cd "$src" && SOURCE_DATE_EPOCH=$(date -d "$DATE" +%s) DEB_BUILD_OPTIONS="nocheck parallel=$(nproc)" \
            dpkg-buildpackage -b -uc -us) >"$log" 2>&1; then
            tail -n 40 "$log" >&2
            die "$SOURCE failed to build"
        fi
        for binary in $BINARIES; do
            rm -f "$out/${binary}"_*.deb
            deb="$tree/${binary}_${VERSION}_${ARCHITECTURE}.deb"
            [ -f "$deb" ] || die "$SOURCE built no $(basename "$deb")"
            cp "$deb" "$out/"
        done
        echo "$stamp" >"$out/$SOURCE.stamp"
        rm -rf "$tree"
        say "$SOURCE $VERSION: $BINARIES in packages/$ARCHITECTURE"
    done
    programs
}

# The version of OxyOS's own packages: the OS's version and the commit's time, so a later commit's
# packages are newer and the same commit's are the same.
program_version() {
    . /src/config/oxyos.conf
    printf '%s.%s' "${OS_VERSION}" "${SOURCE_DATE_EPOCH:-0}"
}

# OxyOS's own programs (tools/image/programs/<name>/program): each built from the Cargo workspace
# with the toolchain rust-toolchain.toml pins (rustup's, kept under /work/packages/rust) and
# Cargo.lock (--locked), and packaged with its files as /work/packages/<arch>/<package>_<version>_<arch>.deb.
# Debian's Rust is older than the workspace's toolchain; the crates are checked against Cargo.lock's
# checksums as they are fetched.
programs() {
    out=/work/packages/$ARCHITECTURE
    rust=/work/packages/rust
    install_tools rustup ca-certificates build-essential pkg-config
    toolchain=$(sed -n 's/^channel = "\(.*\)"$/\1/p' /src/rust-toolchain.toml)
    [ -n "$toolchain" ] || die "no channel in rust-toolchain.toml"
    export RUSTUP_HOME="$rust/rustup" CARGO_HOME="$rust/cargo"
    rustup toolchain install "$toolchain" --profile minimal >/dev/null 2>&1 ||
        die "rustup could not install Rust $toolchain"
    version=$(program_version)
    for program in /src/tools/image/programs/*/program; do
        [ -f "$program" ] || continue
        # shellcheck disable=SC1090 # the program's own pin
        . "$program"
        say "building $PACKAGE $version from $CRATE"
        log=/work/packages/$PACKAGE-$ARCHITECTURE.log
        if ! (cd /src && rustup run "$toolchain" cargo build --release --locked -p "$CRATE" \
            --target-dir "$rust/target-$ARCHITECTURE") >"$log" 2>&1; then
            tail -n 40 "$log" >&2
            die "$CRATE failed to build"
        fi
        tree=/work/packages/build/$ARCHITECTURE/$PACKAGE
        rm -rf "$tree" && mkdir -p "$tree/DEBIAN"
        for binary in $BINARIES; do
            name=${binary%%:*}
            install -D -m 0755 "$rust/target-$ARCHITECTURE/release/$name" "$tree/${binary#*:}"
        done
        cp -a "/src/$FILES/." "$tree/"
        mkdir -p "$tree/usr/share/doc/$PACKAGE"
        printf '%s\n' "$PACKAGE: OxyOS's own program, built from https://github.com/OxyHQ/OxyOS at ${OXYOS_COMMIT:-unknown}." \
            "License: see the repository." >"$tree/usr/share/doc/$PACKAGE/copyright"
        size=$(du -sk --exclude=DEBIAN "$tree" | cut -f1)
        {
            printf 'Package: %s\nVersion: %s\nArchitecture: %s\n' "$PACKAGE" "$version" "$ARCHITECTURE"
            printf 'Maintainer: OxyHQ <hello@oxy.so>\nSection: admin\nPriority: optional\n'
            printf 'Installed-Size: %s\nDepends: %s\nDescription: %s\n' "$size" "$DEPENDS" "$DESCRIPTION"
        } >"$tree/DEBIAN/control"
        find "$tree" -exec touch -h -d "@${SOURCE_DATE_EPOCH:-0}" {} +
        rm -f "$out/${PACKAGE}"_*.deb
        dpkg-deb --root-owner-group --build "$tree" "$out/${PACKAGE}_${version}_${ARCHITECTURE}.deb" >/dev/null
        rm -rf "$tree"
        say "$PACKAGE $version in packages/$ARCHITECTURE"
    done
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
    # cryptsetup-bin: live-build's binary_dm-verity runs veritysetup on the build's own system, and
    # without it writes an empty root hash and no hash tree, saying nothing.
    install_tools "live-build=${LIVE_BUILD_VERSION}" apt-utils cpio file wget xz-utils bzip2 rsync \
        ca-certificates cryptsetup-bin || die "live-build ${LIVE_BUILD_VERSION} is not installable; update LIVE_BUILD_VERSION in tools/image/container.sh deliberately"

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
    # OxyOS's patched Debian packages (packages(), run first by build.sh): live-build installs
    # every package of config/packages.chroot from a local repository, above the archive's.
    for dir in /src/tools/image/packages/*/; do
        [ -f "$dir/source" ] || continue
        # shellcheck disable=SC1091 # the package's own pin
        . "$dir/source"
        mkdir -p config/packages.chroot
        for binary in $BINARIES; do
            deb=/work/packages/$ARCHITECTURE/${binary}_${VERSION}_${ARCHITECTURE}.deb
            [ -f "$deb" ] || die "no $(basename "$deb"): tools/image/build.sh packages builds it"
            cp "$deb" config/packages.chroot/
        done
    done
    # OxyOS's own programs (programs(), run by packages first).
    version=$(program_version)
    for program in /src/tools/image/programs/*/program; do
        [ -f "$program" ] || continue
        # shellcheck disable=SC1090 # the program's own pin
        . "$program"
        deb=/work/packages/$ARCHITECTURE/${PACKAGE}_${version}_${ARCHITECTURE}.deb
        [ -f "$deb" ] || die "no $(basename "$deb"): tools/image/build.sh packages builds it"
        mkdir -p config/packages.chroot
        cp "$deb" config/packages.chroot/
    done
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

    # The image's dm-verity hash tree and root hash (LB_DM_VERITY): the installer and the recovery
    # refuse an image without them.
    if [ "$(sed -n 's/^LB_DM_VERITY="\(.*\)"$/\1/p' config/binary)" = true ]; then
        [ -s binary/live/filesystem.squashfs.verity ] && grep -qE '^[0-9a-f]{64}$' binary/live/filesystem.squashfs.roothash ||
            die "the image has no dm-verity hash tree or root hash"
    fi
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
    packages) packages ;;
    *) die "usage: container.sh build | extract IMAGE | clean | packages" ;;
esac
