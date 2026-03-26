use serde::Serialize;
use std::fs;
use std::process::Command;

#[derive(Serialize)]
pub struct BatteryInfo {
    level: u8,
    charging: bool,
}

#[derive(Serialize)]
pub struct WifiInfo {
    enabled: bool,
    ssid: Option<String>,
    strength: u8,
}

#[derive(Serialize)]
pub struct VolumeInfo {
    level: u8,
    muted: bool,
}

#[derive(Serialize)]
pub struct DesktopApp {
    name: String,
    exec: String,
    icon: String,
    categories: String,
}

#[tauri::command]
async fn launch_app(exec: String) -> Result<(), String> {
    let parts: Vec<&str> = exec.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }

    let program = parts[0];
    let args = &parts[1..];

    Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch '{}': {}", exec, e))?;

    Ok(())
}

#[tauri::command]
async fn get_battery_info() -> BatteryInfo {
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

#[tauri::command]
async fn get_wifi_info() -> WifiInfo {
    let output = Command::new("nmcli")
        .args(["-t", "-f", "ACTIVE,SSID,SIGNAL", "dev", "wifi"])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 3 && fields[0] == "yes" {
                    let ssid = if fields[1].is_empty() {
                        None
                    } else {
                        Some(fields[1].to_string())
                    };
                    let strength = fields[2].parse::<u8>().unwrap_or(0);
                    return WifiInfo {
                        enabled: true,
                        ssid,
                        strength,
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
            enabled: true,
            ssid: None,
            strength: 0,
        },
    }
}

#[tauri::command]
async fn get_volume() -> VolumeInfo {
    let level = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
        .ok()
        .and_then(|out| {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            // Output format: "Volume: front-left: 42345 /  65% / -11.22 dB, ..."
            stdout
                .split('/')
                .nth(1)
                .and_then(|s| s.trim().trim_end_matches('%').parse::<u8>().ok())
        })
        .unwrap_or(70);

    let muted = Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .ok()
        .map(|out| {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            stdout.contains("yes")
        })
        .unwrap_or(false);

    VolumeInfo { level, muted }
}

#[tauri::command]
async fn set_volume(level: u8) -> Result<(), String> {
    let vol = format!("{}%", level.min(100));
    Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &vol])
        .output()
        .map_err(|e| format!("Failed to set volume: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn get_brightness() -> u8 {
    let backlight_dir = fs::read_dir("/sys/class/backlight/").ok();

    if let Some(mut entries) = backlight_dir {
        if let Some(Ok(entry)) = entries.next() {
            let path = entry.path();
            let brightness = fs::read_to_string(path.join("brightness"))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());
            let max_brightness = fs::read_to_string(path.join("max_brightness"))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());

            if let (Some(cur), Some(max)) = (brightness, max_brightness) {
                if max > 0.0 {
                    return ((cur / max) * 100.0).round() as u8;
                }
            }
        }
    }

    80
}

#[tauri::command]
async fn set_brightness(level: u8) -> Result<(), String> {
    let backlight_dir =
        fs::read_dir("/sys/class/backlight/").map_err(|e| format!("No backlight found: {}", e))?;

    for entry in backlight_dir.flatten() {
        let path = entry.path();
        let max_brightness = fs::read_to_string(path.join("max_brightness"))
            .map_err(|e| format!("Failed to read max_brightness: {}", e))?
            .trim()
            .parse::<f64>()
            .map_err(|e| format!("Failed to parse max_brightness: {}", e))?;

        let target = ((level.min(100) as f64 / 100.0) * max_brightness).round() as u64;
        fs::write(path.join("brightness"), target.to_string())
            .map_err(|e| format!("Failed to write brightness: {}", e))?;

        return Ok(());
    }

    Err("No backlight device found".to_string())
}

#[tauri::command]
async fn power_action(action: String) -> Result<(), String> {
    match action.as_str() {
        "shutdown" => {
            Command::new("systemctl")
                .arg("poweroff")
                .spawn()
                .map_err(|e| format!("Failed to shutdown: {}", e))?;
        }
        "restart" => {
            Command::new("systemctl")
                .arg("reboot")
                .spawn()
                .map_err(|e| format!("Failed to restart: {}", e))?;
        }
        "lock" => {
            // Shell handles lock screen in the frontend
        }
        "logout" => {
            Command::new("loginctl")
                .args(["terminate-session", "self"])
                .spawn()
                .map_err(|e| format!("Failed to logout: {}", e))?;
        }
        _ => return Err(format!("Unknown power action: {}", action)),
    }
    Ok(())
}

#[tauri::command]
async fn list_desktop_apps() -> Vec<DesktopApp> {
    let mut apps = Vec::new();

    let entries = match fs::read_dir("/usr/share/applications") {
        Ok(entries) => entries,
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
        let mut icon = String::new();
        let mut categories = String::new();
        let mut no_display = false;
        let mut in_desktop_entry = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed == "[Desktop Entry]" {
                in_desktop_entry = true;
                continue;
            }
            if trimmed.starts_with('[') && trimmed != "[Desktop Entry]" {
                if in_desktop_entry {
                    break;
                }
                continue;
            }

            if !in_desktop_entry {
                continue;
            }

            if let Some(val) = trimmed.strip_prefix("Name=") {
                if name.is_empty() {
                    name = val.to_string();
                }
            } else if let Some(val) = trimmed.strip_prefix("Exec=") {
                exec = clean_exec_field(val);
            } else if let Some(val) = trimmed.strip_prefix("Icon=") {
                icon = val.to_string();
            } else if let Some(val) = trimmed.strip_prefix("Categories=") {
                categories = val.to_string();
            } else if trimmed == "NoDisplay=true" {
                no_display = true;
            }
        }

        if no_display || name.is_empty() || exec.is_empty() {
            continue;
        }

        apps.push(DesktopApp {
            name,
            exec,
            icon,
            categories,
        });
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Remove field codes (%u, %U, %f, %F, %d, %D, %n, %N, %i, %c, %k, %v, %m) from Exec values.
fn clean_exec_field(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|token| {
            !matches!(
                *token,
                "%u" | "%U" | "%f" | "%F" | "%d" | "%D" | "%n" | "%N" | "%i" | "%c" | "%k"
                    | "%v" | "%m"
            )
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

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
            list_desktop_apps,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
