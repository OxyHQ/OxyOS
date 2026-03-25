import { create } from "zustand";

interface LauncherState {
  isOpen: boolean;
  toggle: () => void;
  close: () => void;
}

export const useLauncherStore = create<LauncherState>((set) => ({
  isOpen: false,
  toggle: () => set((s) => ({ isOpen: !s.isOpen })),
  close: () => set({ isOpen: false }),
}));
