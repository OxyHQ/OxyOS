#!/usr/bin/env python3
"""Checks tools/deploy/system-files against the tree (docs/developing.mdx, "Deploying to a test machine").

What must be installed is the tree's: each binary of tools/oxyos-session, tools/oxyos-login and
tools/oxyos-accountd (src/main.rs, the package's own, and every src/bin/*.rs), their systemd units,
PAM services (pam.d/*), sysusers and tmpfiles files; and for Safe Mode on a converted machine
(docs/adr/0054-recovery-and-erasing.mdx, decision 10) the recovery's program (tools/oxyos-recovery,
at /usr/sbin as its package installs it), each kernel-install plugin its package has and each polkit
rule the image has, at the paths they have there; and for backups (docs/adr/0059-backups.mdx) the
backup service's program (tools/oxyos-backupd, at the path its units start it from) and each of its
units. Each must be in the manifest, from where it is, and a PAM file where its header says it is
"Installed as". Every FROM must exist (a target/release binary must be one of those). Prints the cargo
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
# Crash evidence (docs/adr/0068-crash-evidence-and-kernel-panics.mdx): a native crash's backtrace.
EVIDENCE_PACKAGES = ("systemd-coredump",)
IMAGE_MANIFEST = "config/package-lists/live.list.chroot"


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
    return want


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
    errors += [f"{package} is not in {IMAGE_MANIFEST}" for package in BACKUP_PACKAGES + EVIDENCE_PACKAGES
               if package not in image]
    return errors


# What makes a system service of OxyOS never given up on (docs/adr/0061-crashes-and-resetting-an-
# application.mdx, decision 6, amended): no start limit, and restarts throttled, never stopped.
NEVER_GIVEN_UP = {
    "Unit": ["StartLimitIntervalSec=0"],
    "Service": ["Restart=always", "RestartSteps=", "RestartMaxDelaySec="],
}


def given_up_errors():
    """Every OxyOS unit that runs a service (tools/oxyos-*/*.service with an ExecStart= and no
    Type=oneshot, and not started by a socket for one conversation) is never given up on."""
    errors = []
    for path in sorted(ROOT.glob("tools/oxyos-*/*.service")):
        text = path.read_text()
        if "ExecStart=" not in text or re.search(r"^Type=oneshot", text, re.M):
            continue
        # A socket's service (StandardInput=socket) is started again by its socket, per connection.
        if re.search(r"^StandardInput=socket", text, re.M):
            continue
        sections = {}
        section = None
        for line in text.splitlines():
            if line.startswith("["):
                section = line.strip("[]")
            elif line and not line.startswith("#"):
                sections.setdefault(section, []).append(line.strip())
        for section, keys in NEVER_GIVEN_UP.items():
            for key in keys:
                if not any(line.startswith(key) for line in sections.get(section, [])):
                    errors.append(f"{path.relative_to(ROOT)} is given up on: no {key} in [{section}]")
    return errors


def main():
    entries = manifest()
    have = {source: (target, mode) for source, target, mode in entries}
    errors = backup_errors() + given_up_errors()
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
    elif sys.argv[1:] == ["--apt-evidence"]:
        print(" ".join(EVIDENCE_PACKAGES))
    else:
        print(f"tools/deploy/system-files: {len(entries)} files, as the tree has them")


main()
