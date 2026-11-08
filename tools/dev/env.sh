# Where tools/dev/bootstrap puts what it fetches and builds, and what the gate and a deploy read
# from it (docs/developing.mdx). Sourced by sh (tools/dev/bootstrap, tools/gate/run-scenarios.sh,
# tools/deploy/deploy.sh), and by a developer's shell to use the same tools:
#   . tools/dev/env.sh
# The machine's own values (another cache, a local Bloom checkout, the deploy's host) go in
# tools/dev/local.conf, ignored by git (tools/dev/local.conf.example says what it may hold).
# The scripts set OXYOS_TREE to their checkout; a shell sources this from the checkout's root.
OXYOS_TREE=${OXYOS_TREE:-$(pwd)}
[ -f "$OXYOS_TREE/tools/dev/env.sh" ] || { echo "env.sh: source it from an OxyOS checkout's root, or set OXYOS_TREE" >&2; return 1 2>/dev/null || exit 1; }
[ -f "$OXYOS_TREE/tools/dev/local.conf" ] && . "$OXYOS_TREE/tools/dev/local.conf"
OXYOS_CACHE=${OXYOS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/oxyos}
case $OXYOS_CACHE in /tmp | /tmp/*) echo "env.sh: OXYOS_CACHE under /tmp: use a directory on disk" >&2; return 1 2>/dev/null || exit 1 ;; esac
# The host's toolchain (tools/rn-host-spike/toolchain.sh), the input methods' engine, the
# applications' checkouts at their pins (tools/dev/pins) unless local.conf names others.
OXYOS_TOOLCHAIN=${OXYOS_TOOLCHAIN:-$OXYOS_CACHE/toolchain}
OXYOS_INPUT_METHODS_WORK=$OXYOS_CACHE/input-methods
BLOOM_DIR=${BLOOM_DIR:-$OXYOS_CACHE/src/Bloom}
DROPSEA_DIR=${DROPSEA_DIR:-$OXYOS_CACHE/src/Dropsea/packages/frontend}
ATLAS_DIR=${ATLAS_DIR:-$OXYOS_CACHE/src/Atlas/packages/frontend}
# Node and bun as bootstrap fetched them, and rustup's cargo.
PATH=$OXYOS_CACHE/node/bin:$OXYOS_CACHE/bun/bin:${CARGO_HOME:-$HOME/.cargo}/bin:$PATH
export OXYOS_TREE OXYOS_CACHE OXYOS_TOOLCHAIN BLOOM_DIR DROPSEA_DIR ATLAS_DIR PATH
case $(dpkg --print-architecture 2>/dev/null) in
  amd64) OXYOS_ARCH=amd64 ;;
  arm64) OXYOS_ARCH=arm64 ;;
  *) OXYOS_ARCH=unsupported ;;
esac
