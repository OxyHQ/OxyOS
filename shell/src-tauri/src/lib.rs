use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;
use tauri::WebviewUrl;
use tauri::WebviewWindowBuilder;

// ── Data types ──

#[derive(Serialize, Clone, PartialEq)]
pub struct BatteryInfo {
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
pub struct SystemUpdate {
    battery: BatteryInfo,
    wifi: WifiInfo,
    volume: VolumeInfo,
    brightness: u8,
}

#[derive(Serialize)]
pub struct DesktopApp {
    name: String,
    exec: String,
    icon: String,
    categories: String,
}

// ── System readers ──

fn read_battery() -> BatteryInfo {
    let level = fs::read_to_string("/sys/class/power_supply/BAT0/capacity")
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(100);

    let charging = fs::read_to_string("/sys/class/power_supply/BAT0/status")
        .ok()
        .map(|s| s.trim() == "Charging")
        .unwrap_or(false);

    BatteryInfo { level, charging }
}

fn read_wifi() -> WifiInfo {
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
                        ssid: if fields[1].is_empty() { None } else { Some(fields[1].to_string()) },
                        strength: fields[2].parse::<u8>().unwrap_or(0),
                    };
                }
            }
            WifiInfo { enabled: true, ssid: None, strength: 0 }
        }
        Err(_) => WifiInfo { enabled: false, ssid: None, strength: 0 },
    }
}

fn read_volume() -> VolumeInfo {
    let level = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
        .ok()
        .and_then(|out| {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            stdout.split('/').nth(1).and_then(|s| s.trim().trim_end_matches('%').parse::<u8>().ok())
        })
        .unwrap_or(70);

    let muted = Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("yes"))
        .unwrap_or(false);

    VolumeInfo { level, muted }
}

fn read_brightness() -> u8 {
    if let Ok(mut entries) = fs::read_dir("/sys/class/backlight/") {
        if let Some(Ok(entry)) = entries.next() {
            let path = entry.path();
            let cur = fs::read_to_string(path.join("brightness"))
                .ok().and_then(|s| s.trim().parse::<f64>().ok());
            let max = fs::read_to_string(path.join("max_brightness"))
                .ok().and_then(|s| s.trim().parse::<f64>().ok());
            if let (Some(c), Some(m)) = (cur, max) {
                if m > 0.0 { return ((c / m) * 100.0).round() as u8; }
            }
        }
    }
    80
}

// ── Background system monitor (event-driven) ──

fn start_system_monitor(app_handle: tauri::AppHandle) {
    thread::spawn(move || {
        let mut prev = SystemUpdate {
            battery: BatteryInfo { level: 255, charging: false },
            wifi: WifiInfo { enabled: false, ssid: None, strength: 255 },
            volume: VolumeInfo { level: 255, muted: false },
            brightness: 255,
        };

        loop {
            let current = SystemUpdate {
                battery: read_battery(),
                wifi: read_wifi(),
                volume: read_volume(),
                brightness: read_brightness(),
            };

            if current != prev {
                let _ = app_handle.emit("system-update", &current);
                prev = current;
            }

            thread::sleep(Duration::from_secs(5));
        }
    });
}

// ── Multi-window setup ──

const SHELF_HEIGHT: f64 = 52.0;

fn create_shell_windows(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let (screen_w, screen_h) = if let Some(monitor) = app.primary_monitor()? {
        let scale = monitor.scale_factor();
        let size = monitor.size();
        (size.width as f64 / scale, size.height as f64 / scale)
    } else {
        (1920.0, 1080.0)
    };

    // Desktop window — fullscreen background, always behind
    let _desktop = WebviewWindowBuilder::new(
        app,
        "desktop",
        WebviewUrl::App("index.html".into()),
    )
    .title("OxyOS Desktop")
    .decorations(false)
    .resizable(false)
    .maximized(true)
    .transparent(false)
    .build()?;

    // Shelf window — dock at bottom, always on top
    let shelf = WebviewWindowBuilder::new(
        app,
        "shelf",
        WebviewUrl::App("index.html".into()),
    )
    .title("OxyOS Shelf")
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .transparent(true)
    .inner_size(screen_w, SHELF_HEIGHT)
    .position(0.0, screen_h - SHELF_HEIGHT)
    .skip_taskbar(true)
    .build()?;

    // Prevent shelf from being focused when clicked — keep focus on active app
    shelf.set_ignore_cursor_events(false)?;

    // Launcher window — fullscreen overlay, hidden by default
    let launcher = WebviewWindowBuilder::new(
        app,
        "launcher",
        WebviewUrl::App("index.html".into()),
    )
    .title("OxyOS Launcher")
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .transparent(true)
    .maximized(true)
    .visible(false)
    .skip_taskbar(true)
    .build()?;

    // Hide launcher initially
    launcher.hide()?;

    Ok(())
}

// ── Tauri commands ──

#[tauri::command]
async fn launch_app(exec: String) -> Result<(), String> {
    let parts: Vec<&str> = exec.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }
    Command::new(parts[0])
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
    if let Some(win) = app.get_webview_window("launcher") {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn hide_launcher(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("launcher") {
        win.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn toggle_launcher(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("launcher") {
        if win.is_visible().unwrap_or(false) {
            win.hide().map_err(|e| e.to_string())?;
        } else {
            win.show().map_err(|e| e.to_string())?;
            win.set_focus().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

static TERMINAL_COUNTER: AtomicU32 = AtomicU32::new(0);

#[tauri::command]
async fn open_terminal(app: tauri::AppHandle) -> Result<String, String> {
    let id = TERMINAL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("terminal-{}", id);

    let win = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::App("index.html".into()),
    )
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
async fn get_volume() -> VolumeInfo {
    read_volume()
}

#[tauri::command]
async fn set_volume(level: u8) -> Result<(), String> {
    Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", level.min(100))])
        .output()
        .map_err(|e| format!("Failed to set volume: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn get_brightness() -> u8 {
    read_brightness()
}

#[tauri::command]
async fn set_brightness(level: u8) -> Result<(), String> {
    let dir = fs::read_dir("/sys/class/backlight/")
        .map_err(|e| format!("No backlight found: {}", e))?;
    for entry in dir.flatten() {
        let path = entry.path();
        let max: f64 = fs::read_to_string(path.join("max_brightness"))
            .map_err(|e| e.to_string())?
            .trim().parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
        let target = ((level.min(100) as f64 / 100.0) * max).round() as u64;
        fs::write(path.join("brightness"), target.to_string())
            .map_err(|e| format!("Failed to write brightness: {}", e))?;
        return Ok(());
    }
    Err("No backlight device found".to_string())
}

#[tauri::command]
async fn power_action(action: String) -> Result<(), String> {
    match action.as_str() {
        "shutdown" => { Command::new("systemctl").arg("poweroff").spawn().map_err(|e| e.to_string())?; }
        "restart" => { Command::new("systemctl").arg("reboot").spawn().map_err(|e| e.to_string())?; }
        "lock" => {} // Frontend handles lock screen
        "logout" => { Command::new("loginctl").args(["terminate-session", "self"]).spawn().map_err(|e| e.to_string())?; }
        _ => return Err(format!("Unknown action: {}", action)),
    }
    Ok(())
}

#[tauri::command]
async fn get_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "User".to_string())
}

fn resolve_icon_path(icon: &str) -> String {
    if icon.is_empty() { return String::new(); }
    // Already an absolute path
    if icon.starts_with('/') {
        if Path::new(icon).exists() { return icon.to_string(); }
        return String::new();
    }
    // Search common icon directories in preference order
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
    for p in &search_paths {
        if Path::new(p).exists() { return p.clone(); }
    }
    // Return original name as fallback (frontend handles it)
    icon.to_string()
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
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") { continue; }
        let content = match fs::read_to_string(&path) { Ok(c) => c, Err(_) => continue };

        let mut name = String::new();
        let mut exec = String::new();
        let mut icon = String::new();
        let mut categories = String::new();
        let mut no_display = false;
        let mut in_section = false;

        for line in content.lines() {
            let t = line.trim();
            if t == "[Desktop Entry]" { in_section = true; continue; }
            if t.starts_with('[') { if in_section { break; } continue; }
            if !in_section { continue; }

            if let Some(v) = t.strip_prefix("Name=") { if name.is_empty() { name = v.to_string(); } }
            else if let Some(v) = t.strip_prefix("Exec=") { exec = clean_exec(v); }
            else if let Some(v) = t.strip_prefix("Icon=") { icon = v.to_string(); }
            else if let Some(v) = t.strip_prefix("Categories=") { categories = v.to_string(); }
            else if t == "NoDisplay=true" { no_display = true; }
        }

        if no_display || name.is_empty() || exec.is_empty() { continue; }
        let resolved_icon = resolve_icon_path(&icon);
        apps.push(DesktopApp { name, exec, icon: resolved_icon, categories });
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn clean_exec(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|t| !matches!(*t, "%u"|"%U"|"%f"|"%F"|"%d"|"%D"|"%n"|"%N"|"%i"|"%c"|"%k"|"%v"|"%m"))
        .collect::<Vec<&str>>()
        .join(" ")
}

// ── App entry point ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Create the three shell windows
            create_shell_windows(app)?;

            // Start background system monitor — emits "system-update" events
            start_system_monitor(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            launch_app,
            show_launcher,
            hide_launcher,
            toggle_launcher,
            open_terminal,
            get_battery_info,
            get_wifi_info,
            get_volume,
            set_volume,
            get_brightness,
            set_brightness,
            power_action,
            get_username,
            list_desktop_apps,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
