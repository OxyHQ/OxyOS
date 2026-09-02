use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;
use tauri::WebviewUrl;
use tauri::WebviewWindowBuilder;

// ── Data types ──

#[derive(Serialize, Clone, PartialEq)]
pub struct BatteryInfo {
    present: bool,
    level: u8,
    charging: bool,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct WifiInfo {
    enabled: bool,
    ssid: Option<String>,
    strength: u8,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct VolumeInfo {
    level: u8,
    muted: bool,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct BrightnessInfo {
    present: bool,
    level: u8,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct SystemUpdate {
    battery: BatteryInfo,
    wifi: WifiInfo,
    bluetooth: bool,
    night_light: bool,
    volume: VolumeInfo,
    brightness: BrightnessInfo,
}

#[derive(Serialize)]
pub struct DesktopApp {
    name: String,
    exec: String,
    icon: String,
    categories: String,
}

static NIGHT_LIGHT_ENABLED: AtomicBool = AtomicBool::new(false);

fn command_error(action: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("{action} failed with status {}", output.status)
    } else {
        format!("{action} failed: {stderr}")
    }
}

fn run_command(program: &str, args: &[&str], action: &str) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{action} could not start: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(action, &output))
    }
}

// ── File → data URL helper ──

fn sniff_image_mime(data: &[u8]) -> &'static str {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(b"\x89PNG") {
        "image/png"
    } else if data.starts_with(b"GIF8") {
        "image/gif"
    } else if data.starts_with(b"\x00\x00\x01\x00") {
        "image/x-icon"
    } else if data.windows(4).take(64).any(|w| w == b"<svg") || data.starts_with(b"<?xml") {
        "image/svg+xml"
    } else {
        "image/png"
    }
}

fn file_as_data_url(path: &str) -> Option<String> {
    use base64::Engine;
    let data = fs::read(path).ok()?;
    if data.is_empty() {
        return None;
    }
    let mime = sniff_image_mime(&data);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    Some(format!("data:{mime};base64,{b64}"))
}

// ── System readers ──

fn read_battery() -> BatteryInfo {
    let entries = match fs::read_dir("/sys/class/power_supply/") {
        Ok(e) => e,
        Err(_) => {
            return BatteryInfo {
                present: false,
                level: 0,
                charging: false,
            }
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" {
            continue;
        }
        let level = fs::read_to_string(path.join("capacity"))
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(0);
        let charging = fs::read_to_string(path.join("status"))
            .ok()
            .map(|s| s.trim() == "Charging")
            .unwrap_or(false);
        return BatteryInfo {
            present: true,
            level,
            charging,
        };
    }
    BatteryInfo {
        present: false,
        level: 0,
        charging: false,
    }
}

fn read_wifi() -> WifiInfo {
    let radio_on = Command::new("nmcli")
        .args(["-t", "radio", "wifi"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false);
    if !radio_on {
        return WifiInfo {
            enabled: false,
            ssid: None,
            strength: 0,
        };
    }
    let output = Command::new("nmcli")
        .args(["-t", "-f", "ACTIVE,SSID,SIGNAL", "dev", "wifi"])
        .output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 3 && fields[0] == "yes" {
                    return WifiInfo {
                        enabled: true,
                        ssid: if fields[1].is_empty() {
                            None
                        } else {
                            Some(fields[1].to_string())
                        },
                        strength: fields[2].parse::<u8>().unwrap_or(0),
                    };
                }
            }
            WifiInfo {
                enabled: true,
                ssid: None,
                strength: 0,
            }
        }
        Err(_) => WifiInfo {
            enabled: false,
            ssid: None,
            strength: 0,
        },
    }
}

fn read_bluetooth() -> bool {
    Command::new("bluetoothctl")
        .arg("show")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim().starts_with("Powered:") && l.trim().ends_with("yes"))
        })
        .unwrap_or(false)
}

fn read_night_light() -> bool {
    NIGHT_LIGHT_ENABLED.load(Ordering::Relaxed)
}

fn read_volume() -> VolumeInfo {
    let level = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
        .ok()
        .and_then(|out| {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            stdout
                .split('/')
                .nth(1)
                .and_then(|s| s.trim().trim_end_matches('%').parse::<u8>().ok())
        })
        .unwrap_or(0);
    let muted = Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("yes"))
        .unwrap_or(false);
    VolumeInfo { level, muted }
}

fn read_brightness() -> BrightnessInfo {
    let entries = match fs::read_dir("/sys/class/backlight/") {
        Ok(e) => e,
        Err(_) => {
            return BrightnessInfo {
                present: false,
                level: 0,
            }
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let cur = fs::read_to_string(path.join("brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok());
        let max = fs::read_to_string(path.join("max_brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok());
        if let (Some(c), Some(m)) = (cur, max) {
            if m > 0.0 {
                return BrightnessInfo {
                    present: true,
                    level: ((c / m) * 100.0).round() as u8,
                };
            }
        }
    }
    BrightnessInfo {
        present: false,
        level: 0,
    }
}

// ── Background system monitor ──

fn start_system_monitor(app_handle: tauri::AppHandle) {
    thread::spawn(move || {
        let mut prev: Option<SystemUpdate> = None;
        loop {
            let current = SystemUpdate {
                battery: read_battery(),
                wifi: read_wifi(),
                bluetooth: read_bluetooth(),
                night_light: read_night_light(),
                volume: read_volume(),
                brightness: read_brightness(),
            };
            if prev.as_ref() != Some(&current) {
                let _ = app_handle.emit("system-update", &current);
                prev = Some(current);
            }
            thread::sleep(Duration::from_secs(5));
        }
    });
}

// ── Window setup ──
//
// Two modes:
//   • Test mode: running inside an existing desktop environment. We render
//     all shell UI (shelf, launcher, login, desktop) inside ONE Tauri window
//     so there is exactly one rendering surface. This matches how ChromeOS
//     Aura works — one compositor owns the screen — and avoids the flicker /
//     white-flash / Z-order issues that come with fighting Mutter for
//     multi-window placement.
//   • Production mode: running as the actual session shell (no other WM).
//     Spawns separate desktop/shelf/launcher windows with explicit Z-order
//     and override_redirect, because nothing else is making placement
//     decisions. Activated when XDG_CURRENT_DESKTOP is empty or "OxyOS".

const SHELF_HEIGHT: u32 = 52;

fn running_inside_existing_de() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|s| !s.is_empty() && s != "OxyOS")
        .unwrap_or(false)
}

fn create_panel_windows(
    app: &tauri::App,
    phys_w: u32,
    phys_h: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let panel_specs: &[(&str, u32, u32)] = &[
        ("quicksettings", 320, 540),
        ("notifications", 360, 560),
        ("calendar", 360, 420),
    ];

    for (label, width, height) in panel_specs {
        let panel_x = phys_w.saturating_sub(*width + 8) as f64;
        let panel_y = phys_h.saturating_sub(SHELF_HEIGHT + *height + 8) as f64;
        let panel = WebviewWindowBuilder::new(app, *label, WebviewUrl::App("index.html".into()))
            .title(*label)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .transparent(true)
            .skip_taskbar(true)
            .visible(false)
            .inner_size(*width as f64, *height as f64)
            .position(panel_x, panel_y)
            .build()?;

        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            if let Ok(gtk_window) = panel.gtk_window() {
                gtk_window.set_type_hint(gdk::WindowTypeHint::Utility);
                gtk_window.set_skip_taskbar_hint(true);
                gtk_window.set_skip_pager_hint(true);
                gtk_window.set_keep_above(true);
            }
        }

        let panel_on_blur = panel.clone();
        panel.on_window_event(move |event| {
            if let tauri::WindowEvent::Focused(false) = event {
                let _ = panel_on_blur.hide();
            }
        });

        log::info!(
            "panel {}: built {}x{} at ({}, {})",
            label,
            width,
            height,
            panel_x as i32,
            panel_y as i32
        );
    }

    Ok(())
}

fn create_shell_windows(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let monitor = app.primary_monitor()?.or_else(|| {
        app.available_monitors()
            .ok()
            .and_then(|mons| mons.into_iter().next())
    });
    let (phys_w, phys_h) = match monitor {
        Some(m) => {
            let size = m.size();
            log::info!("Monitor: {}x{}", size.width, size.height);
            (size.width, size.height)
        }
        None => {
            log::warn!("No monitor detected, falling back to 1920x1080");
            (1920, 1080)
        }
    };

    let phys_w_f = phys_w as f64;
    let phys_h_f = phys_h as f64;

    if running_inside_existing_de() {
        log::info!("Test mode: desktop + opaque shelf (ChromeOS-style layering)");

        // ── Desktop window: everything except the shelf ──
        // Regular window so app windows (firefox, etc.) can render above it.
        let _desktop =
            WebviewWindowBuilder::new(app, "desktop", WebviewUrl::App("index.html".into()))
                .title("OxyOS")
                .decorations(false)
                .resizable(false)
                .transparent(false)
                .inner_size(phys_w_f, phys_h_f)
                .position(0.0, 0.0)
                .maximized(true)
                .build()?;

        // ── Shelf window: opaque dock at the bottom ──
        // Build visible from the start so .inner_size and .position aren't deferred.
        let shelf_y = phys_h.saturating_sub(SHELF_HEIGHT) as i32;
        let shelf_w = phys_w as i32;

        let shelf = WebviewWindowBuilder::new(app, "shelf", WebviewUrl::App("index.html".into()))
            .title("OxyOS Shelf")
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .transparent(false)
            .skip_taskbar(true)
            .inner_size(phys_w_f, SHELF_HEIGHT as f64)
            .min_inner_size(phys_w_f, SHELF_HEIGHT as f64)
            .max_inner_size(phys_w_f, SHELF_HEIGHT as f64)
            .position(0.0, shelf_y as f64)
            .build()?;

        // Apply GTK Dock hint immediately so Mutter respects our position.
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            if let Ok(gtk_win) = shelf.gtk_window() {
                gtk_win.set_type_hint(gdk::WindowTypeHint::Dock);
                gtk_win.set_skip_taskbar_hint(true);
                gtk_win.set_skip_pager_hint(true);
                gtk_win.set_keep_above(true);
                gtk_win.stick();
                gtk_win.set_size_request(shelf_w, SHELF_HEIGHT as i32);
                gtk_win.resize(shelf_w, SHELF_HEIGHT as i32);
                gtk_win.move_(0, shelf_y);
            }
        }

        // Force final geometry via Tauri API too.
        let _ = shelf.set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: phys_w,
            height: SHELF_HEIGHT,
        }));
        let _ = shelf.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: 0,
            y: shelf_y,
        }));

        log::info!(
            "shelf: built at (0, {}) size {}x{}",
            shelf_y,
            shelf_w,
            SHELF_HEIGHT
        );

        create_panel_windows(app, phys_w, phys_h)?;
        return Ok(());
    }

    // ── Production mode: multi-window, OxyOS owns the screen ──
    log::info!("Production mode: multi-window (no host DE)");

    let _desktop = WebviewWindowBuilder::new(app, "desktop", WebviewUrl::App("index.html".into()))
        .title("OxyOS Desktop")
        .decorations(false)
        .resizable(false)
        .transparent(false)
        .inner_size(phys_w_f, phys_h_f)
        .position(0.0, 0.0)
        .build()?;

    let shelf_y = phys_h.saturating_sub(SHELF_HEIGHT) as i32;
    let shelf = WebviewWindowBuilder::new(app, "shelf", WebviewUrl::App("index.html".into()))
        .title("OxyOS Shelf")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .transparent(true)
        .skip_taskbar(true)
        .visible(false)
        .inner_size(phys_w_f, SHELF_HEIGHT as f64)
        .min_inner_size(phys_w_f, SHELF_HEIGHT as f64)
        .max_inner_size(phys_w_f, SHELF_HEIGHT as f64)
        .position(0.0, shelf_y as f64)
        .build()?;

    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        if let Ok(gtk_win) = shelf.gtk_window() {
            gtk_win.set_decorated(false);
            gtk_win.set_skip_taskbar_hint(true);
            gtk_win.set_skip_pager_hint(true);
            gtk_win.stick();
            gtk_win.set_default_size(phys_w as i32, SHELF_HEIGHT as i32);
            gtk_win.realize();
            if let Some(gdk_win) = gtk_win.window() {
                gdk_win.set_override_redirect(true);
            }
        }
    }

    shelf.show()?;

    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        if let Ok(gtk_win) = shelf.gtk_window() {
            if let Some(gdk_win) = gtk_win.window() {
                gdk_win.move_resize(0, shelf_y, phys_w as i32, SHELF_HEIGHT as i32);
            }
        }
    }

    let launcher = WebviewWindowBuilder::new(app, "launcher", WebviewUrl::App("index.html".into()))
        .title("OxyOS Launcher")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .transparent(true)
        .visible(false)
        .skip_taskbar(true)
        .inner_size(phys_w_f, phys_h_f)
        .position(0.0, 0.0)
        .build()?;
    launcher.hide()?;

    let launcher_on_blur = launcher.clone();
    launcher.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let _ = launcher_on_blur.hide();
        }
    });

    create_panel_windows(app, phys_w, phys_h)?;

    Ok(())
}

// ── Tauri commands ──

#[tauri::command]
async fn launch_app(exec: String) -> Result<(), String> {
    let parts = shell_words::split(&exec)
        .map_err(|error| format!("Invalid application command '{exec}': {error}"))?;
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }
    Command::new(&parts[0])
        .args(&parts[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch '{}': {}", exec, e))?;
    Ok(())
}

#[tauri::command]
async fn show_launcher(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("launcher") {
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }

    app.emit("launcher-show", ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn hide_launcher(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("launcher") {
        return window.hide().map_err(|error| error.to_string());
    }

    app.emit("launcher-hide", ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn toggle_launcher(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("launcher") {
        let is_visible = window.is_visible().map_err(|error| error.to_string())?;
        if is_visible {
            return window.hide().map_err(|error| error.to_string());
        }

        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }

    app.emit("launcher-toggle", ())
        .map_err(|error| error.to_string())
}

fn toggle_panel_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("Panel window '{label}' is not available"))?;
    let is_visible = window.is_visible().map_err(|error| error.to_string())?;
    if is_visible {
        window.hide().map_err(|error| error.to_string())?;
    } else {
        for sibling in ["quicksettings", "notifications", "calendar"] {
            if sibling != label {
                if let Some(sibling_window) = app.get_webview_window(sibling) {
                    sibling_window.hide().map_err(|error| error.to_string())?;
                }
            }
        }
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[tauri::command]
async fn toggle_quicksettings(app: tauri::AppHandle) -> Result<(), String> {
    toggle_panel_window(&app, "quicksettings")
}

#[tauri::command]
async fn toggle_notifications(app: tauri::AppHandle) -> Result<(), String> {
    toggle_panel_window(&app, "notifications")
}

#[tauri::command]
async fn toggle_calendar(app: tauri::AppHandle) -> Result<(), String> {
    toggle_panel_window(&app, "calendar")
}

static TERMINAL_COUNTER: AtomicU32 = AtomicU32::new(0);

#[tauri::command]
async fn open_terminal(app: tauri::AppHandle) -> Result<String, String> {
    let id = TERMINAL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("terminal-{}", id);
    let win = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("Terminal")
        .decorations(false)
        .inner_size(780.0, 500.0)
        .min_inner_size(400.0, 280.0)
        .transparent(true)
        .build()
        .map_err(|e| format!("Failed to create terminal window: {}", e))?;
    win.set_focus().map_err(|e| e.to_string())?;
    Ok(label)
}

#[tauri::command]
async fn get_battery_info() -> BatteryInfo {
    read_battery()
}

#[tauri::command]
async fn get_wifi_info() -> WifiInfo {
    read_wifi()
}

#[tauri::command]
async fn get_bluetooth_info() -> bool {
    read_bluetooth()
}

#[tauri::command]
async fn get_night_light_info() -> bool {
    read_night_light()
}

#[tauri::command]
async fn set_wifi(enabled: bool) -> Result<(), String> {
    run_command(
        "nmcli",
        &["radio", "wifi", if enabled { "on" } else { "off" }],
        "Setting Wi-Fi",
    )
}

#[tauri::command]
async fn set_bluetooth(enabled: bool) -> Result<(), String> {
    run_command(
        "bluetoothctl",
        &["power", if enabled { "on" } else { "off" }],
        "Setting Bluetooth",
    )
}

#[tauri::command]
async fn set_night_light(enabled: bool) -> Result<(), String> {
    let args: &[&str] = if enabled { &["-O", "4000"] } else { &["-x"] };
    run_command("gammastep", args, "Setting night light")?;
    NIGHT_LIGHT_ENABLED.store(enabled, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
async fn get_volume() -> VolumeInfo {
    read_volume()
}

#[tauri::command]
async fn set_volume(level: u8) -> Result<(), String> {
    let percentage = format!("{}%", level.min(100));
    run_command(
        "pactl",
        &["set-sink-volume", "@DEFAULT_SINK@", &percentage],
        "Setting volume",
    )
}

#[tauri::command]
async fn get_brightness() -> BrightnessInfo {
    read_brightness()
}

#[tauri::command]
async fn set_brightness(level: u8) -> Result<(), String> {
    let percentage = format!("{}%", level.min(100));
    run_command(
        "brightnessctl",
        &["set", &percentage],
        "Setting display brightness",
    )
}

#[tauri::command]
async fn power_action(action: String) -> Result<(), String> {
    match action.as_str() {
        "shutdown" => {
            Command::new("systemctl")
                .arg("poweroff")
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        "restart" => {
            Command::new("systemctl")
                .arg("reboot")
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        "lock" => {
            run_command("loginctl", &["lock-session"], "Locking session")?;
        }
        "logout" => {
            Command::new("loginctl")
                .args(["terminate-session", "self"])
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        _ => return Err(format!("Unknown action: {}", action)),
    }
    Ok(())
}

#[tauri::command]
async fn get_username() -> String {
    let login = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "user".to_string());
    if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let fields: Vec<&str> = line.splitn(7, ':').collect();
            if fields.len() >= 5 && fields[0] == login {
                let display = fields[4].split(',').next().unwrap_or("").trim();
                if !display.is_empty() {
                    return display.to_string();
                }
            }
        }
    }
    login
}

#[tauri::command]
async fn get_user_avatar() -> Option<String> {
    let login = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_else(|_| format!("/home/{}", login));
    let candidates = [
        format!("/var/lib/AccountsService/icons/{login}"),
        format!("{home}/.face"),
        format!("{home}/.face.icon"),
    ];
    candidates.iter().find_map(|p| file_as_data_url(p))
}

fn resolve_icon_path(icon: &str) -> Option<String> {
    if icon.is_empty() {
        return None;
    }
    if icon.starts_with('/') {
        if Path::new(icon).exists() {
            return Some(icon.to_string());
        }
        return None;
    }
    let search_paths = [
        format!("/usr/share/icons/hicolor/256x256/apps/{icon}.png"),
        format!("/usr/share/icons/hicolor/256x256/apps/{icon}.svg"),
        format!("/usr/share/icons/hicolor/128x128/apps/{icon}.png"),
        format!("/usr/share/icons/hicolor/scalable/apps/{icon}.svg"),
        format!("/usr/share/icons/hicolor/96x96/apps/{icon}.png"),
        format!("/usr/share/icons/hicolor/64x64/apps/{icon}.png"),
        format!("/usr/share/icons/hicolor/48x48/apps/{icon}.png"),
        format!("/usr/share/pixmaps/{icon}.png"),
        format!("/usr/share/pixmaps/{icon}.svg"),
        format!("/usr/share/pixmaps/{icon}.xpm"),
    ];
    search_paths.iter().find(|p| Path::new(p).exists()).cloned()
}

#[tauri::command]
async fn list_desktop_apps() -> Vec<DesktopApp> {
    let mut apps = Vec::new();
    let entries = match fs::read_dir("/usr/share/applications") {
        Ok(e) => e,
        Err(_) => return apps,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut name = String::new();
        let mut exec = String::new();
        let mut icon_name = String::new();
        let mut categories = String::new();
        let mut no_display = false;
        let mut in_section = false;

        for line in content.lines() {
            let t = line.trim();
            if t == "[Desktop Entry]" {
                in_section = true;
                continue;
            }
            if t.starts_with('[') {
                if in_section {
                    break;
                }
                continue;
            }
            if !in_section {
                continue;
            }

            if let Some(v) = t.strip_prefix("Name=") {
                if name.is_empty() {
                    name = v.to_string();
                }
            } else if let Some(v) = t.strip_prefix("Exec=") {
                exec = clean_exec(v);
            } else if let Some(v) = t.strip_prefix("Icon=") {
                icon_name = v.to_string();
            } else if let Some(v) = t.strip_prefix("Categories=") {
                categories = v.to_string();
            } else if t == "NoDisplay=true" {
                no_display = true;
            }
        }

        if no_display || name.is_empty() || exec.is_empty() {
            continue;
        }

        let icon = resolve_icon_path(&icon_name)
            .as_deref()
            .and_then(file_as_data_url)
            .unwrap_or_default();

        apps.push(DesktopApp {
            name,
            exec,
            icon,
            categories,
        });
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

fn clean_exec(exec: &str) -> String {
    let Ok(parts) = shell_words::split(exec) else {
        return exec.to_string();
    };

    shell_words::join(parts.into_iter().filter(|token| {
        !matches!(
            token.as_str(),
            "%u" | "%U"
                | "%f"
                | "%F"
                | "%d"
                | "%D"
                | "%n"
                | "%N"
                | "%i"
                | "%c"
                | "%k"
                | "%v"
                | "%m"
        )
    }))
}

// ── Authentication ──

#[tauri::command]
async fn verify_password(password: String) -> bool {
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = match pam::Client::with_password("login") {
            Ok(c) => c,
            Err(_) => return false,
        };
        client
            .conversation_mut()
            .set_credentials(&username, &password);
        client.authenticate().is_ok()
    })
    .await
    .unwrap_or(false)
}

// ── PTY manager ──

struct PtyEntry {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
}

#[derive(Default)]
struct PtyManager(Mutex<HashMap<String, PtyEntry>>);

#[tauri::command]
async fn pty_spawn(
    state: tauri::State<'_, PtyManager>,
    app: tauri::AppHandle,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty: {e}"))?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn: {e}"))?;
    let killer = child.clone_killer();
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("writer: {e}"))?;

    let data_event = format!("pty-data-{id}");
    let exit_event = format!("pty-exit-{id}");

    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app.emit(&data_event, text);
                }
            }
        }
        let _ = app.emit(&exit_event, ());
    });

    drop(pair.slave);

    state
        .0
        .lock()
        .map_err(|_| "PTY manager lock is poisoned".to_string())?
        .insert(
            id,
            PtyEntry {
                master: pair.master,
                writer,
                killer,
            },
        );
    Ok(())
}

#[tauri::command]
async fn pty_write(
    state: tauri::State<'_, PtyManager>,
    id: String,
    data: String,
) -> Result<(), String> {
    let mut map = state
        .0
        .lock()
        .map_err(|_| "PTY manager lock is poisoned".to_string())?;
    let entry = map
        .get_mut(&id)
        .ok_or_else(|| format!("PTY session '{id}' was not found"))?;
    entry
        .writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("write: {e}"))
}

#[tauri::command]
async fn pty_resize(
    state: tauri::State<'_, PtyManager>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let map = state
        .0
        .lock()
        .map_err(|_| "PTY manager lock is poisoned".to_string())?;
    let entry = map
        .get(&id)
        .ok_or_else(|| format!("PTY session '{id}' was not found"))?;
    entry
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("resize: {e}"))
}

#[tauri::command]
async fn pty_kill(state: tauri::State<'_, PtyManager>, id: String) -> Result<(), String> {
    let mut map = state
        .0
        .lock()
        .map_err(|_| "PTY manager lock is poisoned".to_string())?;
    if let Some(mut entry) = map.remove(&id) {
        entry.killer.kill().map_err(|e| format!("kill: {e}"))?;
    }
    Ok(())
}

// ── App entry point ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyManager::default())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            create_shell_windows(app)?;
            start_system_monitor(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            launch_app,
            show_launcher,
            hide_launcher,
            toggle_launcher,
            toggle_quicksettings,
            toggle_notifications,
            toggle_calendar,
            open_terminal,
            get_battery_info,
            get_wifi_info,
            get_bluetooth_info,
            get_night_light_info,
            set_wifi,
            set_bluetooth,
            set_night_light,
            get_volume,
            set_volume,
            get_brightness,
            set_brightness,
            power_action,
            get_username,
            get_user_avatar,
            verify_password,
            list_desktop_apps,
            pty_spawn,
            pty_write,
            pty_resize,
            pty_kill,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::clean_exec;

    #[test]
    fn clean_exec_preserves_quoted_arguments() {
        assert_eq!(
            clean_exec("example --title 'Hello world' %U"),
            "example --title 'Hello world'"
        );
    }

    #[test]
    fn clean_exec_removes_desktop_field_codes() {
        assert_eq!(
            clean_exec("example %f --new-window %u"),
            "example --new-window"
        );
    }
}
