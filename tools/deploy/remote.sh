#!/bin/sh
# The test machine's half of a deploy (tools/deploy/deploy.sh, docs/developing.mdx), run there over
# ssh in ~/oxyos-validation, where deploy.sh has put deploy.tgz (the runtime, the system
# applications, their entries and policy, Inbox and Allo, the input methods, the sounds and
# wallpapers, as the gate tested them) and try-src.tgz (this tree's source): it builds the session
# there, installs everything as the image will (root-owned, each tree swapped in whole, the one before
# kept as .old), checks the one-GPU rule on the machine itself, and restarts the login screen (only
# while nobody is signed in) or the development session (probar.sh on terminal 8).
# The machine needs passwordless sudo for its user, and rustup's cargo in ~/.cargo.
set -e
V=$HOME/oxyos-validation
cd "$V"
# The source is replaced whole (only its build output kept), so a file the tree no longer has (an
# old build.rs) is not built.
mkdir -p oxyos-try && find oxyos-try -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} +
(cd oxyos-try && tar xzf ../try-src.tgz)
# The system sounds play through PipeWire (docs/adr/0044-system-sounds.mdx): the session links
# libpipewire through its Rust bindings, built with its headers and libclang. Only in a tree that has it.
if [ -d oxyos-try/crates/oxyos-sound ]; then
  missing=$(for p in libpipewire-0.3-dev libclang-dev pkg-config; do dpkg-query -W -f="\${Status}" $p 2>/dev/null | grep -q "ok installed" || echo $p; done)
  [ -z "$missing" ] || sudo -n apt-get install -y -q --no-install-recommends $missing > /dev/null
fi
# The session, the screen reader, and the login, accounts and PIN stack, built here.
(cd oxyos-try && ~/.cargo/bin/cargo build -q --release --locked -p oxyos-reader $(cat ../build-packages) 2>&1 | tail -3)
# Signing in with an Oxy account and a PIN (ADR 0020, ADR 0010): every file of tools/deploy/system-files
# (checked against the tree by the deploy) root-owned at its path, then the users, directories and
# Yama ptrace scope 1 its files make (no application can read the token of another).
# oxyos-accountd is enabled and restarted; oxyos-login is never started or restarted here (it would
# end a signed-in session, and which program owns tty1 is for the owner of the machine to choose):
# its new binary takes effect at its next start.
T=oxyos-try
# Each staged beside its place and renamed over the one before, so none is ever missing (a session
# started meanwhile still finds its program); a program running keeps the file it started with.
grep -v '^#' $T/tools/deploy/system-files | while read -r from to mode; do
  [ -n "$from" ] || continue
  staged="${to%/*}/.${to##*/}.new"
  sudo -n install -D -o root -g root -m "$mode" "$T/$from" "$staged"
  sudo -n mv -fT "$staged" "$to"
done
for f in $(awk '$2 ~ "^/usr/lib/sysusers.d/" { print $2 }' $T/tools/deploy/system-files); do sudo -n systemd-sysusers "${f##*/}"; done
# The modules the image loads at boot (zswap's compressor) loaded now, before the tmpfiles that use
# them, as systemd-modules-load runs before systemd-tmpfiles-setup.
for f in $(awk '$2 ~ "^/usr/lib/modules-load.d/" { print $2 }' $T/tools/deploy/system-files); do
  for m in $(grep -v '^#' "$f"); do sudo -n modprobe "$m"; done
done
for f in $(awk '$2 ~ "^/usr/lib/tmpfiles.d/" { print $2 }' $T/tools/deploy/system-files); do sudo -n systemd-tmpfiles --create "${f##*/}"; done
sudo -n sysctl -q --system
sudo -n systemctl daemon-reload
sudo -n systemctl enable -q oxyos-accountd
sudo -n systemctl restart oxyos-accountd
systemctl is-active -q oxyos-accountd || { echo "accounts: oxyos-accountd is not running"; exit 1; }
[ "$(cat /proc/sys/kernel/yama/ptrace_scope)" = 1 ] || { echo "accounts: ptrace scope is not 1"; exit 1; }
getent passwd oxyos-login > /dev/null || { echo "accounts: no oxyos-login user"; exit 1; }
echo "accounts: $(grep -vc '^#' $T/tools/deploy/system-files) files installed, oxyos-accountd active, ptrace scope 1 (tty1: $(systemctl is-enabled oxyos-login 2>/dev/null || true))"
# Memory (docs/adr/0066-quality-of-service.mdx, decision 7): zswap and MGLRU's thrashing protection
# were set above (the image's tmpfiles file, as every boot sets them); systemd-oomd, the image's, is
# installed, enabled and started. The slices' protections and oomd's watch on the applications'
# slice are drop-ins, read at the reload above: a session's scope and a user manager take theirs as
# they start, so a person signed in now keeps the session they have until their next sign-in, and
# nothing of it is changed here.
pkgs=$(python3 $T/tools/deploy/system-files.py --apt-memory)
missing=$(for p in $pkgs; do dpkg-query -W -f="\${Status}" $p 2>/dev/null | grep -q "ok installed" || echo $p; done)
[ -z "$missing" ] || sudo -n apt-get install -y -q --no-install-recommends $missing > /dev/null
sudo -n systemctl enable -q --now systemd-oomd.socket systemd-oomd.service
systemctl is-active -q systemd-oomd || { echo "memory: systemd-oomd is not running"; exit 1; }
zswap=/sys/module/zswap/parameters
[ "$(cat $zswap/enabled)" = Y ] && [ "$(cat $zswap/compressor)" = lz4 ] || { echo "memory: zswap is not on with lz4"; exit 1; }
[ "$(cat /sys/kernel/mm/lru_gen/min_ttl_ms)" = 1000 ] || { echo "memory: MGLRU's min_ttl_ms is not 1000"; exit 1; }
echo "memory: zswap on ($(cat $zswap/compressor), $(cat $zswap/zpool), $(cat $zswap/max_pool_percent) %, shrinker $(cat $zswap/shrinker_enabled)), MGLRU min_ttl_ms $(cat /sys/kernel/mm/lru_gen/min_ttl_ms), systemd-oomd active; the sessions' protections from their next start"
# Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10) where the machine boots systemd-boot
# with kernel-install's entries (one converted to OxyOS, decision 9): the plugin installed above writes
# each new kernel's Safe Mode entry beside its own; the kernels already installed get theirs here, from
# their entries as they are, and one whose kernel is gone loses its own. Where kernel-install puts them
# and what it names them by are its own (`kernel-install inspect`), as the plugin is told. Run again,
# nothing changes. The boot loader's default and entries are otherwise left as they are.
kernel=/boot/vmlinuz-$(uname -r)
inspect=$(sudo -n kernel-install inspect "$kernel" 2>/dev/null || true)
layout=$(echo "$inspect" | sed -n 's/.*KERNEL_INSTALL_LAYOUT=//p')
boot_root=$(echo "$inspect" | sed -n 's/.*KERNEL_INSTALL_BOOT_ROOT=//p')
token=$(echo "$inspect" | sed -n 's/.*KERNEL_INSTALL_ENTRY_TOKEN=//p')
if [ "$layout" = bls ] && [ -n "$boot_root" ] && [ -n "$token" ]; then
  synced=$(sudo -n /usr/sbin/oxyos-recovery safe-mode-entry sync --boot-root "$boot_root" --entry-token "$token")
  echo "$synced" | sed 's/^/safe mode: /'
else
  echo "safe mode: no systemd-boot entries of kernel-install's here (layout '${layout:-none}'): none written"
fi
# Backups (docs/adr/0059-backups.mdx) as the image has them: Debian's restic and rclone, the engine and
# the SMB client the service runs (the image's manifest's, tools/deploy/system-files.py --apt), then the
# service installed above enabled and restarted onto its new program, and the migration's socket
# enabled (its service answers only while no account is enrolled, so never on a machine in use). A
# signed-in session follows the service as soon as its socket is there. No destination is configured
# here: choosing one, and its password, is the person's (Settings › General › Backups).
pkgs=$(python3 $T/tools/deploy/system-files.py --apt)
missing=$(for p in $pkgs; do dpkg-query -W -f="\${Status}" $p 2>/dev/null | grep -q "ok installed" || echo $p; done)
[ -z "$missing" ] || sudo -n apt-get install -y -q --no-install-recommends $missing > /dev/null
for p in $pkgs; do command -v $p > /dev/null || { echo "backups: no $p"; exit 1; }; done
sudo -n systemctl enable -q oxyos-backupd oxyos-migrate.socket
sudo -n systemctl restart oxyos-backupd
sudo -n systemctl start oxyos-migrate.socket
systemctl is-active -q oxyos-backupd || { echo "backups: oxyos-backupd is not running"; exit 1; }
systemctl is-active -q oxyos-migrate.socket || { echo "backups: oxyos-migrate.socket is not listening"; exit 1; }
# It answers on its socket once it has read its state (Type=exec: started is not yet listening).
rm -f backup-state.json
for i in 1 2 3 4 5 6 7 8 9 10; do sudo -n /usr/libexec/oxyos-backupd --client '"status"' > backup-state.json 2>/dev/null && break; sleep 1; done
[ -s backup-state.json ] || { echo "backups: oxyos-backupd does not answer on its socket"; exit 1; }
echo "backups: $(restic version | cut -d' ' -f1-2), $(rclone version | head -1), oxyos-backupd active and answering, the migration's socket listening"
# One GPU (ADR 0031, decision 14), on the machine itself before the session restarts: the
# compositor and a painter opened as the session and a Bloom host open them, under strace; any
# DRM node of a GPU that drives no display (a laptop's sleeping discrete GPU, for one), any NVK
# library, or a nouveau line in the kernel log stops the deploy. Only in a tree that has it.
if [ -d oxyos-try/crates/oxyos-gpu-device ]; then
  (cd oxyos-try && ~/.cargo/bin/cargo build -q --release --locked --example compositors-gpu -p oxyos-paint 2>&1 | tail -3)
  # strace as a package (a development tool, like the compilers: an unpacked copy breaks when the
  # libraries it needs are not on the machine, as after the conversion of 7 November 2026).
  command -v strace >/dev/null || sudo -n apt-get install -y -q --no-install-recommends strace >/dev/null
  st=$(command -v strace)
  before=$(sudo -n journalctl -k --no-pager | grep -c nouveau || true)
  sudo -n timeout 60 $st -f -e trace=openat,open -o gpu-check/deploy.strace oxyos-try/target/release/examples/compositors-gpu
  allowed=$(for c in /sys/class/drm/card*-*; do n=${c##*/}; ls /sys/class/drm/${n%%-*}/device/drm; done | sort -u)
  bad=$(grep -v ENOENT gpu-check/deploy.strace | grep -oE "/dev/dri/(card|renderD)[0-9]+" | sort -u | while read p; do echo "$allowed" | grep -qx "${p##*/}" || echo $p; done)
  [ -z "$bad" ] || { echo "a GPU that drives no display was opened: $bad"; exit 1; }
  ! grep -q libvulkan_nouveau gpu-check/deploy.strace || { echo "NVK was loaded"; exit 1; }
  after=$(sudo -n journalctl -k --no-pager | grep -c nouveau || true)
  [ "$before" = "$after" ] || { echo "nouveau logged $((after - before)) lines"; exit 1; }
  echo "one GPU: only $(echo $allowed | tr "\n" " ")opened"
fi
rm -rf $V/.unpack && mkdir -p $V/.unpack && tar xzf deploy.tgz -C $V/.unpack
# The runtime and the system applications, root-owned and read-only to everyone else, each tree
# swapped in whole (the one before kept as .old until the next deploy).
for tree in runtimes/1 apps; do
  sudo -n rm -rf /usr/lib/oxyos/$tree.new; sudo -n mkdir -p /usr/lib/oxyos/$tree.new
  sudo -n cp -a $V/.unpack/deploy/$tree/. /usr/lib/oxyos/$tree.new/
  sudo -n chown -R root:root /usr/lib/oxyos/$tree.new; sudo -n chmod -R go-w /usr/lib/oxyos/$tree.new
done
test -x /usr/lib/oxyos/runtimes/1.new/oxyos-bloom-host && test -s /usr/lib/oxyos/runtimes/1.new/framework/framework.hbc
test -s /usr/lib/oxyos/apps.new/so.oxy.settings/index.hbc && test -s /usr/lib/oxyos/apps.new/so.oxy.shell/index.hbc
for tree in runtimes/1 apps; do
  sudo -n rm -rf /usr/lib/oxyos/$tree.old
  [ -e /usr/lib/oxyos/$tree ] && sudo -n mv /usr/lib/oxyos/$tree /usr/lib/oxyos/$tree.old
  sudo -n mv /usr/lib/oxyos/$tree.new /usr/lib/oxyos/$tree
done
sudo -n chown root:root /usr/lib/oxyos /usr/lib/oxyos/runtimes && sudo -n chmod 755 /usr/lib/oxyos /usr/lib/oxyos/runtimes
# The input-method engine (ADR 0041): its toolkit-free packages from apt, then the tree swapped in
# whole as the runtime is, root-owned.
if [ -d $V/.unpack/deploy/input-methods ]; then
  pkgs=$(cat $V/.unpack/deploy/input-methods.packages)
  missing=$(for p in $pkgs; do dpkg-query -W -f="\${Status}" $p 2>/dev/null | grep -q "ok installed" || echo $p; done)
  [ -z "$missing" ] || sudo -n apt-get install -y -q --no-install-recommends $missing > /dev/null
  sudo -n rm -rf /usr/lib/oxyos/input-methods.new
  sudo -n cp -a $V/.unpack/deploy/input-methods /usr/lib/oxyos/input-methods.new
  sudo -n chown -R root:root /usr/lib/oxyos/input-methods.new; sudo -n chmod -R go-w /usr/lib/oxyos/input-methods.new
  test -x /usr/lib/oxyos/input-methods.new/bin/fcitx5
  sudo -n rm -rf /usr/lib/oxyos/input-methods.old
  [ -e /usr/lib/oxyos/input-methods ] && sudo -n mv /usr/lib/oxyos/input-methods /usr/lib/oxyos/input-methods.old
  sudo -n mv /usr/lib/oxyos/input-methods.new /usr/lib/oxyos/input-methods
fi
# The system sound theme (docs/adr/0044-system-sounds.mdx), root-owned, swapped in whole as the
# runtime is, with its provenance.
if [ -d $V/.unpack/deploy/wallpapers ]; then
  sudo -n rm -rf /usr/share/oxyos/wallpapers.new; sudo -n mkdir -p /usr/share/oxyos
  sudo -n cp -a $V/.unpack/deploy/wallpapers /usr/share/oxyos/wallpapers.new
  sudo -n chown -R root:root /usr/share/oxyos/wallpapers.new; sudo -n chmod -R go-w /usr/share/oxyos/wallpapers.new
  sudo -n rm -rf /usr/share/oxyos/wallpapers.old
  [ -e /usr/share/oxyos/wallpapers ] && sudo -n mv /usr/share/oxyos/wallpapers /usr/share/oxyos/wallpapers.old
  sudo -n mv /usr/share/oxyos/wallpapers.new /usr/share/oxyos/wallpapers
  [ -d $V/.unpack/deploy/wallpapers-doc ] && sudo -n install -D -m 644 -o root -g root $V/.unpack/deploy/wallpapers-doc/copyright /usr/share/doc/oxyos-wallpapers/copyright
fi
if [ -d $V/.unpack/deploy/sounds ]; then
  sudo -n rm -rf /usr/share/sounds/oxyos.new; sudo -n mkdir -p /usr/share/sounds
  sudo -n cp -a $V/.unpack/deploy/sounds /usr/share/sounds/oxyos.new
  sudo -n chown -R root:root /usr/share/sounds/oxyos.new; sudo -n chmod -R go-w /usr/share/sounds/oxyos.new
  test -s /usr/share/sounds/oxyos.new/stereo/trash-empty.oga && test -s /usr/share/sounds/oxyos.new/stereo/message-new-instant.oga
  sudo -n rm -rf /usr/share/sounds/oxyos.old
  [ -e /usr/share/sounds/oxyos ] && sudo -n mv /usr/share/sounds/oxyos /usr/share/sounds/oxyos.old
  sudo -n mv /usr/share/sounds/oxyos.new /usr/share/sounds/oxyos
  sudo -n install -d -m 755 -o root -g root /usr/share/doc/oxyos-sounds
  sudo -n install -m 644 -o root -g root $V/.unpack/deploy/sounds-doc/copyright /usr/share/doc/oxyos-sounds/copyright
fi
# The system entries, then the policy naming them, each staged beside its place (a dot name a
# registry never lists) and renamed over the one before: a running session never finds one missing
# or half written, so never leaves a system application out meanwhile. Then Dropsea's and Atlas's
# entries go when they are not sent, and so.oxy.stash's, the id Dropsea had before its rename
# (2 November 2026), always goes.
A=/usr/share/oxyos/applications
sudo -n install -d -m 755 -o root -g root $A
for f in $V/.unpack/deploy/entries/*.desktop; do
  n=${f##*/}
  sudo -n install -m 644 -o root -g root "$f" "$A/.$n.new"
  sudo -n mv -fT "$A/.$n.new" "$A/$n"
done
sudo -n install -m 644 -o root -g root $V/.unpack/deploy/system-apps.json /usr/share/oxyos/.system-apps.json.new
sudo -n mv -fT /usr/share/oxyos/.system-apps.json.new /usr/share/oxyos/system-apps.json
for n in so.oxy.dropsea.desktop so.oxy.atlas.desktop; do
  [ -e "$V/.unpack/deploy/entries/$n" ] || sudo -n rm -f "$A/$n"
done
sudo -n rm -f $A/so.oxy.stash.desktop
# The entry the one-bundle deploy installed: its id is the policy's now, its bundle gone from it.
sudo -n rm -f /usr/local/share/oxyos/applications/so.oxy.settings.desktop
# Inbox and Allo, the user's: their packages and the development registry naming them.
rm -rf packages registry && cp -a $V/.unpack/deploy/packages $V/.unpack/deploy/registry .
[ -e probar.sh.before-packages ] || [ ! -e probar.sh ] || cp -a probar.sh probar.sh.before-packages
install -m 755 $V/.unpack/deploy/probar.sh probar.sh
rm -rf $V/.unpack
sudo -n systemctl stop oxyos-probar 2>/dev/null || true; sleep 2; sudo -n systemctl reset-failed oxyos-probar 2>/dev/null || true
if systemctl is-enabled -q oxyos-login 2>/dev/null; then
  # The machine signs in at the OxyOS login screen (ADR 0020, 7 November 2026): the development
  # session (probar.sh on terminal 8) is not started and the display is never switched, so nobody
  # signed in loses it. A session started from now runs what was installed above; the login
  # screen restarts onto it only while nobody is signed in on the seat.
  if loginctl list-sessions --no-legend | grep -q " seat0 .* user "; then
    echo "session: someone is signed in; the new build runs from their next sign-in"
  else
    sudo -n systemctl restart oxyos-login; sleep 6; systemctl is-active oxyos-login; echo "session: the login screen restarted on the new build"
  fi
else
  sudo -n systemd-run --unit=oxyos-probar -p PAMName=login -p TTYPath=/dev/tty8 -p User=$(id -un) -p StandardOutput=truncate:$V/probar.log -p StandardError=truncate:$V/probar.log $V/probar.sh >/dev/null
  sudo -n chvt 8; sleep 8; systemctl is-active oxyos-probar; tail -6 probar.log
fi
