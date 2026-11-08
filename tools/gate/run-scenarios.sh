#!/bin/sh
# The scenario gate (docs/developing.mdx): builds this tree's runtime (the Bloom host with this tree's
# C ABI, its framework) and the applications' packages (Dropsea's and Atlas's), then runs every
# scenario of the tree against them with the input methods' engine: the check a pull request and
# the merge queue pass before main takes them, and what a deploy (tools/deploy/deploy.sh) runs
# before it sends anything. It runs tools/dev/bootstrap first, so a checkout is all it needs.
#
# Usage: tools/gate/run-scenarios.sh [--out DIR]
#   --out  the run's directory, kept (a deploy sends its machine/: runtimes/ and packages/ as tested,
#          the input methods' tree named in machine/input-methods). By default a new one under
#          OXYOS_CACHE/runs, removed when the gate passes and kept for two days when it fails (its
#          failed scenarios' records).
# Environment (tools/dev/env.sh, tools/dev/local.conf):
#   BLOOM_DIR, DROPSEA_DIR, ATLAS_DIR  the applications (default: bootstrap's, at tools/dev/pins)
#   GATE_JOBS  scenarios at once (default 8); JOBS, the builds' parallelism (default 12)
#   SCENARIOS  only these scenarios (names, space-separated), for a quick look; the gate is the suite
#
# The runs, in order:
#   1. every scenario but those below, GATE_JOBS at a time;
#   2. one at a time, alone on the cores the machine reserves for the gate (oxyos-gate-cpus, where
#      the machine has it, tools/gate/machine.sh): the scenarios that hold a budget on time (perf-*,
#      refresh-*), which beside siblings would measure each other, and then once more each that
#      failed in 1 or 2; a scenario failing twice fails the gate;
#   3. the scenarios through libinput on the kernel's virtual devices (`uinput`), with /dev/uinput
#      (tools/gate/machine.sh grants it), else as root through sudo, else the gate fails;
#   4. with a GPU (a render node and a Vulkan device that is not the CPU): the one-GPU checks (ADR
#      0031, decision 14: the device chosen on fake sysfs trees, the painter never opening a decoy
#      driver, and only the render nodes of a GPU driving a display opened here), gpu-client with
#      tools/oxyos-gpu-client as its client, and the session's scenarios composed on the GPU alone
#      (--gpu-only). Without one this is said and skipped.
# Left out: astro* (an Astro engine: a deploy's ASTRO_PACKAGE), terminal (a real display, --kms),
# fidelity-thresholds (fidelity.py's settings, not a scenario).
set -eu
OXYOS_TREE=$(cd "$(dirname "$0")/../.." && pwd)
TREE=$OXYOS_TREE
R=
while [ $# -gt 0 ]; do
  case $1 in
    --out) R=$2; shift 2 ;;
    *) sed -n '8,12p' "$0" >&2; exit 2 ;;
  esac
done
"$TREE/tools/dev/bootstrap"
. "$TREE/tools/dev/env.sh"
W=$OXYOS_TOOLCHAIN
IM=$OXYOS_INPUT_METHODS_WORK/prefix
mkdir -p "$OXYOS_CACHE/runs"
find "$OXYOS_CACHE/runs" -mindepth 1 -maxdepth 1 -mmin +2880 -exec rm -rf {} +
own=; [ -n "$R" ] || { R=$(mktemp -d "$OXYOS_CACHE/runs/run.XXXXXX"); own=1; }
mkdir -p "$R" && R=$(cd "$R" && pwd)
M=$R/machine
GATE_JOBS=${GATE_JOBS:-8}
# The session's scenarios composed on the GPU alone (--gpu-only), where there is one.
GPU_ONLY_SCENARIOS=${GPU_ONLY_SCENARIOS:-session-shell}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$TREE/target}
T=$CARGO_TARGET_DIR/release
started=$(date +%s)
phase() { echo "[$(( $(date +%s) - started ))s] $*"; }
fail() { echo "gate: $*"; exit 1; }
# step LOG COMMAND...: the command's output in the run's LOG, its last lines said if it fails.
step() { log=$R/$1; shift; "$@" > "$log" 2>&1 || { tail -30 "$log"; fail "$(basename "$log" .log) failed (log: $log)"; }; }

phase "OxyOS $(git -C "$TREE" rev-parse --short HEAD), Bloom $(git -C "$BLOOM_DIR" rev-parse --short HEAD), Dropsea $(git -C "$DROPSEA_DIR" rev-parse --short HEAD), Atlas $(git -C "$ATLAS_DIR" rev-parse --short HEAD); run $R"
cd "$TREE"
step cargo.log cargo build --release --locked -p oxyos-client-ffi -p oxyos-offscreen -p oxyos-reader -p oxyos-pack \
  -p oxyos-login -p oxyos-installd -p oxyos-accountd -p oxyos-gpu-client -p oxyos-networks
phase "native parts built"

# The runtime: the host built with this tree's C ABI (a build directory of this tree's, kept between
# runs), installed as runtime 1, its framework, then every package built against it.
HOST=$W/host-$(printf '%s' "$TREE" | sha256sum | cut -c1-12)
step cmake.log env CC="$W/clang/usr/bin/clang-19" CXX="$W/clang/usr/bin/clang++-19" "$W/tools/bin/cmake" -S tools/rn-host-spike -B "$HOST" -G Ninja \
  $(sed 's/^/-D/' "$W/cmake-vars.txt") -DOXYOS_CLIENT_LIB="$T/liboxyos_client_ffi.a"
step ninja.log "$W/tools/bin/ninja" -C "$HOST" -j "${JOBS:-12}" oxyos-bloom-host
rm -rf "$M" && mkdir -p "$M"
step install.log "$W/tools/bin/cmake" --install "$HOST" --prefix "$M/runtimes/1"
cd tools/rn-host-spike/sample
step npm.log npm ci --no-audit --no-fund
step framework.log env BLOOM_DIR="$BLOOM_DIR" node framework.mjs "$M/runtimes/1"
step packages.log env OXYOS_PACK="$T/oxyos-pack" OXYOS_RUNTIME="$M/runtimes/1" node packages.mjs "$M/packages"
cd "$TREE"
echo "$IM" > "$M/input-methods"
phase "runtime and packages built: $(ls "$M/packages" | grep -c '^so\.')"

# The scenarios.
uinput() { python3 -c 'import json, sys; sys.exit(0 if json.load(open(sys.argv[1])).get("uinput") else 1)' "$1"; }
: > "$R/list"; : > "$R/list-uinput"
for f in tools/rn-host-spike/scenarios/*.json; do
  n=$(basename "$f" .json)
  case $n in astro* | terminal | fidelity-thresholds | gpu-client) continue ;; esac
  if [ -n "${SCENARIOS:-}" ]; then echo " $SCENARIOS " | grep -q " $n " || continue; fi
  if uinput "$f"; then echo "$n" >> "$R/list-uinput"; else echo "$n" >> "$R/list"; fi
done
# run-one NAME [regress.py arguments...]: one run, its record kept only when it fails. Phases 1 to 3
# paint on the CPU, as the scenarios' pixels were written (a machine with a GPU offers it to the
# hosts, whose Ganesh draws a level or two apart, and keeps scratch textures); phase 4 is the GPU's.
cat > "$R/run-one.sh" <<EOF
n=\$1; shift
out=$R/scenarios/\$n
python3 -X faulthandler tools/rn-host-spike/regress.py tools/rn-host-spike/scenarios/\${n%%@*}.json --offscreen $T/oxyos-offscreen \\
  --runtimes $M/runtimes --packages $M/packages --input-methods $IM --out \$out "\$@" < /dev/null > \$out.txt 2>&1 && { rm -rf \$out \$out.txt; exit 0; }
rm -rf \$out.failed; mv \$out \$out.failed 2>/dev/null; mkdir -p \$out.failed; mv \$out.txt \$out.failed/regress.txt
exit 1
EOF
mkdir -p "$R/scenarios"; : > "$R/failed"
said() { grep FAIL "$R/scenarios/$1.failed/regress.txt" 2>/dev/null | head -3 | cut -c1-240; }

grep -vE '^(perf|refresh)-' "$R/list" | xargs -P "$GATE_JOBS" -I{} sh -c "sh '$R/run-one.sh' {} --renderer cpu || echo {} >> '$R/failed'"
phase "$(grep -vcE '^(perf|refresh)-' "$R/list") scenarios run, $GATE_JOBS at a time: $(wc -l < "$R/failed") failed"

# Alone, on the reserved cores: one run at a time on this machine (the lock), and the cores given
# back however this ends.
lock=/run/lock/oxyos-gate-timing.lock; [ -w $lock ] || lock=$W/.locks/timing
exec 8>>"$lock"
flock 8
reserve=/usr/local/libexec/oxyos-gate-cpus
reserved=
if [ -x $reserve ] && sudo -n $reserve reserve 2>/dev/null; then
  reserved=1; trap 'sudo -n $reserve release' EXIT
  grep -q oxyosgate.slice /proc/self/cgroup || echo "gate: not in oxyosgate.slice: the timing runs share their cores"
else
  echo "gate: no reserved cores on this machine (tools/gate/machine.sh): the timing runs share theirs"
fi
grep -E '^(perf|refresh)-' "$R/list" | while read -r n; do sh "$R/run-one.sh" "$n" --renderer cpu || echo "$n" >> "$R/failed"; done
for n in $(cat "$R/failed"); do
  echo "scenario $n failed once (load $(cut -d' ' -f1 /proc/loadavg)), again alone:"; said "$n"
  grep -rh "fatal signal" -A12 "$R/scenarios/$n.failed" 2>/dev/null | head -14
  mv "$R/scenarios/$n.failed" "$R/scenarios/$n.failed-first"
  sh "$R/run-one.sh" "$n" --renderer cpu || { said "$n"; fail "scenario $n failed twice (records: $R/scenarios/$n.failed*)"; }
done
[ -n "$reserved" ] && { sudo -n $reserve release; trap - EXIT; }
flock -u 8
phase "$(grep -cE '^(perf|refresh)-' "$R/list") timing scenarios run alone; every failure passed alone"

# Through the kernel's virtual devices (docs/adr/0042-scrolling.mdx): each reads only its own
# devices (libinput's path backend), so they run one at a time beside nothing else of theirs.
if [ -s "$R/list-uinput" ]; then
  if [ -w /dev/uinput ]; then as=
  elif sudo -n true 2>/dev/null; then
    sudo -n modprobe -q uinput evdev 2>/dev/null || true
    as="sudo -n --preserve-env"
  else fail "the uinput scenarios need /dev/uinput (tools/gate/machine.sh) or sudo"; fi
  while read -r n; do
    $as sh "$R/run-one.sh" "$n" --renderer cpu || { said "$n"; $as sh "$R/run-one.sh" "$n" --renderer cpu; } \
      || { said "$n"; fail "scenario $n failed twice"; }
    [ -z "$as" ] || sudo -n chown -R "$(id -u):$(id -g)" "$R/scenarios" 2>/dev/null || true
  done < "$R/list-uinput"
  phase "$(wc -l < "$R/list-uinput") scenarios through uinput run"
fi

# The GPU: one GPU (ADR 0031, decision 14), the GPU client and the session composed on it alone.
gpu=
if ls /dev/dri/renderD* >/dev/null 2>&1 && command -v vulkaninfo >/dev/null &&
  vulkaninfo --summary 2>/dev/null | grep -q "PHYSICAL_DEVICE_TYPE_\(INTEGRATED\|DISCRETE\)_GPU"; then gpu=1; fi
if [ -z "$gpu" ] || [ -n "${SCENARIOS:-}" ]; then
  [ -n "$gpu" ] || echo "gate: no GPU here (render node and Vulkan device): its runs are not made"
else
  step one-gpu-tests.log cargo test --release --locked -p oxyos-gpu-device -p oxyos-paint -p oxyos-dmabuf
  step one-gpu-example.log cargo build --release --locked --example compositors-gpu -p oxyos-paint
  # What a compositor and a painter open, as the session and a Bloom host open them: only the
  # render and card nodes of GPUs that drive a display (a connector in sysfs).
  timeout 60 strace -f -e trace=openat,open -o "$R/one-gpu.strace" "$T/examples/compositors-gpu" > "$R/one-gpu.log" 2>&1 \
    || { tail -20 "$R/one-gpu.log"; fail "the one-GPU check's compositors did not run"; }
  allowed=$(for c in /sys/class/drm/card*-*; do [ -e "$c" ] || continue; n=${c##*/}; ls "/sys/class/drm/${n%%-*}/device/drm"; done | sort -u)
  bad=$(grep -v ENOENT "$R/one-gpu.strace" | grep -oE "/dev/dri/(card|renderD)[0-9]+" | sort -u | while read -r p; do
    echo "$allowed" | grep -qx "${p##*/}" || echo "$p"; done)
  [ -z "$bad" ] || fail "a GPU that drives no display was opened: $bad"
  phase "one GPU: only $(echo $allowed | tr '\n' ' ')opened"
  sh "$R/run-one.sh" gpu-client --client "$T/oxyos-gpu-client" \
    || { said gpu-client; sh "$R/run-one.sh" gpu-client --client "$T/oxyos-gpu-client"; } || { said gpu-client; fail "gpu-client failed twice"; }
  for n in $GPU_ONLY_SCENARIOS; do
    sh "$R/run-one.sh" "$n@gpu-only" --gpu-only || { said "$n@gpu-only"; sh "$R/run-one.sh" "$n@gpu-only" --gpu-only; } \
      || { said "$n@gpu-only"; fail "scenario $n failed twice composed on the GPU alone"; }
  done
  phase "gpu-client and $(echo $GPU_ONLY_SCENARIOS | wc -w) scenarios composed on the GPU alone run"
fi
phase "scenarios passed"
[ -z "$own" ] || rm -rf "$R"
