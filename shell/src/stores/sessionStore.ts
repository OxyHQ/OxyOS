import { create } from "zustand";

interface SessionState {
  isLoggedIn: boolean;
  username: string;
  avatarUrl: string | null;
  login: () => void;
  logout: () => void;
}

export const useSessionStore = create<SessionState>((set) => ({
  isLoggedIn: false,
  username: "User",
  avatarUrl: null,
  login: () => set({ isLoggedIn: true }),
  logout: () => set({ isLoggedIn: false }),
}));
