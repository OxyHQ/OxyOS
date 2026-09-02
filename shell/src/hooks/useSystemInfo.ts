import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";
import { useSessionStore } from "../stores/sessionStore";
import { invoke, isNative } from "../lib/tauri";

interface BatteryInfo { present: boolean; level: number; charging: boolean }
interface WifiInfo { enabled: boolean; ssid: string | null; strength: number }
interface VolumeInfo { level: number; muted: boolean }
interface BrightnessInfo { present: boolean; level: number }

interface SystemUpdate {
  battery: BatteryInfo;
  wifi: WifiInfo;
  bluetooth: boolean;
  night_light: boolean;
  volume: VolumeInfo;
  brightness: BrightnessInfo;
}

export function useSystemInfo() {
  useEffect(() => {
    if (!isNative()) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;

    Promise.all([
      invoke<BatteryInfo>("get_battery_info"),
      invoke<WifiInfo>("get_wifi_info"),
      invoke<boolean>("get_bluetooth_info"),
      invoke<boolean>("get_night_light_info"),
      invoke<VolumeInfo>("get_volume"),
      invoke<BrightnessInfo>("get_brightness"),
      invoke<string>("get_username"),
      invoke<string | null>("get_user_avatar"),
    ]).then(([battery, wifi, bluetooth, nightLight, vol, bright, username, avatarPath]) => {
      useSystemStore.setState({
        ...(battery != null && {
          batteryPresent: battery.present,
          batteryLevel: battery.level,
          isCharging: battery.charging,
        }),
        ...(wifi != null && { wifiEnabled: wifi.enabled }),
        ...(bluetooth != null && { bluetoothEnabled: bluetooth }),
        ...(nightLight != null && { nightLightEnabled: nightLight }),
        ...(vol != null && { volume: vol.level }),
        ...(bright != null && {
          brightnessPresent: bright.present,
          brightness: bright.level,
        }),
      });
      const session = useSessionStore.getState();
      if (username) session.setUsername(username);
      if (avatarPath) session.setAvatarUrl(avatarPath);
    }).catch((error: unknown) => {
      console.error("Failed to load native system state", error);
    });

    const registration = import("@tauri-apps/api/event")
      .then(({ listen }) => listen<SystemUpdate>("system-update", (event) => {
        const { battery, wifi, bluetooth, night_light, volume, brightness } = event.payload;
        useSystemStore.setState({
          batteryPresent: battery.present,
          batteryLevel: battery.level,
          isCharging: battery.charging,
          wifiEnabled: wifi.enabled,
          bluetoothEnabled: bluetooth,
          nightLightEnabled: night_light,
          volume: volume.level,
          brightnessPresent: brightness.present,
          brightness: brightness.level,
        });
      }))
      .then((registeredUnlisten) => {
        if (disposed) {
          registeredUnlisten();
        } else {
          unlisten = registeredUnlisten;
        }
      })
      .catch((error: unknown) => {
        console.error("Failed to subscribe to native system updates", error);
      });

    return () => {
      disposed = true;
      unlisten?.();
      void registration;
    };
  }, []);
}
