#!/usr/bin/env python3
"""Check the resolved native workspace; the image config is check-image-config.py's."""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
FORBIDDEN = ("x11", "xcb", "wayland", "smithay", "winit", "tauri", "webkit", "gtk", "gdk", "sdl")


def check(metadata):
    violations = []
    for package in metadata["packages"]:
        name = package["name"].lower()
        if any(fragment in name for fragment in FORBIDDEN):
            violations.append(f"forbidden native dependency: {name}")
    for package in metadata["packages"]:
        if package["id"] not in metadata["workspace_members"]:
            continue
        path = pathlib.Path(package["manifest_path"]).resolve()
        try:
            relative = path.relative_to(ROOT)
        except ValueError:
            violations.append(f"workspace member outside repository: {path}")
            continue
        if relative.parts[0] in ("shell", "config"):
            violations.append(f"retired build path in native workspace: {relative}")
    return violations


def main():
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    metadata = json.loads(result.stdout)
    violations = check(metadata)
    if violations:
        print("\n".join(violations), file=sys.stderr)
        return 1
    print(f"Native dependency boundary passed ({len(metadata['packages'])} resolved packages).")
    print("Scope: Cargo workspace only; not an ISO, system-library or Bloom-host audit.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
