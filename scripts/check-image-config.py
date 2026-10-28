#!/usr/bin/env python3
"""Check the live-build image configuration and CI against the retired desktop stack.

Scope: the files in this repository. It does not resolve Debian dependencies or
inspect the packages of a built image; that needs a real build once a native
session and its boot acceptance tests exist.
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
}

# Image publication is retired until a native session exists. A workflow that
# builds or releases an image must arrive together with boot acceptance tests.
IMAGE_PUBLICATION = re.compile(r"\blb build\b|action-gh-release|\bgh release\b|oxy-downloads")

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
                    f"{path.relative_to(ROOT)}:{number}: image build/publication {match.group(0)!r} "
                    "without a native session and boot acceptance tests"
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
    source = CONFIG / "oxyos.conf"
    installed = CONFIG / "includes.chroot_after_packages" / "etc" / "oxyos" / "oxyos.conf"
    if source.read_bytes() != installed.read_bytes():
        violations.append(f"{installed.relative_to(ROOT)} differs from {source.relative_to(ROOT)}")
    return violations


def main():
    violations = check_packages() + check_references() + check_publication() + check_hooks() + check_image_settings()
    if violations:
        print("\n".join(violations), file=sys.stderr)
        return 1
    print("Image configuration check passed.")
    print("Scope: repository files only; not a dependency closure or built-image audit.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
