import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  SEARCH_CATEGORIES,
  categoriesInOrder,
  folderName,
  folderPath,
  selectedAfterRemoving,
  shown,
  statusLine,
  withExcluded,
  withoutExcluded,
} from "../src/spotlight.ts";
import { CATALOGUES } from "../src/messages.ts";
import type { SearchPreferences } from "../src/model.ts";

const en = CATALOGUES.en.spotlight;
const es = CATALOGUES.es.spotlight;

test("every kind of result has its name, listed alphabetically in the person's language", () => {
  for (const words of [en, es]) {
    assert.deepEqual(Object.keys(words.categories).sort(), [...SEARCH_CATEGORIES].sort());
  }
  const english = categoriesInOrder(en, "en").map((category) => en.categories[category]);
  assert.deepEqual(english.slice(0, 4), ["Applications", "Archives", "Contacts", "Developer"]);
  assert.equal(english.length, SEARCH_CATEGORIES.length);
  const spanish = categoriesInOrder(es, "es").map((category) => es.categories[category]);
  assert.deepEqual(spanish.slice(0, 3), ["Ajustes del Sistema", "Aplicaciones", "Archivos comprimidos"]);
});

test("a folder is shown by its name and its path, whatever its escapes", () => {
  assert.equal(folderPath("file:///home/ana/Mis%20cosas/"), "/home/ana/Mis cosas");
  assert.equal(folderName("file:///home/ana/Mis%20cosas/"), "Mis cosas");
  assert.equal(folderPath("file:///"), "/");
  assert.equal(folderName("file:///"), "/");
  assert.equal(folderPath("file://localhost/srv/data"), "/srv/data");
  assert.equal(folderPath("file:///bad%zzname"), "/bad%zzname");
});

test("a folder is kept out once, removed by its path, and the selection moves to its neighbour", () => {
  const kept = ["file:///home/ana/Private", "file:///home/ana/Mis%20cosas"];
  assert.deepEqual(withExcluded(kept, "file:///home/ana/Private/"), kept);
  assert.deepEqual(withExcluded(kept, "file:///home/ana/Mis cosas"), kept);
  assert.deepEqual(withExcluded(kept, "file:///tmp"), [...kept, "file:///tmp"]);
  assert.deepEqual(withoutExcluded(kept, "file:///home/ana/Private/"), ["file:///home/ana/Mis%20cosas"]);
  assert.equal(selectedAfterRemoving(kept, kept[0]), kept[1]);
  assert.equal(selectedAfterRemoving(kept, kept[1]), kept[0]);
  assert.equal(selectedAfterRemoving([kept[0]], kept[0]), null);
});

test("a kind the preferences do not name is shown, and the index's state is a line of text", () => {
  const preferences = { categories: { image: false }, excluded: [] } as unknown as SearchPreferences;
  assert.equal(shown(preferences, "image"), false);
  assert.equal(shown(preferences, "pdf"), true);
  assert.equal(statusLine({ indexing: true, items: 1, partial: false }, en), "Indexing… 1 item so far");
  assert.equal(statusLine({ indexing: false, items: 1200, partial: true }, en), "1,200 items indexed. Some folders couldn’t be indexed.");
  assert.equal(statusLine({ indexing: false, items: 3, partial: false }, es), "3 ítems indexados");
});

test("how far a crawl went is the items it went through, never a percentage; a rebuild says so", () => {
  // A crawl going on: the items it went through, not those the index held before it.
  assert.equal(
    statusLine({ indexing: true, items: 40, partial: false, rebuilding: false, found: 1234 }, en),
    "Indexing… 1,234 items so far",
  );
  // Indexing with no crawl going on (a change settling): what the index holds.
  assert.equal(statusLine({ indexing: true, items: 40, partial: false, rebuilding: false, found: null }, en), "Indexing… 40 items so far");
  // Rebuilding: the new index's crawl, whatever the old one, still answering, holds.
  assert.equal(
    statusLine({ indexing: true, items: 48210, partial: false, rebuilding: true, found: 1 }, en),
    "Rebuilding the index… 1 item so far",
  );
  assert.equal(
    statusLine({ indexing: true, items: 48210, partial: false, rebuilding: true, found: null }, es),
    "Reconstruyendo el índice… 0 ítems hasta ahora",
  );
  assert.equal(
    statusLine({ indexing: true, items: 10, partial: true, rebuilding: true, found: 2500 }, es),
    "Reconstruyendo el índice… 2500 ítems hasta ahora. Algunas carpetas no se han podido indexar.",
  );
  // Done: what the new index holds.
  assert.equal(statusLine({ indexing: false, items: 3, partial: false, rebuilding: false, found: null }, en), "3 items indexed");
  for (const words of [en, es]) {
    assert.ok(words.rebuild.endsWith("…"), "the button asks before it acts, as its ellipsis says");
    assert.ok(words.rebuildTitle && words.rebuildDescription && words.rebuildConfirm && words.cancel);
  }
});

/** The pages Settings shows, by key, with where each one's title is in the catalogue. */
function settingsPages(): Map<string, string[]> {
  const source = readFileSync(new URL("../src/Settings.tsx", import.meta.url), "utf8");
  const pages = new Map<string, string[]>();
  // A title is `words.x`, or a choice between two (`… ? words.battery : words.energy`), the page
  // known by the first.
  for (const match of source.matchAll(/"?([a-z][a-z-]*)"?:\s*\{\s*title:\s*(?:[^,{}]*\?\s*)?(words|strings)\.([\w.]+)[,\s]/g)) {
    const [, key, root, path] = match;
    pages.set(key, root === "words" ? ["pages", ...path.split(".")] : path.split("."));
  }
  return pages;
}

type Searchable = {
  id: string;
  url: string;
  titles: Record<"en" | "es", string>;
  keywords: Record<"en" | "es", string[]>;
};

test("searchable.json declares every Settings page, titled as its sidebar, with words to find it by", () => {
  const declared: Searchable[] = JSON.parse(readFileSync(new URL("../searchable.json", import.meta.url), "utf8"));
  const pages = settingsPages();
  assert.ok(pages.size >= 18, `found ${pages.size} pages`);
  assert.deepEqual(
    declared.map((entry) => entry.id.replace(/^settings:/, "")).sort(),
    [...pages.keys()].sort(),
  );
  for (const entry of declared) {
    const page = entry.id.replace(/^settings:/, "");
    assert.equal(entry.id, `settings:${page}`);
    assert.equal(entry.url, `oxyos-settings://${page}`);
    for (const language of ["en", "es"] as const) {
      const title = pages.get(page)!.reduce<unknown>((at, key) => (at as Record<string, unknown>)[key], CATALOGUES[language]);
      assert.equal(entry.titles[language], title, `${page} (${language})`);
      const words = entry.keywords[language];
      assert.ok(words.length >= 5, `${page} (${language}) has too few keywords`);
      assert.ok(words.every((word) => word === word.toLowerCase() && word.trim() === word && word.length > 0));
      assert.equal(new Set(words).size, words.length, `${page} (${language}) repeats a keyword`);
    }
  }
});
