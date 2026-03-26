import { create } from "zustand";

interface RunningAppsState {
  running: Set<string>;
  launch: (appName: string) => void;
  close: (appName: string) => void;
  isRunning: (appName: string) => boolean;
}

export const useRunningAppsStore = create<RunningAppsState>((set, get) => ({
  running: new Set(["Browser", "Terminal"]),
  launch: (appName: string) =>
    set((state) => {
      const next = new Set(state.running);
      next.add(appName);
      return { running: next };
    }),
  close: (appName: string) =>
    set((state) => {
      const next = new Set(state.running);
      next.delete(appName);
      return { running: next };
    }),
  isRunning: (appName: string) => get().running.has(appName),
}));
