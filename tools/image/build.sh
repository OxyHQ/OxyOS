#!/bin/sh
# Build the console base image of config/ on a developer's machine, without touching the host.
#
# Everything runs in a privileged debian:trixie container (pinned by digest): live-build needs
# chroot, mounts and device nodes, and the container gives it them without installing anything
# on the host. The repository is mounted read-only; every byte the build writes is under the
# work directory. The result is the console base of docs/iso-build.mdx, never the OxyOS desktop,
# and is never published: no workflow runs this (scripts/check-image-config.py fails one that
# does). See docs/iso-build.mdx, "Building and testing locally".
#
#   tools/image/build.sh [build] [--arch A]  build the image for A (amd64, the default, or
#                                            arm64), then audit what it installed
#   tools/image/build.sh audit [ISO]         audit an image (default: the newest in out/)
#   tools/image/build.sh clean [--arch A]    remove A's build tree, keep its package cache
#
# The build runs natively for its architecture: an arm64 image is built in the arm64 container,
# which an x86-64 host runs through QEMU's user-mode emulation (Debian's qemu-user-binfmt, whose
# binfmt_misc entry has the F flag so the container needs no emulator of its own). live-build
# then bootstraps, installs and squashes as on an arm64 machine; nothing is cross-built.
#
# Environment:
#   OXYOS_BUILD_DIR  work directory (default ~/oxyos-build): build/<arch>/ (live-build's tree
#                    and cache/), out/ (images, logs, package lists), audit/ (extracted
#                    databases). Not /tmp: a build needs several GB, and /tmp is often RAM.
#   OXYOS_FRESH=1    drop the package and bootstrap caches before building.
#   DOCKER           the docker command (default: docker, else "sudo -n docker" when that works).
#
# Reproducible inputs: the container image by digest, live-build by version (container.sh), the
# configuration by commit (an uncommitted change marks the build "-dirty"), SOURCE_DATE_EPOCH
# from the commit's time. Debian packages come from the archive at build time; their resolved
# versions are written next to the image (.packages), so two builds can be compared.
set -eu

# debian:trixie, the image index sha256:2dc419c6d7fe3dad0a2f9fd469c34cc90b0f9ba642d8fde837fe3d7d34a0e7f1,
# pinned by each platform's manifest in it: Docker's classic image store keeps one platform per
# digest, so an index digest cannot serve both architectures on one host.
container_image() {
    case "$1" in
        amd64) echo "debian@sha256:e2914ec98e08b24b4cee3cefcb9f90dfb5519f8d215a2ccf0c9a49b10b6e43de" ;;
        arm64) echo "debian@sha256:39f862f398f8ea1a2ffee9bbe02c649be6779244bd6a834c4f71155172f801f2" ;;
    esac
}

# The machine's own architecture, in Debian's names.
host_architecture() {
    case "$(uname -m)" in
        x86_64) echo amd64 ;;
        aarch64 | arm64) echo arm64 ;;
        *) uname -m ;;
    esac
}

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${OXYOS_BUILD_DIR:-$HOME/oxyos-build}

die() {
    printf 'build.sh: %s\n' "$*" >&2
    exit 1
}

case "$WORK" in
    /tmp | /tmp/*) die "OXYOS_BUILD_DIR under /tmp: use a directory on disk" ;;
esac

if [ -z "${DOCKER:-}" ]; then
    if docker info >/dev/null 2>&1; then
        DOCKER=docker
    elif sudo -n docker info >/dev/null 2>&1; then
        DOCKER="sudo -n docker"
    else
        die "no usable docker (tried docker and sudo -n docker); set DOCKER"
    fi
fi

commit=$(git -C "$ROOT" rev-parse --short=12 HEAD)
if [ -n "$(git -C "$ROOT" status --porcelain -- config tools/image)" ]; then
    commit="${commit}-dirty"
fi
SOURCE_DATE_EPOCH=$(git -C "$ROOT" log -1 --format=%ct)

# container ARCH COMMAND...: container.sh in the pinned container of ARCH.
container() {
    # POSIX sh has no local variables: these names are the function's own.
    container_architecture=$1
    shift
    container_ref=$(container_image "$container_architecture")
    if [ "$container_architecture" != "$(host_architecture)" ] && [ "$container_architecture" = arm64 ] &&
        [ ! -e /proc/sys/fs/binfmt_misc/qemu-aarch64 ]; then
        die "an arm64 build on $(host_architecture) needs QEMU's user-mode emulation registered (Debian: qemu-user-binfmt)"
    fi
    mkdir -p "$WORK"
    # shellcheck disable=SC2086 # DOCKER may be "sudo -n docker"
    $DOCKER run --rm --privileged \
        --platform "linux/$container_architecture" \
        --name "oxyos-image-$$" \
        -v "$ROOT:/src:ro" \
        -v "$WORK:/work" \
        -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
        -e SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
        -e OXYOS_COMMIT="$commit" \
        -e OXYOS_FRESH="${OXYOS_FRESH:-0}" \
        -e OXYOS_ARCHITECTURE="$container_architecture" \
        -e OXYOS_CONTAINER_IMAGE="$container_ref" \
        "$container_ref" /bin/sh /src/tools/image/container.sh "$@"
}

# --arch A from the arguments, amd64 by default.
architecture_of() {
    architecture=amd64
    while [ $# -gt 0 ]; do
        case "$1" in
            --arch) [ $# -ge 2 ] || die "--arch needs amd64 or arm64"; architecture=$2; shift 2 ;;
            --arch=*) architecture=${1#--arch=}; shift ;;
            *) die "unknown argument $1" ;;
        esac
    done
    [ -n "$(container_image "$architecture")" ] || die "architecture $architecture: amd64 or arm64"
    echo "$architecture"
}

# newest_image [ARCH]: the newest image in out/, of ARCH when given.
newest_image() {
    ls -t "$WORK"/out/oxyos-base-*-"${1:-*}".iso 2>/dev/null | head -n 1
}

audit() {
    image=${1:-$(newest_image)}
    [ -n "$image" ] && [ -f "$image" ] || die "no image to audit (build one first)"
    image=$(cd "$(dirname "$image")" && pwd)/$(basename "$image")
    case "$image" in
        "$WORK"/*) ;;
        *) die "the image must be under $WORK (the container sees only that)" ;;
    esac
    # Extraction reads files and runs no program of the image: the machine's own container.
    container "$(host_architecture)" extract "/work${image#"$WORK"}"
    python3 "$ROOT/scripts/check-image-contents.py" "$WORK/audit/$(basename "$image" .iso)"
}

command=build
case "${1:-}" in
    build | audit | clean) command=$1; shift ;;
esac
case "$command" in
    build)
        architecture=$(architecture_of "$@")
        container "$architecture" build
        audit "$(newest_image "$architecture")"
        ;;
    audit)
        audit "$@"
        ;;
    clean)
        architecture=$(architecture_of "$@")
        container "$architecture" clean
        ;;
esac
