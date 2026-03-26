import { create } from "zustand";

export interface WindowState {
  id: string;
  appId: string;
  title: string;
  x: number;
  y: number;
  width: number;
  height: number;
  zIndex: number;
  isMinimized: boolean;
  isMaximized: boolean;
}

interface WindowStoreState {
  windows: WindowState[];
  nextZ: number;
  openWindow: (appId: string, title: string, defaults?: Partial<WindowState>) => string;
  closeWindow: (id: string) => void;
  focusWindow: (id: string) => void;
  moveWindow: (id: string, x: number, y: number) => void;
  resizeWindow: (id: string, width: number, height: number) => void;
  toggleMaximize: (id: string) => void;
  toggleMinimize: (id: string) => void;
  getWindow: (id: string) => WindowState | undefined;
}

export const useWindowStore = create<WindowStoreState>((set, get) => ({
  windows: [],
  nextZ: 100,

  openWindow: (appId, title, defaults) => {
    const id = crypto.randomUUID();
    const z = get().nextZ;
    const win: WindowState = {
      id,
      appId,
      title,
      x: 80 + (get().windows.length % 5) * 30,
      y: 60 + (get().windows.length % 5) * 30,
      width: 780,
      height: 500,
      zIndex: z,
      isMinimized: false,
      isMaximized: false,
      ...defaults,
    };
    set((s) => ({ windows: [...s.windows, win], nextZ: z + 1 }));
    return id;
  },

  closeWindow: (id) =>
    set((s) => ({ windows: s.windows.filter((w) => w.id !== id) })),

  focusWindow: (id) =>
    set((s) => {
      const z = s.nextZ;
      return {
        windows: s.windows.map((w) => (w.id === id ? { ...w, zIndex: z } : w)),
        nextZ: z + 1,
      };
    }),

  moveWindow: (id, x, y) =>
    set((s) => ({
      windows: s.windows.map((w) => (w.id === id ? { ...w, x, y } : w)),
    })),

  resizeWindow: (id, width, height) =>
    set((s) => ({
      windows: s.windows.map((w) => (w.id === id ? { ...w, width, height } : w)),
    })),

  toggleMaximize: (id) =>
    set((s) => ({
      windows: s.windows.map((w) =>
        w.id === id ? { ...w, isMaximized: !w.isMaximized } : w,
      ),
    })),

  toggleMinimize: (id) =>
    set((s) => ({
      windows: s.windows.map((w) =>
        w.id === id ? { ...w, isMinimized: !w.isMinimized } : w,
      ),
    })),

  getWindow: (id) => get().windows.find((w) => w.id === id),
}));
