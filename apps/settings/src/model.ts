import type { ImageSourcePropType } from "react-native";

/**
 * What the Settings application shows (docs/adr/0019-settings.mdx): the user's preferences, which
 * it may change, and the system's settings and facts, which it only shows for now. Supplied by
 * the platform (OxyOS's `OxyOSSettings`, or a fixture in the browser), never invented by the UI.
 */
export type Appearance = "light" | "dark" | "auto";
/** When the lock screen may show what notifications say (docs/adr/0026-notifications.mdx). */
export type NotificationPreviews = "always" | "unlocked";

/** The user's preferences, as `crates/oxyos-preferences` keeps them. */
export type Preferences = {
  appearance: Appearance;
  /** One of Bloom's accent colours, by name. */
  accent: string;
  reduceMotion: boolean;
  reduceTransparency: boolean;
  /** The wallpaper's folder; null is the session's own. */
  wallpaper: string | null;
  /** Minutes idle before the session locks; 0 never. */
  lockAfter: number;
  screenReader: boolean;
  clock24h: boolean;
  /** Do Not Disturb: notifications are kept, none shown as a banner (docs/adr/0026-notifications.mdx). */
  doNotDisturb: boolean;
  notificationPreviews: NotificationPreviews;
};

export type Wallpaper = { id: string; name: string; preview?: ImageSourcePropType };

/** The system's settings and facts (systemd's timedated, localed and hostnamed; os-release). */
export type SystemFacts = {
  computerName: string;
  /** OxyOS's version, from os-release's PRETTY_NAME. */
  version: string;
  processor: string;
  memoryBytes: number;
  /** The display's size and scale, as "1920 × 1200 at 1.5×". */
  display: string;
  timeZone: string;
  automaticTime: boolean;
  keyboardLayout: string;
};

export type SettingsSnapshot = {
  preferences: Preferences;
  wallpapers: readonly Wallpaper[];
  system: SystemFacts;
};

/** The keys a change names, as the session's preferences file does. */
export const PREFERENCE_KEYS = {
  appearance: "appearance",
  accent: "accent",
  reduceMotion: "reduce_motion",
  reduceTransparency: "reduce_transparency",
  wallpaper: "wallpaper",
  lockAfter: "lock_after",
  screenReader: "screen_reader",
  clock24h: "clock_24h",
  doNotDisturb: "do_not_disturb",
  notificationPreviews: "notification_previews",
} as const satisfies Record<keyof Preferences, string>;

/**
 * Intents: the platform validates and applies a change, and the next snapshot says what it is
 * now. A refused change leaves the snapshot as it was.
 */
export type SettingsActions = {
  setPreference: <K extends keyof Preferences>(key: K, value: Preferences[K]) => void;
};

/** A preference's value as the session's file writes it. */
export function preferenceText<K extends keyof Preferences>(value: Preferences[K]): string {
  if (value === null) return "";
  return String(value);
}

/** The choices of how long before the session locks, in minutes (0: never). */
export const LOCK_AFTER_CHOICES = [1, 2, 5, 10, 15, 30, 60, 0] as const;

/** The accent colours offered, in the order a Mac offers its own: Bloom's preset names. */
export const ACCENTS = ["oxy", "blue", "purple", "pink", "red", "orange", "yellow", "green", "gray"] as const;

/** Memory in the units people read it in: "16 GB", "512 MB". */
export function formatMemory(bytes: number): string {
  const gib = bytes / 1024 ** 3;
  if (gib >= 1) return `${Math.round(gib)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}
