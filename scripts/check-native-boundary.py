#!/usr/bin/env python3
"""Check the resolved native workspace; the image config is check-image-config.py's."""

import json
import pathlib
import re
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
FORBIDDEN = ("x11", "xcb", "wayland", "smithay", "winit", "tauri", "webkit", "gtk", "gdk", "sdl")

# The workspace denies `unsafe` (Cargo.toml, [workspace.lints]) rather than forbidding it, so that
# these crates, and only these, may allow it where they use it, each block saying why it is sound
# (clippy's undocumented_unsafe_blocks). A crate joins this list in review, never by itself.
UNSAFE_CRATES = {
    "oxyos-account": "the peer's pidfd (SO_PEERPIDFD), one getsockopt rustix lacks",
    "oxyos-auth": "PAM's C conversation and its malloc'd answers",
    "oxyos-client-ffi": "the C ABI the Bloom host links",
    "oxyos-dmabuf": "DMA-BUF, sync file and DRM syncobj ioctls",
    "oxyos-gpu": "Vulkan (ash)",
    "oxyos-gpu-device": "Vulkan (ash), the device's identity",
    "oxyos-keychain": "mlock(2) on the keys' memory",
    "oxyos-paint": "Vulkan (ash), the client's painter",
    "oxyos-search": "the scheduling policy and I/O priority, malloc_trim",
    "oxyos-vt": "the virtual terminal's ioctls",
    "oxyos-gpu-client": "Vulkan (ash), the development GPU client",
}
# Any lint attribute naming unsafe_code: allow, expect, warn, a cfg_attr around one.
UNSAFE_LINT = re.compile(r"#!?\[[^\]]*\bunsafe_code\b")


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


def check_lints(metadata):
    """Every member takes the workspace's lints, and only UNSAFE_CRATES relax `unsafe_code`."""
    violations = []
    members = [p for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
    for package in members:
        manifest = pathlib.Path(package["manifest_path"])
        with manifest.open("rb") as file:
            lints = tomllib.load(file).get("lints")
        if lints != {"workspace": True}:
            violations.append(f"{package['name']}: lints other than `[lints] workspace = true`")
        if package["name"] in UNSAFE_CRATES:
            continue
        for source in sorted(manifest.parent.rglob("*.rs")):
            if "target" in source.relative_to(manifest.parent).parts:
                continue
            for number, line in enumerate(source.read_text().splitlines(), 1):
                if UNSAFE_LINT.search(line):
                    where = source.relative_to(ROOT)
                    violations.append(
                        f"{where}:{number}: unsafe_code relaxed outside the crates that may "
                        "(scripts/check-native-boundary.py, UNSAFE_CRATES)"
                    )
    names = {p["name"] for p in members}
    for stale in sorted(set(UNSAFE_CRATES) - names):
        violations.append(f"UNSAFE_CRATES names {stale}, which is not a workspace member")
    return violations


def main():
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    metadata = json.loads(result.stdout)
    violations = check(metadata) + check_lints(metadata)
    if violations:
        print("\n".join(violations), file=sys.stderr)
        return 1
    print(f"Native dependency boundary passed ({len(metadata['packages'])} resolved packages).")
    print("Scope: Cargo workspace only; not an ISO, system-library or Bloom-host audit.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
