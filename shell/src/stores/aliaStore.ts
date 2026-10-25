import { create } from 'zustand';

interface AliaState {
  isOpen: boolean;
  open: () => void;
  close: () => void;
  toggle: () => void;
}

/** Window state only; Alia's SDK owns conversations, streaming, and account isolation. */
export const useAliaStore = create<AliaState>((set) => ({
  isOpen: false,
  open: () => set({ isOpen: true }),
  close: () => set({ isOpen: false }),
  toggle: () => set((state) => ({ isOpen: !state.isOpen })),
}));
