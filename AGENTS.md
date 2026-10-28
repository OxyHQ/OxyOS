# OxyOS

> Universal standards live in `~/AGENTS.md`, Oxy-wide gotchas in `~/Oxy/AGENTS.md`. Documentation belongs in `docs/`, history in git, status in issues. This file holds only RULES, commands and pointers. **Budget: under 8 KB.**

Linux native-platform rebuild. Read `docs/native-architecture.mdx`, `docs/bloom-native-audit.mdx` and `docs/native-foundation.mdx` first. Target session: custom Oxy compositor/protocol + actual BloomUI, no X11/XWayland/Wayland. The root Rust workspace is the foundation. The development compositor (`oxy-offscreen --launch --kms`) can show a live Bloom app on a real output through DRM/KMS, but it is a fixture with scripted input, not a session. Never install `oxy-offscreen`, `oxy-kms-show` or the headless probe as a compositor/session.

Bloom's application-layout compositor is distinct from the OS display compositor. Preserve existing Bloom APIs and layout engine; do not create Petal or lookalike replacements. Metal/macOS code from Astro is not a required backend. Issue #9's Wayland/Petal choices are superseded. Website: https://oxy.so/os (served by OxyHQ/website).

## Checks

Native: `cargo fmt --all --check`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `python3 scripts/check-native-boundary.py`, `cargo run --locked -p oxy-platform-probe`; `oxy-offscreen` is a CPU fixture, not DRM/KMS. Use `rust-toolchain.toml`; root `target/` is generated. Add real modules as functionality arrives, not empty scaffolds.

Image: `python3 scripts/check-image-config.py`. The boundary script audits Cargo only; the image script audits `config/` and workflows only. Neither is a built-image or dependency-closure audit.

## Image rules (`config/`, see `docs/iso-build.mdx`)

- `config/` is a live-build config for a console-only Debian Trixie base. It is not the OxyOS desktop and is never published as one.
- No workflow builds or publishes an ISO. Image publication returns only with a real compositor/runtime, boot acceptance tests and a final package-contents audit. No fake binaries, placeholder login, missing-binary success, fallback session or compatibility launcher.
- Never add X.org, XWayland, Wayland compositors, greeters, GTK/Qt/WebKit runtimes or desktop apps to the manifest, and never export `DISPLAY`/`WAYLAND_DISPLAY`/`GDK_BACKEND`/`QT_QPA_PLATFORM` from session or build paths. The retired React/Tauri shell is gone; recover history from `6c35e60`, do not reintroduce it (`docs/retired-desktop-stack.mdx`).
- `package-lists/live.list.chroot` is the one package manifest. PipeWire is the one audio stack.
- `config/oxyos.conf` and `includes.chroot_after_packages/etc/oxyos/oxyos.conf` stay identical. Build identifiers use the `oxyos` prefix.
- Hooks are POSIX `sh`; no device-group grants or pre-created accounts (live-config owns the live user).
- The ISO build is not a bun/Node project; there is no JS in the build pipeline.
