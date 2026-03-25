# OxyOS Shell

The OxyOS desktop shell — a ChromeOS-inspired UI built with React, TypeScript, Tailwind CSS, and Tauri v2.

## Prerequisites

- [Node.js](https://nodejs.org/) 20+
- [pnpm](https://pnpm.io/) 9+
- [Rust](https://www.rust-lang.org/tools/install) (stable)
- Tauri v2 CLI: `cargo install tauri-cli --version "^2"`
- System dependencies (Debian/Ubuntu):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
  ```

## Quick Start

```bash
cd shell
pnpm install
pnpm dev          # Runs Vite dev server (browser-only mode)
```

To run inside the Tauri native window:

```bash
cargo tauri dev
```

## Building for Production

```bash
pnpm build                # Frontend bundle → build/
cargo tauri build          # Full native binary
```

## Wayland Compositor Setup (for native shell mode)

During development the shell runs in a browser or Tauri window. For full desktop-shell mode on a Linux system, you need a Wayland compositor that supports `wlr-layer-shell`:

### Sway

Add to `~/.config/sway/config`:

```
# Disable the default bar
bar {
    mode invisible
}

# Launch OxyOS shell (adjust path)
exec /path/to/oxyos-shell
```

### Mutter (GNOME)

Use `gnome-session` with a custom session that replaces `gnome-shell` with the OxyOS shell binary.

## Architecture

```
shell/
├── src/
│   ├── components/
│   │   ├── LoginScreen/   # Lock screen with PIN auth
│   │   ├── Desktop/       # Wallpaper + window area
│   │   ├── Shelf/         # Bottom taskbar (launcher, pinned apps, tray)
│   │   ├── AppLauncher/   # Full-screen app grid
│   │   └── SystemTray/    # Quick settings panel
│   ├── stores/            # Zustand state (session, system, launcher)
│   ├── hooks/             # useClock, etc.
│   ├── styles/            # Design tokens (ChromeOS Jelly)
│   ├── App.tsx
│   └── main.tsx
└── src-tauri/             # Tauri v2 Rust backend
```

## Default PIN

The login screen accepts `1234` as the default PIN during development.
