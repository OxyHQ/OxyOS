#!/bin/sh
# Prepares a Debian trixie machine to run OxyOS's scenario gate (docs/developing.mdx) beside the
# machine's own work, which keeps priority. Run as root; running it again changes nothing that is
# already so.
#   - a developer's packages (tools/dev/packages), QEMU and its firmware, Podman;
#   - the gate's user, oxyos-ci (GATE_USER), with no sudo but for oxyos-gate-cpus: its GitHub
#     Actions runner and every run of the gate are its, in oxyosgate.slice;
#   - oxyosgate.slice (no dash: systemd nests a slice under the part of its name before one): a
#     weight of 20 against system.slice's 100 (the machine's own work), at most GATE_CPU_QUOTA of
#     CPU and GATE_MEMORY_MAX of memory;
#   - the GPU's render node (group render), and uinput with the event nodes of the development
#     compositor's own virtual devices (named "oxyos-offscreen ...") for the scenarios that go
#     through libinput (group oxyos-uinput), instead of running those as root;
#   - rootless Podman for the image's builds (subordinate ids, lingering), KVM (group kvm) for their
#     QEMU acceptance runs;
#   - /usr/local/libexec/oxyos-gate-cpus and /etc/oxyos-gate.conf: the cores the scenarios that
#     hold a budget on time run on alone (the first four physical cores and their SMT siblings).
# The runner itself is registered with `machine.sh runner` (a registration token on stdin).
set -eu
[ "$(id -u)" = 0 ] || { echo "machine.sh: run as root" >&2; exit 1; }
HERE=$(cd "$(dirname "$0")" && pwd)
GATE_USER=${GATE_USER:-oxyos-ci}
GATE_HOME=/home/$GATE_USER
GATE_CPU_QUOTA=${GATE_CPU_QUOTA:-2400%}
GATE_MEMORY_HIGH=${GATE_MEMORY_HIGH:-20G}
GATE_MEMORY_MAX=${GATE_MEMORY_MAX:-24G}
RUNNER_NAME=${RUNNER_NAME:-$(hostname)-oxyos}
# The runner's own directory, outside the user's home (which holds only what tools/dev/bootstrap makes).
RUNNER_DIR=/var/lib/oxyos-gate/actions-runner
RUNNER_LABEL=oxyos-gate

if [ "${1:-}" = runner ]; then
  # The runner of OxyHQ/OxyOS, label oxyos-gate, as GATE_USER in oxyosgate.slice. The registration
  # token comes on stdin (gh api -X POST repos/OxyHQ/OxyOS/actions/runners/registration-token
  # --jq .token | sudo sh machine.sh runner) and is never written anywhere.
  read -r token
  R=$RUNNER_DIR
  install -d -o "$GATE_USER" -g "$GATE_USER" -m 750 "$R"
  if [ ! -x "$R/config.sh" ]; then
    release=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest)
    version=$(printf '%s' "$release" | python3 -c 'import json, sys; print(json.load(sys.stdin)["tag_name"].lstrip("v"))')
    tarball=actions-runner-linux-x64-$version.tar.gz
    # The release's asset carries its SHA-256.
    sum=$(printf '%s' "$release" | python3 -c 'import json, sys; print(*[a["digest"].removeprefix("sha256:") for a in json.load(sys.stdin)["assets"] if a["name"] == sys.argv[1]])' "$tarball")
    runuser -u "$GATE_USER" -- curl -fsSL -o "$R/$tarball" "https://github.com/actions/runner/releases/download/v$version/$tarball"
    echo "$sum  $R/$tarball" | sha256sum -c --quiet
    runuser -u "$GATE_USER" -- tar xzf "$R/$tarball" -C "$R"
    rm -f "$R/$tarball"
  fi
  (cd "$R" && runuser -u "$GATE_USER" -- ./config.sh --unattended --replace --url https://github.com/OxyHQ/OxyOS \
    --token "$token" --name "$RUNNER_NAME" --labels "$RUNNER_LABEL" --work _work >/dev/null)
  # The runner's service entry point, put in place as its own svc.sh does.
  runuser -u "$GATE_USER" -- cp "$R/bin/runsvc.sh" "$R/runsvc.sh"
  unit=oxyos-gate-runner.service
  cat > /etc/systemd/system/$unit <<EOF
# OxyOS's scenario gate runner (docs/developing.mdx), written by tools/gate/machine.sh.
[Unit]
Description=GitHub Actions runner for OxyHQ/OxyOS (label $RUNNER_LABEL)
After=network-online.target
Wants=network-online.target

[Service]
User=$GATE_USER
WorkingDirectory=$R
ExecStart=$R/runsvc.sh
Slice=oxyosgate.slice
# A /tmp of the runner's own, gone with it: nothing of a job's is left in the machine's.
PrivateTmp=yes
KillMode=process
KillSignal=SIGTERM
TimeoutStopSec=5min
Restart=always
RestartSec=30

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable --now $unit
  exit 0
fi

# Packages: what a developer's machine needs (tools/dev/packages; tools/dev/bootstrap fetches the
# rest into the user's cache), and QEMU with its firmware for the image's acceptance runs.
apt-get install -y -q --no-install-recommends $(grep -v '^#' "$HERE/../dev/packages") \
  qemu-system-x86 qemu-utils ovmf seabios swtpm podman uidmap passt fuse-overlayfs >/dev/null

groupadd -f --system oxyos-uinput
getent passwd "$GATE_USER" >/dev/null ||
  useradd --system --create-home --home-dir "$GATE_HOME" --shell /bin/bash --user-group "$GATE_USER"
usermod -aG render,kvm,oxyos-uinput "$GATE_USER"
chmod 750 "$GATE_HOME"
# Images (tools/image/build.sh) in rootless Podman: subordinate ids after the last range given, and
# lingering, so Podman's runtime files are in /run/user/<uid>, never /tmp.
if ! grep -q "^$GATE_USER:" /etc/subuid; then
  first=$(awk -F: 'BEGIN { end = 100000 } { if ($2 + $3 > end && $2 < 1000000) end = $2 + $3 } END { print end }' /etc/subuid)
  usermod --add-subuids "$first-$((first + 65535))" --add-subgids "$first-$((first + 65535))" "$GATE_USER"
fi
loginctl enable-linger "$GATE_USER"

cat > /etc/systemd/system/oxyosgate.slice <<EOF
# OxyOS's scenario gate (docs/developing.mdx), written by tools/gate/machine.sh: below the
# machine's own work, capped.
[Unit]
Description=OxyOS scenario gate

[Slice]
CPUWeight=20
IOWeight=20
CPUQuota=$GATE_CPU_QUOTA
MemoryHigh=$GATE_MEMORY_HIGH
MemoryMax=$GATE_MEMORY_MAX
EOF

cat > /etc/udev/rules.d/70-oxyos-gate.rules <<'EOF'
# OxyOS's scenario gate (docs/developing.mdx), written by tools/gate/machine.sh: uinput, and the
# event nodes of the development compositor's own virtual devices, for group oxyos-uinput.
KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="oxyos-uinput", MODE="0660"
SUBSYSTEM=="input", KERNEL=="event*", ATTRS{name}=="oxyos-offscreen *", GROUP="oxyos-uinput", MODE="0660"
EOF
echo uinput > /etc/modules-load.d/oxyos-gate.conf
modprobe uinput
udevadm control --reload
udevadm trigger --subsystem-match=misc --sysname-match=uinput

# The reserved cores: the first four physical cores with their SMT siblings for the gate, every
# other CPU for the rest of the machine.
python3 - > /etc/oxyos-gate.conf <<'EOF'
import glob, os
def cpus(text):
    out = set()
    for part in text.strip().split(","):
        a, _, b = part.partition("-")
        out.update(range(int(a), int(b or a) + 1))
    return out
online = cpus(open("/sys/devices/system/cpu/online").read())
gate = set()
for cpu in sorted(online)[:4]:
    gate |= cpus(open(f"/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list").read())
fmt = lambda s: ",".join(map(str, sorted(s)))
print("# The scenario gate's reserved cores (tools/gate/machine.sh, oxyos-gate-cpus).")
print(f"GATE_CPUS={fmt(gate)}")
print(f"OWNER_CPUS={fmt(online - gate)}")
print("RESERVE_LIMIT=30min")
EOF
install -o root -g root -m 0755 "$HERE/oxyos-gate-cpus" /usr/local/libexec/oxyos-gate-cpus
cat > /etc/sudoers.d/oxyos-gate <<EOF
# OxyOS's scenario gate (docs/developing.mdx): its user reserves and releases its cores, nothing else.
$GATE_USER ALL=(root) NOPASSWD: /usr/local/libexec/oxyos-gate-cpus reserve, /usr/local/libexec/oxyos-gate-cpus release
EOF
chmod 440 /etc/sudoers.d/oxyos-gate
visudo -cq -f /etc/sudoers.d/oxyos-gate
# One run at a time on the reserved cores, whoever runs it (the runner, a deploy).
echo "f /run/lock/oxyos-gate-timing.lock 0666 root root -" > /etc/tmpfiles.d/oxyos-gate.conf
systemd-tmpfiles --create /etc/tmpfiles.d/oxyos-gate.conf
systemctl daemon-reload
echo "machine.sh: $GATE_USER, oxyosgate.slice, $(sed -n 's/^GATE_CPUS=//p' /etc/oxyos-gate.conf) reserved"
