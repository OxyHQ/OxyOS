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

# The architectures config/ builds (docs/iso-build.mdx, "Architectures"). live-build reads
# config/<file>.<architecture> after config/<file>; the manifest's "#if ARCHITECTURES" blocks
# hold each one's own packages.
ARCHITECTURES = ("amd64", "arm64")
CONFFILES = ("all", "common", "bootstrap", "chroot", "binary", "source")

# What the BIOS bootloader of each architecture's image must be: none on arm64, UEFI only.
BIOS_BOOTLOADER = {"amd64": "syslinux", "arm64": ""}

# Packages that do not belong in the UEFI image whatever the architecture, with why.
NOT_IN_UEFI_IMAGE = {
    "raspi-firmware": "the Raspberry Pi boots its own firmware from a FAT partition, not this UEFI image's",
}


def text_files(*roots):
    for root in roots:
        for path in sorted(root.rglob("*")):
            if path.is_symlink() or not path.is_file():
                continue
            try:
                yield path, path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue


def expand_list(path, architecture):
    """A package list as live-build expands it for `architecture` (functions/packagelists.sh):
    "#if VAR value..." to "#endif" keeps its lines when LB_VAR holds one of the values ("#nif"
    when it holds none), not nested; other "#" lines are comments. Returns ([(line number,
    package)], [problem]). Only ARCHITECTURES conditions are allowed: the others' values
    depend on live-build's state this check cannot know."""
    entries, problems = [], []
    relative = path.relative_to(ROOT)
    block = None  # (line number, whether its lines are kept)
    for number, line in enumerate(path.read_text().splitlines(), 1):
        stripped = line.strip()
        if "%" in line or "\\" in line:
            # live-build expands a list with printf "$(cat list)": % and \ are its directives.
            problems.append(f"{relative}:{number}: % or \\ in a package list (live-build passes it to printf)")
        directive = stripped.split()
        if stripped.startswith(("#if ", "#nif ")):
            if block is not None:
                problems.append(f"{relative}:{number}: nested conditional (live-build does not support it)")
            if len(directive) < 3 or directive[1] != "ARCHITECTURES":
                problems.append(f"{relative}:{number}: only \"#if ARCHITECTURES <architecture>...\" is allowed")
                block = (number, True)
                continue
            unknown = [a for a in directive[2:] if a not in ARCHITECTURES]
            if unknown:
                problems.append(f"{relative}:{number}: unknown architecture {', '.join(unknown)} "
                                f"(config/ builds {', '.join(ARCHITECTURES)})")
            holds = architecture in directive[2:]
            block = (number, holds if directive[0] == "#if" else not holds)
        elif stripped.startswith("#endif"):
            if block is None:
                problems.append(f"{relative}:{number}: #endif without #if")
            block = None
        elif stripped.startswith("#") or not stripped:
            continue
        elif block is None or block[1]:
            entries += [(number, name) for name in stripped.split()]
    if block is not None:
        problems.append(f"{relative}:{block[0]}: conditional without #endif")
    return entries, problems


def package_lists():
    return sorted((CONFIG / "package-lists").glob("*.list*"))


def check_packages():
    violations = []
    for path in package_lists():
        relative = path.relative_to(ROOT)
        for architecture in ARCHITECTURES:
            entries, problems = expand_list(path, architecture)
            violations += problems
            for number, name in entries:
                if FORBIDDEN_PACKAGE.search(name):
                    violations.append(f"{relative}:{number}: retired package {name}")
                if SERVER_FIRMWARE.search(name):
                    violations.append(f"{relative}:{number}: server firmware {name}; OxyOS runs on laptops and desktops")
                if name in NOT_IN_UEFI_IMAGE:
                    violations.append(f"{relative}:{number}: {name}: {NOT_IN_UEFI_IMAGE[name]}")
    # A list's problems once, though every architecture reads it.
    return list(dict.fromkeys(violations))


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
    # An override for an architecture config/ does not build is never read: a typo, or a port
    # that has not been made.
    for path in sorted(CONFIG.glob("*.*")):
        stem, _, suffix = path.name.partition(".")
        if stem in CONFFILES and suffix not in ARCHITECTURES:
            violations.append(f"{path.relative_to(ROOT)}: an override for {suffix}, which config/ does not build "
                              f"({', '.join(ARCHITECTURES)})")
    for architecture in ARCHITECTURES:
        where = f"config/binary ({architecture})"
        # The graphical debian-installer runs an X.org server from the install media.
        # Source the file as live-build does, so only the effective value counts.
        gui = sourced(CONFIG / "binary", "LB_DEBIAN_INSTALLER_GUI", architecture)
        if gui != "false":
            violations.append(f'{where}: LB_DEBIAN_INSTALLER_GUI is "{gui}", must be "false"')
        # live-build's firmware detection installs whatever has a file under /lib/firmware, drivers
        # and their dependencies included (Cairo and the X11 client libraries, through astronomy
        # camera drivers): the manifest names the firmware instead.
        for setting in ("LB_FIRMWARE_CHROOT", "LB_FIRMWARE_BINARY"):
            firmware = sourced(CONFIG / "binary", setting, architecture)
            if firmware != "false":
                violations.append(f'{where}: {setting} is "{firmware}", must be "false" (the manifest names the firmware)')
        effective = sourced(CONFIG / "bootstrap", "LB_ARCHITECTURE", architecture)
        if effective != architecture:
            violations.append(f"config/bootstrap: LB_ARCHITECTURE is {effective!r} when the build asks for {architecture}")
        flavours = sourced(CONFIG / "chroot", "LB_LINUX_FLAVOURS_WITH_ARCH", architecture)
        if flavours != architecture:
            violations.append(f"config/chroot ({architecture}): LB_LINUX_FLAVOURS_WITH_ARCH is {flavours!r}, "
                              f"must be {architecture!r}")
        bios = sourced(CONFIG / "binary", "LB_BOOTLOADER_BIOS", architecture)
        if bios != BIOS_BOOTLOADER[architecture]:
            violations.append(f"{where}: LB_BOOTLOADER_BIOS is {bios!r}, must be {BIOS_BOOTLOADER[architecture]!r}")
        if sourced(CONFIG / "binary", "LB_BOOTLOADER_EFI", architecture) != "grub-efi":
            violations.append(f"{where}: LB_BOOTLOADER_EFI must be grub-efi")
    source = CONFIG / "oxyos.conf"
    installed = CONFIG / "includes.chroot_after_packages" / "etc" / "oxyos" / "oxyos.conf"
    if source.read_bytes() != installed.read_bytes():
        violations.append(f"{installed.relative_to(ROOT)} differs from {source.relative_to(ROOT)}")
    return violations


BOOT_LOGO = CONFIG / "includes.chroot_after_packages" / "usr" / "share" / "plymouth" / "themes" / "oxyos"


def manifest_packages(architecture="amd64"):
    """The packages the manifest installs on `architecture`, its conditionals expanded."""
    names = set()
    for path in package_lists():
        names.update(name for _, name in expand_list(path, architecture)[0])
    return names


def sourced(path, name, architecture="amd64"):
    """A setting as live-build sees it for `architecture`: the file, then its override
    config/<file>.<architecture> when there is one, with LB_ARCHITECTURE in the environment as
    tools/image/container.sh exports it."""
    override = path.with_name(f"{path.name}.{architecture}")
    files = [str(path)] + ([str(override)] if override.exists() else [])
    script = " && ".join(f'. "${i + 1}"' for i in range(len(files))) + f' && printf %s "${name}"'
    return subprocess.run(
        ["sh", "-c", script, "sh", *files],
        capture_output=True, text=True, env={"LB_ARCHITECTURE": architecture},
    ).stdout


def check_boot_logo():
    """The boot logo (docs/adr/0017-boot-logo.mdx): Plymouth's script plugin on DRM/KMS, with
    nothing that draws text (the label plugin brings Pango, Cairo and GLib)."""
    violations = []
    for architecture in ARCHITECTURES:
        packages = manifest_packages(architecture)
        for name in ("plymouth-themes", "plymouth-label"):
            if name in packages:
                violations.append(f"config/package-lists ({architecture}): {name} (Pango, Cairo, GLib); "
                                  "the oxyos theme needs neither")
        if "plymouth" in packages and "fontconfig" not in packages:
            violations.append(f"config/package-lists ({architecture}): plymouth without fontconfig: "
                              "Debian's initramfs hook fails for a graphical theme")
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
    for architecture in ARCHITECTURES:
        live = sourced(CONFIG / "binary", "LB_BOOTAPPEND_LIVE", architecture).split()
        for argument in ("quiet", "splash", "loglevel=3", "vt.global_cursor_default=0",
                         "plymouth.ignore-serial-consoles"):
            if argument not in live:
                violations.append(f"config/binary ({architecture}): LB_BOOTAPPEND_LIVE lacks {argument}")
        if "nosplash" not in sourced(CONFIG / "binary", "LB_BOOTAPPEND_LIVE_FAILSAFE", architecture).split():
            violations.append(f"config/binary ({architecture}): the failsafe entry must keep nosplash")
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
