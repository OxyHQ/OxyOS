import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";
import { isNative } from "../lib/tauri";

interface SystemUpdate {
  battery: { level: number; charging: boolean };
  wifi: { enabled: boolean; ssid: string | null; strength: number };
  volume: { level: number; muted: boolean };
  brightness: number;
}

/**
 * Listens for "system-update" events pushed from the Tauri Rust backend.
 * The backend only emits when values actually change — no frontend polling.
 * In browser mode this is a no-op.
 */
export function useSystemInfo() {
  useEffect(() => {
    if (!isNative()) return;

    let unlisten: (() => void) | undefined;

    async function setup() {
      try {
        const { listen } = await import("@tauri-apps/api/event");

        unlisten = await listen<SystemUpdate>("system-update", (event) => {
          const { battery, wifi, volume, brightness } = event.payload;
          // Single batched state update instead of multiple individual sets
          useSystemStore.setState({
            batteryLevel: battery.level,
            isCharging: battery.charging,
            wifiEnabled: wifi.enabled,
            volume: volume.level,
            brightness,
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
