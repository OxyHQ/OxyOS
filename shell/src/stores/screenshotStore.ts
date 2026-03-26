import { create } from "zustand";

interface ScreenshotState {
  isActive: boolean;
  capturedUrl: string | null;
  activate: () => void;
  deactivate: () => void;
  setCapturedUrl: (url: string | null) => void;
}

export const useScreenshotStore = create<ScreenshotState>((set) => ({
  isActive: false,
  capturedUrl: null,
  activate: () => set({ isActive: true }),
  deactivate: () => set({ isActive: false, capturedUrl: null }),
  setCapturedUrl: (url) => set({ capturedUrl: url }),
}));
