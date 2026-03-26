use serde::Serialize;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;

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
        apps.push(DesktopApp { name, exec, icon, categories });
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
            // Start background system monitor — emits "system-update" events
            start_system_monitor(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            launch_app,
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
