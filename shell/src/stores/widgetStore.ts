import { create } from "zustand";

interface WidgetState {
  notes: string;
  setNotes: (notes: string) => void;
}

export const useWidgetStore = create<WidgetState>((set) => ({
  notes: "",
  setNotes: (notes) => set({ notes }),
}));
