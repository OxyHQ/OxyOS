"""Installing, the recovery and erasing, in QEMU (ADR 0023, ADR 0054): `--recovery`.

One machine, a blank virtual disk under the run's directory (never a disk of the host) and its
firmware's variables kept from boot to boot, as a computer keeps its disk and NVRAM:

1. **Install.** The live medium boots (its GRUB entry edited for the serial console, as the boot
   tests do), and the development installer writes the image onto the disk, encrypted, with a
   development user whose password unlocks the disk and a recovery key.
2. **The installed system.** It starts from the disk alone, through shim and systemd-boot (with
   Secure Boot on under `--firmware secureboot`); Plymouth asks for the disk's password, typed on
   the keyboard over QMP. Then, over systemd's debug shell on a second serial port (ttyS1, asked for
   by the firmware's OEM strings, `io.systemd.boot.kernel-cmdline-extra`, as a virtual machine is
   given a console without changing the image): the layout, the entries and their counting, the
   keyslots, nothing failed.
3. **The recovery, by a key.** Space held on the (USB) keyboard as the machine starts shows
   systemd-boot's menu; *OxyOS Recovery* is chosen. The recovery runs from its own partition,
   read-only, its image verified by dm-verity, with its program on the console and no login.
   *Restart* goes back to OxyOS.
4. **The recovery, by failed starts.** Three starts cut off at the password prompt; the fourth
   starts the recovery by itself, which says OxyOS could not start. *Restart* gives the tries back.
5. **Erase All Content and Settings.** In the running system, `oxyos-erase.service` (what the
   account service starts once an administrator confirmed it in Settings) arms the erase; the next
   start is the recovery, which erases the disk and installs again. The next start is a first boot:
   no password asked (the install key), no user, no home, a new machine id, a new LUKS2 volume that
   neither the old password nor the old recovery key opens and whose first 64 MiB do not hold the
   old volume's UUID; the recovery untouched.
6. **Erase and Reinstall** from the recovery's own menu, with the same result.

Screens, both serial logs and QEMU's log of each boot are kept under `--out`/recovery/.
"""

import pathlib
import re
import time

import frames
from machine import Failure, Machine

PASSWORD = "oxyos-acceptance-1"
DISK_BYTES = 24 << 30
# The firmware's OEM string systemd-boot reads: the kernel's console on the first serial port (the
# recovery's program and the kernel's messages there), systemd's debug shell on the second.
CMDLINE_EXTRA = "io.systemd.boot.kernel-cmdline-extra=console=tty0 console=ttyS0,115200 systemd.debug_shell=ttyS1"
RECOVERY_TYPE = "ec68fd51-3b26-41e5-a32e-b0dff6fc5484"


class Scenario:
    def __init__(self, args, run, helpers):
        self.args, self.run, self.helpers = args, run, helpers
        self.directory = run.out / "recovery"
        self.directory.mkdir(parents=True, exist_ok=True)
        self.disk = self.directory / "disk.raw"
        self.variables = self.directory / "VARS.fd"
        for path in (self.disk, self.variables):
            path.unlink(missing_ok=True)
        with open(self.disk, "wb") as disk:
            disk.truncate(DISK_BYTES)
        self.facts = run.facts.setdefault("recovery", {})
        theme = helpers["theme"]
        width, height = args.resolution
        self.mark = frames.centred_on_black(frames.read_png(theme / f"splash-{width}x{height}.png"), width, height)
        self.recovery_key = None
        self.old = {}
        self.m = None
        self.shown = 0

    def check(self, name, passed, detail=""):
        return self.run.check(name, passed, detail)

    def machine(self, name, medium=None):
        machine = Machine(medium, self.directory / name, self.args.firmware, self.args.resolution,
                          self.args.display, self.args.accel, memory=4096, architecture=self.args.arch,
                          disk=self.disk, variables=self.variables, smbios=[] if medium else [CMDLINE_EXTRA],
                          debug_serial=not medium, usb_keyboard=not medium)
        return machine.start()

    def screen(self):
        path = self.m.directory / "screen.ppm"
        self.m.screendump(path)
        return frames.read_ppm(path)

    def save(self, frame, name):
        self.shown += 1
        (self.m.directory / f"{self.shown:02d}-{name}.png").write_bytes(frame.png())

    # Starting.

    def wait_for_prompt(self, timeout):
        """The disk's password prompt: the boot logo, then the screen changes but for the mark (the
        passphrase field drawn under it)."""
        deadline = time.monotonic() + timeout
        seen_mark = False
        while time.monotonic() < deadline:
            frame = self.screen()
            if frame.rgb == self.mark.rgb:
                seen_mark = True
            elif seen_mark and not frame.is_black():
                return frame
            if self.m.exited():
                raise Failure("QEMU exited before the password prompt")
            time.sleep(0.1)
        raise Failure(f"no password prompt within {timeout} s")

    def unlock(self, password=PASSWORD):
        prompt = self.wait_for_prompt(self.args.timeout)
        self.save(prompt, "passphrase-field")
        self.m.type(password)
        time.sleep(0.5)
        typed = self.screen()
        self.save(typed, "passphrase-typed")
        self.m.keys("ret")
        return prompt, typed

    def shell(self, after):
        """systemd's debug shell on ttyS1 of the start after `after` (an offset in its log)."""
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            self.m.debug.sendall(b"\r")
            time.sleep(1)
            if re.search(r"[#$] ", self.m.debug_text()[after:]):
                break
            if self.m.exited():
                raise Failure("QEMU exited before the debug shell")
        else:
            raise Failure("no debug shell on ttyS1")
        self.m.run("export SYSTEMD_PAGER= SYSTEMD_COLORS=0 TERM=dumb; stty cols 250 -echo", timeout=30, debug=True)
        return lambda command, timeout=120: self.m.run(command, timeout=timeout, debug=True)

    def restart(self, sh):
        """Restarts from the running system's shell; returns the offsets the next start's output
        begins after (the console's, the debug shell's)."""
        marks = len(self.m.serial_text()), len(self.m.debug_text())
        sh("systemctl --no-block reboot", timeout=30)
        return marks

    def hold_space(self, after, timeout=120):
        """Space held as the machine starts (QEMU's keyboard does not repeat a held key: it is pressed
        again and again) until systemd-boot shows its menu."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.m.execute("send-key", keys=[{"type": "qcode", "data": "spc"}], **{"hold-time": 10})
            time.sleep(0.03)
            if "OxyOS Recovery" in self.m.serial_text()[after:]:
                time.sleep(0.5)
                return True
        return False

    # The phases.

    def install(self):
        machine = self.machine("1-install", self.args.image)
        try:
            screen = self.helpers["Screen"](machine, machine.directory / "screen.ppm")
            self.helpers["wait_for_menu"](screen, self.args.timeout)
            self.helpers["edit_entry"](machine, self.args.firmware, self.helpers["test_arguments"](self.args.arch))
            needle, offset = machine.wait_serial("login:", "$ ", timeout=self.args.timeout)
            if needle == "login:":
                machine.send_serial("user\r")
                _, offset = machine.wait_serial("Password:", timeout=60, after=offset)
                machine.send_serial("live\r")
                machine.wait_serial("$ ", timeout=60, after=offset)
            machine.run("export TERM=dumb; stty cols 200", timeout=30)
            mark = len(machine.serial_text())
            machine.send_serial("sudo oxyos-recovery install --developer --disk /dev/vda --developer-user dev\r")
            _, at = machine.wait_serial("Password for dev: ", timeout=120, after=mark)
            machine.send_serial(PASSWORD + "\r")
            _, at = machine.wait_serial("Again: ", timeout=30, after=at)
            machine.send_serial(PASSWORD + "\r")
            _, at = machine.wait_serial("Type /dev/vda to erase it and install: ", timeout=30, after=at)
            started = time.monotonic()
            machine.send_serial("/dev/vda\r")
            needle, end = machine.wait_serial("OxyOS is installed on /dev/vda", "oxyos-recovery: ", timeout=1800, after=at)
            said = machine.serial_text()[at:end]
            self.facts["install_seconds"] = round(time.monotonic() - started, 1)
            installed = needle.startswith("OxyOS is installed")
            self.check("the installer writes the image onto the disk", installed,
                       f"{self.facts['install_seconds']} s" if installed else machine.serial_text()[at:][-1200:])
            if not installed:
                raise Failure("the install failed")
            key = re.search(r"\b([cbdefghijklnrtuv]{8}(?:-[cbdefghijklnrtuv]{8}){7})\b", said)
            self.check("the installer shows the disk's recovery key once", key is not None)
            self.recovery_key = key.group(1) if key else None
            self.facts["install_steps"] = [line.strip() for line in said.splitlines() if line.strip().endswith("…")]
            machine.send_serial("sudo poweroff\r")
            machine.process.wait(180)
        finally:
            machine.stop()

    def system_checks(self, sh, label):
        state = sh("systemctl is-system-running --wait", timeout=self.args.timeout).strip()
        self.check(f"{label}: the boot finishes running, not degraded", state == "running",
                   state if state == "running" else state + "; " + sh("systemctl --failed --no-legend --plain"))
        failed = sh("systemctl --failed --no-legend --plain").strip()
        self.check(f"{label}: no failed unit", not failed, failed or "none")
        secure = sh("od -An -t u1 /sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"
                    " | awk '{print $NF}'").strip()
        wanted = "1" if self.args.firmware == "secureboot" else "0"
        self.check(f"{label}: Secure Boot is {'on' if wanted == '1' else 'off'}, as the firmware has it", secure == wanted,
                   f"SecureBoot {secure}")

    def entries(self, sh, esp="/efi"):
        return sh(f"ls {esp}/loader/entries").split()

    def snapshot(self, sh):
        return {
            "luks": sh("cryptsetup luksUUID /dev/vda3").strip(),
            "machine_id": sh("cat /etc/machine-id").strip(),
            "recovery_entry": sh("sha256sum /efi/loader/entries/oxyos-recovery.conf | cut -d' ' -f1").strip(),
            "recovery_partuuid": sh("blkid -s PARTUUID -o value /dev/vda2").strip(),
        }

    def installed(self):
        self.m = self.machine("2-installed")
        prompt, typed = self.unlock()
        self.check("Plymouth asks for the disk's password on the boot logo", prompt.rgb != self.mark.rgb)
        self.check("each character typed shows in the passphrase field", typed.rgb != prompt.rgb)
        sh = self.shell(0)
        self.system_checks(sh, "installed")
        loader = sh("bootctl status 2>/dev/null | sed -n 's/^ *Product: //p;s/^ *Current Entry: //p'")
        self.facts["loader"] = loader
        shim = sh("efibootmgr | grep -i oxyos || true")
        self.check("shim starts systemd-boot, which started OxyOS", "systemd-boot" in loader and "shimx64.efi" in shim.lower(),
                   f"{loader}; {shim}")
        layout = sh("lsblk --noheadings --raw --output NAME,PARTTYPE,PARTLABEL,FSTYPE /dev/vda")
        self.facts["layout"] = layout
        self.check("the disk is the ESP, OxyOS Recovery and an encrypted root",
                   "vda1 c12a7328-f81f-11d2-ba4b-00a0c93ec93b ESP vfat" in layout
                   and f"vda2 {RECOVERY_TYPE} OxyOS\\x20Recovery ext4" in layout
                   and "crypto_LUKS" in layout, layout)
        sh("systemctl is-active --wait oxyos-boot-tries.service", timeout=180)
        entries = self.entries(sh)
        self.facts["entries_after_first_boot"] = entries
        self.check("after a complete boot the entry has its three tries again, beside the recovery's",
                   any(re.fullmatch(r"oxyos-.+\+3\.conf", e) for e in entries) and "oxyos-recovery.conf" in entries,
                   " ".join(entries))
        slots = sh("cryptsetup luksDump /dev/vda3 | grep -c '^  [0-9]*: luks2'").strip()
        self.check("the disk opens with the developer's password and the recovery key, not the install key",
                   slots == "2" and " none " in sh("cat /etc/crypttab"), f"{slots} keyslots")
        live = sh("dpkg-query -W -f='${Package} ${db:Status-Abbrev}\\n' live-boot live-config user-setup 2>/dev/null"
                  " | grep -c ' ii' || true").strip()
        self.check("the live system's packages are purged", live == "0", live)
        self.check("the development user is there", "dev:" in sh("getent passwd dev"))
        sh("echo 'files of the person' > /home/dev/marker && sync")
        self.old = self.snapshot(sh)
        self.facts["before_erase"] = self.old
        return self.restart(sh)

    def recovery_checks(self, sh):
        cmdline = sh("cat /proc/cmdline")
        self.check("the recovery boots its own image into oxyos-recovery.target, its root hash on the command line",
                   "systemd.unit=oxyos-recovery.target" in cmdline and "dm-verity-root-hash=filesystem.squashfs:" in cmdline, cmdline)
        medium = sh("findmnt --noheadings --output SOURCE,OPTIONS /run/live/medium")
        self.check("the recovery's partition is mounted read-only", medium.startswith("/dev/vda2 ro"), medium)
        verity = sh("dmsetup table 2>/dev/null | grep -c ' verity ' || true").strip()
        self.check("the recovery's image is read through dm-verity", verity not in ("", "0"), verity)
        users = sh("getent passwd user || echo none").strip()
        self.check("the recovery has no live user and no login", users == "none"
                   and "login:" not in self.m.serial_text()[self.since:], users)
        self.system_checks(sh, "recovery")

    def recovery_by_key(self, marks):
        console, debug = marks
        shown = self.hold_space(console)
        menu = self.screen()
        self.save(menu, "boot-menu")
        self.check("holding Space as the machine starts shows the boot menu, with OxyOS Recovery", shown and not menu.is_black())
        # The menu's first entry is OxyOS; the recovery is the next.
        self.m.keys("down")
        self.m.keys("ret")
        self.since = console
        self.m.wait_serial("Choose an option: ", timeout=self.args.timeout, after=console)
        self.save(self.screen(), "recovery")
        sh = self.shell(debug)
        self.recovery_checks(sh)
        marks = len(self.m.serial_text()), len(self.m.debug_text())
        self.m.send_serial("6\r")
        self.m.wait_serial("Restarting", timeout=60, after=marks[0])
        self.unlock()
        sh = self.shell(marks[1])
        sh("systemctl is-active --wait oxyos-boot-tries.service", timeout=180)
        entries = self.entries(sh)
        self.check("Restart in the recovery goes back to OxyOS", any(re.fullmatch(r"oxyos-.+\+3\.conf", e) for e in entries),
                   " ".join(entries))
        return self.restart(sh)

    def failed_starts(self, marks):
        console, debug = marks
        for _ in range(3):
            self.wait_for_prompt(self.args.timeout)
            self.m.execute("system_reset")
            time.sleep(1)
        _, at = self.m.wait_serial("Choose an option: ", timeout=self.args.timeout, after=console)
        self.since = console
        self.check("after three starts that never complete, the recovery starts by itself and says so",
                   "couldn't start several times" in self.m.serial_text()[console:at])
        sh = self.shell(debug)
        entries = self.entries(sh, "/run/oxyos-recovery/esp")
        self.check("OxyOS's entry has no tries left", any(re.fullmatch(r"oxyos-.+\+0-3\.conf", e) for e in entries),
                   " ".join(entries))
        marks = len(self.m.serial_text()), len(self.m.debug_text())
        self.m.send_serial("6\r")
        self.m.wait_serial("Restarting", timeout=60, after=marks[0])
        self.unlock()
        sh = self.shell(marks[1])
        sh("systemctl is-active --wait oxyos-boot-tries.service", timeout=180)
        entries = self.entries(sh)
        self.check("Restart gave OxyOS its tries back", any(re.fullmatch(r"oxyos-.+\+3\.conf", e) for e in entries),
                   " ".join(entries))
        # Erase All Content and Settings, as the account service asks for it.
        armed = sh("systemctl start oxyos-erase.service && echo armed || echo refused").strip()
        request = sh("cat /efi/oxyos-recovery/erase.json")
        entries = self.entries(sh)
        self.check("oxyos-erase.service arms the erase: the request, and OxyOS's entries with no tries",
                   armed.endswith("armed") and '"state": "armed"' in request
                   and all("+0" in e for e in entries if e != "oxyos-recovery.conf"), f"{armed}; {' '.join(entries)}")
        return self.restart(sh)

    def erased(self, name, debug, started):
        """The machine after an erase: its first start."""
        sh = self.shell(debug)
        self.system_checks(sh, name)
        new = self.snapshot(sh)
        self.facts[name] = new
        people = sh("getent passwd dev || true; ls -A /home; ls /var/lib/oxyos/accounts 2>/dev/null || true").strip()
        self.check(f"{name}: no user, no home, no account is left", not people, people or "none")
        first = sh("journalctl -b -o cat | grep -c -i 'machine ID' || true").strip()
        self.check(f"{name}: a first boot, with a new machine id",
                   new["machine_id"] not in ("", self.old["machine_id"]) and first != "0", new["machine_id"])
        self.check(f"{name}: the root is a new LUKS2 volume, opened by the install key", new["luks"] and new["luks"] != self.old["luks"]
                   and "/etc/cryptsetup-keys.d/oxyos-root.key" in sh("cat /etc/crypttab"), f"{self.old['luks']} -> {new['luks']}")
        for secret, what in ((PASSWORD, "the old password"), (self.recovery_key, "the old recovery key")):
            if secret is None:
                continue
            opened = sh(f"printf %s '{secret}' | cryptsetup open --test-passphrase --key-file=- /dev/vda3"
                        " && echo opened || echo refused").strip()
            self.check(f"{name}: {what} opens nothing", opened.endswith("refused"), opened)
        trace = sh(f"head -c 67108864 /dev/vda3 | grep -a -c '{self.old['luks']}' || true").strip()
        self.check(f"{name}: the old volume's header is gone", trace == "0", f"{trace} matches of its UUID")
        self.check(f"{name}: the recovery is untouched", new["recovery_entry"] == self.old["recovery_entry"]
                   and new["recovery_partuuid"] == self.old["recovery_partuuid"])
        files = sh("ls /efi/oxyos-recovery/").split()
        entries = self.entries(sh)
        self.check(f"{name}: no request is left, and the new kernel's entry is counted",
                   "erase.json" not in files and any(re.fullmatch(r"oxyos-.+\+[0-9]\.conf", e) for e in entries),
                   " ".join(entries))
        self.facts[f"{name}_seconds"] = round(time.monotonic() - started, 1)
        self.old = {**self.old, **new}
        return sh

    def erase_from_settings(self, marks):
        console, debug = marks
        _, at = self.m.wait_serial("Erasing this computer", timeout=self.args.timeout, after=console)
        started = time.monotonic()
        self.check("the next start is the recovery, which carries the erase out", True)
        needle, end = self.m.wait_serial("It restarts into setup", "Something went wrong", timeout=1800, after=at)
        done = needle.startswith("It restarts")
        self.check("the recovery erases the disk and installs OxyOS again", done,
                   "" if done else self.m.serial_text()[at:][-1500:])
        if not done:
            raise Failure("the erase failed")
        self.facts["erase_seconds"] = round(time.monotonic() - started, 1)
        sh = self.erased("erased", len(self.m.debug_text()), started)
        return self.restart(sh)

    def reinstall_from_recovery(self, marks):
        console, debug = marks
        self.check("holding Space shows the menu again", self.hold_space(console))
        self.m.keys("down")
        self.m.keys("ret")
        _, at = self.m.wait_serial("Choose an option: ", timeout=self.args.timeout, after=console)
        self.m.send_serial("1\r")
        _, at = self.m.wait_serial("to erase this computer", timeout=60, after=at)
        started = time.monotonic()
        self.m.send_serial("erase\r")
        needle, _ = self.m.wait_serial("It restarts into setup", "Something went wrong", timeout=1800, after=at)
        done = needle.startswith("It restarts")
        self.check("Erase and Reinstall in the recovery erases the disk and installs OxyOS again", done,
                   "" if done else self.m.serial_text()[at:][-1500:])
        if not done:
            raise Failure("the reinstall failed")
        self.erased("reinstalled", len(self.m.debug_text()), started)


def recovery(args, run, helpers):
    scenario = Scenario(args, run, helpers)
    scenario.install()
    try:
        marks = scenario.installed()
        marks = scenario.recovery_by_key(marks)
        marks = scenario.failed_starts(marks)
        marks = scenario.erase_from_settings(marks)
        scenario.reinstall_from_recovery(marks)
    finally:
        if scenario.m:
            scenario.m.stop()
