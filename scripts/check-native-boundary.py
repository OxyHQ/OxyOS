#!/usr/bin/env python3
"""Check the resolved native workspace; the image config is check-image-config.py's.

Two checks over `cargo metadata`:

1. No forbidden third-party crate (X11, Wayland, toolkits) is resolved, and no workspace member
   lives in a retired build path.
2. The crate graph keeps its layers (docs/native-architecture.mdx, Crate layers): every workspace
   member has a layer in `LAYERS`, and each of its dependencies on another member points at a layer
   its own may use (`MAY_USE`), never at a program (no tool is another tool's library), and never at
   a crate whose `VISIBLE_TO` leaves the depender out. Today's exceptions are `ALLOWED`, each with
   its reason; an allowed edge the graph no longer has fails too, so the list only shrinks.

Only normal and build dependencies count: a test may reach any crate (dev-dependencies are not
linked into what ships). Tests: scripts/test_check_native_boundary.py."""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
FORBIDDEN = ("x11", "xcb", "wayland", "smithay", "winit", "tauri", "webkit", "gtk", "gdk", "sdl")

# The layers, lowest first, and what each holds.
#   leaf      pure data and logic: no I/O, no process, no device
#   protocol  the display protocol and its transport (the display, clients and programs speak it)
#   system    OS adapters and in-process libraries over the system: devices, sockets, files, engines
#   service   what a system or session service does: accounts, installs, files, backups, networks...
#   display   the display compositor: its policy (core) and its I/O
#   client    an application's side of the display: the client library, its painter, the C ABI
#   program   a binary (tools/*): it may use any layer below, never another program
LAYERS = {
    "leaf": (
        "oxyos-gestures", "oxyos-intl", "oxyos-package", "oxyos-preferences", "oxyos-solar",
        "oxyos-splash", "oxyos-vector",
    ),
    "protocol": ("oxyos-ipc", "oxyos-protocol"),
    "system": (
        "oxyos-app-storage", "oxyos-apps", "oxyos-audio", "oxyos-auth", "oxyos-dmabuf",
        "oxyos-gpu", "oxyos-gpu-device", "oxyos-image", "oxyos-input", "oxyos-input-methods",
        "oxyos-keychain", "oxyos-kms", "oxyos-net", "oxyos-priority", "oxyos-seat", "oxyos-seqpacket",
        "oxyos-sound", "oxyos-speech", "oxyos-vt", "oxyos-wallpaper",
    ),
    "service": (
        "oxyos-account", "oxyos-activity", "oxyos-backup", "oxyos-bluetooth", "oxyos-controls",
        "oxyos-files", "oxyos-install", "oxyos-networks", "oxyos-power", "oxyos-recovery",
        "oxyos-search",
    ),
    "display": ("oxyos-compositor", "oxyos-compositor-core"),
    "client": ("oxyos-client", "oxyos-client-ffi", "oxyos-paint"),
    "program": (
        "oxyos-accountd", "oxyos-backupd", "oxyos-boot-logo", "oxyos-gpu-client", "oxyos-installd",
        "oxyos-kms-probe", "oxyos-kms-show", "oxyos-login", "oxyos-offscreen", "oxyos-pack",
        "oxyos-platform-probe", "oxyos-reader", "oxyos-recovery-tool", "oxyos-session",
    ),
}

# The layers each layer may depend on. A client never links a service's implementation: it reaches
# a service through its socket (the service's client half belongs in a lower crate).
MAY_USE = {
    "leaf": {"leaf"},
    "protocol": {"leaf", "protocol", "system"},
    "system": {"leaf", "system"},
    "service": {"leaf", "system", "service"},
    "display": {"leaf", "protocol", "system", "service", "display"},
    "client": {"leaf", "protocol", "system", "client"},
    "program": {"leaf", "protocol", "system", "service", "display", "client"},
}

# Crates only the named members may depend on, whatever their layers.
VISIBLE_TO = {
    # The compositor is the session: only the session's programs run it.
    "oxyos-compositor": {"oxyos-session", "oxyos-offscreen"},
}

# Today's exceptions, each with why it is there and what removes it. Remove an entry with the edge.
ALLOWED = {
    ("oxyos-client-ffi", "oxyos-account"): (
        "the app runtime reuses the accounts service's socket listener, PIN and store types and "
        "Secret; they move to small crates of their own"
    ),
    ("oxyos-client-ffi", "oxyos-install"): (
        "the app runtime reuses the install service's socket wrapper (oxyos_install::socket); it "
        "moves to a service-socket crate"
    ),
    ("oxyos-client-ffi", "oxyos-files"): (
        "the app runtime serves an application's file access in process (oxyos_files::service, "
        "ADR 0028); that part moves below the services"
    ),
    ("oxyos-controls", "oxyos-protocol"): (
        "Control, FULL_LEVEL, BatteryState, WifiState and BluetoothState are defined in the wire "
        "crate; they move below it"
    ),
    ("oxyos-sound", "oxyos-protocol"): (
        "SystemSound is defined in the wire crate; it moves below it"
    ),
    ("oxyos-speech", "oxyos-protocol"): (
        "AccessibleRole, AccessibleState and Politeness are defined in the wire crate; they move "
        "below it"
    ),
    ("oxyos-offscreen", "oxyos-backupd"): (
        "the development compositor runs the backup service in process from its program crate; the "
        "service's run moves into oxyos-backup"
    ),
}


def edges(metadata):
    """Each dependency of one workspace member on another, (from, to); dev-only ones left out."""
    workspace = set(metadata["workspace_members"])
    members = {p["name"] for p in metadata["packages"] if p["id"] in workspace}
    found = set()
    for package in metadata["packages"]:
        if package["name"] not in members:
            continue
        for dependency in package["dependencies"]:
            if dependency["name"] in members and dependency.get("kind") != "dev":
                found.add((package["name"], dependency["name"]))
    return members, found


def check_layers(metadata, layers=LAYERS, may_use=MAY_USE, visible_to=VISIBLE_TO, allowed=ALLOWED):
    violations = []
    layer_of = {}
    for layer, crates in layers.items():
        for crate in crates:
            if crate in layer_of:
                violations.append(f"{crate} is in two layers: {layer_of[crate]} and {layer}")
            layer_of[crate] = layer
    members, found = edges(metadata)
    for crate in sorted(members - layer_of.keys()):
        violations.append(
            f"workspace member without a layer: {crate} "
            "(add it to LAYERS in scripts/check-native-boundary.py)"
        )
    for crate in sorted(layer_of.keys() - members):
        violations.append(f"layer table names a crate the workspace lacks: {crate}")
    for source, target in sorted(found):
        if source not in layer_of or target not in layer_of:
            continue
        reasons = []
        if layer_of[target] not in may_use[layer_of[source]]:
            reasons.append(f"a {layer_of[source]} crate may not use the {layer_of[target]} layer")
        if target in visible_to and source not in visible_to[target]:
            reasons.append(f"only {', '.join(sorted(visible_to[target]))} may use it")
        if not reasons:
            continue
        if (source, target) in allowed:
            continue
        violations.append(
            f"layering: {source} ({layer_of[source]}) -> {target} ({layer_of[target]}): "
            + "; ".join(reasons)
        )
    for source, target in sorted(allowed.keys() - found):
        violations.append(
            f"allowed edge no longer exists, remove it from ALLOWED: {source} -> {target}"
        )
    return violations


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
    return violations + check_layers(metadata)


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
    members = len(metadata["workspace_members"])
    print(f"Native dependency boundary passed ({len(metadata['packages'])} resolved packages).")
    print(
        f"Crate layers passed ({members} members in {len(LAYERS)} layers, "
        f"{len(ALLOWED)} allowed exceptions)."
    )
    print("Scope: Cargo workspace only; not an ISO, system-library or Bloom-host audit.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
