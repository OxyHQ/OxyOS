#!/usr/bin/env python3
"""Audit a built image's contents: what it installed, not what config/ asks for.

scripts/check-image-config.py checks the repository's configuration; this checks the artifact.
It reads what `tools/image/build.sh audit` extracts from an image (the live root's dpkg database
and file list, the ISO's file list, the initramfs's file list and the boot logo from both):

- no installed package is a display server, Wayland compositor, greeter, toolkit runtime or
  desktop application (check-image-config.py's own list), and no file is one either;
- every installed package is in the manifest or in its dependency closure (Pre-Depends,
  Depends and Recommends, as the build installs them) from what live-build adds itself
  (debootstrap's required and important packages, the kernel, the firmware), and every
  manifest package is installed;
- the configuration's includes are in the image unchanged, and the boot logo is in the
  initramfs with Plymouth configured for it;
- sizes: the ISO, the root, and the largest packages.

Known residue (Debian's packaging, documented in docs/iso-build.mdx) is reported; it fails only
with --gate, the bar a published image must clear (AGENTS.md's image rules).

Usage: check-image-contents.py AUDIT_DIR [--gate] [--json FILE]
"""

import argparse
import importlib.util
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CONFIG = ROOT / "config"
INCLUDES = CONFIG / "includes.chroot_after_packages"


def load_config_check():
    spec = importlib.util.spec_from_file_location("check_image_config", ROOT / "scripts" / "check-image-config.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CONFIG_CHECK = load_config_check()
FORBIDDEN_PACKAGE = CONFIG_CHECK.FORBIDDEN_PACKAGE

# Debian's packaging, not the configuration: libpipewire-0.3-modules depends on libpulse0, which
# links the X11 client libraries. Inert libraries, no server, no DISPLAY (docs/iso-build.mdx,
# "Known residue"). Each must be named here with its reason; anything else forbidden fails.
RESIDUE = {
    "libx11-6": "libpulse0 (from libpipewire-0.3-modules) links it",
    "libx11-data": "libx11-6's locale data",
    "libx11-xcb1": "libpulse0 links it",
    "libxcb1": "libx11-6 depends on it",
}

# Files that are a display server, compositor, greeter, session or toolkit, whatever package (or
# none) put them there. Matched against paths relative to the image's root.
FORBIDDEN_FILE = re.compile(
    r"^usr/(local/)?s?bin/(X|Xorg|Xwayland|xinit|startx|labwc|sway|weston|greetd|lightdm|gdm3?|sddm|lxdm)$"
    r"|^usr/lib/xorg/|^etc/X11/(xorg\.conf|xinit/|Xsession$|Xwrapper)|^usr/share/(xsessions|wayland-sessions)/"
    r"|/lib(wayland-(server|client|egl|cursor)|wlroots|gtk-[34]|gdk-[34]|Qt[56]|webkit2gtk|"
    r"javascriptcoregtk)[^/]*\.so"
)

# The residue's files: the libraries above, and /etc/X11 directories other packages keep data
# in (xkb-data's keyboard layouts, which xkbcommon reads; dbus-user-session's snippet for X
# sessions, which nothing here runs).
RESIDUE_FILE = re.compile(r"/lib(X11|X11-xcb|xcb)\.so|^etc/X11/")

# Paths a build generates in /usr that no package lists.
GENERATED = re.compile(
    r"^usr/lib/locale/locale-archive$|^usr/share/info/dir$|^usr/lib/modules/[^/]+/modules\.|"
    r"^usr/share/mime/|\.cache$|^usr/lib/x86_64-linux-gnu/gconv/gconv-modules\.cache$|"
    r"^usr/share/fonts/.*fonts\.(dir|scale)$|^usr/lib/python3[^/]*/.*__pycache__|"
    r"^usr/share/icons/[^/]+/icon-theme\.cache$|^usr/lib/firmware/|^usr/lib/udev/hwdb\.bin$|"
    r"^usr/share/glib-2\.0/schemas/gschemas\.compiled$"
)

LISTING = re.compile(
    r"^(?P<mode>\S+) (?P<owner>\S+)\s+(?P<size>\d+|\d+,\s*\d+) \d{4}-\d\d-\d\d \d\d:\d\d squashfs-root(?P<path>.*)$"
)


def parse_status(path):
    """dpkg's status file as one dict per stanza, continuation lines folded."""
    packages = {}
    leftovers = []
    for stanza in path.read_text(encoding="utf-8", errors="replace").split("\n\n"):
        fields = {}
        key = None
        for line in stanza.splitlines():
            if line[:1] in (" ", "\t") and key:
                fields[key] += "\n" + line.strip()
            elif ":" in line:
                key, value = line.split(":", 1)
                fields[key] = value.strip()
        if "Package" not in fields:
            continue
        status = fields.get("Status", "")
        if status == "install ok installed":
            packages[fields["Package"]] = fields
        elif status.endswith("config-files"):
            leftovers.append(fields["Package"])
    return packages, sorted(leftovers)


def relations(fields, *names):
    """Each relation as its list of alternatives' package names."""
    groups = []
    for name in names:
        for group in fields.get(name, "").split(","):
            alternatives = []
            for alternative in group.split("|"):
                word = alternative.strip().split(" ", 1)[0].split("(", 1)[0]
                if word:
                    alternatives.append(word.split(":", 1)[0])
            if alternatives:
                groups.append(alternatives)
    return groups


def providers(packages):
    provided = {}
    for name, fields in packages.items():
        provided.setdefault(name, set()).add(name)
        for (virtual,) in (g[:1] for g in relations(fields, "Provides")):
            provided.setdefault(virtual, set()).add(name)
    return provided


def sourced(path, name):
    return CONFIG_CHECK.sourced(path, name)


def package_files(audit):
    """Each package's paths, from dpkg's own lists in the image."""
    files = {}
    for listing in (audit / "root" / "var" / "lib" / "dpkg" / "info").glob("*.list"):
        name = listing.stem.split(":", 1)[0]
        files[name] = [line.lstrip("/") for line in listing.read_text(encoding="utf-8", errors="replace").splitlines()]
    return files


def diverted(audit):
    """Paths dpkg-divert moved: the diversion's target is the package's file under a new name."""
    path = audit / "root" / "var" / "lib" / "dpkg" / "diversions"
    lines = path.read_text().splitlines() if path.exists() else []
    return {line.lstrip("/") for line in lines[1::3]}


FIRMWARE_PATH = re.compile(r"^(usr/)?lib/firmware/.")


def roots(packages, files):
    """What is installed on purpose: the manifest, and what live-build installs itself."""
    reasons = {}
    for name in CONFIG_CHECK.manifest_packages():
        reasons[name] = "manifest"
    for name, fields in packages.items():
        if fields.get("Essential") == "yes" or fields.get("Priority") in ("required", "important"):
            reasons.setdefault(name, f"debootstrap ({fields.get('Priority', 'essential')})")
    for word in sourced(CONFIG / "common", "DEBOOTSTRAP_OPTIONS").split():
        if word.startswith("--include="):
            for name in word.split("=", 1)[1].split(","):
                reasons.setdefault(name, "debootstrap --include")
    for name in sourced(CONFIG / "chroot", "LB_KEYRING_PACKAGES").split():
        reasons.setdefault(name, "LB_KEYRING_PACKAGES")
    kernel = sourced(CONFIG / "chroot", "LB_LINUX_PACKAGES").split()
    flavours = sourced(CONFIG / "chroot", "LB_LINUX_FLAVOURS_WITH_ARCH").split()
    for base in kernel:
        for flavour in flavours:
            reasons.setdefault(f"{base}-{flavour.split(':', 1)[0]}", "LB_LINUX_PACKAGES")
    if sourced(CONFIG / "binary", "LB_FIRMWARE_CHROOT") == "true":
        # live-build installs every package of the archive areas whose files are under
        # /lib/firmware (its Contents), and the microcode.
        for name in packages:
            if (name.startswith("firmware-") or name.endswith("-microcode")
                    or any(FIRMWARE_PATH.match(path) for path in files.get(name, ()))):
                reasons.setdefault(name, "LB_FIRMWARE_CHROOT")
    return reasons


def closure(packages, reasons):
    provided = providers(packages)
    explained = {name: reason for name, reason in reasons.items() if name in packages}
    queue = list(explained)
    while queue:
        name = queue.pop()
        for alternatives in relations(packages[name], "Pre-Depends", "Depends", "Recommends"):
            for alternative in alternatives:
                for installed in provided.get(alternative, ()):
                    if installed not in explained:
                        explained[installed] = f"dependency of {name}"
                        queue.append(installed)
    return explained


def parse_listing(path):
    files = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = LISTING.match(line)
        if not match or not match.group("path"):
            continue
        rel = match.group("path").lstrip("/").split(" -> ", 1)[0]
        size = match.group("size")
        files[rel] = (match.group("mode"), 0 if "," in size else int(size))
    return files


def mib(size):
    return f"{size / 1048576:.1f} MiB"


def audit_image(audit, gate):
    violations, residue, notes = [], [], []
    report = {}
    status = audit / "root" / "var" / "lib" / "dpkg" / "status"
    if not status.exists():
        return [f"{audit}: no var/lib/dpkg/status (run tools/image/build.sh audit)"], [], [], {}
    packages, leftovers = parse_status(status)

    files_of = package_files(audit)
    reasons = roots(packages, files_of)
    explained = closure(packages, reasons)

    def chain(name):
        """Why a package is installed: the dependencies from what put it there."""
        links = [name]
        while explained.get(links[-1], "").startswith("dependency of ") and len(links) < 20:
            links.append(explained[links[-1]][len("dependency of "):])
        return " <- ".join(links) + f" ({explained.get(links[-1], 'unexplained')})"

    # 1. Forbidden packages.
    for name in sorted(packages):
        if FORBIDDEN_PACKAGE.search(name):
            if name in RESIDUE:
                residue.append(f"package {name}: {RESIDUE[name]}; {chain(name)}")
            else:
                violations.append(f"forbidden package installed: {name} {packages[name].get('Version', '')}; {chain(name)}")

    # 2. The dependency closure.
    selected = sorted(n for n, r in reasons.items() if r == "LB_FIRMWARE_CHROOT" and n in packages
                      and not (n.startswith("firmware-") or n.endswith("-microcode")))
    if selected:
        notes.append("live-build's firmware detection also installed: " + ", ".join(selected))
    unexplained = sorted(set(packages) - set(explained))
    for name in unexplained:
        violations.append(f"package {name} is neither in the manifest nor in its dependency closure")
    for name in sorted(CONFIG_CHECK.manifest_packages()):
        if name not in packages:
            violations.append(f"manifest package {name} is not installed")
    if leftovers:
        notes.append(f"removed packages with configuration left: {', '.join(leftovers)}")

    # 3. Files.
    files = parse_listing(audit / "files.txt")
    for rel in sorted(files):
        if FORBIDDEN_FILE.search(rel):
            violations.append(f"forbidden file: /{rel}")
        elif RESIDUE_FILE.search(rel):
            shown = "/".join(rel.split("/")[:3]) if rel.startswith("etc/X11/") else rel
            if f"file /{shown}" not in residue:
                residue.append(f"file /{shown}")
    owned = {path for paths in files_of.values() for path in paths} | diverted(audit)
    if owned:
        includes = {str(p.relative_to(INCLUDES)) for p in INCLUDES.rglob("*") if p.is_file()}
        unowned = sorted(
            rel for rel, (mode, _) in files.items()
            if rel.startswith("usr/") and mode.startswith("-") and rel not in owned
            and rel not in includes and not GENERATED.search(rel)
        )
        report["unowned_usr_files"] = unowned
        if unowned:
            notes.append(f"{len(unowned)} file(s) in /usr no package owns: " + ", ".join(unowned[:10])
                         + (" ..." if len(unowned) > 10 else ""))

    # 4. The configuration's includes, unchanged in the image.
    extracted = audit / "root"
    for source in sorted(p for p in INCLUDES.rglob("*") if p.is_file()):
        rel = source.relative_to(INCLUDES)
        if str(rel) not in files:
            violations.append(f"include /{rel} is not in the image")
            continue
        copy = extracted / rel
        if copy.exists() and copy.read_bytes() != source.read_bytes():
            violations.append(f"include /{rel} differs in the image")
    os_release = extracted / "etc" / "os-release"
    if os_release.exists():
        conf = (CONFIG / "oxyos.conf").read_text()
        wanted = re.search(r'^OS_ID="?([^"\n]+)"?', conf, re.M).group(1)
        if f"ID={wanted}" not in os_release.read_text().splitlines():
            violations.append(f"/etc/os-release does not say ID={wanted}")

    # 5. The boot logo in the initramfs.
    initrd = audit / "initrd-files.txt"
    if initrd.exists():
        listed = set(initrd.read_text().split())
        for rel in ("usr/share/plymouth/themes/oxyos/oxyos.script", "usr/share/plymouth/themes/oxyos/oxyos.plymouth",
                    "usr/share/plymouth/themes/oxyos/mark-1200.png", "usr/lib/x86_64-linux-gnu/plymouth/script.so"):
            if not any(path.endswith(rel) for path in listed):
                violations.append(f"initramfs lacks /{rel}")
        conf = audit / "initrd" / "etc" / "plymouth" / "plymouthd.conf"
        if not conf.exists() or "Theme=oxyos" not in conf.read_text().splitlines():
            violations.append("initramfs: /etc/plymouth/plymouthd.conf does not select Theme=oxyos")
        for name in ("plymouth-label", "label-pango.so", "label-freetype.so"):
            if any(path.endswith(name) for path in listed):
                violations.append(f"initramfs carries {name}: the theme draws no text")
    else:
        violations.append("no initramfs listing (initrd-files.txt)")

    # 6. Sizes.
    installed = sorted(((int(f.get("Installed-Size", "0")) * 1024, n) for n, f in packages.items()), reverse=True)
    iso_files = {}
    for line in (audit / "iso-files.txt").read_text(errors="replace").splitlines():
        parts = line.split()
        if len(parts) >= 9 and parts[4].isdigit():
            iso_files[parts[-1].strip("'")] = int(parts[4])
    firmware = sum(size for rel, (_, size) in files.items() if FIRMWARE_PATH.match(rel))
    report.update({
        "packages": len(packages),
        "installed_bytes": sum(s for s, _ in installed),
        "firmware_bytes": firmware,
        "firmware_selected_non_firmware": selected,
        "root_file_bytes": sum(size for _, size in files.values()),
        "squashfs_bytes": int((audit / "squashfs-size").read_text()) if (audit / "squashfs-size").exists() else None,
        "iso_bytes": int((audit / "iso-size").read_text()) if (audit / "iso-size").exists() else sum(iso_files.values()),
        "initrd_bytes": iso_files.get("/live/initrd.img"),
        "largest_packages": [{"package": n, "bytes": s} for s, n in installed[:15]],
        "largest_iso_files": [{"path": p, "bytes": s} for p, s in sorted(iso_files.items(), key=lambda i: -i[1])[:8]],
        "roots": {k: v for k, v in sorted(reasons.items()) if k in packages},
        "unexplained": unexplained,
        "residue": residue,
    })
    if gate:
        violations += [f"residue (fails the publication gate): {item}" for item in residue]
    return violations, residue, notes, report


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("audit", type=pathlib.Path, help="directory tools/image/build.sh audit extracted into")
    parser.add_argument("--gate", action="store_true", help="fail on known residue too: the publication bar")
    parser.add_argument("--json", type=pathlib.Path, help="write the report as JSON")
    args = parser.parse_args()
    violations, residue, notes, report = audit_image(args.audit, args.gate)
    if report:
        print(f"{args.audit.name}: {report['packages']} packages, {mib(report['installed_bytes'])} installed "
              f"({mib(report['firmware_bytes'])} of firmware files); root {mib(report['root_file_bytes'])} in files; "
              f"squashfs {mib(report['squashfs_bytes'] or 0)}; ISO {mib(report['iso_bytes'])}")
        print("largest packages: " + ", ".join(f"{p['package']} {mib(p['bytes'])}" for p in report["largest_packages"][:10]))
        print("largest ISO files: " + ", ".join(f"{p['path']} {mib(p['bytes'])}" for p in report["largest_iso_files"][:5]))
    for item in residue:
        print(f"residue: {item}")
    for note in notes:
        print(f"note: {note}")
    if args.json:
        args.json.write_text(json.dumps({"violations": violations, "notes": notes, **report}, indent=2) + "\n")
    if violations:
        print("\n".join(violations), file=sys.stderr)
        return 1
    print("Image contents audit passed" + (" (publication gate)." if args.gate else
          "; known residue reported, which the publication gate (--gate) refuses."))
    return 0


if __name__ == "__main__":
    sys.exit(main())
