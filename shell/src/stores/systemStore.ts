import { create } from "zustand";

interface SystemState {
  wifiEnabled: boolean;
  batteryLevel: number;
  volume: number;
  brightness: number;
  time: string;
  toggleWifi: () => void;
  setVolume: (v: number) => void;
  setBrightness: (b: number) => void;
  setTime: (t: string) => void;
}

export const useSystemStore = create<SystemState>((set) => ({
  wifiEnabled: true,
  batteryLevel: 85,
  volume: 70,
  brightness: 80,
  time: "",
  toggleWifi: () => set((s) => ({ wifiEnabled: !s.wifiEnabled })),
  setVolume: (volume) => set({ volume }),
  setBrightness: (brightness) => set({ brightness }),
  setTime: (time) => set({ time }),
}));
