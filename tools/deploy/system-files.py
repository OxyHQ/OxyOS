#!/usr/bin/env python3
"""Checks tools/deploy/system-files against the tree (docs/developing.mdx, "Deploying to a test machine").

What must be installed is the tree's: each binary of tools/oxyos-session, tools/oxyos-login and
tools/oxyos-accountd (src/main.rs, the package's own, and every src/bin/*.rs), their systemd units,
PAM services (pam.d/*), sysusers and tmpfiles files; and for Safe Mode on a converted machine
(docs/adr/0054-recovery-and-erasing.mdx, decision 10) the recovery's program (tools/oxyos-recovery,
at /usr/sbin as its package installs it), each kernel-install plugin its package has and each polkit
rule the image has, at the paths they have there; and for backups (docs/adr/0059-backups.mdx) the
backup service's program (tools/oxyos-backupd, at the path its units start it from) and each of its
units; and for quality of service (docs/adr/0066-quality-of-service.mdx) the image's udev rules,
systemd generators and drop-ins for the units systemd ships, each at its place in the image. Each must
be in the manifest, from where it is, and a PAM file where its header says it is "Installed as". Every FROM must exist (a target/release binary must be one of those). Prints the cargo
packages a deploy builds with `--packages`, and the Debian packages it installs with `--apt`: restic
and rclone, which the backup service runs, each checked to be in the image's manifest.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
STACKS = ("oxyos-session", "oxyos-login", "oxyos-accountd")
# Safe Mode's parts on a converted machine (ADR 0054, decision 10): the recovery's program, built from
# its package (tools/oxyos-recovery is `oxyos-recovery-tool`), where tools/image/programs installs it;
# its package's kernel-install plugins; and the image's polkit rules, each at its place in the image.
RECOVERY = ("oxyos-recovery-tool", "target/release/oxyos-recovery", "/usr/sbin/oxyos-recovery")
PLUGINS = "tools/oxyos-recovery/package/usr/lib/kernel/install.d"
POLKIT = "config/includes.chroot_after_packages/etc/polkit-1/rules.d"
# Backups (ADR 0059): the service's program, built from its package, where its units start it; its
# units (the service, the migration's socket and service); and the engine and its SMB client, Debian's,
# as the image's manifest has them (installed by tools/deploy/remote.sh with apt).
BACKUPS = ("oxyos-backupd", "target/release/oxyos-backupd", "/usr/libexec/oxyos-backupd")
BACKUP_UNITS = "tools/oxyos-backupd"
BACKUP_PACKAGES = ("restic", "rclone")
IMAGE_MANIFEST = "config/package-lists/live.list.chroot"
# Quality of service (docs/adr/0066-quality-of-service.mdx): what the image gives the machine's service
# managers and devices, each at its place in the image: its udev rules, its systemd generators (the
# system's and the users' managers'), and its drop-ins for units systemd ships (`<unit>.d/` beside
# the system's and the users' units). A generator is a program (0755), the rest are read (0644).
INCLUDES = "config/includes.chroot_after_packages"
QOS = ("usr/lib/udev/rules.d", "usr/lib/systemd/system-generators", "usr/lib/systemd/user-generators")
QOS_DROP_INS = ("usr/lib/systemd/system", "usr/lib/systemd/user")


def manifest():
    entries = []
    for number, line in enumerate((ROOT / "tools/deploy/system-files").read_text().splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if line:
            fields = line.split()
            if len(fields) != 3 or not re.fullmatch(r"0[0-7]{3}", fields[2]):
                sys.exit(f"tools/deploy/system-files:{number}: FROM TO MODE, not {line!r}")
            entries.append(tuple(fields))
    return entries


def required():
    """{FROM: (TO or None, MODE)} that the tree says must be installed."""
    want = {}
    for stack in STACKS:
        tool = ROOT / "tools" / stack
        if (tool / "src/main.rs").exists():
            want[f"target/release/{stack}"] = (None, "0755")
        for binary in sorted((tool / "src/bin").glob("*.rs")):
            want[f"target/release/{binary.stem}"] = (None, "0755")
        for path in sorted(tool.glob("*.service")) + sorted(tool.glob("*.sysusers")) + sorted(tool.glob("*.tmpfiles")):
            want[str(path.relative_to(ROOT))] = (None, "0644")
        for path in sorted((tool / "pam.d").glob("*")):
            # "Installed as /etc/pam.d/NAME", across a comment's line break.
            header = " ".join(line.lstrip("# ") for line in path.read_text().splitlines() if line.startswith("#"))
            said = re.search(r"Installed as (/etc/pam\.d/[\w-]+)", header)
            want[str(path.relative_to(ROOT))] = (said.group(1) if said else f"/etc/pam.d/{path.name}", "0644")
    want[RECOVERY[1]] = (RECOVERY[2], "0755")
    for path in sorted((ROOT / PLUGINS).glob("*.install")):
        want[str(path.relative_to(ROOT))] = (f"/usr/lib/kernel/install.d/{path.name}", "0755")
    for path in sorted((ROOT / POLKIT).glob("*.rules")):
        want[str(path.relative_to(ROOT))] = (f"/etc/polkit-1/rules.d/{path.name}", "0644")
    want[BACKUPS[1]] = (BACKUPS[2], "0755")
    for path in backup_units():
        want[str(path.relative_to(ROOT))] = (f"/usr/lib/systemd/system/{path.name}", "0644")
    for path in qos_files():
        image = path.relative_to(ROOT / INCLUDES)
        want[str(path.relative_to(ROOT))] = (f"/{image}", "0755" if "generators" in image.parts[-2] else "0644")
    return want


def qos_files():
    """The image's files for quality of service (QOS, QOS_DROP_INS), a generator's link as the program
    it names."""
    includes = ROOT / INCLUDES
    found = [path for directory in QOS for path in sorted((includes / directory).glob("*"))]
    found += [path for directory in QOS_DROP_INS for path in sorted((includes / directory).glob("*.d/*"))]
    return [path for path in found if path.is_file()]


def backup_units():
    tool = ROOT / BACKUP_UNITS
    return sorted(tool.glob("*.service")) + sorted(tool.glob("*.socket"))


def backup_errors():
    """The backup service's units start the program where a deploy installs it, and the Debian
    packages a deploy installs for it are in the image's manifest."""
    errors = []
    for path in backup_units():
        for start in re.findall(r"^ExecStart=(\S+)", path.read_text(), re.M):
            if start != BACKUPS[2]:
                errors.append(f"{path.relative_to(ROOT)} starts {start}, not {BACKUPS[2]}")
    image = {line.strip() for line in (ROOT / IMAGE_MANIFEST).read_text().splitlines() if not line.startswith("#")}
    errors += [f"{package} is not in {IMAGE_MANIFEST}" for package in BACKUP_PACKAGES if package not in image]
    return errors


def main():
    entries = manifest()
    have = {source: (target, mode) for source, target, mode in entries}
    errors = backup_errors()
    for source, (target, mode) in required().items():
        if source not in have:
            errors.append(f"{source} is not in tools/deploy/system-files")
        elif target and have[source][0] != target:
            errors.append(f"{source} goes to {have[source][0]}, not {target}")
        elif have[source][1] != mode:
            errors.append(f"{source} is mode {have[source][1]}, not {mode}")
    built = {source for source in required() if source.startswith("target/release/")}
    for source, _, _ in entries:
        if source.startswith("target/release/"):
            if source not in built:
                errors.append(f"{source} is no binary of {', '.join(STACKS)}, the recovery's or the backup service's")
        elif not (ROOT / source).is_file():
            errors.append(f"{source} does not exist")
    if errors:
        sys.exit("\n".join(errors))
    if sys.argv[1:] == ["--packages"]:
        print(" ".join(f"-p {package}" for package in (*STACKS, RECOVERY[0], BACKUPS[0])))
    elif sys.argv[1:] == ["--apt"]:
        print(" ".join(BACKUP_PACKAGES))
    else:
        print(f"tools/deploy/system-files: {len(entries)} files, as the tree has them")


main()
