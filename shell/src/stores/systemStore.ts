import { create } from "zustand";

interface SystemState {
  wifiEnabled: boolean;
  bluetoothEnabled: boolean;
  airdropEnabled: boolean;
  dndEnabled: boolean;
  nightLightEnabled: boolean;
  focusMode: string | null;
  batteryLevel: number;
  isCharging: boolean;
  volume: number;
  brightness: number;
  time: string;
  toggleWifi: () => void;
  toggleBluetooth: () => void;
  toggleAirdrop: () => void;
  toggleDnd: () => void;
  toggleNightLight: () => void;
  setFocusMode: (mode: string | null) => void;
  setVolume: (v: number) => void;
  setBrightness: (b: number) => void;
  setTime: (t: string) => void;
  setBatteryLevel: (level: number) => void;
  setCharging: (charging: boolean) => void;
}

export const useSystemStore = create<SystemState>((set) => ({
  wifiEnabled: true,
  bluetoothEnabled: true,
  airdropEnabled: false,
  dndEnabled: false,
  nightLightEnabled: false,
  focusMode: null,
  batteryLevel: 85,
  isCharging: true,
  volume: 70,
  brightness: 80,
  time: "",
  toggleWifi: () => set((s) => ({ wifiEnabled: !s.wifiEnabled })),
  toggleBluetooth: () => set((s) => ({ bluetoothEnabled: !s.bluetoothEnabled })),
  toggleAirdrop: () => set((s) => ({ airdropEnabled: !s.airdropEnabled })),
  toggleDnd: () => set((s) => ({ dndEnabled: !s.dndEnabled })),
  toggleNightLight: () => set((s) => ({ nightLightEnabled: !s.nightLightEnabled })),
  setFocusMode: (focusMode) => set({ focusMode }),
  setVolume: (volume) => set({ volume }),
  setBrightness: (brightness) => set({ brightness }),
  setTime: (time) => set({ time }),
  setBatteryLevel: (level) => set({ batteryLevel: level }),
  setCharging: (isCharging) => set({ isCharging }),
}));
