"""Boot acceptance tests for an OxyOS image, in QEMU.

    python3 tools/image/acceptance IMAGE.iso [--firmware uefi|secureboot|bios] [--arch amd64|arm64]
                                             [--out DIR]

The image's architecture comes from its name (oxyos-base-…-arm64.iso) unless --arch says it:
amd64 boots in qemu-system-x86_64 (q35, OVMF or SeaBIOS), arm64 in qemu-system-aarch64 (virt,
AAVMF; UEFI only), with KVM when the machine runs that architecture, else emulated (TCG).

Two boots of the image as built, through its own firmware path and bootloader:

1. **The boot logo** (ADR 0017). The default entry is chosen as a person would (Enter at the
   menu), so the command line is exactly the shipped one. The screen is read over QMP from then
   on, several times a second. Once the kernel has taken the display (the first black frame),
   every frame until the Oxy mark must be black: no kernel text, no cursor. The mark must be,
   pixel for pixel, the theme's own `splash-WxH.png` centred on black, as the session's splash
   is. After Plymouth goes, the console must show its prompt.
2. **The system** (a test-only command line). At the menu the default entry is edited, as a
   person edits it, to add `console=tty0 console=ttyS0,115200` (`ttyAMA0` on arm64, the virt
   machine's PL011): the kernel's standard way to put a console on the serial port, nothing in
   the image. live-config logs its live user in there as
   it does on the first terminal. Over the serial console: the boot finishes `running`, no
   system or user unit failed, the services the base needs are active, Plymouth is gone, and
   systemd's own boot time.

Screens (PNG, small), the serial log and results.json are written under --out. The exit status
is non-zero when a check fails.
"""

import argparse
import json
import pathlib
import sys
import time

import frames
from machine import FIRMWARE, SERIAL_CONSOLE, Failure, Machine, default_accel

ROOT = pathlib.Path(__file__).resolve().parents[3]
THEME = ROOT / "config" / "includes.chroot_after_packages" / "usr" / "share" / "plymouth" / "themes" / "oxyos"

# What the console base must be running once booted. Bluetooth and UPower start on demand or
# when their hardware is there, which QEMU's machine does not have.
REQUIRED_UNITS = ["systemd-journald.service", "systemd-udevd.service", "dbus.service", "systemd-logind.service",
                  "NetworkManager.service", "getty@tty1.service"]
REQUIRED_USER_UNITS = ["pipewire.socket", "pipewire-pulse.socket"]


def test_arguments(architecture):
    return f"console=tty0 console={SERIAL_CONSOLE[architecture]},115200"


def required_units(architecture):
    return REQUIRED_UNITS + [f"serial-getty@{SERIAL_CONSOLE[architecture]}.service"]


class Run:
    def __init__(self, out):
        self.out = out
        self.out.mkdir(parents=True, exist_ok=True)
        self.checks = []
        self.facts = {}

    def check(self, name, passed, detail=""):
        self.checks.append({"check": name, "passed": bool(passed), "detail": detail})
        print(f"{'PASS' if passed else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        return passed

    def save(self, frame, name):
        path = self.out / f"{name}.png"
        path.write_bytes(frame.png())
        return str(path)


class Screen:
    """Reads the machine's screen and says what it shows."""

    def __init__(self, machine, scratch):
        self.machine, self.scratch = machine, scratch
        self.expected = {}

    def grab(self):
        self.machine.screendump(self.scratch)
        return frames.read_ppm(self.scratch)

    def mark_for(self, width, height):
        key = (width, height)
        if key not in self.expected:
            path = THEME / f"splash-{width}x{height}.png"
            self.expected[key] = frames.centred_on_black(frames.read_png(path), width, height) if path.exists() else None
        return self.expected[key]

    def classify(self, frame):
        if frame.is_black():
            return "black", None
        expected = self.mark_for(frame.width, frame.height)
        if expected is not None and frame.rgb == expected.rgb:
            return "mark", None
        box = frame.lit()
        x0, y0, x1, y1 = box
        centre_x, centre_y = (x0 + x1) / 2, (y0 + y1) / 2
        side = frame.height // 8
        if (abs(centre_x - frame.width / 2) <= side / 2 and abs(centre_y - frame.height / 2) <= side / 2
                and x1 - x0 <= side * 1.5 and y1 - y0 <= side * 1.5):
            # Something mark-sized in the middle that is not the theme's pixels.
            detail = {"box": box}
            if expected is not None:
                detail["differing_pixels"], detail["largest_difference"] = frame.difference(expected)
            else:
                detail["note"] = f"the theme has no splash-{frame.size}.png to compare with"
            return "mark-mismatch", detail
        return "other", {"box": box}


def wait_for_menu(screen, timeout, settle=3.0):
    """The bootloader's menu: the first picture that stays unchanged for `settle` seconds."""
    deadline = time.monotonic() + timeout
    last, since = None, None
    while time.monotonic() < deadline:
        frame = screen.grab()
        if frame.is_black():
            last, since = None, None
        elif last is None or frame.rgb != last.rgb:
            last, since = frame, time.monotonic()
        elif time.monotonic() - since >= settle:
            return frame
        time.sleep(0.2)
    raise Failure(f"no stable bootloader menu within {timeout} s")


def boot_logo(args, run):
    machine = Machine(args.image, run.out / "boot-logo", args.firmware, args.resolution, args.display, args.accel,
                      architecture=args.arch)
    machine.start()
    run.facts["accel"] = machine.accel
    screen = Screen(machine, machine.directory / "screen.ppm")
    try:
        menu = wait_for_menu(screen, args.timeout)
        run.facts["menu_seconds"] = round(machine.elapsed(), 2)
        run.save(menu, "1-bootloader-menu")
        machine.keys("ret")
        pressed = time.monotonic()
        timeline, phase, saved = [], None, set()
        first = {}
        console_since, deadline = None, pressed + args.timeout
        before_mark_violation = None
        mismatch = None
        mark_sizes = set()
        while time.monotonic() < deadline:
            frame = screen.grab()
            at = round(time.monotonic() - pressed, 2)
            kind, detail = screen.classify(frame)
            if kind == "other" and "black" not in first and "mark" not in first:
                # Until the kernel's first black frame, the bootloader's picture is still up.
                kind = "bootloader"
            if kind not in first:
                first[kind] = at
            if (kind, frame.size) != phase:
                timeline.append({"at": at, "shows": kind, "size": frame.size, **({"detail": detail} if detail else {})})
                phase = (kind, frame.size)
            if kind == "mark":
                mark_sizes.add(frame.size)
            if kind == "other" and "mark" not in first and before_mark_violation is None:
                before_mark_violation = {"at": at, **detail}
                run.save(frame, "2-drawn-before-mark")
            name = {"black": "2-black", "mark": "3-mark", "mark-mismatch": "3-mark-mismatch"}.get(kind)
            if name and name not in saved:
                saved.add(name)
                run.save(frame, name)
            if kind == "mark-mismatch" and mismatch is None:
                mismatch = {"at": at, **detail}
            if "mark" in first and kind == "other":
                console_since = console_since or time.monotonic()
                if time.monotonic() - console_since >= 3:
                    run.save(frame, "5-console")
                    break
            else:
                console_since = None
            time.sleep(args.interval)
        run.facts["boot_logo_timeline"] = timeline
        run.facts["seconds_from_enter"] = {k: v for k, v in first.items()}
        run.check("the kernel takes the display to black, with nothing drawn on it", "black" in first,
                  f"first black frame {first.get('black')} s after Enter")
        run.check("no text or cursor between the bootloader and the Oxy mark", before_mark_violation is None,
                  f"something drawn at {before_mark_violation}" if before_mark_violation else "every frame black")
        wanted = "x".join(map(str, args.resolution))
        run.check("the Oxy mark is the theme's own pixels, centred on black",
                  "mark" in first and mismatch is None,
                  f"splash-{'/'.join(sorted(mark_sizes))}.png byte for byte, {first.get('mark')} s after Enter"
                  if "mark" in first and mismatch is None else f"mismatch {mismatch}, mark seen: {'mark' in first}")
        run.check("Plymouth draws at the display's preferred mode", mark_sizes == {wanted},
                  f"mark at {', '.join(sorted(mark_sizes)) or 'no size'}, the display prefers {wanted}")
        if "black" in first and "mark" in first:
            run.facts["dark_seconds"] = round(first["mark"] - first["black"], 2)
        run.check("the console's prompt follows the mark", console_since is not None,
                  f"console after {first.get('other')} s" if console_since else "no console within the timeout")
    finally:
        machine.stop()


def edit_entry(machine, firmware, arguments):
    """Add the test-only arguments to the default entry's command line, as a person would."""
    if firmware == "bios":
        # isolinux: Tab shows the entry's command line, ready to extend.
        machine.keys("tab")
        time.sleep(0.5)
        machine.type(" " + arguments)
        machine.keys("ret")
    else:
        # GRUB: e edits the entry. Its lines are `setparams 'Live system (<architecture>)'`, an
        # empty line, then the kernel's (live-build's grub.cfg); Ctrl+E goes to the line's end.
        machine.keys("e")
        time.sleep(1)
        machine.keys("down")
        machine.keys("down")
        machine.keys("ctrl", "e")
        machine.type(" " + arguments)
        machine.keys("ctrl", "x")


def system(args, run):
    machine = Machine(args.image, run.out / "system", args.firmware, args.resolution, args.display, args.accel,
                      architecture=args.arch)
    machine.start()
    screen = Screen(machine, machine.directory / "screen.ppm")
    arguments = test_arguments(args.arch)
    units = required_units(args.arch)
    try:
        wait_for_menu(screen, args.timeout)
        edit_entry(machine, args.firmware, arguments)
        edited = machine.elapsed()
        needle, offset = machine.wait_serial("login:", "$ ", timeout=args.timeout)
        reached = machine.elapsed()
        if needle == "login:":
            machine.send_serial("user\r")
            _, offset = machine.wait_serial("Password:", timeout=60, after=offset)
            machine.send_serial("live\r")
            machine.wait_serial("$ ", timeout=60, after=offset)
        run.check("the system reaches its console login on the serial port", True,
                  f"{'a login prompt' if needle == 'login:' else 'the live user logged in'} "
                  f"{reached - edited:.1f} s after the bootloader")
        machine.run("export SYSTEMD_PAGER= SYSTEMD_COLORS=0 TERM=dumb; stty cols 200", timeout=30)
        cmdline = machine.run("cat /proc/cmdline")
        run.facts["cmdline"] = cmdline
        run.check("the shipped command line, plus only the test's console",
                  "quiet splash loglevel=3 vt.global_cursor_default=0 plymouth.ignore-serial-consoles" in cmdline
                  and arguments in cmdline, cmdline)
        state = machine.run("systemctl is-system-running --wait", timeout=args.timeout)
        run.check("the boot finishes running, not degraded", state.strip() == "running", state.strip())
        failed = machine.run("systemctl --failed --no-legend --plain")
        run.check("no failed system unit", not failed.strip(), failed.strip() or "none")
        user_failed = machine.run("systemctl --user --failed --no-legend --plain")
        run.check("no failed user unit", not user_failed.strip(), user_failed.strip() or "none")
        active = machine.run("systemctl is-active " + " ".join(units)).split()
        inactive = [u for u, s in zip(units, active) if s != "active"]
        run.check("the base's services are active", len(active) == len(units) and not inactive,
                  ", ".join(f"{u} {s}" for u, s in zip(units, active)))
        user_active = machine.run("systemctl --user is-active " + " ".join(REQUIRED_USER_UNITS)).split()
        run.check("PipeWire listens in the user's session", user_active == ["active"] * len(REQUIRED_USER_UNITS),
                  ", ".join(f"{u} {s}" for u, s in zip(REQUIRED_USER_UNITS, user_active)))
        plymouth = machine.run("pidof plymouthd || echo gone")
        run.check("Plymouth has quit", plymouth.strip() == "gone", plymouth.strip())
        sources = machine.run("ls /etc/apt/sources.list /etc/apt/sources.list.d/ 2>&1 | tr '\\n' ' '")
        run.check("apt's sources are deb822, the first boot's setup done",
                  "debian.sources" in sources and "No such file" in sources and "oxyos.list" not in sources, sources)
        identity = machine.run(". /etc/os-release && printf '%s %s\\n' \"$ID\" \"$VERSION_ID\"")
        run.check("the system is OxyOS's base", identity.startswith("oxyos "), identity)
        machine_architecture = machine.run("dpkg --print-architecture").strip()
        run.check("the system is the image's architecture", machine_architecture == args.arch,
                  f"{machine_architecture} ({machine.run('uname -m').strip()})")
        if args.firmware != "bios":
            secure = machine.run("od -An -t u1 /sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"
                                 " 2>/dev/null | awk '{print $NF}'").strip()
            wanted = "1" if args.firmware == "secureboot" else ""
            run.facts["secure_boot"] = secure == "1"
            run.check("booted through UEFI" + (" with Secure Boot on (shim)" if wanted else ""),
                      (secure == "1") == (wanted == "1"), f"SecureBoot variable {secure or 'off'}")
        # Each connector and its first modes: the first is the preferred one, which Plymouth takes.
        run.facts["display_modes"] = machine.run(
            "for c in /sys/class/drm/card*-*; do echo \"${c##*/} $(cat $c/status) $(head -n 3 $c/modes | tr '\\n' ' ')\"; done")
        analyze = machine.run("systemd-analyze")
        run.facts["systemd_analyze"] = analyze
        run.facts["blame"] = machine.run("systemd-analyze blame | head -n 12")
        run.facts["journal_errors"] = machine.run("journalctl -b -p err -q --no-pager -o short-monotonic | tail -n 40")
        run.check("systemd's boot time", "Startup finished" in analyze, analyze.splitlines()[0] if analyze else "")
        run.save(screen.grab(), "6-console-after-boot")
    finally:
        machine.stop()


def main():
    parser = argparse.ArgumentParser(prog="tools/image/acceptance", description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("image", type=pathlib.Path)
    parser.add_argument("--firmware", choices=["uefi", "secureboot", "bios"], default="uefi")
    parser.add_argument("--arch", choices=sorted(FIRMWARE),
                        help="the image's architecture (default: from its name, else amd64)")
    parser.add_argument("--resolution", default="1920x1200",
                        help="the display's preferred mode; the theme must have a splash of this size")
    parser.add_argument("--display", choices=["std", "virtio"],
                        help="amd64: QEMU's standard VGA (default) or virtio-vga; arm64 has virtio-gpu only")
    parser.add_argument("--accel", choices=["kvm", "tcg"])
    parser.add_argument("--timeout", type=float,
                        help="seconds for each wait (default 240 with KVM, 1500 emulated, 3000 emulated arm64)")
    parser.add_argument("--interval", type=float, default=0.02, help="seconds between screen reads")
    parser.add_argument("--only", choices=["boot-logo", "system"])
    parser.add_argument("--out", type=pathlib.Path)
    args = parser.parse_args()
    args.image = args.image.resolve()
    args.resolution = tuple(int(n) for n in args.resolution.split("x"))
    if args.arch is None:
        args.arch = next((a for a in FIRMWARE if args.image.stem.endswith("-" + a)), "amd64")
    if args.firmware not in FIRMWARE[args.arch]:
        parser.error(f"{args.arch} boots only through {' or '.join(FIRMWARE[args.arch])}")
    if args.display is None:
        args.display = "virtio" if args.arch == "arm64" else "std"
    elif args.arch == "arm64" and args.display != "virtio":
        parser.error("the virt machine has no VGA: arm64 boots with --display virtio (virtio-gpu)")
    if args.timeout is None:
        accel = args.accel or default_accel(args.arch)
        args.timeout = 240 if accel == "kvm" else 3000 if args.arch == "arm64" else 1500
    out = args.out or args.image.parent.parent / "acceptance" / args.image.stem / args.firmware
    run = Run(out.resolve())
    started = time.time()
    try:
        if args.only in (None, "boot-logo"):
            boot_logo(args, run)
        if args.only in (None, "system"):
            system(args, run)
    except Failure as error:
        run.check("the run completes", False, str(error))
    failed = [c for c in run.checks if not c["passed"]]
    result = {"image": str(args.image), "architecture": args.arch, "firmware": args.firmware,
              "resolution": "x".join(map(str, args.resolution)),
              "display": args.display, "seconds": round(time.time() - started, 1), "passed": not failed,
              "checks": run.checks, **run.facts}
    (run.out / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"{len(run.checks) - len(failed)}/{len(run.checks)} checks passed; results in {run.out}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
