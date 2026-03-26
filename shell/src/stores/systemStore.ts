import { create } from "zustand";

interface SystemState {
  wifiEnabled: boolean;
  bluetoothEnabled: boolean;
  quickShareEnabled: boolean;
  dndEnabled: boolean;
  nightLightEnabled: boolean;
  focusMode: string | null;
  batteryLevel: number;
  isCharging: boolean;
  volume: number;
  brightness: number;
  time: string;
  wallpaper: string;
  toggleWifi: () => void;
  toggleBluetooth: () => void;
  toggleQuickShare: () => void;
  toggleDnd: () => void;
  toggleNightLight: () => void;
  setFocusMode: (mode: string | null) => void;
  setVolume: (v: number) => void;
  setBrightness: (b: number) => void;
  setTime: (t: string) => void;
  setBatteryLevel: (level: number) => void;
  setCharging: (charging: boolean) => void;
  setWallpaper: (wallpaper: string) => void;
}

export const useSystemStore = create<SystemState>((set) => ({
  wifiEnabled: true,
  bluetoothEnabled: true,
  quickShareEnabled: false,
  dndEnabled: false,
  nightLightEnabled: false,
  focusMode: null,
  batteryLevel: 85,
  isCharging: true,
  volume: 70,
  brightness: 80,
  time: "",
  wallpaper: "default",
  toggleWifi: () => set((s) => ({ wifiEnabled: !s.wifiEnabled })),
  toggleBluetooth: () => set((s) => ({ bluetoothEnabled: !s.bluetoothEnabled })),
  toggleQuickShare: () => set((s) => ({ quickShareEnabled: !s.quickShareEnabled })),
  toggleDnd: () => set((s) => ({ dndEnabled: !s.dndEnabled })),
  toggleNightLight: () => set((s) => ({ nightLightEnabled: !s.nightLightEnabled })),
  setFocusMode: (focusMode) => set({ focusMode }),
  setVolume: (volume) => set({ volume }),
  setBrightness: (brightness) => set({ brightness }),
  setTime: (time) => set({ time }),
  setBatteryLevel: (level) => set({ batteryLevel: level }),
  setCharging: (isCharging) => set({ isCharging }),
  setWallpaper: (wallpaper) => set({ wallpaper }),
}));
