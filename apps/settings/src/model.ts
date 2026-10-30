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

/** One mode a display can be driven at: its pixels and refresh (millihertz), and whether it is the
    display's own (native) mode. */
export type DisplayMode = { width: number; height: number; refreshMhz: number; preferred: boolean };

/**
 * A display the session shows (docs/adr/0018-several-displays.mdx), as it tells Settings: its
 * place in the arrangement (logical pixels), its scale (in 120ths: 150 is 1.25), its mode (an
 * index of its modes, the largest first) and the scales it may take, from the most space to the
 * largest text, with the one its density recommends.
 */
export type Display = {
  output: number;
  name: string;
  builtin: boolean;
  primary: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  mode: number;
  modes: readonly DisplayMode[];
  scales: readonly number[];
  recommended: number;
  widthMm: number;
  heightMm: number;
};

/** The displays, and a mode on trial: shown, and going back unless kept within `seconds`. */
export type DisplaysSnapshot = {
  displays: readonly Display[];
  trial: { output: number; seconds: number } | null;
};

export type SettingsSnapshot = {
  preferences: Preferences;
  wallpapers: readonly Wallpaper[];
  system: SystemFacts;
  displays: DisplaysSnapshot;
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
  /** A display at one of its modes: on trial until kept (`confirmDisplays`). */
  setDisplayMode: (output: number, mode: DisplayMode) => void;
  /** A display at one of its scales, kept at once. */
  setDisplayScale: (output: number, scale: number) => void;
  /** Where a display is wanted (logical pixels): the session places it beside the others. */
  setDisplayOrigin: (output: number, x: number, y: number) => void;
  /** The display with the dock. */
  setPrimaryDisplay: (output: number) => void;
  /** Keep the mode on trial, or go back to what the display showed. */
  confirmDisplays: (keep: boolean) => void;
};

/** The resolutions a display offers, the largest first, each once, with whether it is its own. */
export function resolutions(display: Display): { width: number; height: number; native: boolean }[] {
  const seen: { width: number; height: number; native: boolean }[] = [];
  for (const mode of display.modes) {
    const listed = seen.find((r) => r.width === mode.width && r.height === mode.height);
    if (listed) listed.native ||= mode.preferred;
    else seen.push({ width: mode.width, height: mode.height, native: mode.preferred });
  }
  return seen;
}

/** The modes of one resolution: its refresh rates, the fastest first. */
export function refreshRates(display: Display, width: number, height: number): DisplayMode[] {
  return display.modes
    .filter((mode) => mode.width === width && mode.height === height)
    .sort((a, b) => b.refreshMhz - a.refreshMhz);
}

/** A refresh rate as people read it: "60 Hz", "59.94 Hz", "143.98 Hz". */
export function formatRefresh(refreshMhz: number): string {
  const hz = Math.round(refreshMhz / 10) / 100;
  return `${Number.isInteger(hz) ? hz.toFixed(0) : String(hz)} Hz`;
}

/** What a display looks like at a scale: its mode's pixels in logical pixels, rounded up as the
    session rounds them (`arrangement::logical`). */
export function looksLike(display: Display, scale: number): { width: number; height: number } {
  const mode = display.modes[display.mode];
  const at = (pixels: number) => Math.max(1, Math.ceil((pixels * 120) / scale));
  return { width: at(mode?.width ?? display.width), height: at(mode?.height ?? display.height) };
}

/** A size as "1920 × 1200". */
export function formatSize(width: number, height: number): string {
  return `${width} × ${height}`;
}

/** A display's diagonal in whole inches, from its physical size; null when unknown. */
export function diagonalInches(widthMm: number, heightMm: number): number | null {
  if (widthMm < 50 || heightMm < 30) return null;
  return Math.round(Math.hypot(widthMm, heightMm) / 25.4);
}

/** Where a display moved to one side of all the others goes (logical pixels), for the session to
    place it there beside them: an accessible way to arrange, as a drag is the visible one. */
export function besideOthers(
  displays: readonly Display[],
  output: number,
  side: "left" | "right" | "above" | "below",
): { x: number; y: number } {
  const display = displays.find((d) => d.output === output);
  const others = displays.filter((d) => d.output !== output);
  if (!display || others.length === 0) return { x: display?.x ?? 0, y: display?.y ?? 0 };
  const left = Math.min(...others.map((d) => d.x));
  const top = Math.min(...others.map((d) => d.y));
  const right = Math.max(...others.map((d) => d.x + d.width));
  const bottom = Math.max(...others.map((d) => d.y + d.height));
  switch (side) {
    case "left":
      return { x: left - display.width, y: top };
    case "right":
      return { x: right, y: top };
    case "above":
      return { x: left, y: top - display.height };
    case "below":
      return { x: left, y: bottom };
  }
}

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
