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

    let cancelled = false;
    let unlisten: (() => void) | undefined;

    async function setup() {
      try {
        // Register listener first to avoid missing events during initial fetch
        const { listen } = await import("@tauri-apps/api/event");

        const unlistenFn = await listen<SystemUpdate>("system-update", (event) => {
          const { battery, wifi, volume, brightness } = event.payload;
          useSystemStore.setState({
            batteryLevel: battery.level,
            isCharging: battery.charging,
            wifiEnabled: wifi.enabled,
            volume: volume.level,
            brightness,
          });
        });

        if (cancelled) { unlistenFn(); return; }
        unlisten = unlistenFn;

        // Fetch initial values so the UI doesn't show stale defaults
        const [battery, wifi, vol, bright, username] = await Promise.all([
          invoke<{ level: number; charging: boolean }>("get_battery_info"),
          invoke<{ enabled: boolean; ssid: string | null; strength: number }>("get_wifi_info"),
          invoke<{ level: number; muted: boolean }>("get_volume"),
          invoke<number>("get_brightness"),
          invoke<string>("get_username"),
        ]);

        if (cancelled) return;

        useSystemStore.setState({
          ...(battery != null && { batteryLevel: battery.level, isCharging: battery.charging }),
          ...(wifi != null && { wifiEnabled: wifi.enabled }),
          ...(vol != null && { volume: vol.level }),
          ...(bright != null && { brightness: bright }),
        });

        if (username) {
          useSessionStore.getState().setUsername(username);
        }
      } catch {
        // Tauri API not available
      }
    }

    setup();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
