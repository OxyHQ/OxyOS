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
        // Fire username fetch in parallel but don't block system metrics on it
        const usernamePromise = invoke<string>("get_username");

        const [battery, wifi, vol, bright] = await Promise.all([
          invoke<{ level: number; charging: boolean }>("get_battery_info"),
          invoke<{ enabled: boolean; ssid: string | null; strength: number }>("get_wifi_info"),
          invoke<{ level: number; muted: boolean }>("get_volume"),
          invoke<number>("get_brightness"),
        ]);

        if (cancelled) return;

        const update: Partial<ReturnType<typeof useSystemStore.getState>> = {};
        if (battery != null) { update.batteryLevel = battery.level; update.isCharging = battery.charging; }
        if (wifi != null) { update.wifiEnabled = wifi.enabled; }
        if (vol != null) { update.volume = vol.level; }
        if (bright != null) { update.brightness = bright; }
        useSystemStore.setState(update);

        const username = await usernamePromise;
        if (!cancelled && username != null) {
          useSessionStore.getState().setUsername(username);
        }

        // Listen for ongoing changes (backend polls every 5s)
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

        if (cancelled) {
          unlistenFn();
        } else {
          unlisten = unlistenFn;
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
