/**
 * Settings › Spotlight's model (docs/adr/0051-search.mdx): the kinds of results in the order a
 * Mac's list shows them, the folders kept out of the index, and what the index's state says. Pure,
 * so it is tested without a renderer.
 */
import type { Messages } from "./messages";
import type { SearchCategory, SearchPreferences, SearchStatus } from "./model";

/** Every kind of result Spotlight shows. */
export const SEARCH_CATEGORIES: readonly SearchCategory[] = [
  "application",
  "settings",
  "folder",
  "document",
  "pdf",
  "text",
  "source",
  "presentation",
  "spreadsheet",
  "image",
  "movie",
  "music",
  "archive",
  "contact",
  "event",
  "mail",
  "message",
  "other",
];

/** The kinds as a Mac's Search results list shows them: by their names, alphabetically in the
    person's language (`localeCompare`: the Bloom host's Hermes has no `Intl.Collator`). */
export function categoriesInOrder(words: Messages["spotlight"], language: string): SearchCategory[] {
  const name = (category: SearchCategory) => words.categories[category] ?? category;
  return [...SEARCH_CATEGORIES].sort((a, b) => name(a).localeCompare(name(b), language, { sensitivity: "base" }));
}

/** A `file://` URI's path, decoded, without its trailing slash (but the root's). */
export function folderPath(uri: string): string {
  const path = uri.replace(/^file:\/\/[^/]*/, "");
  let decoded = path;
  try {
    decoded = decodeURIComponent(path);
  } catch {
    // An escape that is not one: the path as it is written.
  }
  return decoded.length > 1 ? decoded.replace(/\/+$/, "") : decoded || "/";
}

/** A folder's name, as the list shows it: its path's last part, or "/" for the root. */
export function folderName(uri: string): string {
  const path = folderPath(uri);
  return path === "/" ? "/" : (path.split("/").pop() ?? path);
}

/** The folders kept out with `uri` added, once: one already kept out (whatever its escapes or
    trailing slash) is not added again. */
export function withExcluded(excluded: readonly string[], uri: string): string[] {
  const path = folderPath(uri);
  return excluded.some((one) => folderPath(one) === path) ? [...excluded] : [...excluded, uri];
}

/** The folders kept out without `uri`. */
export function withoutExcluded(excluded: readonly string[], uri: string): string[] {
  const path = folderPath(uri);
  return excluded.filter((one) => folderPath(one) !== path);
}

/** The folder selected once `removed` is gone: the one after it, else the one before, else none. */
export function selectedAfterRemoving(excluded: readonly string[], removed: string): string | null {
  const at = excluded.indexOf(removed);
  if (at < 0) return excluded[0] ?? null;
  return excluded[at + 1] ?? excluded[at - 1] ?? null;
}

/** Whether a kind is shown: those the preferences do not name are, as the service shows them. */
export function shown(preferences: SearchPreferences, category: SearchCategory): boolean {
  return preferences.categories[category] !== false;
}

/** The index's state in a line of small text: being built and how far, or how much it holds,
    with a word when some folders could not be read. */
export function statusLine(status: SearchStatus, words: Messages["spotlight"]): string {
  const line = status.indexing ? words.indexing(status.items) : words.indexed(status.items);
  return status.partial ? `${line}. ${words.partial}` : line;
}
