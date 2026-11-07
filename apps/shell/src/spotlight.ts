/**
 * Spotlight's model (docs/adr/0051-search.mdx), as a Mac's: the panel's layout, the results in
 * sections (Top Hit first, then each kind in the order of its best result), the selection the
 * arrows move through them, and what opening or revealing a result does. Pure, so it is tested
 * without a renderer.
 */
import type { ShellApplication } from "./model";
import type { SearchCategory, SearchItem, SearchResults } from "./platform";
import type { ShellMessages } from "./messages";

/*
 * The panel's sizes, in logical pixels. Every box is a multiple of 4, so it lands on whole pixels
 * at the scales a computer's display has (1, 1.25, 1.5, 2), as the rest of the shell does.
 */
/** The panel's widest, a Mac's Spotlight's. */
export const SPOTLIGHT_WIDTH = 680;
/** The search field's height, and the panel's alone with no results: a capsule. */
export const FIELD_HEIGHT = 56;
/** The panel's corners: half the field, so the field alone is a capsule. */
export const PANEL_RADIUS = 28;
/** The results' area below the field: its tallest and its least. */
export const RESULTS_HEIGHT = 400;
const LEAST_RESULTS = 160;
/** The list's width beside the preview, and the least room the preview needs beside it. */
export const LIST_WIDTH = 280;
const LEAST_PREVIEW = 280;
/** A section's header, a row, the Top Hit's row (its icon larger), and the list's padding. */
export const HEADER_HEIGHT = 24;
export const ROW_HEIGHT = 28;
export const TOP_HIT_HEIGHT = 44;
export const LIST_PADDING = 8;
/** The most rows a section shows (Top Hit's one aside), as a Mac shows a handful of each kind. */
export const ROWS_PER_SECTION = 5;
/** The most results Spotlight asks the service for. */
export const QUERY_LIMIT = 60;
/** The room kept between the panel and the screen's edges. */
const MARGIN = 24;

const floor4 = (value: number) => Math.max(0, Math.floor(value / 4) * 4);

/** Where the panel is, for a screen `width` × `height` (logical pixels). */
export type SpotlightLayout = {
  left: number;
  /** The field's top: about a fifth of the way down the screen. */
  top: number;
  width: number;
  /** The results' area's height below the field. */
  resultsHeight: number;
  /** Whether the selected result's preview shows beside the list (wide enough for both). */
  preview: boolean;
  /** The list's width: beside the preview, or the panel's. */
  listWidth: number;
};

export function spotlightLayout(width: number, height: number): SpotlightLayout {
  const panel = Math.min(SPOTLIGHT_WIDTH, floor4(width - 2 * MARGIN));
  const left = floor4((width - panel) / 2);
  let top = floor4(height / 5);
  // The results fit below the field; on a short screen they are shorter, then the panel higher.
  let resultsHeight = Math.min(RESULTS_HEIGHT, floor4(height - top - FIELD_HEIGHT - MARGIN));
  if (resultsHeight < LEAST_RESULTS) {
    resultsHeight = Math.min(LEAST_RESULTS, floor4(height - FIELD_HEIGHT - 2 * MARGIN));
    top = floor4(Math.max(MARGIN, height - MARGIN - FIELD_HEIGHT - resultsHeight));
  }
  const preview = panel >= LIST_WIDTH + LEAST_PREVIEW;
  return { left, top, width: panel, resultsHeight, preview, listWidth: preview ? LIST_WIDTH : panel };
}

/** A section: Top Hit, or the results of one kind. */
export type SectionKey = "top-hit" | SearchCategory;
export type SpotlightSection = { key: SectionKey; items: readonly SearchItem[] };

/**
 * The results as Spotlight lists them: the top hit, where the service has one, alone in Top Hit;
 * then the rest grouped by kind, each kind where its best result is in the ranking, at most
 * `perSection` of each.
 */
export function sectionsOf(results: SearchResults, perSection = ROWS_PER_SECTION): SpotlightSection[] {
  const top = results.topHit === null ? undefined : results.items.find((item) => item.id === results.topHit);
  const sections: SpotlightSection[] = top ? [{ key: "top-hit", items: [top] }] : [];
  const byKind = new Map<SearchCategory, SearchItem[]>();
  for (const item of results.items) {
    if (item === top) continue;
    let kind = byKind.get(item.category);
    if (!kind) {
      kind = [];
      byKind.set(item.category, kind);
      sections.push({ key: item.category, items: kind });
    }
    if (kind.length < perSection) kind.push(item);
  }
  return sections;
}

/** Every row, in order across the sections (the headers are no rows). */
export function rowsOf(sections: readonly SpotlightSection[]): SearchItem[] {
  return sections.flatMap((section) => section.items);
}

/**
 * The row selected once the results change: the one selected before if it is still there and the
 * search is the same (the index changed under it); otherwise the first, Top Hit, as a Mac selects
 * it as each letter is typed. Null with no rows.
 */
export function selectedAfter(
  sections: readonly SpotlightSection[],
  before: string | null,
  sameSearch: boolean,
): string | null {
  const rows = rowsOf(sections);
  if (sameSearch && before !== null && rows.some((item) => item.id === before)) return before;
  return rows[0]?.id ?? null;
}

/** What moves the selection: an arrow (a row), or with the applications' ⌘ (Ctrl on OxyOS) a
    section, as ⌘↑ and ⌘↓ do in a Mac's Spotlight. */
export type SpotlightMove = "up" | "down" | "previous-section" | "next-section";

/**
 * Where the selection goes from `selected` (a row's id): ↓ and ↑ go a row, staying at the ends;
 * the next section's first row, or the last row from the last section; the current section's
 * first row, or from there the previous section's.
 */
export function moveSelection(
  sections: readonly SpotlightSection[],
  selected: string | null,
  move: SpotlightMove,
): string | null {
  const rows = rowsOf(sections);
  if (rows.length === 0) return null;
  const index = rows.findIndex((item) => item.id === selected);
  if (index < 0) return rows[0].id;
  switch (move) {
    case "down":
      return rows[Math.min(rows.length - 1, index + 1)].id;
    case "up":
      return rows[Math.max(0, index - 1)].id;
    case "next-section":
    case "previous-section": {
      const at = sections.findIndex((section) => section.items.some((item) => item.id === selected));
      if (move === "next-section") {
        const next = sections[at + 1];
        return next ? next.items[0].id : rows[rows.length - 1].id;
      }
      const first = sections[at].items[0].id;
      if (first !== selected || at === 0) return first;
      return sections[at - 1].items[0].id;
    }
  }
}

/** Each row's place in the list, top and height from the list's top, and the list's height. */
export function rowPlaces(sections: readonly SpotlightSection[]): {
  rows: Map<string, { y: number; height: number }>;
  height: number;
} {
  const rows = new Map<string, { y: number; height: number }>();
  let y = LIST_PADDING;
  for (const section of sections) {
    y += HEADER_HEIGHT;
    const height = section.key === "top-hit" ? TOP_HIT_HEIGHT : ROW_HEIGHT;
    for (const item of section.items) {
      rows.set(item.id, { y, height });
      y += height;
    }
  }
  return { rows, height: y + LIST_PADDING };
}

/** Where the list scrolls to show a row whole, from `offset` in a view `visible` tall: where it
    is, if the row shows; else just far enough, its section's header shown with its first row. */
export function scrolledTo(
  place: { y: number; height: number },
  offset: number,
  visible: number,
  first: boolean,
): number {
  const top = place.y - (first ? HEADER_HEIGHT : 0);
  if (top < offset) return Math.max(0, top - (first ? LIST_PADDING : 0));
  if (place.y + place.height > offset + visible) return place.y + place.height + LIST_PADDING - visible;
  return offset;
}

/** The rules of the desktop's files Spotlight follows (desktop.ts): which application opens a
    type, a file's folder, and Dropsea's URL for a folder. */
export type FileRules = {
  opensType: (opens: readonly string[] | undefined, type: string) => boolean;
  parentOf: (uri: string) => string;
  filesUrl: (folderUri: string) => string;
};

/** What opening a result does. */
export type SpotlightOpening =
  | { kind: "application"; id: string }
  | { kind: "files"; application: string; uris: string[] }
  | { kind: "url"; url: string };

const isFile = (item: SearchItem) => typeof item.uri === "string" && item.uri.startsWith("file://");

/**
 * Opening a result (docs/adr/0051-search.mdx), as a Mac's Return: an application launched or
 * brought forward; a folder shown in Dropsea; a file opened with the application that opens it
 * (the service's, else the first that declares its type, as the desktop opens files, ADR 0047),
 * or, when none does, shown in Dropsea; a settings page and anything else with a URL opened at it;
 * an application's item without one, its application. Null when there is nothing to open.
 */
export function openingOf(
  item: SearchItem,
  applications: readonly Pick<ShellApplication, "id" | "opens">[],
  rules: FileRules,
): SpotlightOpening | null {
  if (item.category === "application") {
    const id = applicationOf(item);
    return id ? { kind: "application", id } : null;
  }
  if (item.category === "settings") {
    const page = item.id.startsWith("settings:") ? item.id.slice("settings:".length) : "";
    const url = item.url ?? (page ? `oxyos-settings://${page}` : "");
    return url ? { kind: "url", url } : null;
  }
  if (isFile(item)) {
    const uri = item.uri!;
    if (item.category === "folder") return { kind: "url", url: rules.filesUrl(uri) };
    const told = item.application ? applications.find((app) => app.id === item.application) : undefined;
    const opener = told ?? applications.find((app) => rules.opensType(app.opens, item.contentType ?? ""));
    if (opener) return { kind: "files", application: opener.id, uris: [uri] };
    return { kind: "url", url: revealUrl(item, rules)! };
  }
  if (item.url) return { kind: "url", url: item.url };
  if (item.application) return { kind: "application", id: item.application };
  return null;
}

/** An application result's application: the one the service names, else its id's
    (`application:<id>`). Null for any other result. */
export function applicationOf(item: SearchItem): string | null {
  if (item.category !== "application") return null;
  if (item.application) return item.application;
  return item.id.startsWith("application:") ? item.id.slice("application:".length) || null : null;
}

/** Showing a file or a folder where it is, as a Mac's ⌘Return shows it in Finder: Dropsea at its
    folder, it selected. Null for anything else. */
export function revealUrl(item: SearchItem, rules: Pick<FileRules, "parentOf" | "filesUrl">): string | null {
  if (!isFile(item)) return null;
  const uri = item.uri!;
  return `${rules.filesUrl(rules.parentOf(uri))}&select=${encodeURIComponent(uri)}`;
}

/** Whether a result can be dragged out (a file or a folder, as the desktop's icons are). */
export function draggable(item: SearchItem): boolean {
  return isFile(item);
}

/** A section's title, said as its header. */
export function sectionTitle(key: SectionKey, messages: ShellMessages): string {
  return key === "top-hit" ? messages.spotlight.topHit : messages.spotlight.sections[key];
}

/** What a result is, as its preview's Kind says it. */
export function kindName(item: SearchItem, messages: ShellMessages): string {
  return messages.spotlight.kinds[item.category];
}

/** What a row says in grey after its title: where it is; else, in Top Hit, whose header does not
    say it, what it is (an application nothing more, as a Mac's row of one). Elsewhere its
    section's header says what it is. */
export function rowDetail(item: SearchItem, messages: ShellMessages, top = false): string {
  if (item.location) return item.location;
  return top && item.category !== "application" ? kindName(item, messages) : "";
}

/** A row's name for assistive technologies: "Title, kind, location". */
export function rowLabel(item: SearchItem, messages: ShellMessages): string {
  return [item.title, kindName(item, messages), item.location].filter(Boolean).join(", ");
}

const normalize = (value: string) =>
  value
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .trim();

/**
 * Without the system's search (the browser preview), Spotlight finds the snapshot's applications
 * by name: a name that begins with the text first, then one with a word that does, then one that
 * holds it; alphabetical within each. The first is the top hit when it begins with the text or a
 * word of it does.
 */
export function localResults(applications: readonly ShellApplication[], text: string): SearchResults {
  const wanted = normalize(text);
  if (!wanted) return { items: [], topHit: null, indexing: false };
  const scored: { app: ShellApplication; score: number }[] = [];
  for (const app of applications) {
    const name = normalize(app.name);
    const score = name.startsWith(wanted)
      ? 0
      : name.split(/[\s\-_.]+/).some((word) => word.startsWith(wanted))
        ? 1
        : name.includes(wanted)
          ? 2
          : -1;
    if (score >= 0) scored.push({ app, score });
  }
  scored.sort((a, b) => a.score - b.score || a.app.name.localeCompare(b.app.name));
  const items: SearchItem[] = scored.map(({ app }) => ({
    id: `application:${app.id}`,
    category: "application",
    title: app.name,
    application: app.id,
  }));
  return { items, topHit: scored.length > 0 && scored[0].score <= 1 ? items[0].id : null, indexing: false };
}
