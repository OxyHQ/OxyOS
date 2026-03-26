import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";
import { invoke } from "../lib/tauri";

interface BatteryInfo {
  level: number;
  charging: boolean;
}

interface WifiInfo {
  enabled: boolean;
  ssid: string | null;
  strength: number;
}

interface VolumeInfo {
  level: number;
  muted: boolean;
}

/**
 * Polls the Tauri backend for real system data every 5 seconds
 * and pushes updates into the Zustand system store.
 * No-ops gracefully when running in a browser (demo mode).
 */
export function useSystemInfo() {
  useEffect(() => {
    async function poll() {
      const battery = await invoke<BatteryInfo>("get_battery_info");
      if (battery) {
        useSystemStore.getState().setBatteryLevel(battery.level);
        useSystemStore.getState().setCharging(battery.charging);
      }

      const wifi = await invoke<WifiInfo>("get_wifi_info");
      if (wifi) {
        if (wifi.enabled !== useSystemStore.getState().wifiEnabled) {
          useSystemStore.getState().toggleWifi();
        }
      }

      const vol = await invoke<VolumeInfo>("get_volume");
      if (vol) {
        useSystemStore.getState().setVolume(vol.level);
      }

      const brightness = await invoke<number>("get_brightness");
      if (brightness !== null) {
        useSystemStore.getState().setBrightness(brightness);
      }
    }

    poll();
    const id = setInterval(poll, 5000);
    return () => clearInterval(id);
  }, []);
}
