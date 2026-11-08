import type { AccountActions, AccountSnapshot } from "./account";
import type { BluetoothActions, BluetoothState } from "./bluetooth";
import type { LowPowerMode, PowerActions, PowerState } from "./battery";
import type { EraseActions } from "./erase";
import type { InputMethod, InputSourcesState } from "./input-sources";
import type { NetworkActions, NetworkState } from "./network";
import type { SoundActions, SoundState } from "./sound";
import type { StorageActions } from "./storage";
import type { PrivacyActions, PrivacyApplication, UpdatesActions, UpdatesSnapshot } from "./updates";
import type { AddedWallpaper, WallpaperCatalog } from "./wallpaper";

/**
 * What the Settings application shows (docs/adr/0019-settings.mdx): the user's preferences, which
 * it may change, and the system's settings and facts, which it only shows for now. Supplied by
 * the platform (OxyOS's `OxyOSSettings`, or a fixture in the browser), never invented by the UI.
 */
export type Appearance = "light" | "dark" | "auto";
/** When the lock screen may show what notifications say (docs/adr/0026-notifications.mdx). */
export type NotificationPreviews = "always" | "unlocked";
/** A language OxyOS speaks, or empty for the system's (docs/adr/0040-language-and-region.mdx). */
export type LanguagePreference = "" | "en" | "es";
/** Twelve or 24 hours: the region's (`auto`), or the person's choice. */
export type HourCycle = "auto" | "h12" | "h23";
/** How many fingers swipe for one of Trackpad's More Gestures (docs/adr/0046-interactive-gestures.mdx),
    or none: as a Mac's trackpad offers them, three or four fingers, or off. */
export type SwipeFingers = "three" | "four" | "off";

/** The user's preferences, as `crates/oxyos-preferences` keeps them. */
export type Preferences = {
  appearance: Appearance;
  /** One of Bloom's accent colours, by name. */
  accent: string;
  reduceMotion: boolean;
  reduceTransparency: boolean;
  /** A text field's caret blinks (docs/adr/0037-text-editing.mdx); off, it stays lit. */
  caretBlink: boolean;
  /** Play user interface sound effects (docs/adr/0044-system-sounds.mdx): the Trash's sounds. */
  uiSounds: boolean;
  /** Natural scrolling on a touchpad (docs/adr/0042-scrolling.mdx, decision 6): the content follows
      the fingers. On, as a Mac's trackpad ships. */
  trackpadNaturalScrolling: boolean;
  /** Natural scrolling on a mouse's wheel and every other pointing device: off, as every mouse has. */
  mouseNaturalScrolling: boolean;
  /** Trackpad's More Gestures (docs/adr/0046-interactive-gestures.mdx), as a Mac's Trackpad settings
      offers them, each three fingers by default. Mission Control: a swipe up. The session links it
      to App Exposé as a Mac does: choosing three or four fingers for one chooses the same for the
      other unless the other is off, and the next snapshot says both. */
  missionControlSwipe: SwipeFingers;
  /** App Exposé: a swipe down (docs/adr/0045-app-expose.mdx). */
  appExposeSwipe: SwipeFingers;
  /** A swipe left or right between desktops (ADR 0046, decision 4). */
  desktopsSwipe: SwipeFingers;
  /** Launchpad: a pinch with the thumb and three fingers. On, as a Mac's. */
  launchpadPinch: boolean;
  /** The wallpaper where Settings' window is, as the session tells it (docs/adr/0048-wallpapers.mdx,
      decision 4): a choice's text (`src/wallpaper.ts`), empty for the catalog's default. */
  wallpaper: string;
  /** *Show on all desktops*: off, a choice is the current desktop's alone. */
  wallpaperAllDesktops: boolean;
  /** Minutes idle before the session locks; 0 never. */
  lockAfter: number;
  screenReader: boolean;
  /** The session's language (ADR 0040, decision 1): empty is the system's. */
  language: LanguagePreference;
  /** A region as CLDR names one, two ASCII capitals ("ES", "US"); empty is the system's. */
  region: string;
  hourCycle: HourCycle;
  /** Do Not Disturb: notifications are kept, none shown as a banner (docs/adr/0026-notifications.mdx). */
  doNotDisturb: boolean;
  notificationPreviews: NotificationPreviews;
  /** The applications whose notifications play no sound (docs/adr/0044-system-sounds.mdx), by id. */
  notificationSoundsOff: string[];
  /** The applications the session opens at login, in order (Open at Login, the dock's menu's too,
      docs/adr/0060-login-items-and-background-activity.mdx), by id. */
  openAtLogin: string[];
  /** The input methods added after the keyboard layout, in order (docs/adr/0041-input-methods.mdx,
      decision 6). */
  inputSources: InputMethod[];
  /** The current input source: `layout`, or one of `inputSources`. */
  inputSource: string;
  /** The battery's percentage beside its icon in the menu bar (docs/adr/0057-battery-and-energy.mdx). */
  batteryPercentage: boolean;
  /** When Low Power Mode is on. */
  lowPowerMode: LowPowerMode;
  /** Minutes with no input before the display turns off and the computer sleeps, on battery and
      on the power adapter; 0 never. */
  displayOffBattery: number;
  displayOffAdapter: number;
  sleepBattery: number;
  sleepAdapter: number;
  /** The display slightly dimmer on battery. */
  dimOnBattery: boolean;
  /** *Empty Trash Automatically* (docs/adr/0062-storage.mdx): the session erases what has been in
      the Trash for more than 30 days. Off, as a Mac ships. */
  emptyTrashAutomatically: boolean;
};

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
  /** The system's locale, as its environment gives it ("es-ES"): the session's language and region
      when the person chose none (docs/adr/0040-language-and-region.mdx, decision 1). */
  locale: string;
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

/** How long a mode is on trial, as the session keeps it (docs/adr/0018-several-displays.mdx,
    `oxyos_compositor::displays::TRIAL`): a Mac's 15 seconds. */
export const TRIAL_SECONDS = 15;

/** The displays, and a mode on trial: shown, and going back unless kept within `seconds`. */
export type DisplaysSnapshot = {
  displays: readonly Display[];
  trial: { output: number; seconds: number } | null;
};

export type SettingsSnapshot = {
  preferences: Preferences;
  /** The system's wallpapers (docs/adr/0048-wallpapers.mdx, decision 1), names in the session's
      language. */
  wallpapers: WallpaperCatalog;
  /** The photos and folders the person added, the latest first, as Settings keeps them. */
  addedWallpapers?: readonly AddedWallpaper[];
  system: SystemFacts;
  displays: DisplaysSnapshot;
  /** Software Update (docs/adr/0030-installing-applications.mdx, decision 16): none without the
      platform's `updates` grant. */
  updates?: UpdatesSnapshot;
  /** Privacy (decision 8): the installed applications' capabilities; none without `privacy`. */
  privacy?: PrivacyApplication[] | null;
  /** Wi-Fi and networks (docs/adr/0038-networks.mdx), as the session tells them: none without the
      platform's `network` grant; null when granted but not told. */
  network?: NetworkState | null;
  /** Bluetooth (docs/adr/0055-bluetooth.mdx), as the session tells it: none without the platform's
      `bluetooth` grant (the page and its sidebar item are not there). */
  bluetooth?: BluetoothState;
  /** The sound's outputs and inputs (docs/adr/0056-sound-devices.mdx), as the session tells them:
      null until it has. */
  sound?: SoundState | null;
  /** The battery and energy (docs/adr/0057-battery-and-energy.mdx), as the session tells them:
      none where the platform tells none; null until it has. */
  power?: PowerState | null;
  /** The input sources (docs/adr/0041-input-methods.mdx, decision 6) as the session tells them;
      null until it has. */
  inputSources?: InputSourcesState | null;
  /** The session's applications, by id and name (its registry): Notifications' sound for each
      (docs/adr/0044-system-sounds.mdx), and what Open at Login's + offers
      (docs/adr/0060-login-items-and-background-activity.mdx). */
  applications?: readonly { id: string; name: string }[];
  /** The machine's Oxy session (docs/adr/0020-oxy-account-login.mdx), as the platform tells every
      application: none outside a session, which shows no account item. */
  account?: AccountSnapshot | null;
};

/** The keys a change names, as the session's preferences file does. */
export const PREFERENCE_KEYS = {
  appearance: "appearance",
  accent: "accent",
  reduceMotion: "reduce_motion",
  reduceTransparency: "reduce_transparency",
  caretBlink: "caret_blink",
  uiSounds: "ui_sounds",
  trackpadNaturalScrolling: "trackpad_natural_scrolling",
  mouseNaturalScrolling: "mouse_natural_scrolling",
  missionControlSwipe: "mission_control_swipe",
  appExposeSwipe: "app_expose_swipe",
  desktopsSwipe: "desktops_swipe",
  launchpadPinch: "launchpad_pinch",
  wallpaper: "wallpaper",
  wallpaperAllDesktops: "wallpaper_all_desktops",
  lockAfter: "lock_after",
  screenReader: "screen_reader",
  language: "language",
  region: "region",
  hourCycle: "hour_cycle",
  doNotDisturb: "do_not_disturb",
  notificationPreviews: "notification_previews",
  notificationSoundsOff: "notification_sounds_off",
  openAtLogin: "open_at_login",
  inputSources: "input_sources",
  inputSource: "input_source",
  batteryPercentage: "battery_percentage",
  lowPowerMode: "low_power_mode",
  displayOffBattery: "display_off_battery",
  displayOffAdapter: "display_off_adapter",
  sleepBattery: "sleep_battery",
  sleepAdapter: "sleep_adapter",
  dimOnBattery: "dim_on_battery",
  emptyTrashAutomatically: "empty_trash_automatically",
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
  /** Settings › Wallpaper's platform: thumbnails, folders and the file chooser. */
  wallpaper?: WallpaperActions;
  updates?: UpdatesActions;
  privacy?: PrivacyActions;
  network?: NetworkActions;
  bluetooth?: BluetoothActions;
  sound?: SoundActions;
  account?: AccountActions;
  power?: PowerActions;
  /** Erasing the computer (General › Transfer or Reset, docs/adr/0054-recovery-and-erasing.mdx):
      none where the platform has no account session to ask (a development session started from a
      terminal), which offers no erase. */
  erase?: EraseActions;
  /** Settings › Spotlight's platform (docs/adr/0051-search.mdx). */
  search?: SettingsSearch;
  /** Settings › Storage's platform (docs/adr/0062-storage.mdx): none where the session measures
      nothing (no `settings` grant), which shows Storage is not available here. */
  storage?: StorageActions;
  /** The system's folder chooser (docs/adr/0028-file-access.mdx): the folder chosen, as a
      `file://` URI, or null when cancelled. Without it, nothing asks for a folder. */
  chooseFolder?: () => Promise<string | null>;
};

/** What a search result is (docs/adr/0051-search.mdx), as the shell's `SearchCategory`. */
export type SearchCategory =
  | "application"
  | "settings"
  | "folder"
  | "document"
  | "pdf"
  | "text"
  | "source"
  | "presentation"
  | "spreadsheet"
  | "image"
  | "movie"
  | "music"
  | "archive"
  | "contact"
  | "event"
  | "mail"
  | "message"
  | "other";
/** Spotlight's preferences: the kinds its results show, and the folders kept out of the index
    (`file://` URIs). */
export type SearchPreferences = { categories: Record<SearchCategory, boolean>; excluded: string[] };
/** The index as the search service tells it: being built, how many items it holds (the index
    answering's), whether it holds only part of what it would, whether it is being rebuilt, and how
    many items the crawl under way went through so far (null with none going on). */
export type SearchStatus = {
  indexing: boolean;
  items: number;
  partial: boolean;
  rebuilding?: boolean;
  found?: number | null;
};
/** What Settings › Spotlight asks of the search service (docs/adr/0051-search.mdx). Each change
    answers with the preferences the service kept. */
export type SettingsSearch = {
  preferences(): Promise<SearchPreferences>;
  setPreferences(change: Partial<SearchPreferences>): Promise<SearchPreferences>;
  /** The index's state, followed (docs/adr/0058-the-platform-api.mdx, decision 3): told by the
      service as it changes and only while the page follows it, never asked on a clock. */
  state: { current(): SearchStatus | null; subscribe(listener: () => void): () => void };
  /** Discards the index and builds it again from the files; the old one answers until then. */
  rebuild(): Promise<void>;
};

/**
 * What Settings › Wallpaper asks of the platform (docs/adr/0048-wallpapers.mdx, decision 7). Where
 * a platform lacks one, the page goes without it: no thumbnail cache draws the photo itself, no
 * chooser shows no *Add Photo…*.
 */
export type WallpaperActions = {
  /** A thumbnail of the image at `uri`, `width` × `height` pixels, from the platform's cache: its
      `file://` URI, or null when the image cannot be read. */
  thumbnail?: (uri: string, width: number, height: number) => Promise<string | null>;
  /** A folder's images, as a shuffle takes them (`file://` URIs). */
  folder?: (path: string) => Promise<string[]>;
  /** The system's file chooser: a photo's or a folder's path, or null when cancelled. */
  pickPhoto?: () => Promise<string | null>;
  pickFolder?: () => Promise<string | null>;
  /** Keeps the person's added photos and folders. */
  setAdded?: (added: AddedWallpaper[]) => void;
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

/** A side of another display a display stands at. */
export type Side = "left" | "right" | "above" | "below";

/** Where a display stands in the arrangement, as a person says it: at which side of the display it
    shares the longest edge with ("right of Built-in Display"); null alone, or touching none. */
export function placeOf(displays: readonly Display[], output: number): { side: Side; of: Display } | null {
  const display = displays.find((d) => d.output === output);
  if (!display) return null;
  const overlap = (a0: number, a1: number, b0: number, b1: number) => Math.min(a1, b1) - Math.max(a0, b0);
  let best: { side: Side; of: Display; shared: number } | null = null;
  for (const other of displays) {
    if (other.output === output) continue;
    const across = overlap(display.y, display.y + display.height, other.y, other.y + other.height);
    const along = overlap(display.x, display.x + display.width, other.x, other.x + other.width);
    const side: Side | null =
      across > 0 && display.x === other.x + other.width
        ? "right"
        : across > 0 && display.x + display.width === other.x
          ? "left"
          : along > 0 && display.y + display.height === other.y
            ? "above"
            : along > 0 && display.y === other.y + other.height
              ? "below"
              : null;
    if (side === null) continue;
    const shared = side === "left" || side === "right" ? across : along;
    if (!best || shared > best.shared) best = { side, of: other, shared };
  }
  return best && { side: best.side, of: best.of };
}

/** The arrow key that moves a display to a side of the others, as the arrangement takes them. */
export function sideOfKey(key: string): Side | null {
  switch (key) {
    case "ArrowLeft":
      return "left";
    case "ArrowRight":
      return "right";
    case "ArrowUp":
      return "above";
    case "ArrowDown":
      return "below";
    default:
      return null;
  }
}

/** A preference's value as the session's file writes it: a list as its names joined by commas. */
export function preferenceText<K extends keyof Preferences>(value: Preferences[K]): string {
  if (value === null) return "";
  if (Array.isArray(value)) return value.join(",");
  return String(value);
}

/** Accessibility's "Prefer non-blinking cursor", as macOS names and places it: on is the session's
    `caret_blink` off, so every text field's caret stays lit. */
export function prefersSteadyCaret(preferences: Pick<Preferences, "caretBlink">): boolean {
  return !preferences.caretBlink;
}

/** Whose notifications play no sound once `id`'s are turned on or off (Settings' Notifications page,
    docs/adr/0044-system-sounds.mdx): the list in the order it was, `id` added at its end or taken out. */
export function notificationSoundsWith(off: readonly string[], id: string, on: boolean): string[] {
  const rest = off.filter((silent) => silent !== id);
  return on ? rest : [...rest, id];
}

/** The energy preferences as the session's defaults have them (crates/oxyos-preferences): the
    display off after 2 minutes on battery and 10 on the adapter, asleep after 10 on battery and
    never on the adapter, dimmer on battery, no Low Power Mode, no percentage in the menu bar. */
export const ENERGY_DEFAULTS = {
  batteryPercentage: false,
  lowPowerMode: "never",
  displayOffBattery: 2,
  displayOffAdapter: 10,
  sleepBattery: 10,
  sleepAdapter: 0,
  dimOnBattery: true,
} as const satisfies Pick<
  Preferences,
  | "batteryPercentage"
  | "lowPowerMode"
  | "displayOffBattery"
  | "displayOffAdapter"
  | "sleepBattery"
  | "sleepAdapter"
  | "dimOnBattery"
>;

/** The choices a swipe of Trackpad's More Gestures offers, in a Mac's order: three fingers, four,
    off. */
export const SWIPE_FINGERS = ["three", "four", "off"] as const satisfies readonly SwipeFingers[];

/** A swipe's menu: each choice in that order, named in a Mac's words for that swipe ("Swipe up
    with three fingers"). */
export function swipeItems(words: Record<SwipeFingers, string>): { value: SwipeFingers; label: string }[] {
  return SWIPE_FINGERS.map((fingers) => ({ value: fingers, label: words[fingers] }));
}

/** A swipe's fingers as the session's file keeps them: three unless it says four or off, as the
    session reads it (crates/oxyos-preferences). */
export function swipeFingersOf(text: string | undefined): SwipeFingers {
  return text === "four" || text === "off" ? text : "three";
}

/** Trackpad's More Gestures as the session's defaults have them (crates/oxyos-preferences), a Mac's:
    three fingers for Mission Control, App Exposé and the desktops, and Launchpad's pinch on. */
export const GESTURE_DEFAULTS = {
  missionControlSwipe: "three",
  appExposeSwipe: "three",
  desktopsSwipe: "three",
  launchpadPinch: true,
} as const satisfies Pick<Preferences, "missionControlSwipe" | "appExposeSwipe" | "desktopsSwipe" | "launchpadPinch">;

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
