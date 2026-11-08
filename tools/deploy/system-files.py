#!/usr/bin/env python3
"""Checks tools/deploy/system-files against the tree (docs/developing.mdx, "Deploying to a test machine").

What must be installed is the tree's: each binary of tools/oxyos-session, tools/oxyos-login and
tools/oxyos-accountd (src/main.rs, the package's own, and every src/bin/*.rs), their systemd units,
PAM services (pam.d/*), sysusers and tmpfiles files. Each must be in the manifest, from where it is,
and a PAM file where its header says it is "Installed as". Every FROM must exist (a target/release
binary must be one of those). Prints the cargo packages a deploy builds with `--packages`.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
STACKS = ("oxyos-session", "oxyos-login", "oxyos-accountd")


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
    return want


def main():
    entries = manifest()
    have = {source: (target, mode) for source, target, mode in entries}
    errors = []
    for source, (target, mode) in required().items():
        if source not in have:
            errors.append(f"{source} is not in tools/deploy/system-files")
        elif target and have[source][0] != target:
            errors.append(f"{source} goes to {have[source][0]}, its header says {target}")
        elif have[source][1] != mode:
            errors.append(f"{source} is mode {have[source][1]}, not {mode}")
    built = {source for source in required() if source.startswith("target/release/")}
    for source, _, _ in entries:
        if source.startswith("target/release/"):
            if source not in built:
                errors.append(f"{source} is no binary of {', '.join(STACKS)}")
        elif not (ROOT / source).is_file():
            errors.append(f"{source} does not exist")
    if errors:
        sys.exit("\n".join(errors))
    if sys.argv[1:] == ["--packages"]:
        print(" ".join(f"-p {stack}" for stack in STACKS))
    else:
        print(f"tools/deploy/system-files: {len(entries)} files, as the tree has them")


main()
