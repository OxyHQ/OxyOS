#!/bin/sh
# perf.py saturation on a development machine (docs/performance.mdx, "Saturation";
# docs/adr/0066-quality-of-service.mdx): the run in a transient system unit of its own, its control
# group delegated to it (the benchmark makes a session's machine there), on the CPUs named and no
# other, one run at a time on the machine. On a device someone is signed in to, perf-device.sh runs
# it instead, by its rules.
#
# Usage, from an OxyOS checkout:
#   tools/rn-host-spike/perf-saturation.sh --cpus LIST --offscreen BIN --runtimes DIR \
#     --packages DIR --out DIR [perf.py options: --runs N, --loads S0,..., --interactions scroll,...,
#     --gpu-only, --check]
#   --cpus       the CPUs the run saturates (cpuset syntax, 8-11,24-27): never the cores a gate
#                reserves for its timing scenarios (GATE_CPUS, /etc/oxyos-gate.conf), which would
#                measure each other
#   --out        the run's directory: its home, runtime directory, shader cache and perf.py's results
#
# On those CPUs the run comes first: its slice (oxyosperf.slice) weighs CPU_WEIGHT against the
# machine's slices' 100 (system.slice: the owner's work; every gate's at 20), so the machine's other
# work runs there only in what the run leaves; elsewhere it is untouched. (A cpuset partition would
# make them the run's alone, but the gate's slices hold every CPU in their cpuset.cpus once a
# reservation is given back, which the kernel takes as overlapping it: the partition is refused.)
#
# What it needs: sudo without a password (the unit is a system one, run as the caller), systemd-run,
# flock. Runs wait for each other (a lock in /run/lock/oxyos-gate, the gate's, or the caller's runtime
# directory): two saturation runs at once would load each other.
set -eu
TREE=$(cd "$(dirname "$0")/../.." && pwd)
CPUS= OUT=
CPU_WEIGHT=1000
ARGS=
while [ $# -gt 0 ]; do
  case $1 in
    --cpus) CPUS=$2; shift 2 ;;
    --out) OUT=$2; shift 2 ;;
    --offscreen | --runtimes | --packages)
      # Absolute, for the unit's working directory.
      ARGS="$ARGS $1 $(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    *) ARGS="$ARGS $1"; shift ;;
  esac
done
[ -n "$CPUS" ] && [ -n "$OUT" ] || { sed -n '8,15p' "$0" >&2; exit 2; }
say() { echo "perf-saturation: $*" >&2; }
sudo -n true 2>/dev/null || { say "needs sudo without a password (the run's unit is a system one)"; exit 1; }
if [ -r /etc/oxyos-gate.conf ]; then
  gate=$(sed -n 's/^GATE_CPUS=//p' /etc/oxyos-gate.conf)
  python3 - "$CPUS" "$gate" <<'EOF' || exit 1
import sys
def cpus(text):
    out = set()
    for part in filter(None, text.split(",")):
        a, _, b = part.partition("-")
        out |= set(range(int(a), int(b or a) + 1))
    return out
both = cpus(sys.argv[1]) & cpus(sys.argv[2])
if both:
    sys.exit(f"perf-saturation: --cpus takes {sorted(both)}, the gate's reserved cores (GATE_CPUS)")
EOF
fi
mkdir -p "$OUT" && OUT=$(cd "$OUT" && pwd)
mkdir -p "$OUT/home" "$OUT/run" "$OUT/tmp" "$OUT/shader-cache"
chmod 700 "$OUT/run"
LOCK=/run/lock/oxyos-gate/saturation.lock
[ -w /run/lock/oxyos-gate ] || LOCK=${XDG_RUNTIME_DIR:-/tmp}/oxyos-saturation.lock
UNIT=oxyos-saturation-$$
# The slice's weight, set as the unit starts and given back as it stops, by systemd as root (`+`),
# whatever the run does (and the slice is gone once empty).
set -- -p "ExecStartPre=+systemctl set-property --runtime oxyosperf.slice CPUWeight=$CPU_WEIGHT" \
  -p "ExecStopPost=+systemctl set-property --runtime oxyosperf.slice CPUWeight=100"
stop() { sudo -n systemctl stop "$UNIT" 2>/dev/null || true; }
trap 'stop; exit 130' INT TERM HUP
say "waiting for the machine's other saturation runs ($LOCK)"
# shellcheck disable=SC2086
flock "$LOCK" sudo -n systemd-run --quiet --wait --pipe --collect --unit="$UNIT" --slice=oxyosperf.slice "$@" \
  -p User="$(id -un)" -p Group="$(id -gn)" -p WorkingDirectory="$TREE" -p Delegate=yes -p AllowedCPUs="$CPUS" \
  -p Environment="HOME=$OUT/home XDG_RUNTIME_DIR=$OUT/run TMPDIR=$OUT/tmp PATH=/usr/local/bin:/usr/bin:/bin" \
  -p Environment="MESA_SHADER_CACHE_DIR=$OUT/shader-cache LANG=C.UTF-8 SATURATION_TOOLCHAIN_HOME=$HOME" \
  -p SupplementaryGroups="$(stat -c %G /dev/dri/renderD128 2>/dev/null || echo render)" \
  -- python3 tools/rn-host-spike/perf.py --out "$OUT/perf" $ARGS saturation
