import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";
import { useSessionStore } from "../stores/sessionStore";
import { invoke, isNative } from "../lib/tauri";

interface SystemUpdate {
  battery: { level: number; charging: boolean };
  wifi: { enabled: boolean; ssid: string | null; strength: number };
  volume: { level: number; muted: boolean };
  brightness: number;
}

/**
 * Fetches real system data on startup (native mode only),
 * then listens for "system-update" events pushed from the Tauri backend.
 * In browser mode this is a no-op — hardcoded defaults in stores are used.
 */
export function useSystemInfo() {
  useEffect(() => {
    if (!isNative()) return;

    let unlisten: (() => void) | undefined;

    async function setup() {
      try {
        // Fetch initial values immediately so the UI doesn't show stale defaults
        const [battery, wifi, vol, bright, username] = await Promise.all([
          invoke<{ level: number; charging: boolean }>("get_battery_info"),
          invoke<{ enabled: boolean; ssid: string | null; strength: number }>("get_wifi_info"),
          invoke<{ level: number; muted: boolean }>("get_volume"),
          invoke<number>("get_brightness"),
          invoke<string>("get_username"),
        ]);

        useSystemStore.setState({
          ...(battery && { batteryLevel: battery.level, isCharging: battery.charging }),
          ...(wifi && { wifiEnabled: wifi.enabled }),
          ...(vol && { volume: vol.level }),
          ...(bright != null && { brightness: bright }),
        });

        if (username) {
          useSessionStore.getState().setUsername(username);
        }

        // Listen for ongoing changes (backend polls every 5s)
        const { listen } = await import("@tauri-apps/api/event");

        unlisten = await listen<SystemUpdate>("system-update", (event) => {
          const { battery: b, wifi: w, volume: v, brightness: br } = event.payload;
          useSystemStore.setState({
            batteryLevel: b.level,
            isCharging: b.charging,
            wifiEnabled: w.enabled,
            volume: v.level,
            brightness: br,
          });
        });
      } catch {
        // Tauri API not available
      }
    }

    setup();
    return () => { unlisten?.(); };
  }, []);
}
