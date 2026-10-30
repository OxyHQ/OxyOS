#!/usr/bin/env python3
"""Check the live-build image configuration and CI against the retired desktop stack.

Scope: the files in this repository. It does not resolve Debian dependencies or
inspect the packages of a built image: scripts/check-image-contents.py audits a
built image (tools/image/build.sh), with this script's list of what is forbidden.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CONFIG = ROOT / "config"
WORKFLOWS = ROOT / ".github" / "workflows"

# Display servers, compositors, greeters, toolkit runtimes and the desktop
# applications of the retired session. None of them runs on the Oxy display
# protocol. Adding one back needs a supported port, not an edit to this list.
FORBIDDEN_PACKAGE = re.compile(
    r"^(xserver-|xorg|xinit|xauth|x11-|xwayland|labwc|sway|weston|libwlroots|greetd|seatd|"
    r"lxdm|lightdm|gdm3|sddm|dbus-x11|gammastep|firefox|thunar|foot|vlc|gnome-|eog|"
    r"file-roller|gparted|transmission-|gvfs|adwaita-icon-theme|dmz-cursor-theme)"
    r"|x11|xcb|wayland|gtk|gdk|webkit|appindicator|qt5|qt6"
)

# Firmware only servers and data centres have: OxyOS runs on laptops and desktops
# (docs/iso-build.mdx, "Package manifest"). Consumer hardware's firmware is in the manifest.
SERVER_FIRMWARE = re.compile(
    r"^firmware-(bnx2|bnx2x|cavium|myricom|netronome|netxen|qlogic|marvell-prestera|ast|"
    r"nvidia-tesla-[0-9]+-gsp|liquidio)$"
)

# Executables, configuration paths and environment of the retired session.
STALE_REFERENCE = re.compile(
    r"oxyos-shell|labwc|xwayland|greetd|seatd|lxdm|tauri|webkit|/etc/X11|xorg\.conf|"
    r"/install/gtk|WAYLAND_DISPLAY|\bDISPLAY=|GDK_BACKEND|QT_QPA_PLATFORM|"
    r"MOZ_ENABLE_WAYLAND|XDG_SESSION_TYPE|XDG_CURRENT_DESKTOP|\bshell/",
    re.IGNORECASE,
)

# Files whose job is to name the retired stack in order to keep it out.
STALE_REFERENCE_ALLOWED = {
    CONFIG / "archives" / "no-display-stack.pref.chroot",
    ROOT / "scripts" / "check-native-boundary.py",
    ROOT / "scripts" / "check-image-contents.py",
}

# Image publication is retired until a native session exists. A workflow that
# builds or releases an image must arrive together with the session; tools/image/
# (the local build and its acceptance tests) is for developers' machines only.
IMAGE_PUBLICATION = re.compile(r"\blb build\b|tools/image/|action-gh-release|\bgh release\b|oxy-downloads")

LIVE_BUILD_HOOKS = "/usr/share/live/build/hooks/"


def text_files(*roots):
    for root in roots:
        for path in sorted(root.rglob("*")):
            if path.is_symlink() or not path.is_file():
                continue
            try:
                yield path, path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue


def check_packages():
    violations = []
    for path in sorted((CONFIG / "package-lists").glob("*.list*")):
        for number, line in enumerate(path.read_text().splitlines(), 1):
            for name in line.split("#", 1)[0].split():
                if FORBIDDEN_PACKAGE.search(name):
                    violations.append(f"{path.relative_to(ROOT)}:{number}: retired package {name}")
                if SERVER_FIRMWARE.search(name):
                    violations.append(f"{path.relative_to(ROOT)}:{number}: server firmware {name}; OxyOS runs on laptops and desktops")
    return violations


def check_references():
    violations = []
    for path, text in text_files(CONFIG, WORKFLOWS, ROOT / "scripts"):
        if path in STALE_REFERENCE_ALLOWED or path == pathlib.Path(__file__).resolve():
            continue
        for number, line in enumerate(text.splitlines(), 1):
            match = STALE_REFERENCE.search(line)
            if match:
                violations.append(f"{path.relative_to(ROOT)}:{number}: stale reference {match.group(0)!r}")
    return violations


def check_publication():
    violations = []
    for path, text in text_files(WORKFLOWS):
        for number, line in enumerate(text.splitlines(), 1):
            match = IMAGE_PUBLICATION.search(line)
            if match:
                violations.append(
                    f"{path.relative_to(ROOT)}:{number}: image build/publication {match.group(0)!r}: "
                    "no workflow builds or publishes an image (AGENTS.md, image rules)"
                )
    return violations


def check_hooks():
    violations = []
    for path in sorted((CONFIG / "hooks").rglob("*")):
        relative = path.relative_to(ROOT)
        if path.is_symlink():
            if not str(path.readlink()).startswith(LIVE_BUILD_HOOKS):
                violations.append(f"{relative}: hook symlink outside {LIVE_BUILD_HOOKS}")
        elif path.is_file():
            result = subprocess.run(["sh", "-n", str(path)], capture_output=True, text=True)
            if result.returncode != 0:
                violations.append(f"{relative}: {result.stderr.strip()}")
    return violations


def check_includes():
    """A script the image includes must be executable (a unit's ExecStart= of a file without the
    bit fails at every boot) and, when it is sh, parse."""
    violations = []
    includes = CONFIG / "includes.chroot_after_packages"
    for path in sorted(p for p in includes.rglob("*") if p.is_file() and not p.is_symlink()):
        with path.open("rb") as file:
            first = file.readline()
        if not first.startswith(b"#!"):
            continue
        relative = path.relative_to(ROOT)
        if not path.stat().st_mode & 0o111:
            violations.append(f"{relative}: a script without the executable bit")
        if first.split()[0] in (b"#!/bin/sh", b"#!/usr/bin/sh"):
            result = subprocess.run(["sh", "-n", str(path)], capture_output=True, text=True)
            if result.returncode != 0:
                violations.append(f"{relative}: {result.stderr.strip()}")
        else:
            violations.append(f"{relative}: scripts are POSIX sh ({first.decode(errors='replace').strip()})")
    return violations


def check_image_settings():
    violations = []
    # The graphical debian-installer runs an X.org server from the install media.
    # Source the file as live-build does, so only the effective value counts.
    gui = subprocess.run(
        ["sh", "-c", '. "$1" && printf %s "$LB_DEBIAN_INSTALLER_GUI"', "sh", str(CONFIG / "binary")],
        capture_output=True, text=True, env={},
    ).stdout
    if gui != "false":
        violations.append(f'config/binary: LB_DEBIAN_INSTALLER_GUI is "{gui}", must be "false"')
    # live-build's firmware detection installs whatever has a file under /lib/firmware, drivers
    # and their dependencies included (Cairo and the X11 client libraries, through astronomy
    # camera drivers): the manifest names the firmware instead.
    for setting in ("LB_FIRMWARE_CHROOT", "LB_FIRMWARE_BINARY"):
        firmware = sourced(CONFIG / "binary", setting)
        if firmware != "false":
            violations.append(f'config/binary: {setting} is "{firmware}", must be "false" (the manifest names the firmware)')
    source = CONFIG / "oxyos.conf"
    installed = CONFIG / "includes.chroot_after_packages" / "etc" / "oxyos" / "oxyos.conf"
    if source.read_bytes() != installed.read_bytes():
        violations.append(f"{installed.relative_to(ROOT)} differs from {source.relative_to(ROOT)}")
    return violations


BOOT_LOGO = CONFIG / "includes.chroot_after_packages" / "usr" / "share" / "plymouth" / "themes" / "oxyos"


def manifest_packages():
    names = set()
    for path in sorted((CONFIG / "package-lists").glob("*.list*")):
        for line in path.read_text().splitlines():
            names.update(line.split("#", 1)[0].split())
    return names


def sourced(path, name):
    return subprocess.run(
        ["sh", "-c", f'. "$1" && printf %s "${name}"', "sh", str(path)],
        capture_output=True, text=True, env={},
    ).stdout


def check_boot_logo():
    """The boot logo (docs/adr/0017-boot-logo.mdx): Plymouth's script plugin on DRM/KMS, with
    nothing that draws text (the label plugin brings Pango, Cairo and GLib)."""
    violations = []
    packages = manifest_packages()
    for name in ("plymouth-themes", "plymouth-label"):
        if name in packages:
            violations.append(f"config/package-lists: {name} (Pango, Cairo, GLib); the oxyos theme needs neither")
    if "plymouth" in packages and "fontconfig" not in packages:
        violations.append("config/package-lists: plymouth without fontconfig: Debian's initramfs hook fails for a graphical theme")
    conf = CONFIG / "includes.chroot_after_packages" / "etc" / "plymouth" / "plymouthd.conf"
    settings = dict(
        line.split("=", 1) for line in conf.read_text().splitlines() if "=" in line and not line.startswith("#")
    ) if conf.exists() else {}
    if settings.get("Theme") != "oxyos" or settings.get("DeviceScale") != "1":
        violations.append(f"{conf.relative_to(ROOT)}: needs Theme=oxyos and DeviceScale=1")
    theme = BOOT_LOGO / "oxyos.plymouth"
    text = theme.read_text() if theme.exists() else ""
    for line in ("ModuleName=script", "ImageDir=/usr/share/plymouth/themes/oxyos",
                 "ScriptFile=/usr/share/plymouth/themes/oxyos/oxyos.script"):
        if line not in text.splitlines():
            violations.append(f"{theme.relative_to(ROOT)}: missing {line}")
    script = BOOT_LOGO / "oxyos.script"
    if not script.exists() or "Image.Text" in script.read_text():
        violations.append(f"{script.relative_to(ROOT)}: missing, or draws text (the label plugin)")
    live = sourced(CONFIG / "binary", "LB_BOOTAPPEND_LIVE").split()
    for argument in ("quiet", "splash", "loglevel=3", "vt.global_cursor_default=0"):
        if argument not in live:
            violations.append(f"config/binary: LB_BOOTAPPEND_LIVE lacks {argument}")
    if "nosplash" not in sourced(CONFIG / "binary", "LB_BOOTAPPEND_LIVE_FAILSAFE").split():
        violations.append("config/binary: the failsafe entry must keep nosplash")
    return violations


def main():
    violations = (
        check_packages() + check_references() + check_publication() + check_hooks() + check_includes()
        + check_image_settings()
        + check_boot_logo()
    )
    if violations:
        print("\n".join(violations), file=sys.stderr)
        return 1
    print("Image configuration check passed.")
    print("Scope: repository files only; scripts/check-image-contents.py audits a built image.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
