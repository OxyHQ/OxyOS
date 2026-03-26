import { create } from "zustand";

export interface AliaMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  timestamp: number;
}

interface AliaState {
  isOpen: boolean;
  isListening: boolean;
  isStreaming: boolean;
  messages: AliaMessage[];
  open: () => void;
  close: () => void;
  toggle: () => void;
  toggleListening: () => void;
  setStreaming: (v: boolean) => void;
  addMessage: (role: "user" | "assistant", content: string) => void;
  appendToLastMessage: (delta: string) => void;
  clearMessages: () => void;
}

const makeGreeting = (): AliaMessage => ({
  id: "greeting",
  role: "assistant",
  content: "Hi, I'm Alia. How can I help you today?",
  timestamp: Date.now(),
});

export const useAliaStore = create<AliaState>((set) => ({
  isOpen: false,
  isListening: false,
  isStreaming: false,
  messages: [makeGreeting()],

  open: () => set({ isOpen: true }),
  close: () => set({ isOpen: false }),
  toggle: () => set((s) => ({ isOpen: !s.isOpen })),
  toggleListening: () => set((s) => ({ isListening: !s.isListening })),
  setStreaming: (isStreaming) => set({ isStreaming }),

  addMessage: (role, content) =>
    set((s) => {
      const next = [
        ...s.messages,
        { id: crypto.randomUUID(), role, content, timestamp: Date.now() },
      ];
      return { messages: next.length > 200 ? next.slice(-200) : next };
    }),

  appendToLastMessage: (delta) =>
    set((s) => {
      const msgs = [...s.messages];
      const last = msgs[msgs.length - 1];
      if (last && last.role === "assistant") {
        msgs[msgs.length - 1] = { ...last, content: last.content + delta };
      }
      return { messages: msgs };
    }),

  clearMessages: () => set({ messages: [makeGreeting()] }),
}));
