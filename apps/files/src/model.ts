/**
 * OxyOS's Files (docs/adr/0027-files.mdx): its model, with no React and no platform. What the
 * platform's file service gives (`FileSystem`: on OxyOS the host's `OxyOSFiles`, in the browser
 * and in tests a folder tree kept in memory) and what the window does with it: where it is and
 * where it was, how a folder's items are sorted and found, which are selected, what an item is
 * (its kind) and how its size and date read.
 */

/** An item of a folder, as the service lists it. */
export interface FileEntry {
  name: string;
  path: string;
  /** A folder, or a link to one. */
  folder: boolean;
  /** A symbolic link. */
  link: boolean;
  /** Bytes; 0 for a folder. */
  size: number;
  /** When its content last changed, in milliseconds since 1970. */
  modified: number;
  /** Its name begins with a dot. */
  hidden: boolean;
}

/** An item in the Trash: `id` names it there, `name` and `original` where it was. */
export interface TrashedEntry extends FileEntry {
  id: string;
  original: string | null;
  /** When it was put in the Trash, in milliseconds since 1970. */
  deleted: number | null;
}

/** A favourite folder, one of the user's XDG directories. */
export type FavouriteKey = "desktop" | "documents" | "downloads" | "pictures" | "music" | "videos";

/** What the sidebar shows. */
export interface Places {
  home: string;
  favourites: { key: FavouriteKey; path: string }[];
  volumes: { name: string; path: string }[];
  /** How many items the Trash holds. */
  trash: number;
}

/** Why the service did not do something. */
export type FileErrorCode =
  | "refused"
  | "not-found"
  | "exists"
  | "not-a-folder"
  | "invalid-name"
  | "cross-device"
  | "denied"
  | "io"
  | "unavailable";

export class FileError extends Error {
  readonly code: FileErrorCode;

  constructor(code: FileErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

/** One item's result in a request about several. */
export type ItemResult =
  | { item: string; ok: true; to: string }
  | { item: string; ok: false; error: FileErrorCode };

/** What a watch follows. */
export type WatchTarget = { path: string } | { trash: true } | { places: true };

/** The platform's file service, as an application uses it. */
export interface FileSystem {
  places(): Promise<Places>;
  /** A folder's items, and the folder's canonical path. */
  list(path: string): Promise<{ path: string; entries: FileEntry[] }>;
  createFolder(parent: string, name: string): Promise<FileEntry>;
  rename(path: string, name: string): Promise<FileEntry>;
  move(paths: readonly string[], to: string): Promise<ItemResult[]>;
  trash(paths: readonly string[]): Promise<ItemResult[]>;
  trashList(): Promise<TrashedEntry[]>;
  restore(ids: readonly string[]): Promise<ItemResult[]>;
  deleteFromTrash(ids: readonly string[]): Promise<ItemResult[]>;
  emptyTrash(): Promise<number>;
  /** Calls `changed` whenever what `target` names changes, until the returned function is called. */
  watch(target: WatchTarget, changed: () => void): Promise<() => void>;
}

// ---- Where the window is ----

/** A place the window shows: a folder, the Trash, or the computer (its volumes). */
export type Location = { kind: "folder"; path: string } | { kind: "trash" } | { kind: "computer" };

export function sameLocation(a: Location, b: Location): boolean {
  if (a.kind !== b.kind) return false;
  return a.kind !== "folder" || a.path === (b as { path: string }).path;
}

/** Where the window was and is, as a browser's history: back and forward. */
export interface History {
  readonly entries: readonly Location[];
  readonly index: number;
}

export function historyAt(location: Location): History {
  return { entries: [location], index: 0 };
}

export function currentLocation(history: History): Location {
  return history.entries[history.index];
}

/** Going somewhere new forgets what was forward; going where it is changes nothing. */
export function go(history: History, location: Location): History {
  if (sameLocation(currentLocation(history), location)) return history;
  const entries = [...history.entries.slice(0, history.index + 1), location];
  return { entries, index: entries.length - 1 };
}

export function canGoBack(history: History): boolean {
  return history.index > 0;
}

export function canGoForward(history: History): boolean {
  return history.index < history.entries.length - 1;
}

export function back(history: History): History {
  return canGoBack(history) ? { ...history, index: history.index - 1 } : history;
}

export function forward(history: History): History {
  return canGoForward(history) ? { ...history, index: history.index + 1 } : history;
}

/** Replaces where the window is (a folder that went is left for its parent) without a new entry. */
export function replace(history: History, location: Location): History {
  const entries = history.entries.slice();
  entries[history.index] = location;
  return { entries, index: history.index };
}

// ---- Paths ----

/** A path's parent folder; the root's is itself. */
export function parentOf(path: string): string {
  const at = path.replace(/\/+$/, "").lastIndexOf("/");
  return at <= 0 ? "/" : path.slice(0, at);
}

export function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1) || "/";
}

export function joinPath(folder: string, name: string): string {
  return folder.endsWith("/") ? folder + name : `${folder}/${name}`;
}

/** Whether `path` is `root` or inside it, by whole components. */
export function isWithin(path: string, root: string): boolean {
  return path === root || path.startsWith(root.endsWith("/") ? root : `${root}/`);
}

/** The root a path is in: the home or a mounted volume (the deepest), or none: outside what
 * applications reach (docs/adr/0027-files.mdx, decision 2), so the window never goes there. */
export function rootOf(
  path: string,
  places: Places,
): { kind: "home" | "volume"; path: string; name: string } | null {
  const volume = places.volumes
    .filter((v) => isWithin(path, v.path))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (volume) return { kind: "volume", path: volume.path, name: volume.name };
  if (isWithin(path, places.home)) return { kind: "home", path: places.home, name: baseName(places.home) };
  return null;
}

/** The folders from a path's root down to it, each with the name the path bar shows: the home
 * and a volume by their names, the rest by their own. Empty outside the scope. */
export function pathParts(path: string, places: Places): { name: string; path: string }[] {
  const root = rootOf(path, places);
  if (!root) return [];
  const parts = [{ name: root.name, path: root.path }];
  const rest = path.slice(root.path.length).split("/").filter(Boolean);
  let at = root.path;
  for (const name of rest) {
    at = joinPath(at, name);
    parts.push({ name, path: at });
  }
  return parts;
}

// ---- What an item is ----

export type Kind =
  | "folder"
  | "image"
  | "audio"
  | "video"
  | "pdf"
  | "text"
  | "code"
  | "document"
  | "spreadsheet"
  | "presentation"
  | "archive"
  | "disk-image"
  | "font"
  | "file";

const KINDS: Record<string, Kind> = {};
for (const [kind, extensions] of Object.entries({
  image: "png jpg jpeg gif webp heic heif avif bmp tif tiff svg ico raw cr2 nef dng",
  audio: "mp3 m4a aac flac wav ogg oga opus aiff aif wma mid midi",
  video: "mp4 m4v mov mkv webm avi wmv mpg mpeg ogv 3gp",
  pdf: "pdf",
  text: "txt md markdown rtf log csv tsv nfo",
  code: "js jsx ts tsx mjs cjs json rs c h cc cpp hpp py rb go java kt swift sh zsh bash fish css scss html htm xml yaml yml toml ini conf sql lua php pl mdx",
  document: "doc docx odt pages epub",
  spreadsheet: "xls xlsx ods numbers",
  presentation: "ppt pptx odp key",
  archive: "zip tar gz tgz bz2 xz zst 7z rar deb rpm",
  "disk-image": "iso img dmg qcow2 vmdk",
  font: "ttf otf woff woff2",
} as Record<Exclude<Kind, "folder" | "file">, string>)) {
  for (const extension of extensions.split(" ")) KINDS[extension] = kind as Kind;
}

/** An item's extension, lowercased; none for a name without one or a dot file's own dot. */
export function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : "";
}

/** What an item is, by whether it is a folder and then by its extension. */
export function kindOf(entry: Pick<FileEntry, "name" | "folder">): Kind {
  if (entry.folder) return "folder";
  return KINDS[extensionOf(entry.name)] ?? "file";
}

/** Images the host draws as thumbnails: the formats it decodes (PNG, JPEG), and not so large
 * that decoding one for an icon would be slow. */
export const THUMBNAIL_MAX_BYTES = 24 * 1024 * 1024;

export function hasThumbnail(entry: FileEntry): boolean {
  const extension = extensionOf(entry.name);
  return (
    !entry.folder &&
    (extension === "png" || extension === "jpg" || extension === "jpeg") &&
    entry.size > 0 &&
    entry.size <= THUMBNAIL_MAX_BYTES
  );
}

// ---- Order ----

export type SortColumn = "name" | "modified" | "size" | "kind";
export type SortDirection = "ascending" | "descending";
export interface SortOrder {
  column: SortColumn;
  direction: SortDirection;
}

/** A name folded for comparing and finding: lowercase, without accents. */
export function fold(text: string): string {
  return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

/** Names compared as a Mac's Finder compares them: case and accents aside, and runs of digits
 * by their value ("file 2" before "file 10"); equal ones by their code points, so the order is
 * total. */
export function compareNames(a: string, b: string): number {
  const x = fold(a);
  const y = fold(b);
  let i = 0;
  let j = 0;
  while (i < x.length && j < y.length) {
    const cx = x.charCodeAt(i);
    const cy = y.charCodeAt(j);
    const dx = cx >= 48 && cx <= 57;
    const dy = cy >= 48 && cy <= 57;
    if (dx && dy) {
      let ei = i;
      let ej = j;
      while (ei < x.length && x.charCodeAt(ei) >= 48 && x.charCodeAt(ei) <= 57) ei++;
      while (ej < y.length && y.charCodeAt(ej) >= 48 && y.charCodeAt(ej) <= 57) ej++;
      const nx = x.slice(i, ei).replace(/^0+(?=\d)/, "");
      const ny = y.slice(j, ej).replace(/^0+(?=\d)/, "");
      if (nx.length !== ny.length) return nx.length - ny.length;
      if (nx !== ny) return nx < ny ? -1 : 1;
      i = ei;
      j = ej;
      continue;
    }
    if (cx !== cy) return cx - cy;
    i++;
    j++;
  }
  if (x.length - i !== y.length - j) return x.length - i - (y.length - j);
  return a < b ? -1 : a > b ? 1 : 0;
}

const KIND_ORDER: Kind[] = [
  "folder",
  "image",
  "audio",
  "video",
  "pdf",
  "document",
  "spreadsheet",
  "presentation",
  "text",
  "code",
  "archive",
  "disk-image",
  "font",
  "file",
];

/** Items in the order chosen; ties by name, so the order never depends on how they were listed.
 * Folders are among the files, as Finder keeps them by default; with no size, they come first
 * by size. */
export function sortEntries<T extends FileEntry>(entries: readonly T[], order: SortOrder): T[] {
  const sign = order.direction === "ascending" ? 1 : -1;
  const by = (a: T, b: T): number => {
    switch (order.column) {
      case "modified":
        return a.modified - b.modified;
      case "size":
        return (a.folder ? -1 : a.size) - (b.folder ? -1 : b.size);
      case "kind":
        return KIND_ORDER.indexOf(kindOf(a)) - KIND_ORDER.indexOf(kindOf(b));
      default:
        return 0;
    }
  };
  return entries.slice().sort((a, b) => sign * (by(a, b) || compareNames(a.name, b.name)));
}

/** Clicking a column's header: that column, ascending (dates and sizes newest and largest
 * first, as Finder starts them); clicking the sorted column again reverses it. */
export function sortBy(current: SortOrder, column: SortColumn): SortOrder {
  if (current.column === column) {
    return { column, direction: current.direction === "ascending" ? "descending" : "ascending" };
  }
  return { column, direction: column === "modified" || column === "size" ? "descending" : "ascending" };
}

/** The items shown: hidden ones only when asked for, and only those whose name has the search's
 * text, case and accents aside. */
export function visibleEntries<T extends FileEntry>(
  entries: readonly T[],
  search: string,
  showHidden: boolean,
): T[] {
  const query = fold(search.trim());
  return entries.filter(
    (entry) => (showHidden || !entry.hidden) && (!query || fold(entry.name).includes(query)),
  );
}

// ---- Selection ----

/** Which items are selected, by path, with the anchor a Shift-click extends from and the one the
 * keyboard is at. */
export interface Selection {
  readonly paths: readonly string[];
  readonly anchor: string | null;
  readonly focus: string | null;
}

export const NO_SELECTION: Selection = { paths: [], anchor: null, focus: null };

export function isSelected(selection: Selection, path: string): boolean {
  return selection.paths.includes(path);
}

/** How a click selects: alone, extending from the anchor (Shift), or adding and removing one
 * (Cmd on a Mac, Ctrl on OxyOS keyboards). */
export interface ClickModifiers {
  extend?: boolean;
  toggle?: boolean;
}

/** A click on `path`, among the items as `order` shows them. */
export function clickSelect(
  selection: Selection,
  order: readonly string[],
  path: string,
  modifiers: ClickModifiers = {},
): Selection {
  if (modifiers.extend && selection.anchor && order.includes(selection.anchor)) {
    const from = order.indexOf(selection.anchor);
    const to = order.indexOf(path);
    const range = order.slice(Math.min(from, to), Math.max(from, to) + 1);
    const kept = modifiers.toggle ? selection.paths.filter((p) => !range.includes(p)) : [];
    return { paths: [...kept, ...range], anchor: selection.anchor, focus: path };
  }
  if (modifiers.toggle) {
    const paths = isSelected(selection, path)
      ? selection.paths.filter((p) => p !== path)
      : [...selection.paths, path];
    return { paths, anchor: path, focus: path };
  }
  // A click on an item already selected keeps the others, so it can be dragged with them; the
  // window selects it alone if the press is released without a drag (`selectOnly`).
  if (isSelected(selection, path)) return { ...selection, anchor: path, focus: path };
  return { paths: [path], anchor: path, focus: path };
}

/** Only `path` selected, as a click alone selects it (a right-click on an unselected item). */
export function selectOnly(path: string): Selection {
  return { paths: [path], anchor: path, focus: path };
}

export function selectAll(order: readonly string[]): Selection {
  return { paths: order.slice(), anchor: order[0] ?? null, focus: order[order.length - 1] ?? null };
}

/** The selection with items no longer shown left out (gone, renamed or filtered away). */
export function keepShown(selection: Selection, order: readonly string[]): Selection {
  const paths = selection.paths.filter((p) => order.includes(p));
  if (paths.length === selection.paths.length) return selection;
  const keep = (p: string | null) => (p && order.includes(p) ? p : (paths[paths.length - 1] ?? null));
  return { paths, anchor: keep(selection.anchor), focus: keep(selection.focus) };
}

/** An arrow key: the next item `step` places away (1 or −1 in a list; ±1 across and ±columns down
 * a grid), clamped at the ends; with Shift the selection extends from the anchor to it. With
 * nothing selected the first (or last, going back) is. */
export function moveFocus(
  selection: Selection,
  order: readonly string[],
  step: number,
  extend = false,
): Selection {
  if (order.length === 0) return selection;
  const at = selection.focus ? order.indexOf(selection.focus) : -1;
  const next =
    at < 0 ? (step > 0 ? 0 : order.length - 1) : Math.max(0, Math.min(order.length - 1, at + step));
  if (at >= 0 && at + step !== next && Math.abs(step) > 1) {
    // A grid's row below the last: the last item, as Finder goes there.
    if (next === at) return selection;
  }
  const path = order[next];
  if (extend && selection.anchor && order.includes(selection.anchor)) {
    const from = order.indexOf(selection.anchor);
    return {
      paths: order.slice(Math.min(from, next), Math.max(from, next) + 1),
      anchor: selection.anchor,
      focus: path,
    };
  }
  return selectOnly(path);
}

/** Home and End: the first or the last item. */
export function focusEnd(
  selection: Selection,
  order: readonly string[],
  end: "first" | "last",
  extend = false,
): Selection {
  if (order.length === 0) return selection;
  const at = selection.focus ? order.indexOf(selection.focus) : -1;
  const target = end === "first" ? 0 : order.length - 1;
  return moveFocus(selection, order, at < 0 ? (end === "first" ? 1 : -1) : target - at, extend);
}

/** A rectangle in the view's own coordinates. */
export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The rectangle from where a drag began to where it is now. */
export function boxBetween(from: { x: number; y: number }, to: { x: number; y: number }): Box {
  return {
    x: Math.min(from.x, to.x),
    y: Math.min(from.y, to.y),
    width: Math.abs(to.x - from.x),
    height: Math.abs(to.y - from.y),
  };
}

function intersects(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** A rubber band over the items laid out at `frames` (by path, in `order`): those it touches,
 * added to what was selected when it began if a modifier was held (Shift or Ctrl), else alone. */
export function bandSelect(
  before: Selection,
  order: readonly string[],
  frames: ReadonlyMap<string, Box>,
  band: Box,
  adding: boolean,
): Selection {
  const touched = order.filter((path) => {
    const frame = frames.get(path);
    return frame !== undefined && intersects(frame, band);
  });
  const paths = adding
    ? [
        ...before.paths.filter((p) => !touched.includes(p)),
        ...touched.filter((p) => !before.paths.includes(p)),
      ]
    : touched;
  const focus = touched[touched.length - 1] ?? before.focus;
  return { paths, anchor: touched[0] ?? before.anchor, focus: paths.length ? focus : null };
}

/** Typing a name's first letters selects the first item, in the order shown, whose name begins
 * with them, case and accents aside; or, with none, the first that would come after them, as
 * Finder's type-select does. */
export function typeSelect<T extends FileEntry>(shown: readonly T[], typed: string): string | null {
  const prefix = fold(typed);
  if (!prefix) return null;
  const starts = shown.find((entry) => fold(entry.name).startsWith(prefix));
  if (starts) return starts.path;
  const sorted = shown.slice().sort((a, b) => compareNames(a.name, b.name));
  const after = sorted.find((entry) => compareNames(entry.name, typed) > 0);
  return (after ?? sorted[sorted.length - 1])?.path ?? null;
}

/** How long a pause ends what is being typed for type-select. */
export const TYPE_SELECT_PAUSE_MS = 1000;

// ---- Names ----

/** The service's own rule (crates/oxyos-files `valid_name`), so a rename is refused as it is
 * typed: not empty or only spaces, no `/`, not `.` or `..`, at most 255 bytes. */
export function validName(name: string): boolean {
  if (!name.trim() || name.includes("/") || name.includes("\0") || name === "." || name === "..")
    return false;
  return utf8Length(name) <= 255;
}

function utf8Length(text: string): number {
  let bytes = 0;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return bytes;
}

/** A new folder's name: `base`, or `base 2`, `base 3`... the first no item has. */
export function freeName(base: string, taken: readonly string[]): string {
  const names = new Set(taken.map((name) => name.toLowerCase()));
  if (!names.has(base.toLowerCase())) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base} ${n}`;
    if (!names.has(candidate.toLowerCase())) return candidate;
  }
}

/** The part of a name a rename selects first: before its extension, as Finder does. */
export function nameSelection(name: string, folder: boolean): { start: number; end: number } {
  const dot = folder ? -1 : name.lastIndexOf(".");
  return { start: 0, end: dot > 0 ? dot : name.length };
}

// ---- Dragging ----

/** Where dragged items may go: a folder that is not one of them and not inside one of them, and
 * not the folder they are already all in. */
export function canDropInto(
  dragged: readonly string[],
  target: FileEntry | { path: string; folder: true },
): boolean {
  if (!target.folder || dragged.length === 0) return false;
  if (dragged.some((path) => isWithin(target.path, path))) return false;
  return !dragged.every((path) => parentOf(path) === target.path);
}

// ---- How it reads ----

/** A size as Finder writes it: decimal units, one decimal under 10 of a unit. */
export function formatSize(
  bytes: number,
  units: { bytes: (n: number) => string; kb: string; mb: string; gb: string; tb: string },
): string {
  if (bytes < 1000) return units.bytes(bytes);
  const steps = [units.kb, units.mb, units.gb, units.tb];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < steps.length - 1) {
    value /= 1000;
    unit++;
  }
  const text = value < 10 ? (Math.round(value * 10) / 10).toString() : Math.round(value).toString();
  return `${text} ${steps[unit]}`;
}

/** Whether two times are on the same calendar day where the machine is. */
function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** A date as Finder's list writes it: "Today at 10:04", "Yesterday at…", else the date and time. */
export function formatDate(
  ms: number,
  now: number,
  locale: string,
  words: { today: (time: string) => string; yesterday: (time: string) => string },
): string {
  const date = new Date(ms);
  const time = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" }).format(date);
  const today = new Date(now);
  if (sameDay(date, today)) return words.today(time);
  const yesterday = new Date(now - 24 * 60 * 60 * 1000);
  if (sameDay(date, yesterday)) return words.yesterday(time);
  const day = new Intl.DateTimeFormat(locale, { day: "numeric", month: "short", year: "numeric" }).format(
    date,
  );
  return `${day}, ${time}`;
}
