# OxyOS Shell

The OxyOS desktop shell — a ChromeOS-inspired UI built with React, TypeScript, Tailwind CSS, and Tauri v2.

## Prerequisites

- [Node.js](https://nodejs.org/) 20+
- [Bun](https://bun.sh/) 1.0+
- [Rust](https://www.rust-lang.org/tools/install) (stable)
- Tauri v2 CLI: `cargo install tauri-cli --version "^2"`
- System dependencies (Debian/Ubuntu):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
  ```

## Quick Start

```bash
cd shell
bun install
bun dev           # Runs Vite dev server
```

- Open `http://localhost:5173/web.html` for the full interactive web demo
- Run `cargo tauri dev` for native multi-window mode

## Building

```bash
bun run build             # Native frontend bundle → build/
bun run build:web         # Web demo bundle → web-build/ (base: /demo/)
cargo tauri build          # Full native binary
```

## Web Demo

The shell has a separate web entry point (`web.html` → `src/web.tsx` → `WebApp.tsx`) that renders the full interactive desktop in a single browser page — no Tauri required. This is deployed to `os.oxy.so/demo`.

The web demo and native app share all components, stores, hooks, and utilities. Components accept optional callback props (e.g. `onClose`, `onToggleLauncher`) so the web demo can manage launcher visibility via React state instead of Tauri IPC.

## Multi-Window Architecture

The shell uses a **native multi-window** design. Instead of rendering everything in one webview, the shell splits into separate Tauri windows with distinct roles. This ensures proper z-ordering with native app windows — the shelf always stays on top, the launcher overlays everything, and the desktop sits behind all other windows.

### Windows

| Window | Label | Role | Native Hints |
|--------|-------|------|-------------|
| **Desktop** | `desktop` | Wallpaper, Alia assistant, OSD, notifications, settings | Maximized, no decorations |
| **Shelf** | `shelf` | Bottom taskbar — launcher button, pinned apps, system tray, clock | Always-on-top, 52px tall, anchored bottom, skip taskbar |
| **Launcher** | `launcher` | Full-screen app grid with search | Always-on-top, fullscreen, hidden by default, skip taskbar |
| **Terminal** | `terminal-N` | Native terminal windows (spawned on demand) | Decorated by custom title bar, resizable |

### Window Routing

All windows load the same `index.html` entry point. The frontend detects the current window label via `window.__TAURI_INTERNALS__.metadata.currentWebview.label` and renders the appropriate root component:

- `desktop` → `Desktop` (wallpaper + overlays)
- `shelf` → `ShelfWindow` (taskbar)
- `launcher` → `LauncherWindow` (app grid)
- `terminal-*` → `TerminalWindow` (terminal with custom title bar)

### Cross-Window Communication

- **Launcher toggle**: Shelf calls `invoke("toggle_launcher")` which shows/hides the launcher window natively
- **App launch**: Both shelf and launcher call `invoke("launch_app", { exec })` for external apps or `invoke("open_terminal")` for built-in terminal
- **System events**: The Rust backend emits `system-update` events (battery, wifi, volume, brightness) received by all windows via Tauri's event system

### Key Tauri Commands

| Command | Description |
|---------|-------------|
| `launch_app` | Spawn an external Linux process |
| `toggle_launcher` | Show/hide the launcher window |
| `show_launcher` | Show the launcher window |
| `hide_launcher` | Hide the launcher window |
| `open_terminal` | Create a new native terminal window |
| `get_battery_info` | Read battery level and charging state |
| `get_wifi_info` | Read wifi status via `nmcli` |
| `get_volume` / `set_volume` | Read/write PulseAudio volume |
| `get_brightness` / `set_brightness` | Read/write backlight brightness |
| `power_action` | Shutdown, restart, lock, or logout |
| `list_desktop_apps` | Scan `/usr/share/applications` for `.desktop` entries |

## Directory Structure

```
shell/
├── index.html                 # Native Tauri entry
├── web.html                   # Web demo entry
├── vite.config.ts             # Vite config (native build)
├── vite.web.config.ts         # Vite config (web demo build, base: /demo/)
├── src/
│   ├── main.tsx               # Native entry point
│   ├── web.tsx                # Web demo entry point
│   ├── App.tsx                # Window router (native)
│   ├── WebApp.tsx             # Single-page shell (web demo)
│   ├── components/
│   │   ├── Alia/              # AI assistant bubble + panel
│   │   ├── AppLauncher/       # Full-screen app grid + LauncherWindow root
│   │   ├── Desktop/           # Wallpaper, OSD, calendar, screenshot
│   │   ├── LoginScreen/       # Lock screen with PIN auth
│   │   ├── NotificationPanel/ # Notification panel + toast
│   │   ├── Settings/          # Settings panel
│   │   ├── Shelf/             # Bottom taskbar + ShelfWindow root
│   │   ├── SystemTray/        # Quick settings panel
│   │   └── Terminal/          # Terminal app + TerminalWindow root
│   ├── stores/                # Zustand state (session, system, settings, etc.)
│   ├── hooks/                 # useClock, useSystemInfo, useKeyboardShortcuts
│   ├── lib/                   # Tauri IPC, app registry, launch helpers
│   └── styles/                # Design tokens (ChromeOS Jelly)
└── src-tauri/                 # Tauri v2 Rust backend
    ├── src/lib.rs             # Commands, system monitor, multi-window setup
    ├── Cargo.toml
    ├── tauri.conf.json        # App config (no default windows — created in setup)
    └── capabilities/          # Permission grants per window
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

## Default PIN

The login screen accepts `1234` as the default PIN during development.
