import { create } from "zustand";

interface SessionState {
  isLoggedIn: boolean;
  isLocked: boolean;
  username: string;
  avatarUrl: string | null;
  login: () => void;
  logout: () => void;
  lock: () => void;
  unlock: () => void;
}

export const useSessionStore = create<SessionState>((set) => ({
  isLoggedIn: false,
  isLocked: false,
  username: "User",
  avatarUrl: null,
  login: () => set({ isLoggedIn: true, isLocked: false }),
  logout: () => set({ isLoggedIn: false, isLocked: false }),
  lock: () => set({ isLocked: true }),
  unlock: () => set({ isLocked: false }),
}));
