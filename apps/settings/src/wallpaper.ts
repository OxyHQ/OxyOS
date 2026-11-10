/**
 * Settings › Wallpaper's model (docs/adr/0048-wallpapers.mdx): the system's catalog as the
 * platform gives it (`oxyos_wallpaper_catalog`), the person's choice as the session keeps it (one
 * text, decision 2's grammar, which the session judges again), and what the page derives from
 * them. Pure: the page and its tests share it.
 */

/** How a photo covers the display, as a Mac's Fill Screen, Fit to Screen, Stretch to Fill Screen,
    Centre Image and Tile. */
export type WallpaperFit = "fill" | "fit" | "stretch" | "center" | "tile";
export const FITS: readonly WallpaperFit[] = ["fill", "fit", "stretch", "center", "tile"];

/** How often a shuffle or a folder changes: each login, or every so many minutes. */
export type ShuffleEvery = "login" | number;
/** What Settings offers, as a Mac's: when logging in, every 5, 15 and 30 minutes, hour and day. */
export const SHUFFLE_EVERY: readonly ShuffleEvery[] = ["login", 5, 15, 30, 60, 1440];
/** A new shuffle's interval: hourly, as a Mac's starts. */
export const DEFAULT_EVERY: ShuffleEvery = 60;
/** The colour behind what a photo leaves uncovered, until the person picks another. */
export const DEFAULT_MARGIN = "000000";
/** The category id the catalog's colours are a shuffle of. */
export const COLORS_CATEGORY = "colors";

/** The longest choice the session keeps, and the longest path in one, in bytes. */
export const MAX_CHOICE_BYTES = 1100;
export const MAX_PATH_BYTES = 1024;

/** The person's choice, as `oxyos_preferences::wallpaper::Choice` reads it. */
export type WallpaperChoice =
  | { kind: "default" }
  | { kind: "system"; id: string; variant: string | null }
  | { kind: "color"; color: string }
  | { kind: "photo"; fit: WallpaperFit; margin: string; path: string }
  | { kind: "folder"; fit: WallpaperFit; margin: string; every: ShuffleEvery; path: string }
  | { kind: "shuffle"; category: string; every: ShuffleEvery };

/** One way the catalog's wallpaper looks, with its picture (a `file://` URI). */
export type WallpaperVariant = { id: string; name: string; preview: string };
/** A system wallpaper: how Automatic chooses among its variants, its preview, its variants. */
export type SystemWallpaper = {
  id: string;
  name: string;
  automatic: "none" | "appearance" | "sun";
  preview: string;
  variants: readonly WallpaperVariant[];
};
export type WallpaperCategory = { id: string; name: string; wallpapers: readonly SystemWallpaper[] };
/** A solid colour the catalog offers: `#rrggbb` and its name. */
export type CatalogColor = { color: string; name: string };
/** The system's wallpapers, names in the session's language. */
export type WallpaperCatalog = {
  default: string;
  categories: readonly WallpaperCategory[];
  colors: readonly CatalogColor[];
};
export const EMPTY_CATALOG: WallpaperCatalog = { default: "", categories: [], colors: [] };

/** A photo or a folder the person added (*Add Photo…*, *Add Folder…*), which Settings keeps. */
export type AddedWallpaper = { kind: "photo" | "folder"; path: string };
/** At most this many are kept, the latest first. */
export const MAX_ADDED = 24;

const NAME = /^[a-z][a-z0-9-]{0,63}$/;
const HEX = /^[0-9a-f]{6}$/;

function utf8Bytes(text: string): number {
  let bytes = 0;
  for (const char of text) {
    const code = char.codePointAt(0)!;
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return bytes;
}

/** Whether a path is one a choice may name: absolute, at most 1024 bytes, no control character, no
    `.` or `..` component and no `/` at its end (the root alone is no file). */
export function validPath(path: string): boolean {
  if (!path.startsWith("/") || path === "/" || path.endsWith("/")) return false;
  if (utf8Bytes(path) > MAX_PATH_BYTES) return false;
  // biome-ignore lint/suspicious/noControlCharactersInRegex: a path with a control character is refused
  if (/[\u0000-\u001f\u007f]/.test(path)) return false;
  return path
    .slice(1)
    .split("/")
    .every((part) => part !== "" && part !== "." && part !== "..");
}

function parseEvery(text: string): ShuffleEvery | null {
  if (text === "login") return "login";
  if (!/^[1-9][0-9]{0,4}$/.test(text)) return null;
  const minutes = Number(text);
  return minutes <= 10080 ? minutes : null;
}

function parseFit(text: string): WallpaperFit | null {
  return (FITS as readonly string[]).includes(text) ? (text as WallpaperFit) : null;
}

/** Reads a choice as the session keeps it; null for one it would refuse. */
export function parseChoice(text: string): WallpaperChoice | null {
  if (utf8Bytes(text) > MAX_CHOICE_BYTES) return null;
  if (text === "") return { kind: "default" };
  const colon = text.indexOf(":");
  if (colon < 0) {
    const [id, variant, extra] = text.split("/");
    if (extra !== undefined || !NAME.test(id!)) return null;
    if (variant !== undefined && !NAME.test(variant)) return null;
    return { kind: "system", id: id!, variant: variant ?? null };
  }
  const kind = text.slice(0, colon);
  const rest = text.slice(colon + 1);
  // The fields before a path, which may itself hold colons: split only as many as there are.
  const fields = (count: number) => {
    const parts: string[] = [];
    let at = 0;
    for (let i = 0; i < count; i++) {
      const next = rest.indexOf(":", at);
      if (next < 0) return null;
      parts.push(rest.slice(at, next));
      at = next + 1;
    }
    parts.push(rest.slice(at));
    return parts;
  };
  switch (kind) {
    case "color":
      return HEX.test(rest) ? { kind: "color", color: rest } : null;
    case "photo": {
      const parts = fields(2);
      if (!parts) return null;
      const [fitText, margin, path] = parts as [string, string, string];
      const fit = parseFit(fitText);
      return fit && HEX.test(margin) && validPath(path) ? { kind: "photo", fit, margin, path } : null;
    }
    case "folder": {
      const parts = fields(3);
      if (!parts) return null;
      const [fitText, margin, everyText, path] = parts as [string, string, string, string];
      const fit = parseFit(fitText);
      const every = parseEvery(everyText);
      return fit && every !== null && HEX.test(margin) && validPath(path)
        ? { kind: "folder", fit, margin, every, path }
        : null;
    }
    case "shuffle": {
      const parts = rest.split(":");
      if (parts.length !== 2) return null;
      const [category, everyText] = parts as [string, string];
      const every = parseEvery(everyText);
      return NAME.test(category) && every !== null ? { kind: "shuffle", category, every } : null;
    }
    default:
      return null;
  }
}

/** A choice as the session keeps it. */
export function choiceText(choice: WallpaperChoice): string {
  switch (choice.kind) {
    case "default":
      return "";
    case "system":
      return choice.variant ? `${choice.id}/${choice.variant}` : choice.id;
    case "color":
      return `color:${choice.color}`;
    case "photo":
      return `photo:${choice.fit}:${choice.margin}:${choice.path}`;
    case "folder":
      return `folder:${choice.fit}:${choice.margin}:${choice.every}:${choice.path}`;
    case "shuffle":
      return `shuffle:${choice.category}:${choice.every}`;
  }
}

/** The choice the page shows chosen: the catalog's default named, an unreadable one the default. */
export function shownChoice(text: string, catalog: WallpaperCatalog): WallpaperChoice {
  const choice = parseChoice(text) ?? { kind: "default" };
  if (choice.kind === "default" && catalog.default) return { kind: "system", id: catalog.default, variant: null };
  return choice;
}

/** Every system wallpaper, in the catalog's order. */
export function allWallpapers(catalog: WallpaperCatalog): SystemWallpaper[] {
  return catalog.categories.flatMap((category) => category.wallpapers);
}

export function findWallpaper(catalog: WallpaperCatalog, id: string): SystemWallpaper | undefined {
  return allWallpapers(catalog).find((wallpaper) => wallpaper.id === id);
}

/**
 * A tile's value in a row of the page (each row a radio group): what choosing it chooses, the
 * same text whatever options it was chosen with, so the chosen tile stays chosen while its options
 * change. A system wallpaper's tile is the wallpaper (any variant); a shuffle's its category; a
 * photo's and a folder's their path; a colour itself.
 */
export function tileValue(choice: WallpaperChoice): string {
  switch (choice.kind) {
    case "default":
      return "";
    case "system":
      return `system:${choice.id}`;
    case "color":
      return `color:${choice.color}`;
    case "photo":
      return `photo:${choice.path}`;
    case "folder":
      return `folder:${choice.path}`;
    case "shuffle":
      return `shuffle:${choice.category}`;
  }
}

/**
 * What choosing a tile chooses, keeping what the person chose before where it still applies: a
 * photo's fit and colour behind it, a shuffle's interval. A system wallpaper is chosen Automatic.
 */
export function chooseTile(value: string, current: WallpaperChoice): WallpaperChoice | null {
  const colon = value.indexOf(":");
  const kind = value.slice(0, colon);
  const rest = value.slice(colon + 1);
  const fit = current.kind === "photo" || current.kind === "folder" ? current.fit : "fill";
  const margin = current.kind === "photo" || current.kind === "folder" ? current.margin : DEFAULT_MARGIN;
  const every = current.kind === "shuffle" || current.kind === "folder" ? current.every : DEFAULT_EVERY;
  let chosen: WallpaperChoice;
  switch (kind) {
    case "system":
      chosen = { kind: "system", id: rest, variant: null };
      break;
    case "color":
      chosen = { kind: "color", color: rest };
      break;
    case "photo":
      chosen = { kind: "photo", fit, margin, path: rest };
      break;
    case "folder":
      chosen = { kind: "folder", fit, margin, every, path: rest };
      break;
    case "shuffle":
      chosen = { kind: "shuffle", category: rest, every };
      break;
    default:
      return null;
  }
  // Only what the session would keep.
  return parseChoice(choiceText(chosen)) ? chosen : null;
}

/** A colour as `#rrggbb` (what the catalog and a colour well say) or `rrggbb` (a choice's), in
    a choice's form, lowercase; null for anything else. */
export function normalizeHex(text: string): string | null {
  const bare = text.trim().replace(/^#/, "").toLowerCase();
  if (/^[0-9a-f]{3}$/.test(bare)) return bare.replace(/./g, (c) => c + c);
  return HEX.test(bare) ? bare : null;
}

/** Hue (0–360), saturation and brightness (0–1) of `rrggbb`. */
export function hexToHsv(hex: string): { h: number; s: number; v: number } {
  const n = Number.parseInt(hex, 16);
  const r = ((n >> 16) & 255) / 255;
  const g = ((n >> 8) & 255) / 255;
  const b = (n & 255) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = 0;
  if (d > 0) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  return { h, s: max === 0 ? 0 : d / max, v: max };
}

/** `rrggbb` of a hue (0–360), saturation and brightness (0–1). */
export function hsvToHex(h: number, s: number, v: number): string {
  const f = (n: number) => {
    const k = (n + h / 60) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return [f(5), f(3), f(1)]
    .map((c) =>
      Math.round(Math.max(0, Math.min(1, c)) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("");
}

/** Whether white text reads better than black on `rrggbb` (relative luminance below 0.4). */
export function isDark(hex: string): boolean {
  const n = Number.parseInt(hex, 16);
  const lin = (c: number) => {
    const x = c / 255;
    return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  };
  const l = 0.2126 * lin((n >> 16) & 255) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255);
  return l < 0.4;
}

/** A path's last component: a photo's or folder's name as the page shows it. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** A `file://` URI of a path, each component percent-encoded. */
export function fileUri(path: string): string {
  return `file://${path.split("/").map(encodeURIComponent).join("/")}`;
}

/** The path of a `file://` URI (as the file chooser gives one); null for any other. */
export function uriPath(uri: string): string | null {
  if (!uri.startsWith("file://")) return null;
  let path: string;
  try {
    path = decodeURIComponent(uri.slice("file://".length).replace(/^localhost(?=\/)/, ""));
  } catch {
    return null;
  }
  return path.length > 1 && path.endsWith("/") ? path.slice(0, -1) : path;
}

/** The person's added photos and folders with one more: at the front, never twice, at most
    `MAX_ADDED`. */
export function withAdded(list: readonly AddedWallpaper[], item: AddedWallpaper): AddedWallpaper[] {
  const rest = list.filter((kept) => !(kept.kind === item.kind && kept.path === item.path));
  return [item, ...rest].slice(0, MAX_ADDED);
}

/** The list Settings keeps (its document folder's `wallpapers.json`), read: what is valid of it. */
export function parseAdded(text: string | null | undefined): AddedWallpaper[] {
  if (!text) return [];
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return [];
  }
  const items = Array.isArray((value as { added?: unknown })?.added) ? (value as { added: unknown[] }).added : [];
  const added: AddedWallpaper[] = [];
  for (const item of items) {
    const { kind, path } = (item ?? {}) as { kind?: unknown; path?: unknown };
    if ((kind === "photo" || kind === "folder") && typeof path === "string" && validPath(path)) {
      if (!added.some((kept) => kept.kind === kind && kept.path === path)) added.push({ kind, path });
    }
  }
  return added.slice(0, MAX_ADDED);
}

/** The list as Settings keeps it. */
export function addedText(list: readonly AddedWallpaper[]): string {
  return `${JSON.stringify({ added: list }, null, 2)}\n`;
}

/** The added items with the one chosen now, when it is a photo or folder not among them (chosen
    elsewhere, or before the list was kept): shown, so the chosen tile is on the page. */
export function addedWithChosen(list: readonly AddedWallpaper[], choice: WallpaperChoice): AddedWallpaper[] {
  if (choice.kind !== "photo" && choice.kind !== "folder") return [...list];
  if (list.some((item) => item.kind === choice.kind && item.path === choice.path)) return [...list];
  return [{ kind: choice.kind, path: choice.path }, ...list];
}

/** The colours shown: the catalog's, then the one chosen now if it is the person's own. */
export function colorsWithChosen(catalog: WallpaperCatalog, choice: WallpaperChoice): string[] {
  const colors = catalog.colors.map((c) => normalizeHex(c.color)).filter((c): c is string => c !== null);
  if (choice.kind === "color" && !colors.includes(choice.color)) colors.push(choice.color);
  return colors;
}

/** How many tiles a row shows before *Show All*, its shuffle after them: one line of the page. */
export const ROW_TILES = 4;

/** A row's wallpapers as shown: the first `ROW_TILES`, and the chosen one if it is further on, or
    every one once shown all. */
export function shownTiles<T>(items: readonly T[], all: boolean, chosen: (item: T) => boolean): T[] {
  if (all || items.length <= ROW_TILES) return [...items];
  const first = items.slice(0, ROW_TILES);
  if (first.some(chosen)) return first;
  const picked = items.find(chosen);
  return picked ? [...items.slice(0, ROW_TILES - 1), picked] : first;
}
