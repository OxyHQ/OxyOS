import { create } from "zustand";

interface SystemState {
  wifiEnabled: boolean;
  bluetoothEnabled: boolean;
  nightLightEnabled: boolean;
  batteryPresent: boolean;
  batteryLevel: number;
  isCharging: boolean;
  volume: number;
  brightnessPresent: boolean;
  brightness: number;
  time: string;
  setWifi: (v: boolean) => void;
  setBluetooth: (v: boolean) => void;
  setNightLight: (v: boolean) => void;
  setVolume: (v: number) => void;
  setBrightness: (b: number) => void;
  setTime: (t: string) => void;
  setBatteryLevel: (level: number) => void;
  setCharging: (charging: boolean) => void;
}

export const useSystemStore = create<SystemState>((set) => ({
  wifiEnabled: false,
  bluetoothEnabled: false,
  nightLightEnabled: false,
  batteryPresent: false,
  batteryLevel: 0,
  isCharging: false,
  volume: 0,
  brightnessPresent: false,
  brightness: 0,
  time: "",
  setWifi: (wifiEnabled) => set({ wifiEnabled }),
  setBluetooth: (bluetoothEnabled) => set({ bluetoothEnabled }),
  setNightLight: (nightLightEnabled) => set({ nightLightEnabled }),
  setVolume: (volume) => set({ volume }),
  setBrightness: (brightness) => set({ brightness }),
  setTime: (time) => set({ time }),
  setBatteryLevel: (level) => set({ batteryLevel: level }),
  setCharging: (isCharging) => set({ isCharging }),
}));
