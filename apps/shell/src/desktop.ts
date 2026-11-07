/**
 * The desktop's files as a Mac's desktop shows them (docs/adr/0047-finder-drags-and-the-desktop.mdx,
 * decisions 1 to 7): where each icon is on its grid, which are selected, what opening them does and
 * what a drop on the desktop does. Pure: the desktop (components/Desktop.tsx) reads the folder and
 * acts; this decides.
 *
 * The grid is laid out as a Mac's: from the top right, down a column and then the next one to its
 * left. A cell is named by its column, counted from the right edge, and its row, from the top.
 */

/** One item of `~/Desktop` as the desktop shows it. */
export type DesktopItem = {
  name: string;
  /** Its `file://` URI. */
  uri: string;
  folder: boolean;
  /** Its media type as the platform names it (`File.type`); "" for a folder or none known. */
  type: string;
};

/** A place on the desktop's grid: `column` from the right edge, `row` from the top. */
export type Cell = { column: number; row: number };
/** How many cells fit the desktop's area. */
export type Grid = { columns: number; rows: number };
/** Where each icon is kept, by its item's name (the person's arrangement, `desktop.json`). */
export type Arrangement = Readonly<Record<string, Cell>>;
export type Point = { x: number; y: number };
export type Rect = { x: number; y: number; width: number; height: number };

/** The picture's size, a Mac's desktop's default (64 points). */
export const DESKTOP_ICON = 64;
/** The rounded square a selected picture is drawn on: the picture with 4 pixels around it. */
export const DESKTOP_PICTURE_BOX = DESKTOP_ICON + 8;
/** The name's width, and its two lines' height. */
export const DESKTOP_NAME_WIDTH = 92;
export const DESKTOP_NAME_LINE = 15;
/** A cell of the grid, and the margin between the grid and the area's top and right edges. */
export const DESKTOP_CELL = { width: 100, height: 112 } as const;
export const DESKTOP_EDGE = 8;
/** How far the picture is from the cell's top, and the name from the picture. */
const NAME_GAP = 2;

/** The cells that fit an area of `width` × `height` (at least one of each). */
export function gridOf(width: number, height: number): Grid {
  return {
    columns: Math.max(1, Math.floor((width - DESKTOP_EDGE) / DESKTOP_CELL.width)),
    rows: Math.max(1, Math.floor((height - DESKTOP_EDGE) / DESKTOP_CELL.height)),
  };
}

/** A cell's top-left corner in an area `width` wide. */
export function cellOrigin(cell: Cell, width: number): Point {
  return {
    x: width - DESKTOP_EDGE - (cell.column + 1) * DESKTOP_CELL.width,
    y: DESKTOP_EDGE + cell.row * DESKTOP_CELL.height,
  };
}

/** What an icon in `cell` covers: its picture's square and its name's two lines beneath. */
export function iconRect(cell: Cell, width: number): Rect {
  const origin = cellOrigin(cell, width);
  return {
    x: origin.x + (DESKTOP_CELL.width - DESKTOP_NAME_WIDTH) / 2,
    y: origin.y,
    width: DESKTOP_NAME_WIDTH,
    height: DESKTOP_PICTURE_BOX + NAME_GAP + 2 * DESKTOP_NAME_LINE,
  };
}

const keyOf = (cell: Cell) => `${cell.column},${cell.row}`;

/** Whether `cell` is on the grid. */
export function onGrid(cell: Cell, grid: Grid): boolean {
  return cell.column >= 0 && cell.row >= 0 && cell.column < grid.columns && cell.row < grid.rows;
}

/** The cell whose middle is nearest `point` (the area's coordinates), on the grid. */
export function cellNear(point: Point, width: number, grid: Grid): Cell {
  const column = Math.round((width - DESKTOP_EDGE - point.x) / DESKTOP_CELL.width - 0.5);
  const row = Math.round((point.y - DESKTOP_EDGE) / DESKTOP_CELL.height - 0.5);
  return {
    column: Math.min(grid.columns - 1, Math.max(0, column)),
    row: Math.min(grid.rows - 1, Math.max(0, row)),
  };
}

/**
 * The first cell not taken, from the top right, down each column and then leftward; past the
 * grid's last column when every cell of it is taken (an icon is never lost, as a Mac's are piled).
 */
export function firstFree(grid: Grid, taken: ReadonlySet<string>): Cell {
  for (let column = 0; ; column++) {
    for (let row = 0; row < grid.rows; row++) {
      if (!taken.has(keyOf({ column, row }))) return { column, row };
    }
  }
}

/** The free cell of the grid nearest `cell` (by the distance between their middles), else the first free. */
export function freeNear(cell: Cell, grid: Grid, taken: ReadonlySet<string>): Cell {
  let best: Cell | null = null;
  let distance = Infinity;
  for (let column = 0; column < grid.columns; column++) {
    for (let row = 0; row < grid.rows; row++) {
      if (taken.has(keyOf({ column, row }))) continue;
      const dx = (column - cell.column) * DESKTOP_CELL.width;
      const dy = (row - cell.row) * DESKTOP_CELL.height;
      const d = dx * dx + dy * dy;
      // Ties go to the cell first in the grid's order (rightmost, then topmost).
      if (d < distance) {
        best = { column, row };
        distance = d;
      }
    }
  }
  return best ?? firstFree(grid, taken);
}

/**
 * Where each item is shown (decision 4): in the cell kept for it, when that is on the grid and no
 * other item took it first; else in the first free cell. Items are placed in `names`' order, those
 * with a cell kept before the rest.
 */
export function arrange(names: readonly string[], kept: Arrangement, grid: Grid): Map<string, Cell> {
  const placed = new Map<string, Cell>();
  const taken = new Set<string>();
  for (const name of names) {
    const cell = kept[name];
    if (!cell || !onGrid(cell, grid) || taken.has(keyOf(cell))) continue;
    placed.set(name, cell);
    taken.add(keyOf(cell));
  }
  for (const name of names) {
    if (placed.has(name)) continue;
    const cell = firstFree(grid, taken);
    placed.set(name, cell);
    taken.add(keyOf(cell));
  }
  // In `names`' order, as they are listed.
  return new Map(names.flatMap((name) => {
    const cell = placed.get(name);
    return cell ? [[name, cell] as const] : [];
  }));
}

/**
 * The cells kept once the desktop shows `shown`: each item shown where it is, except one whose kept
 * cell is off this grid (a display too small for it), which keeps it; an item gone forgets its own.
 */
export function remembered(shown: ReadonlyMap<string, Cell>, kept: Arrangement, grid: Grid): Arrangement {
  const next: Record<string, Cell> = {};
  for (const [name, cell] of shown) {
    const before = kept[name];
    next[name] = before && !onGrid(before, grid) ? before : cell;
  }
  return next;
}

/** Whether two arrangements keep the same cells. */
export function sameArrangement(a: Arrangement, b: Arrangement): boolean {
  const names = Object.keys(a);
  return (
    names.length === Object.keys(b).length &&
    names.every((name) => b[name] && b[name].column === a[name].column && b[name].row === a[name].row)
  );
}

/**
 * Icons dragged on the desktop and let go (decision 4): the one gripped goes to the cell nearest
 * where its picture was let go, the others keeping their places around it; each that would land
 * off the grid or on another icon takes the free cell nearest. `drop` is the grip's point when let
 * go, `grip` where in its cell it was gripped (from the cell's top left). The new cells of the
 * dragged, by name. With `originalsStay` (their copies, Option held) the cells they are in stay
 * taken.
 */
export function movedTo(
  dragged: readonly string[],
  gripped: string,
  drop: Point,
  grip: Point,
  shown: ReadonlyMap<string, Cell>,
  width: number,
  grid: Grid,
  originalsStay = false,
): Map<string, Cell> {
  const from = shown.get(gripped);
  const moved = new Map<string, Cell>();
  if (!from) return moved;
  const to = cellNear(
    { x: drop.x - grip.x + DESKTOP_CELL.width / 2, y: drop.y - grip.y + DESKTOP_CELL.height / 2 },
    width,
    grid,
  );
  const taken = new Set<string>();
  for (const [name, cell] of shown) if (originalsStay || !dragged.includes(name)) taken.add(keyOf(cell));
  const order = [gripped, ...dragged.filter((name) => name !== gripped)];
  for (const name of order) {
    const cell = shown.get(name);
    if (!cell) continue;
    const wanted = { column: cell.column + to.column - from.column, row: cell.row + to.row - from.row };
    const placed = onGrid(wanted, grid) && !taken.has(keyOf(wanted)) ? wanted : freeNear(wanted, grid, taken);
    moved.set(name, placed);
    taken.add(keyOf(placed));
  }
  return moved;
}

/**
 * Items new to the desktop, dropped on it at `drop` (decision 4): the first in the cell there, or
 * the free one nearest it, the others in the free cells nearest that one.
 */
export function droppedAt(
  names: readonly string[],
  drop: Point,
  shown: ReadonlyMap<string, Cell>,
  width: number,
  grid: Grid,
): Map<string, Cell> {
  const taken = new Set<string>();
  for (const [name, cell] of shown) if (!names.includes(name)) taken.add(keyOf(cell));
  const at = cellNear(drop, width, grid);
  const placed = new Map<string, Cell>();
  for (const name of names) {
    const cell = taken.has(keyOf(at)) ? freeNear(at, grid, taken) : at;
    placed.set(name, cell);
    taken.add(keyOf(cell));
  }
  return placed;
}

/** The item whose icon is at `point`, if any. */
export function itemAt(point: Point, shown: ReadonlyMap<string, Cell>, width: number): string | null {
  for (const [name, cell] of shown) {
    const rect = iconRect(cell, width);
    if (point.x >= rect.x && point.x < rect.x + rect.width && point.y >= rect.y && point.y < rect.y + rect.height) {
      return name;
    }
  }
  return null;
}

/** The items whose icons a band from `from` to `to` touches. */
export function inBand(from: Point, to: Point, shown: ReadonlyMap<string, Cell>, width: number): string[] {
  const left = Math.min(from.x, to.x);
  const right = Math.max(from.x, to.x);
  const top = Math.min(from.y, to.y);
  const bottom = Math.max(from.y, to.y);
  const touched: string[] = [];
  for (const [name, cell] of shown) {
    const rect = iconRect(cell, width);
    if (rect.x < right && rect.x + rect.width > left && rect.y < bottom && rect.y + rect.height > top) touched.push(name);
  }
  return touched;
}

/** The selection, in the grid's order (by column from the right, then by row). */
export function inGridOrder(names: Iterable<string>, shown: ReadonlyMap<string, Cell>): string[] {
  return [...names]
    .filter((name) => shown.has(name))
    .sort((a, b) => {
      const ca = shown.get(a) ?? { column: 0, row: 0 };
      const cb = shown.get(b) ?? { column: 0, row: 0 };
      return ca.column - cb.column || ca.row - cb.row;
    });
}

/**
 * A press on an icon (decision 5): with Shift or Super it is added to the selection, or taken out
 * of it; without, it is the selection, unless it is already in it (the selection is dragged
 * together, and the release chooses it alone: `released`).
 */
export function pressed(selection: readonly string[], name: string, adds: boolean): string[] {
  if (adds) return selection.includes(name) ? selection.filter((other) => other !== name) : [...selection, name];
  return selection.includes(name) ? [...selection] : [name];
}

/** A press on a selected icon released without a drag, nor Shift or Super: it alone is selected. */
export function released(name: string): string[] {
  return [name];
}

/** A band drawn from the background (decision 5): what it touches, with Shift or Super what it
    touches changed in the selection it began with. */
export function banded(base: readonly string[], touched: readonly string[], adds: boolean): string[] {
  if (!adds) return [...touched];
  return [...base.filter((name) => !touched.includes(name)), ...touched.filter((name) => !base.includes(name))];
}

export type Direction = "up" | "down" | "left" | "right";

/**
 * The icon an arrow goes to from `from` (decision 5): the nearest in that direction, first along it
 * and then across it, as a Mac's desktop goes from icon to icon; null when none is that way.
 */
export function neighbour(from: string, direction: Direction, shown: ReadonlyMap<string, Cell>): string | null {
  const at = shown.get(from);
  if (!at) return null;
  let best: string | null = null;
  let score = Infinity;
  for (const [name, cell] of shown) {
    if (name === from) continue;
    // Along: how far that way (columns count from the right, so right is fewer of them).
    const along =
      direction === "down"
        ? cell.row - at.row
        : direction === "up"
          ? at.row - cell.row
          : direction === "right"
            ? at.column - cell.column
            : cell.column - at.column;
    if (along <= 0) continue;
    const across = direction === "up" || direction === "down" ? Math.abs(cell.column - at.column) : Math.abs(cell.row - at.row);
    const value = across * 1000 + along;
    if (value < score) {
      best = name;
      score = value;
    }
  }
  return best;
}

/** An arrow pressed (decision 5): the selection moved to the next icon that way, or with Shift that
    icon added; with nothing selected, the first icon. `anchor` is the icon the arrows go from. */
export function arrowed(
  selection: readonly string[],
  anchor: string | null,
  direction: Direction,
  extend: boolean,
  shown: ReadonlyMap<string, Cell>,
): { selection: string[]; anchor: string | null } {
  const from = anchor && selection.includes(anchor) ? anchor : (inGridOrder(selection, shown)[0] ?? null);
  if (!from) {
    const first = inGridOrder(shown.keys(), shown)[0] ?? null;
    return { selection: first ? [first] : [], anchor: first };
  }
  const next = neighbour(from, direction, shown);
  if (!next) return { selection: [...selection], anchor: from };
  return { selection: extend ? [...selection.filter((name) => name !== next), next] : [next], anchor: next };
}

/** What a kind of item is called (its accessible value) and drawn as. */
export type DesktopKind = "folder" | "image" | "audio" | "video" | "pdf" | "text" | "archive" | "document";

/** An item's kind, from its media type. */
export function kindOf(item: Pick<DesktopItem, "folder" | "type">): DesktopKind {
  if (item.folder) return "folder";
  const type = item.type.toLowerCase();
  const [major] = type.split("/");
  if (major === "image") return "image";
  if (major === "audio") return "audio";
  if (major === "video") return "video";
  if (type === "application/pdf") return "pdf";
  if (major === "text") return "text";
  if (/zip|tar|gzip|compressed|x-7z|rar|xz|bzip/.test(type)) return "archive";
  return "document";
}

/** Whether an application that declares `opens` (its entry's MimeType) opens files of `type`: the
    type itself, or its `type/*`, as the session matches them (docs/adr/0035-drag-and-drop.mdx). */
export function opensType(opens: readonly string[] | undefined, type: string): boolean {
  const wanted = type.toLowerCase().split(";")[0].trim();
  if (!wanted || !opens) return false;
  const major = wanted.split("/")[0];
  return opens.some((declared) => {
    const one = declared.toLowerCase().split(";")[0].trim();
    return one === wanted || one === `${major}/*`;
  });
}

/** The URL that opens Dropsea at a folder (docs/adr/0028-file-access.mdx). */
export function filesUrl(folderUri: string): string {
  return `dropsea://files?uri=${encodeURIComponent(folderUri)}`;
}

/**
 * What opening the selection does (decision 5): each folder opens in Dropsea at it; the files go to
 * the first application (in the registry's order) whose entry declares their type, together by
 * application; a file no application opens shows Dropsea at the desktop's folder, once.
 */
export type Opening = {
  urls: string[];
  files: { application: string; uris: string[] }[];
};
export function openingOf(
  items: readonly DesktopItem[],
  applications: readonly { id: string; opens?: readonly string[] }[],
  desktopUri: string,
): Opening {
  const urls: string[] = [];
  const byApplication = new Map<string, string[]>();
  let unopened = false;
  for (const item of items) {
    if (item.folder) {
      urls.push(filesUrl(item.uri));
      continue;
    }
    const application = applications.find((app) => opensType(app.opens, item.type));
    if (!application) {
      unopened = true;
      continue;
    }
    byApplication.set(application.id, [...(byApplication.get(application.id) ?? []), item.uri]);
  }
  if (unopened) urls.push(filesUrl(desktopUri));
  return { urls, files: [...byApplication].map(([application, uris]) => ({ application, uris })) };
}

/** A URI without its trailing slash (but the root's), with its escapes as they are. */
function trimmed(uri: string): string {
  return uri.length > "file:///".length ? uri.replace(/\/+$/, "") : uri;
}
/** Two `file://` URIs naming the same path, whatever their escapes. */
function decoded(uri: string): string {
  try {
    return decodeURIComponent(trimmed(uri));
  } catch {
    return trimmed(uri);
  }
}
export function sameUri(a: string, b: string): boolean {
  return decoded(a) === decoded(b);
}
/** Whether `uri` is `folder` or inside it, by whole names. */
export function withinUri(uri: string, folder: string): boolean {
  const item = decoded(uri);
  const root = decoded(folder);
  return item === root || item.startsWith(root.endsWith("/") ? root : `${root}/`);
}
/** The folder an item is in. */
export function parentOf(uri: string): string {
  const written = trimmed(uri);
  const slash = written.lastIndexOf("/");
  return slash < "file:///".length ? "file:///" : written.slice(0, slash);
}

/**
 * What files dropped into a folder do (decision 7, with ADR 0035's decision 13): go there; nothing,
 * dropped into one of themselves or inside one; nothing for a move holding one of the platform's
 * places (the home, its standard folders, a volume); nothing for those already there (a move to
 * where they are), the rest going.
 */
export type FolderDrop =
  | { kind: "go"; uris: string[] }
  | { kind: "into-itself" }
  | { kind: "place"; uri: string }
  | { kind: "nothing" };
export function folderDrop(
  uris: readonly string[],
  folder: string,
  action: "copy" | "move",
  places: readonly string[],
): FolderDrop {
  const files = [...new Set(uris.filter((uri) => uri.toLowerCase().startsWith("file:")))];
  if (files.some((uri) => withinUri(folder, uri))) return { kind: "into-itself" };
  if (action === "move") {
    const place = files.find((uri) => places.some((each) => sameUri(each, uri)));
    if (place) return { kind: "place", uri: place };
  }
  const going = action === "move" ? files.filter((uri) => !sameUri(parentOf(uri), folder)) : files;
  return going.length > 0 ? { kind: "go", uris: going } : { kind: "nothing" };
}

/** The arrangement as `desktop.json` keeps it: each name's [column, row]. */
export function readArrangement(text: string): Arrangement {
  try {
    const parsed: unknown = JSON.parse(text);
    const cells = typeof parsed === "object" && parsed !== null ? Reflect.get(parsed, "cells") : null;
    if (typeof cells !== "object" || cells === null) return {};
    const kept: Record<string, Cell> = {};
    for (const [name, value] of Object.entries(cells)) {
      if (
        Array.isArray(value) &&
        value.length === 2 &&
        value.every((n) => typeof n === "number" && Number.isInteger(n) && n >= 0)
      ) {
        kept[name] = { column: value[0], row: value[1] };
      }
    }
    return kept;
  } catch {
    return {};
  }
}
export function writeArrangement(kept: Arrangement): string {
  const cells: Record<string, [number, number]> = {};
  for (const name of Object.keys(kept).sort()) cells[name] = [kept[name].column, kept[name].row];
  return `${JSON.stringify({ cells }, null, 2)}\n`;
}

/** Two presses this close together, on one icon, are a double press (a Mac's default). */
export const DOUBLE_PRESS_MS = 500;
/** The press a double press's second follows: on which icon, and when (its own time). */
export type LastPress = { name: string | null; at: number };
export const NO_PRESS: LastPress = { name: null, at: 0 };
/** Whether a press on `name` at `at` is the second of a double press. */
export function isSecondPress(last: LastPress, name: string, at: number): boolean {
  return last.name === name && at >= last.at && at - last.at < DOUBLE_PRESS_MS;
}
/**
 * When a press happened, by its own clock: the time the platform carries (`nativeEvent.timestamp`),
 * else the time the event was made (`timeStamp`). Never when JavaScript got to it.
 */
export function pressTime(event: { timeStamp: number; nativeEvent: object }): number {
  const timestamp: unknown = Reflect.get(event.nativeEvent, "timestamp");
  return typeof timestamp === "number" && Number.isFinite(timestamp) && timestamp > 0 ? timestamp : event.timeStamp;
}

/**
 * The name Finder gives a copy made in the folder its original is in (Option held as the desktop's
 * own icons are let go on it): `notas copy.txt`, then `notas copy 2.txt`, and so on, `word` the
 * session's language's ("copy", "copia"); a copy of a copy is numbered from its original's name.
 * A folder's whole name is its stem, as is a file's whose only dot begins it.
 */
export function copyName(name: string, folder: boolean, taken: ReadonlySet<string>, word: string): string {
  const dot = folder ? -1 : name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const extension = dot > 0 ? name.slice(dot) : "";
  // A copy's own name (`notas copy`, `notas copy 2`) is its original's for the next.
  const marker = ` ${word}`;
  const at = stem.lastIndexOf(marker);
  const rest = at > 0 ? stem.slice(at + marker.length) : null;
  const root = rest !== null && (rest === "" || /^ \d+$/.test(rest)) ? stem.slice(0, at) : stem;
  for (let number = 1; ; number++) {
    const candidate = `${root} ${word}${number > 1 ? ` ${number}` : ""}${extension}`;
    if (!taken.has(candidate)) return candidate;
  }
}
