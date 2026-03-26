import { create } from "zustand";
import { playNotification } from "../lib/sounds";

export interface Notification {
  id: string;
  app: string;
  appIcon?: string;
  title: string;
  body: string;
  timestamp: number;
}

interface NotificationGroup {
  app: string;
  appIcon?: string;
  notifications: Notification[];
}

interface NotificationState {
  notifications: Notification[];
  add: (notification: Omit<Notification, "id" | "timestamp">) => void;
  dismiss: (id: string) => void;
  clearApp: (app: string) => void;
  clearAll: () => void;
  grouped: () => NotificationGroup[];
}

export const useNotificationStore = create<NotificationState>((set, get) => ({
  notifications: [
    {
      id: "1",
      app: "Mail",
      title: "New message from Alex",
      body: "Hey, are you free for a call later today?",
      timestamp: Date.now() - 2 * 60 * 1000,
    },
    {
      id: "2",
      app: "Mail",
      title: "Weekly digest",
      body: "Your weekly summary is ready to view.",
      timestamp: Date.now() - 15 * 60 * 1000,
    },
    {
      id: "3",
      app: "Calendar",
      title: "Team standup in 10 minutes",
      body: "Daily standup — Conference Room B",
      timestamp: Date.now() - 8 * 60 * 1000,
    },
    {
      id: "4",
      app: "Messages",
      title: "Jordan",
      body: "Sent you a photo",
      timestamp: Date.now() - 30 * 60 * 1000,
    },
    {
      id: "5",
      app: "System",
      title: "Software Update Available",
      body: "OxyOS 2.1 is now available to install.",
      timestamp: Date.now() - 60 * 60 * 1000,
    },
  ],

  add: (notification) => {
    playNotification();
    set((state) => ({
      notifications: [
        {
          ...notification,
          id: crypto.randomUUID(),
          timestamp: Date.now(),
        },
        ...state.notifications,
      ],
    }));
  },

  dismiss: (id) =>
    set((state) => ({
      notifications: state.notifications.filter((n) => n.id !== id),
    })),

  clearApp: (app) =>
    set((state) => ({
      notifications: state.notifications.filter((n) => n.app !== app),
    })),

  clearAll: () => set({ notifications: [] }),

  grouped: () => {
    const { notifications } = get();
    const map = new Map<string, NotificationGroup>();
    for (const n of notifications) {
      const existing = map.get(n.app);
      if (existing) {
        existing.notifications.push(n);
      } else {
        map.set(n.app, {
          app: n.app,
          appIcon: n.appIcon,
          notifications: [n],
        });
      }
    }
    return Array.from(map.values());
  },
}));
