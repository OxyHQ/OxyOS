#!/bin/sh
# Deploys this tree to a test machine (docs/developing.mdx): runs the scenario gate
# (tools/gate/run-scenarios.sh, which bootstraps), then stages what passed as the image installs it and
# sends it, and the test machine builds the session and installs everything (tools/deploy/remote.sh).
#   /usr/lib/oxyos/runtimes/1/                 the runtime (root's)
#   /usr/lib/oxyos/apps/<id>/                  Settings, the shell, the login screen, Dropsea, Atlas, Activity Monitor (root's)
#   /usr/lib/oxyos/input-methods/              the input methods' engine (root's)
#   /usr/share/oxyos/applications/<id>.desktop the system applications' entries (root's)
#   /usr/share/oxyos/system-apps.json          the policy naming them (root's)
#   /usr/share/sounds/oxyos, /usr/share/oxyos/wallpapers  the sound theme and wallpapers (root's)
#   ~/oxyos-validation/packages, registry/     Inbox and Allo, development entries (the user's)
# What is built is a commit: a tree (or a checkout named in tools/dev/local.conf) with changes not
# committed is refused, so what passes and what is sent are one known state.
#
# Usage: tools/deploy/deploy.sh [--host USER@HOST] [--stage-only]
#   --host        the test machine (default: DEPLOY_HOST from tools/dev/local.conf); it needs ssh by
#                 key, passwordless sudo and rustup's cargo
#   --stage-only  build, test and stage, and send nothing
# Environment: ASTRO_PACKAGE (an unpacked astro-browser package or its .tar.xz): every astro*.json
# scenario runs against that engine as `--client` before anything is sent.
set -eu
OXYOS_TREE=$(cd "$(dirname "$0")/../.." && pwd)
TREE=$OXYOS_TREE
. "$TREE/tools/dev/env.sh"
HOST=${DEPLOY_HOST:-}; STAGE_ONLY=
while [ $# -gt 0 ]; do
  case $1 in
    --host) HOST=$2; shift 2 ;;
    --stage-only) STAGE_ONLY=1; shift ;;
    *) sed -n '15,19p' "$0" >&2; exit 2 ;;
  esac
done
[ -n "$HOST" ] || [ -n "$STAGE_ONLY" ] || { echo "deploy.sh: --host USER@HOST (or DEPLOY_HOST in tools/dev/local.conf)" >&2; exit 2; }
clean() { [ -z "$(git -C "$1" status --porcelain --untracked-files=no)" ] || { echo "$1 has uncommitted changes:"; git -C "$1" status --short --untracked-files=no | head; exit 1; }; }
clean "$TREE"
# What the test machine installs for the login, accounts and PIN is the tree's (tools/deploy/system-files).
python3 "$TREE/tools/deploy/system-files.py"
for d in "$BLOOM_DIR" "$DROPSEA_DIR" "$ATLAS_DIR"; do [ -e "$d" ] && clean "$d"; done

# Each deploy in a directory of its own (the last five kept), so a deploy beside a gate run never
# uses the other's files.
mkdir -p "$OXYOS_CACHE/deploys"
ls -dt "$OXYOS_CACHE"/deploys/* 2>/dev/null | tail -n +6 | xargs -r rm -rf
R=$OXYOS_CACHE/deploys/$(date +%Y%m%d-%H%M%S)-$(git -C "$TREE" rev-parse --short HEAD)
mkdir -p "$R"; M=$R/machine
echo "deploy: $R"

# The gate, in a deploy's gate slice when the machine has one (tools/gate/machine.sh) and sudo may put
# it there: the machine's own work keeps its priority, the runners' gates their share, and the
# timing scenarios get the reserved cores.
if systemctl cat oxyosgate-deploy.slice >/dev/null 2>&1 && ! grep -q oxyosgate /proc/self/cgroup && sudo -n true 2>/dev/null; then
  sudo -n --preserve-env systemd-run --scope --quiet --slice=oxyosgate-deploy.slice --uid="$(id -u)" --gid="$(id -g)" -- \
    env PATH="$PATH" HOME="$HOME" sh "$TREE/tools/gate/run-scenarios.sh" --out "$R"
else
  sh "$TREE/tools/gate/run-scenarios.sh" --out "$R"
fi
IM=$(cat "$M/input-methods")
IM_PACKAGES="$(PRINT_RUNTIME_PACKAGES=1 sh "$TREE/tools/input-methods/build.sh") fonts-noto-cjk fonts-noto-color-emoji"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$TREE/target}
cd "$TREE"

# ASTRO_PACKAGE: every astro*.json scenario against that engine as `--client`, with one retry.
if [ -n "${ASTRO_PACKAGE:-}" ]; then
  A=$ASTRO_PACKAGE
  case $A in *.tar.xz) mkdir -p "$R/astro" && tar -xJf "$A" -C "$R/astro" && A=$(ls -d "$R"/astro/*/ | head -1) ;; esac
  [ -x "$A/chrome" ] || { echo "ASTRO_PACKAGE: no chrome in $A"; exit 1; }
  for f in tools/rn-host-spike/scenarios/astro*.json; do
    n=$(basename "$f" .json)
    run() { python3 -X faulthandler tools/rn-host-spike/regress.py "$f" --offscreen "$CARGO_TARGET_DIR/release/oxyos-offscreen" \
      --client "$A/chrome" --renderer cpu --runtimes "$M/runtimes" --packages "$M/packages" --out "$R/astro-runs/$n" > "$R/astro-$n.txt" 2>&1; }
    run || { echo "scenario $n failed once, again:"; grep FAIL "$R/astro-$n.txt" | head -3 | cut -c1-200; rm -rf "$R/astro-runs/$n"; run; } \
      || { echo "scenario $n failed:"; grep FAIL "$R/astro-$n.txt" | head; exit 1; }
  done
  echo "astro scenarios passed"
fi

# Where the test machine keeps the user's applications: its home (asked only when sending; a staged
# run touches no other machine).
REMOTE_HOME=/home/user
[ -n "$STAGE_ONLY" ] || REMOTE_HOME=$(ssh -o BatchMode=yes "$HOST" 'echo $HOME')
# What goes to the test machine, staged as it is installed there.
D=$R/deploy; rm -rf $D && mkdir -p $D/runtimes $D/apps $D/entries $D/packages $D/registry/oxyos/applications
cp -a $M/runtimes/1 $D/runtimes/1
# The input-method engine, as the scenarios ran it, and the packages it needs there.
if [ -n "$IM" ]; then
  cp -a $IM $D/input-methods && rm -f $D/input-methods/.oxyos-build
  echo "$IM_PACKAGES" > $D/input-methods.packages
fi
# The system sound theme (docs/adr/0044-system-sounds.mdx), as the image installs it, with its
# provenance; only in a tree that has it.
if [ -d config/includes.chroot_after_packages/usr/share/sounds/oxyos ]; then
  cp -a config/includes.chroot_after_packages/usr/share/sounds/oxyos $D/sounds
  cp -a config/includes.chroot_after_packages/usr/share/doc/oxyos-sounds $D/sounds-doc
fi
# The wallpapers (docs/adr/0048, 7 November 2026), as the image installs them, with their
# provenance; only in a tree that has them.
if [ -d config/includes.chroot_after_packages/usr/share/oxyos/wallpapers ]; then
  cp -a config/includes.chroot_after_packages/usr/share/oxyos/wallpapers $D/wallpapers
  [ -d config/includes.chroot_after_packages/usr/share/doc/oxyos-wallpapers ] && cp -a config/includes.chroot_after_packages/usr/share/doc/oxyos-wallpapers $D/wallpapers-doc
fi
system="so.oxy.settings so.oxy.shell so.oxy.login"; [ -d $M/packages/so.oxy.dropsea ] && system="$system so.oxy.dropsea"
[ -d $M/packages/so.oxy.aboutthiscomputer ] && system="$system so.oxy.aboutthiscomputer"
[ -d $M/packages/so.oxy.atlas ] && system="$system so.oxy.atlas"
[ -d $M/packages/so.oxy.activitymonitor ] && system="$system so.oxy.activitymonitor"
for id in $system; do cp -a $M/packages/$id $D/apps/$id; done
for id in so.oxy.inbox so.oxy.allo; do cp -a $M/packages/$id $D/packages/$id; done
# The system applications' entries: their name, icon and module, and the package they run (the
# policy names the same host and bundle, and holds the grants).
entry() { # id name name-es module [pinned | nodisplay]
  { echo "# OxyOS's $2, a system application (docs/adr/0030-installing-applications.mdx, decision 13)."
    echo "[Desktop Entry]"; echo "Type=Application"; echo "Name=$2"; [ -n "$3" ] && echo "Name[es]=$3"
    echo "Icon=/usr/lib/oxyos/apps/$1/icon.png"; echo "X-OxyOS-Bloom-Module=$4"
    echo "X-OxyOS-Bloom-Bundle=/usr/lib/oxyos/apps/$1/index.hbc"; echo "X-OxyOS-Runtime=1"
    case "${5:-}" in "") ;; nodisplay) echo "NoDisplay=true" ;; *) echo "X-OxyOS-Pinned=$5" ;; esac
    # What the package declares it opens (its URL schemes, ADR 0034's OpenUrl), as its own entry says.
    grep "^MimeType=" $M/packages/applications/$1.desktop 2>/dev/null || true; } > $D/entries/$1.desktop
}
entry so.oxy.settings Settings Ajustes OxyOSSettings
# Settings' grants as ADR 0030 gives them: `settings`, `updates` (Software Update, decision 16) and
# `privacy` (the Privacy page, decision 8); and `network` (Wi-Fi and Network, ADR 0038) when the
# session built knows it (an older tree's policy reader refuses a grant it does not know).
grants='"settings", "updates", "privacy"'; grep -q '"network" => &mut grants.network' crates/oxyos-apps/src/lib.rs && grants="$grants, \"network\""
# Bluetooth (ADR 0055, 7 November 2026): Settings' Bluetooth page, only in a tree that has the grant.
grep -q '"bluetooth" => &mut grants.bluetooth' crates/oxyos-apps/src/lib.rs && grants="$grants, \"bluetooth\""
policy='"so.oxy.settings": { "entry": "/usr/share/oxyos/applications/so.oxy.settings.desktop", "bloom": { "host": "/usr/lib/oxyos/runtimes/1/oxyos-bloom-host", "bundle": "/usr/lib/oxyos/apps/so.oxy.settings/index.hbc" }, "grants": ['"$grants"'] }'
if [ -d $D/apps/so.oxy.aboutthiscomputer ]; then
  # About This Computer, the Oxy menu's first row: never listed (NoDisplay), with no grant; it reads
  # the facts Settings › About shows, which need none.
  entry so.oxy.aboutthiscomputer "About This Computer" "Acerca de este ordenador" OxyOSAboutThisComputer nodisplay
  policy="$policy,"' "so.oxy.aboutthiscomputer": { "entry": "/usr/share/oxyos/applications/so.oxy.aboutthiscomputer.desktop", "bloom": { "host": "/usr/lib/oxyos/runtimes/1/oxyos-bloom-host", "bundle": "/usr/lib/oxyos/apps/so.oxy.aboutthiscomputer/index.hbc" }, "grants": [] }'
fi
if [ -d $D/apps/so.oxy.dropsea ]; then
  # Dropsea first in the dock, as the development registry places it.
  entry so.oxy.dropsea Dropsea "" Dropsea 1
  policy="$policy,"' "so.oxy.dropsea": { "entry": "/usr/share/oxyos/applications/so.oxy.dropsea.desktop", "bloom": { "host": "/usr/lib/oxyos/runtimes/1/oxyos-bloom-host", "bundle": "/usr/lib/oxyos/apps/so.oxy.dropsea/index.hbc" }, "grants": [] }'
fi
if [ -d $D/apps/so.oxy.atlas ]; then
  # Atlas after Dropsea in the dock, as a Mac keeps the App Store, with the policy's `store` (installing
  # and removing, its alone) and `updates` (the account's updates, as Settings' Software Update).
  entry so.oxy.atlas Atlas "" Atlas 2
  policy="$policy,"' "so.oxy.atlas": { "entry": "/usr/share/oxyos/applications/so.oxy.atlas.desktop", "bloom": { "host": "/usr/lib/oxyos/runtimes/1/oxyos-bloom-host", "bundle": "/usr/lib/oxyos/apps/so.oxy.atlas/index.hbc" }, "grants": ["store", "updates"] }'
fi
if [ -d $D/apps/so.oxy.activitymonitor ]; then
  # Activity Monitor (docs/adr/0063-activity-monitor.mdx), not in the dock, with the policy's
  # `activity`: the machine's use and every process's, and quitting the person's own.
  entry so.oxy.activitymonitor "Activity Monitor" "Monitor de Actividad" OxyOSActivityMonitor
  policy="$policy,"' "so.oxy.activitymonitor": { "entry": "/usr/share/oxyos/applications/so.oxy.activitymonitor.desktop", "bloom": { "host": "/usr/lib/oxyos/runtimes/1/oxyos-bloom-host", "bundle": "/usr/lib/oxyos/apps/so.oxy.activitymonitor/index.hbc" }, "grants": ["activity"] }'
fi
printf '{ "applications": { %s } }\n' "$policy" | python3 -m json.tool > $D/system-apps.json
# Inbox and Allo: development entries, as oxyos-pack writes them, naming where they are on the
# ThinkPad, with the development registry's place in the dock.
for id in so.oxy.inbox so.oxy.allo; do
  { sed "s|$M/packages/$id|$REMOTE_HOME/oxyos-validation/packages/$id|g" $M/packages/applications/$id.desktop
    grep "^X-OxyOS-Pinned=" tools/rn-host-spike/applications/oxyos/applications/$id.desktop || true; } > $D/registry/oxyos/applications/$id.desktop
done
! grep -rl "$OXYOS_CACHE" $D/entries $D/registry $D/system-apps.json || { echo "a staged entry names this machine's paths"; exit 1; }
cat > $D/probar.sh <<'PROBAR'
#!/bin/sh
# Prueba OxyOS a mano en la sesión real (oxyos-session), con el ratón y el teclado del portátil:
# el runtime y las aplicaciones como los instala la imagen (/usr/lib/oxyos, ADR 0030), Inbox y Allo
# como aplicaciones de desarrollo (packages/, registry/).
#   ./probar.sh                 el escritorio de OxyOS (el shell)
#   PUNTERO=ruta[,TAMAÑO]       otro cursor (por defecto, los de macOS de cursores/)
#   DISPOSICION=es              otra distribución de teclado (por defecto, la del sistema)
set -u
here=$(cd "$(dirname "$0")" && pwd)
export XLOCALEDIR="$here/oxyos-try/config/includes.chroot_after_packages/usr/share/X11/locale"
# El fondo sigue al sol (ADR 0012).
# The wallpapers come from /usr/share/oxyos/wallpapers (ADR 0048), installed by the deploy.
# Las aplicaciones de desarrollo (ADR 0013), tras las del sistema (system-apps.json).
export XDG_DATA_DIRS="$here/registry:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
set -- --shell /usr/lib/oxyos/runtimes/1/oxyos-bloom-host,/usr/lib/oxyos/apps/so.oxy.shell/index.hbc,OxyOSShell
puntero=${PUNTERO:-"$here/cursores/1. Sierra and newer/2. With Shadow/1. Normal"}
[ -e "${puntero%,*}" ] && set -- --pointer "$puntero" "$@"
[ -n "${DISPOSICION:-}" ] && set -- --layout "$DISPOSICION" "$@"
exec "$here/oxyos-try/target/release/oxyos-session" "$@"
PROBAR
chmod 755 $D/probar.sh

rm -f $R/try-src.tgz $R/deploy.tgz
git ls-files -co --exclude-standard | grep -v "node_modules\|shell-preview/dist" | tar czf $R/try-src.tgz -T -
python3 tools/deploy/system-files.py --packages > $R/build-packages
(cd $R && tar czf deploy.tgz deploy)
# --stage-only: build, test and stage everything, and send nothing.
[ -z "$STAGE_ONLY" ] || { echo "staged: $R/deploy.tgz $R/try-src.tgz (--stage-only: nothing sent)"; exit 0; }
ssh -o BatchMode=yes "$HOST" 'mkdir -p ~/oxyos-validation'
scp -q "$R/deploy.tgz" "$R/try-src.tgz" "$R/build-packages" "$HOST:oxyos-validation/"
ssh -o BatchMode=yes "$HOST" sh -s < "$TREE/tools/deploy/remote.sh"
