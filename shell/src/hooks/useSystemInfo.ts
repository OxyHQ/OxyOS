import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";

interface SystemUpdate {
  battery: { level: number; charging: boolean };
  wifi: { enabled: boolean; ssid: string | null; strength: number };
  volume: { level: number; muted: boolean };
  brightness: number;
}

/**
 * Listens for "system-update" events pushed from the Tauri Rust backend.
 * The backend runs a background thread that monitors system state and
 * only emits when something actually changes — no frontend polling.
 * In browser mode this is a no-op.
 */
export function useSystemInfo() {
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    async function setup() {
      try {
        if (!(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) return;
        const { listen } = await import("@tauri-apps/api/event");

        unlisten = await listen<SystemUpdate>("system-update", (event) => {
          const { battery, wifi, volume, brightness } = event.payload;
          const s = useSystemStore.getState();
          s.setBatteryLevel(battery.level);
          s.setCharging(battery.charging);
          s.setVolume(volume.level);
          s.setBrightness(brightness);
          if (wifi.enabled !== s.wifiEnabled) {
            s.toggleWifi();
          }
        });
      } catch {
        // Browser mode — silent fallback
      }
    }

    setup();
    return () => { unlisten?.(); };
  }, []);
}
