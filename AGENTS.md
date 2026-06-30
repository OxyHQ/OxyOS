# OxyOS

Lightweight Debian-based desktop Linux distribution built with `live-build`. Website: https://os.oxy.so/

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
```

## Notes

- The ISO build is NOT a bun/Node project — `live-build` is a Debian tool. No JS in the build pipeline.
- `shell/` is self-contained; it has its own `bun.lock` and is NOT part of a bun workspace.
- Build identifiers use the `oxyos` prefix throughout live-build config.
- This repo does NOT use `@oxyhq/*` SDK packages.
