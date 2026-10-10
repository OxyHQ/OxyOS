# OxyOS

> Universal standards live in `~/AGENTS.md`, Oxy-wide gotchas in `~/Oxy/AGENTS.md`. Documentation belongs in `docs/`, history in git, status in issues. This file holds only RULES, commands and pointers. **Budget: under 8 KB.**

Lightweight Debian-based desktop Linux distribution built with `live-build`. Website: https://oxy.so/os (served by OxyHQ/website, which also lists the ISO downloads at https://oxy.so/os/downloads).

## Layout

```
config/
  package-lists/              # apt packages installed into the live system
  includes.chroot_after_packages/  # files copied into target rootfs post-install
  includes.installer/         # installer-time assets
  bootloaders/                # boot splash and bootloader resources
  hooks/                      # live-build hook scripts
  preseed/                    # Debian installer automation defaults
  archives/                   # apt source list fragments
shell/                        # OxyOS desktop shell (React + Tauri v2, see below)
```

## Building the ISO

Requires Docker (recommended — keeps the build environment clean):

```bash
docker run --privileged --cap-add=ALL \
  -v /proc:/proc -v /sys:/sys \
  -v $PWD:/build -w /build \
  -it --rm debian:trixie \
  /bin/sh -c 'apt-get update && apt-get install -y live-build && mkdir -p .build && touch .build/config && lb build'
```

Or natively as root: `apt-get install live-build && lb build`

## Desktop shell (`shell/`)

Separate React + Vite + Tauri v2 app. Stack: React 19, Vite 8, Tailwind v4, xterm.js (terminal widget), Zustand. Multi-window via Tauri native APIs.

```bash
cd shell
bun install
bun run dev          # web preview (Vite only)
bun run build        # Tauri desktop build
bun run build:web    # web-only build
bun run lint         # Biome check (format + lint) + ESLint; CI runs `bunx biome ci .` and `bun run lint:react`
bun run lint:react   # ESLint: React Compiler rules only
bun run lint:fix     # Biome check --write (safe fixes + format)
bun run format       # Biome format --write
```

Biome (pinned `@biomejs/biome` in `shell/package.json`) is the formatter and main linter; config is
`shell/biome.json`. Its scope is the shell's TS/TSX/CSS/JSON only: `src-tauri/` (Rust side and
Tauri JSON), SVG assets and everything outside `shell/` (notably `config/`, which is copied into
the ISO chroot byte for byte) are deliberately not covered. Do not add Prettier. ESLint
(`shell/eslint.config.js`) runs ONLY the React Compiler rules of `eslint-plugin-react-hooks` 7,
which Biome has no equivalent for; never add a rule there that Biome can enforce.

## Notes

- The ISO build is NOT a bun/Node project — `live-build` is a Debian tool. No JS in the build pipeline.
- `shell/` is self-contained; it has its own `bun.lock` and is NOT part of a bun workspace.
- Build identifiers use the `oxyos` prefix throughout live-build config.
- Alia calls the product API through `@oxy.so/core`'s linked authenticated client;
  never construct or persist a bearer token in shell code.
