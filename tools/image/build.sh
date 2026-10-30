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
#   tools/image/build.sh [build]      build the image, then audit what it installed
#   tools/image/build.sh audit [ISO]  audit an image (default: the newest in out/)
#   tools/image/build.sh clean        remove the build tree, keep the package cache
#
# Environment:
#   OXYOS_BUILD_DIR  work directory (default ~/oxyos-build): build/ (live-build's tree and
#                    cache/), out/ (images, logs, package lists), audit/ (extracted databases).
#                    Not /tmp: a build needs several GB, and /tmp is often RAM.
#   OXYOS_FRESH=1    drop the package and bootstrap caches before building.
#   DOCKER           the docker command (default: docker, else "sudo -n docker" when that works).
#
# Reproducible inputs: the container image by digest, live-build by version (container.sh), the
# configuration by commit (an uncommitted change marks the build "-dirty"), SOURCE_DATE_EPOCH
# from the commit's time. Debian packages come from the archive at build time; their resolved
# versions are written next to the image (.packages), so two builds can be compared.
set -eu

CONTAINER_IMAGE="debian:trixie@sha256:2dc419c6d7fe3dad0a2f9fd469c34cc90b0f9ba642d8fde837fe3d7d34a0e7f1"

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

container() {
    mkdir -p "$WORK"
    # shellcheck disable=SC2086 # DOCKER may be "sudo -n docker"
    $DOCKER run --rm --privileged \
        --name "oxyos-image-$$" \
        -v "$ROOT:/src:ro" \
        -v "$WORK:/work" \
        -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
        -e SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
        -e OXYOS_COMMIT="$commit" \
        -e OXYOS_FRESH="${OXYOS_FRESH:-0}" \
        -e OXYOS_CONTAINER_IMAGE="$CONTAINER_IMAGE" \
        "$CONTAINER_IMAGE" /bin/sh /src/tools/image/container.sh "$@"
}

newest_image() {
    ls -t "$WORK"/out/oxyos-base-*.iso 2>/dev/null | head -n 1
}

audit() {
    image=${1:-$(newest_image)}
    [ -n "$image" ] && [ -f "$image" ] || die "no image to audit (build one first)"
    image=$(cd "$(dirname "$image")" && pwd)/$(basename "$image")
    case "$image" in
        "$WORK"/*) ;;
        *) die "the image must be under $WORK (the container sees only that)" ;;
    esac
    container extract "/work${image#"$WORK"}"
    python3 "$ROOT/scripts/check-image-contents.py" "$WORK/audit/$(basename "$image" .iso)"
}

case "${1:-build}" in
    build)
        container build
        audit "$(newest_image)"
        ;;
    audit)
        shift
        audit "$@"
        ;;
    clean)
        container clean
        ;;
    *)
        die "usage: build.sh [build] | audit [ISO] | clean"
        ;;
esac
