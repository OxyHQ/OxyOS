import { create } from "zustand";

export type SettingsSection = "Wi-Fi" | "Bluetooth" | "Display" | "Sound" | "Wallpaper" | "About";

interface SettingsState {
  isOpen: boolean;
  open: () => void;
  close: () => void;
  toggle: () => void;
  activeSection: SettingsSection;
  setActiveSection: (s: SettingsSection) => void;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  isOpen: false,
  open: () => set({ isOpen: true }),
  close: () => set({ isOpen: false }),
  toggle: () => set((s) => ({ isOpen: !s.isOpen })),
  activeSection: "Wi-Fi",
  setActiveSection: (activeSection) => set({ activeSection }),
}));
