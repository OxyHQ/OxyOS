# OxyOS

OxyOS is being rebuilt as a native Linux platform: a custom Oxy display compositor and protocol with BloomUI applications, and no X11, XWayland or Wayland in the target session.

**Status:** the root Rust workspace is a headless foundation. It draws no pixels, does not render Bloom and does not boot a graphical session. No OxyOS image is built or published from this repository until a native session and its boot acceptance tests exist.

- [Architecture and boundaries](docs/native-architecture.mdx)
- [BloomUI source audit and integration plan](docs/bloom-native-audit.mdx)
- [Build and verify the foundation](docs/native-foundation.mdx)
- [Base image configuration](docs/iso-build.mdx): the Debian live-build config, console only
- [Retired desktop stack](docs/retired-desktop-stack.mdx): the removed React/Tauri shell and labwc/greetd session

## Layout

| Path | Contents |
| --- | --- |
| `Cargo.toml`, `crates/`, `tools/` | Native foundation: `oxy-protocol`, `oxy-compositor-core`, `oxy-platform-probe` |
| `config/` | live-build configuration of the Debian Trixie base system |
| `scripts/` | `check-native-boundary.py` (Cargo graph), `check-image-config.py` (image config and CI) |
| `docs/` | Architecture, audits and guides; published at https://oxy.so/os |

## Checks

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/check-native-boundary.py
cargo run --locked -p oxy-platform-probe
python3 scripts/check-image-config.py
```

## Acknowledgements

The original live-build configuration carried this thanks, kept here: we'd like to thank Philip for all his hard work through the years, the legacy he's created, and the bar he's set for sleek high-performance distros.

The repository has no licence file yet. Its licence and asset provenance must be resolved before any release.
