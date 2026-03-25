import { create } from "zustand";

interface SystemState {
  wifiEnabled: boolean;
  batteryLevel: number;
  isCharging: boolean;
  volume: number;
  brightness: number;
  time: string;
  toggleWifi: () => void;
  setVolume: (v: number) => void;
  setBrightness: (b: number) => void;
  setTime: (t: string) => void;
  setBatteryLevel: (level: number) => void;
  setCharging: (charging: boolean) => void;
}

export const useSystemStore = create<SystemState>((set) => ({
  wifiEnabled: true,
  batteryLevel: 85,
  isCharging: true,
  volume: 70,
  brightness: 80,
  time: "",
  toggleWifi: () => set((s) => ({ wifiEnabled: !s.wifiEnabled })),
  setVolume: (volume) => set({ volume }),
  setBrightness: (brightness) => set({ brightness }),
  setTime: (time) => set({ time }),
  setBatteryLevel: (level) => set({ batteryLevel: level }),
  setCharging: (isCharging) => set({ isCharging }),
}));
