#!/bin/sh
# perf.py on a device's own GPU, beside a person's session and never disturbing it
# (docs/performance.mdx, "A device's GPU"): the development compositor composing on the GPU that
# drives the machine's displays and every host drawing on it (`--gpu-only`), with nothing able to
# open another GPU, and the run stopped if the person comes back.
#
# Usage, from an OxyOS checkout on the device:
#   tools/rn-host-spike/perf-device.sh --offscreen BIN --runtimes DIR --packages DIR --out DIR \
#     [--gpu PCI_SLOT] [--runs N] [--only SCENARIOS] [--check] [perf.py parts...]
#   --gpu   the GPU to compose and draw on, by its PCI slot (0000:00:02.0); by default the one GPU that
#           drives a display (a connector in sysfs), as a session chooses it (ADR 0031, decision 14)
#   --out   the run's directory: its home, runtime directory, shader cache, record of every file it
#           opened (opens.strace) and perf.py's results. The caller removes it.
#   the rest is perf.py's (--runs 3 by default; --only, a short set of its scenarios).
#
# What makes it safe on a machine someone is signed in to (the ThinkPad: an Iris Xe driving the
# panel, and an NVIDIA T550 on nouveau that hung the kernel when a Vulkan enumeration woke it):
# - it starts only while the session on seat0 is idle (logind's IdleHint), and stops the run the
#   moment it is not;
# - perf.py runs in a transient system unit as the caller, at the least priority (nice 19, idle I/O,
#   a CPU weight of 1), whose /dev/dri holds the chosen GPU's render node alone (a tmpfs, the node
#   bound in; the unit in the node's group, as the seat's user is given it) and whose device policy
#   allows no other GPU node (DevicePolicy=closed): no other GPU can be opened, by path or by
#   number. It never takes a display (no --kms, no DRM master);
# - Vulkan's loader reads the chosen GPU's driver alone (VK_DRIVER_FILES, which oxyos-gpu-device
#   reads too): no other driver is loaded or enumerated (the gate's vulkaninfo is never run);
# - the machine's own OxyOS (its policy, registry, runtimes) is hidden from the run (empty tmpfs);
# - every file the run's processes open, or try to, is recorded (strace, with seccomp so only the
#   opens stop them); a node or a driver of any other GPU among them fails the run, as does that
#   GPU's runtime power state changing (woken) or its driver logging while the run lasted.
set -eu
TREE=$(cd "$(dirname "$0")/../.." && pwd)
OFFSCREEN= RUNTIMES= PACKAGES= OUT= GPU= RUNS=3 ONLY= CHECK=
usage() { sed -n '6,13p' "$0" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case $1 in
    --offscreen) OFFSCREEN=$2; shift 2 ;;
    --runtimes) RUNTIMES=$2; shift 2 ;;
    --packages) PACKAGES=$2; shift 2 ;;
    --out) OUT=$2; shift 2 ;;
    --gpu) GPU=$2; shift 2 ;;
    --runs) RUNS=$2; shift 2 ;;
    --only) ONLY=$2; shift 2 ;;
    --check) CHECK=--check; shift ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ -n "$OFFSCREEN" ] && [ -n "$RUNTIMES" ] && [ -n "$PACKAGES" ] && [ -n "$OUT" ] || usage
say() { echo "perf-device: $*" >&2; }
fail() { say "$*"; exit 1; }
sudo -n true 2>/dev/null || fail "needs sudo without a password (the run's unit is a system one)"
for tool in systemd-run strace loginctl python3; do command -v $tool >/dev/null || fail "needs $tool"; done
absolute() { (cd "$(dirname "$1")" && echo "$(pwd)/$(basename "$1")"); }
OFFSCREEN=$(absolute "$OFFSCREEN") RUNTIMES=$(absolute "$RUNTIMES") PACKAGES=$(absolute "$PACKAGES")
mkdir -p "$OUT" && OUT=$(cd "$OUT" && pwd)

# The GPU: the one named, else the one driving a display.
pci_of() { basename "$(readlink -f "/sys/class/drm/$1/device")"; }
if [ -z "$GPU" ]; then
  GPU=$(for c in /sys/class/drm/card*-*; do [ -e "$c" ] || continue; n=${c##*/}; pci_of "${n%%-*}"; done | sort -u)
  [ "$(echo "$GPU" | grep -c .)" = 1 ] || fail "no single GPU drives a display ($(echo $GPU)): name one with --gpu"
fi
[ -d "/sys/bus/pci/devices/$GPU/drm" ] || fail "$GPU is no GPU"
RENDER=/dev/dri/$(ls "/sys/bus/pci/devices/$GPU/drm" | grep '^renderD' | head -1)
[ -c "$RENDER" ] || fail "$GPU has no render node"
DRIVER=$(basename "$(readlink -f "/sys/bus/pci/devices/$GPU/driver")")
# Its Vulkan driver's manifest: the one ICD whose library is the driver's.
case $DRIVER in
  i915 | xe) LIB=libvulkan_intel.so ;;
  amdgpu) LIB=libvulkan_radeon.so ;;
  *) fail "no Vulkan driver known for $DRIVER" ;;
esac
ICD=$(grep -l "\"library_path\": *\"[^\"]*$LIB\"" /usr/share/vulkan/icd.d/*.json 2>/dev/null | head -1)
[ -n "$ICD" ] || fail "no Vulkan manifest for $LIB in /usr/share/vulkan/icd.d"

# Every other GPU: its nodes, by path and as found by-path, and what its drivers are called, each
# of which the run must never open; its runtime power state, which must not change.
OTHERS= FORBIDDEN=
for card in /sys/class/drm/card[0-9]*; do
  case ${card##*/} in *-*) continue ;; esac
  pci=$(pci_of "${card##*/}")
  [ "$pci" = "$GPU" ] && continue
  case " $OTHERS " in *" $pci "*) continue ;; esac
  OTHERS="$OTHERS $pci"
  for node in "/sys/bus/pci/devices/$pci/drm/"*; do
    FORBIDDEN="$FORBIDDEN /dev/dri/${node##*/}"
  done
  FORBIDDEN="$FORBIDDEN /dev/dri/by-path/pci-$pci-"
  case $(basename "$(readlink -f "/sys/bus/pci/devices/$pci/driver")") in
    nouveau) FORBIDDEN="$FORBIDDEN nouveau nvk" ;;
    nvidia*) FORBIDDEN="$FORBIDDEN nvidia" ;;
    amdgpu) FORBIDDEN="$FORBIDDEN radeon radv amdgpu" ;;
    i915 | xe) FORBIDDEN="$FORBIDDEN intel_icd libvulkan_intel" ;;
  esac
done
power() { for pci in $OTHERS; do d=/sys/bus/pci/devices/$pci/power
  echo "$pci $(cat $d/runtime_status) $(cat $d/runtime_active_time)"; done; }
say "composing and drawing on $GPU ($DRIVER, $RENDER, $ICD); never opened:$OTHERS ($(echo $FORBIDDEN))"

# The person on seat0: only while their session is idle.
idle() {
  s=$(loginctl show-seat seat0 -p ActiveSession --value 2>/dev/null || true)
  [ -z "$s" ] || [ "$(loginctl show-session "$s" -p IdleHint --value)" = yes ]
}
idle || fail "the session on seat0 is in use (IdleHint=no): run when it is idle"

mkdir -p "$OUT/home" "$OUT/run" "$OUT/tmp" "$OUT/shader-cache" "$OUT/perf"
chmod 700 "$OUT/run"
# The machine's own OxyOS hidden from the run (empty), and /dev/dri the render node alone.
HIDE=/dev/dri
for d in /usr/share/oxyos /usr/local/share/oxyos /usr/lib/oxyos /var/lib/oxyos /opt/oxyos; do
  [ -d "$d" ] && HIDE="$HIDE $d"
done
# The compose tables the session ships, as the earlier device runs gave them.
LOCALE=$TREE/config/includes.chroot_after_packages/usr/share/X11/locale
XLOC=; [ -d "$LOCALE" ] && XLOC="XLOCALEDIR=$LOCALE"
UNIT=oxyos-perf-device-$$
power > "$OUT/power-before"
started=$(date +%s)
stop() { sudo -n systemctl stop "$UNIT" 2>/dev/null || true; }
trap 'stop; exit 130' INT TERM HUP
# shellcheck disable=SC2086
sudo -n systemd-run --quiet --wait --pipe --collect --unit="$UNIT" \
  -p User="$(id -un)" -p Group="$(id -gn)" -p WorkingDirectory="$TREE" \
  -p Nice=19 -p IOSchedulingClass=idle -p CPUWeight=1 \
  -p DevicePolicy=closed -p DeviceAllow="$RENDER rw" -p SupplementaryGroups="$(stat -c %G "$RENDER")" \
  -p TemporaryFileSystem="$HIDE" -p BindPaths="$RENDER" \
  -p Environment="HOME=$OUT/home XDG_RUNTIME_DIR=$OUT/run TMPDIR=$OUT/tmp PATH=/usr/local/bin:/usr/bin:/bin" \
  -p Environment="VK_DRIVER_FILES=$ICD MESA_SHADER_CACHE_DIR=$OUT/shader-cache LANG=C.UTF-8 $XLOC" \
  -- strace -f --seccomp-bpf -qq -e trace=open,openat,openat2 -e signal=none -o "$OUT/opens.strace" \
  python3 tools/rn-host-spike/perf.py --offscreen "$OFFSCREEN" --runtimes "$RUNTIMES" --packages "$PACKAGES" \
    --out "$OUT/perf" --runs "$RUNS" --gpu-only ${ONLY:+--only "$ONLY"} $CHECK "$@" &
run=$!
# Watch the person: back at the machine, the run stops at once. (sudo is root's: kill -0 cannot ask.)
came_back=
while [ -d /proc/$run ]; do
  if ! idle; then came_back=1; say "the session on seat0 is in use: stopping"; stop; break; fi
  sleep 2
done
status=0; wait $run || status=$?
trap - INT TERM HUP
power > "$OUT/power-after"

# What was opened: nothing of another GPU, by node or by driver.
bad=$(for f in $FORBIDDEN; do grep -F -- "$f" "$OUT/opens.strace" || true; done | sort -u | head -20)
[ -z "$bad" ] || fail "another GPU's node or driver was opened (or tried):
$bad"
cmp -s "$OUT/power-before" "$OUT/power-after" || fail "another GPU woke while the run lasted:
$(paste "$OUT/power-before" "$OUT/power-after")"
logged=$(sudo -n journalctl -k -q --no-pager --since "@$started" | grep -iE 'nouveau|nvidia' || true)
[ -z "$logged" ] || fail "another GPU's driver logged while the run lasted:
$logged"
opened=$(grep -c "$RENDER" "$OUT/opens.strace" || true)
say "$(grep -c . "$OUT/opens.strace") opens recorded, $opened of $RENDER; no other GPU's node or driver opened, none woke"
[ -z "$came_back" ] || fail "stopped: the person came back"
exit $status
