import { create } from "zustand";

interface SettingsState {
  isOpen: boolean;
  open: () => void;
  close: () => void;
  toggle: () => void;
  activeSection: string;
  setActiveSection: (s: string) => void;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  isOpen: false,
  open: () => set({ isOpen: true }),
  close: () => set({ isOpen: false }),
  toggle: () => set((s) => ({ isOpen: !s.isOpen })),
  activeSection: "Wi-Fi",
  setActiveSection: (activeSection) => set({ activeSection }),
}));
