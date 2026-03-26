import { create } from "zustand";
import { playLock, playUnlock } from "../lib/sounds";

interface SessionState {
  isLoggedIn: boolean;
  isLocked: boolean;
  username: string;
  avatarUrl: string | null;
  login: () => void;
  logout: () => void;
  lock: () => void;
  unlock: () => void;
  setUsername: (name: string) => void;
}

export const useSessionStore = create<SessionState>((set) => ({
  isLoggedIn: false,
  isLocked: false,
  username: "User",
  avatarUrl: null,
  login: () => { playUnlock(); set({ isLoggedIn: true, isLocked: false }); },
  logout: () => set({ isLoggedIn: false, isLocked: false }),
  lock: () => { playLock(); set({ isLocked: true }); },
  unlock: () => { playUnlock(); set({ isLocked: false }); },
  setUsername: (username) => set({ username }),
}));
