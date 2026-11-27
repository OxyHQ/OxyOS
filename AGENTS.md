# OxyOS

Native Linux platform rebuild: a custom OxyOS display compositor and protocol running BloomUI apps, with no X11, XWayland or Wayland.

## Commands

```bash
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/check-native-boundary.py        # Cargo graph audit
python3 scripts/check-image-config.py           # config/ and workflows
cargo run --locked -p oxyos-platform-probe
npx --yes @biomejs/biome@2.5.15 ci apps tools/rn-host-spike/sample tools/shell-preview scripts
pipx run ruff==0.17.4 check .
tools/dev/bootstrap                             # fetch everything pinned in tools/dev/pins
```

## Map

- `docs/native-architecture.mdx`, `docs/bloom-native-audit.mdx`, `docs/native-foundation.mdx`: read first.
- `docs/developing.mdx`: a machine from nothing, the gate, deploys, the gate's machine.
- `docs/iso-build.mdx`: the console base image config in `config/`.
- `docs/adr/`: decisions, one per file. `docs/agent-rules.mdx`: the full rule set behind this file.
- Entry points: `crates/` (Rust workspace), `tools/` (binaries and dev tooling), `scripts/` (CI audits).

## Conventions that differ from defaults

- OS parts use the `oxyos` prefix: crates and binaries `oxyos-*`, C ABI `oxyos_*`, env vars `OXYOS_*`. Plain `oxy`/`Oxy` means the Oxy ecosystem (accounts, SDK, `@oxy.so/*`).
- Bloom's application-layout compositor is separate from the OS display compositor. Keep existing Bloom APIs; do not create Petal or lookalike replacements.
- Machine-specific values go in `tools/dev/local.conf` (git-ignored), never in the repo.
- Add real modules as functionality arrives, not empty scaffolds.
- Rust uses `rust-toolchain.toml`; root `target/` is generated. Biome and ruff are lint-only with baselines at `warn` (`biome.jsonc`, `ruff.toml`); lint rules: [lint-and-format](https://github.com/OxyHQ/oxy/blob/main/docs/engineering/lint-and-format.md).

## Boundaries

Never:
- install `oxyos-offscreen`, `oxyos-kms-show` or the headless probe as a session: they are fixtures with scripted input. The session is `oxyos-session`.
- add X.org, XWayland, Wayland compositors, GTK/Qt/WebKit runtimes or desktop apps to the image manifest, or export `DISPLAY`/`WAYLAND_DISPLAY`/`GDK_BACKEND`/`QT_QPA_PLATFORM`: the target has no such stack.
- reintroduce the retired React/Tauri shell (history at `6c35e60`, `docs/retired-desktop-stack.mdx`).
- publish an ISO or add fake binaries, placeholder login, fallback sessions or compatibility launchers: images return only with a real runtime, boot acceptance and a contents audit.
- run destructive steps (install, erase) on a real disk: QEMU disk images only.
- build the image in `/tmp`: use `oxyos-build` in your home directory via `tools/image/build.sh` (local only, never CI).

Ask first: changing the image package manifest, `config/oxyos.conf`, or the merge gate (`tools/gate/`).

## Testing

- Rust or protocol change: fmt, test, clippy and `check-native-boundary.py`.
- `config/` or workflow change: `check-image-config.py`.
- `apps/`, `scripts/` or `tools/` JS: the Biome command; Python: ruff.
- Image or boot change: `tools/image/build.sh`, then `python3 tools/image/acceptance ISO` in QEMU (`docs/iso-build.mdx`).
- Done means these pass locally and the merge queue's Scenarios run is green.

## PR and commit rules

- Merge with `gh pr merge N --auto`: the queue's Scenarios run of the combined tree (`tools/gate/run-scenarios.sh`) is the gate; a failing run removes the PR with its failure. The `gate` label runs it on a PR first.
- Docs go in `docs/`, history in git, status in issues.

## Secrets

None are needed to build or test. Machine and deploy credentials live in `tools/dev/local.conf` (git-ignored); never print or commit them.

## Links

- [Developing](docs/developing.mdx), [Agent rules](docs/agent-rules.mdx), [Image build](docs/iso-build.mdx)
- [Oxy engineering standards](https://github.com/OxyHQ/oxy/blob/main/docs/engineering/standards.md)
- [Agent team workflow](https://github.com/OxyHQ/oxy/blob/main/docs/engineering/agent-team-workflow.md)

Owner: Nate Isern (OxyHQ). Last reviewed: 2026-11-11.
