import { test } from "node:test";
import assert from "node:assert/strict";
import {
  FIELD_HEIGHT,
  HEADER_HEIGHT,
  LIST_PADDING,
  LIST_WIDTH,
  PANEL_RADIUS,
  RESULTS_HEIGHT,
  ROW_HEIGHT,
  ROWS_PER_SECTION,
  TOP_HIT_HEIGHT,
  draggable,
  localResults,
  moveSelection,
  openingOf,
  revealUrl,
  rowDetail,
  rowLabel,
  rowPlaces,
  rowsOf,
  scrolledTo,
  sectionsOf,
  sectionTitle,
  selectedAfter,
  spotlightLayout,
} from "../src/spotlight.ts";
import { filesUrl, opensType, parentOf } from "../src/desktop.ts";
import { shellMessages } from "../src/messages.ts";
import type { SearchItem, SearchResults } from "../src/platform.tsx";
import type { ShellApplication } from "../src/model.ts";

const rules = { filesUrl, opensType, parentOf };
const en = shellMessages("en-US");
const es = shellMessages("es-ES");

const app = (id: string, name: string, opens?: string[]): ShellApplication => ({ id, name, icon: { uri: "" }, opens });
const item = (id: string, category: SearchItem["category"], extra: Partial<SearchItem> = {}): SearchItem => ({
  id,
  category,
  title: id,
  ...extra,
});
const results = (items: SearchItem[], topHit: string | null = null): SearchResults => ({ items, topHit, indexing: false });

test("every size lands on whole pixels at 1 and 1.25", () => {
  for (const size of [FIELD_HEIGHT, PANEL_RADIUS, RESULTS_HEIGHT, LIST_WIDTH, HEADER_HEIGHT, ROW_HEIGHT, TOP_HIT_HEIGHT, LIST_PADDING]) {
    assert.ok(Number.isInteger(size * 1.25), `${size}`);
  }
  for (const [width, height] of [
    [1280, 800],
    [1536, 960],
    [1366, 768],
    [1920, 1080],
    [800, 600],
    [640, 420],
    [360, 300],
  ]) {
    const layout = spotlightLayout(width, height);
    for (const value of [layout.left, layout.top, layout.width, layout.resultsHeight, layout.listWidth]) {
      assert.ok(Number.isInteger(value * 1.25), `${width}×${height}: ${value}`);
    }
  }
});

test("the panel is a Mac's: 680 wide, centred, its field a fifth of the way down", () => {
  // The ThinkPad's 1600 × 1000 at 1.25: 1280 × 800 logical pixels.
  const layout = spotlightLayout(1280, 800);
  assert.deepEqual(layout, { left: 300, top: 160, width: 680, resultsHeight: 400, preview: true, listWidth: 280 });
  // Narrower on a small screen, the list alone once the preview has no room beside it.
  const small = spotlightLayout(560, 420);
  assert.equal(small.width, 512);
  assert.equal(small.preview, false);
  assert.equal(small.listWidth, 512);
  // Short: the results shorter, then the panel higher, always on the screen.
  for (const [width, height] of [
    [1280, 600],
    [640, 360],
    [400, 300],
  ]) {
    const short = spotlightLayout(width, height);
    assert.ok(short.top + FIELD_HEIGHT + short.resultsHeight <= height, `${width}×${height}`);
    assert.ok(short.left >= 0 && short.left + short.width <= width);
  }
});

test("results are sectioned: Top Hit, then each kind where its best result ranks", () => {
  const items = [
    item("application:mention", "application"),
    item("file:///home/a/notes.pdf", "pdf"),
    item("settings:wifi", "settings"),
    item("file:///home/a/plan.txt", "document"),
    item("application:allo", "application"),
    item("file:///home/a/old.pdf", "pdf"),
  ];
  const sections = sectionsOf(results(items, "application:mention"));
  assert.deepEqual(
    sections.map((section) => [section.key, section.items.map((one) => one.id)]),
    [
      ["top-hit", ["application:mention"]],
      ["pdf", ["file:///home/a/notes.pdf", "file:///home/a/old.pdf"]],
      ["settings", ["settings:wifi"]],
      ["document", ["file:///home/a/plan.txt"]],
      ["application", ["application:allo"]],
    ],
  );
  // No top hit: no Top Hit section, the best result first in its kind.
  assert.equal(sectionsOf(results(items, null))[0].key, "application");
  // A top hit the answer does not hold is none.
  assert.equal(sectionsOf(results(items, "gone"))[0].key, "application");
  // At most a handful of each kind.
  const many = Array.from({ length: 9 }, (_, n) => item(`file:///${n}.png`, "image"));
  assert.equal(sectionsOf(results(many))[0].items.length, ROWS_PER_SECTION);
  assert.equal(sectionTitle("top-hit", en), "Top Hit");
  assert.equal(sectionTitle("settings", es), "Ajustes del Sistema");
});

test("the selection is Top Hit as the text changes, and stays on its row as the index does", () => {
  const sections = sectionsOf(results([item("a", "application"), item("b", "document"), item("c", "document")], "a"));
  assert.equal(selectedAfter(sections, null, false), "a");
  assert.equal(selectedAfter(sections, "c", true), "c");
  assert.equal(selectedAfter(sections, "c", false), "a");
  assert.equal(selectedAfter(sections, "gone", true), "a");
  assert.equal(selectedAfter([], "a", true), null);
});

test("the arrows go through the rows across sections, never onto a header; ⌘ jumps sections", () => {
  const sections = sectionsOf(
    results([item("top", "application"), item("d1", "document"), item("d2", "document"), item("f1", "folder")], "top"),
  );
  assert.deepEqual(rowsOf(sections).map((one) => one.id), ["top", "d1", "d2", "f1"]);
  assert.equal(moveSelection(sections, "top", "down"), "d1");
  assert.equal(moveSelection(sections, "d2", "down"), "f1");
  assert.equal(moveSelection(sections, "f1", "down"), "f1");
  assert.equal(moveSelection(sections, "top", "up"), "top");
  assert.equal(moveSelection(sections, "f1", "up"), "d2");
  assert.equal(moveSelection(sections, "top", "next-section"), "d1");
  assert.equal(moveSelection(sections, "d1", "next-section"), "f1");
  assert.equal(moveSelection(sections, "f1", "next-section"), "f1");
  assert.equal(moveSelection(sections, "d2", "previous-section"), "d1");
  assert.equal(moveSelection(sections, "d1", "previous-section"), "top");
  assert.equal(moveSelection(sections, "top", "previous-section"), "top");
  assert.equal(moveSelection(sections, null, "down"), "top");
  assert.equal(moveSelection([], null, "down"), null);
});

test("each row has its place, and the list scrolls just far enough to show it", () => {
  const sections = sectionsOf(results([item("top", "application"), item("d1", "document"), item("d2", "document")], "top"));
  const { rows, height } = rowPlaces(sections);
  assert.deepEqual(rows.get("top"), { y: LIST_PADDING + HEADER_HEIGHT, height: TOP_HIT_HEIGHT });
  assert.deepEqual(rows.get("d1"), { y: LIST_PADDING + 2 * HEADER_HEIGHT + TOP_HIT_HEIGHT, height: ROW_HEIGHT });
  assert.equal(height, 2 * LIST_PADDING + 2 * HEADER_HEIGHT + TOP_HIT_HEIGHT + 2 * ROW_HEIGHT);
  // Shown: it stays. Below: just enough. Above: the row's header with it, as its section's first.
  assert.equal(scrolledTo({ y: 40, height: 28 }, 0, 400, false), 0);
  assert.equal(scrolledTo({ y: 500, height: 28 }, 0, 400, false), 500 + 28 + LIST_PADDING - 400);
  assert.equal(scrolledTo({ y: 100, height: 28 }, 200, 400, true), 100 - HEADER_HEIGHT - LIST_PADDING);
});

test("opening a result does what a Mac's Return does", () => {
  const apps = [app("so.oxy.noted", "Noted", ["text/plain"]), app("so.oxy.dropsea", "Dropsea"), app("so.oxy.photos", "Photos", ["image/*"])];
  assert.deepEqual(openingOf(item("application:so.oxy.noted", "application"), apps, rules), {
    kind: "application",
    id: "so.oxy.noted",
  });
  assert.deepEqual(openingOf(item("settings:wifi", "settings", { url: "oxyos-settings://wifi" }), apps, rules), {
    kind: "url",
    url: "oxyos-settings://wifi",
  });
  assert.deepEqual(openingOf(item("settings:sound", "settings"), apps, rules), { kind: "url", url: "oxyos-settings://sound" });
  const folder = "file:///home/ana/Work%20Stuff";
  assert.deepEqual(openingOf(item(folder, "folder", { uri: folder }), apps, rules), {
    kind: "url",
    url: `dropsea://files?uri=${encodeURIComponent(folder)}`,
  });
  // A file: the application the service names, else the first that opens its type.
  const text = "file:///home/ana/notes.txt";
  assert.deepEqual(openingOf(item(text, "text", { uri: text, contentType: "text/plain" }), apps, rules), {
    kind: "files",
    application: "so.oxy.noted",
    uris: [text],
  });
  const picture = "file:///home/ana/cat.png";
  assert.deepEqual(
    openingOf(item(picture, "image", { uri: picture, contentType: "image/png", application: "so.oxy.dropsea" }), apps, rules),
    { kind: "files", application: "so.oxy.dropsea", uris: [picture] },
  );
  assert.deepEqual(openingOf(item(picture, "image", { uri: picture, contentType: "image/png" }), apps, rules), {
    kind: "files",
    application: "so.oxy.photos",
    uris: [picture],
  });
  // One no application opens is shown in Dropsea.
  const odd = "file:///home/ana/data.bin";
  assert.deepEqual(openingOf(item(odd, "other", { uri: odd, contentType: "application/octet-stream" }), apps, rules), {
    kind: "url",
    url: `dropsea://files?uri=${encodeURIComponent("file:///home/ana")}&select=${encodeURIComponent(odd)}`,
  });
  // An application's item: its URL, else its application.
  assert.deepEqual(openingOf(item("so.oxy.inbox:42", "mail", { url: "inbox://message/42" }), apps, rules), {
    kind: "url",
    url: "inbox://message/42",
  });
  assert.deepEqual(openingOf(item("so.oxy.allo:7", "message", { application: "so.oxy.allo" }), apps, rules), {
    kind: "application",
    id: "so.oxy.allo",
  });
  assert.equal(openingOf(item("x:1", "other"), apps, rules), null);
});

test("revealing shows a file or a folder selected in its folder, and nothing else", () => {
  const file = "file:///home/ana/Documents/plan%20B.txt";
  assert.equal(
    revealUrl(item(file, "text", { uri: file }), rules),
    `dropsea://files?uri=${encodeURIComponent("file:///home/ana/Documents")}&select=${encodeURIComponent(file)}`,
  );
  const folder = "file:///home/ana/Documents/";
  assert.equal(
    revealUrl(item(folder, "folder", { uri: folder }), rules),
    `dropsea://files?uri=${encodeURIComponent("file:///home/ana")}&select=${encodeURIComponent(folder)}`,
  );
  assert.equal(revealUrl(item("application:a", "application"), rules), null);
  assert.equal(draggable(item(file, "text", { uri: file })), true);
  assert.equal(draggable(item("settings:wifi", "settings", { url: "oxyos-settings://wifi" })), false);
});

test("a row says its title, kind and location; in grey, where it is, or in Top Hit what it is", () => {
  const doc = item("file:///home/ana/plan.pdf", "pdf", { title: "plan.pdf", location: "Documents" });
  assert.equal(rowLabel(doc, en), "plan.pdf, PDF document, Documents");
  assert.equal(rowLabel(doc, es), "plan.pdf, Documento PDF, Documents");
  assert.equal(rowDetail(doc, en), "Documents");
  assert.equal(rowDetail(item("settings:wifi", "settings"), en), "");
  assert.equal(rowDetail(item("settings:wifi", "settings"), en, true), "System Settings");
  assert.equal(rowDetail(item("application:a", "application"), en, true), "");
  assert.equal(rowLabel(item("application:a", "application", { title: "Allo" }), en), "Allo, Application");
});

test("without the system's search, the snapshot's applications are found by name", () => {
  const apps = [app("mention", "Mention"), app("fair", "FAIRWallet"), app("oxy", "Oxy Accounts"), app("allo", "Allo"), app("clarity", "Clarity")];
  const found = localResults(apps, "al");
  assert.deepEqual(found.items.map((one) => one.id), ["application:allo", "application:fair"]);
  assert.equal(found.topHit, "application:allo");
  assert.equal(found.items[0].category, "application");
  // A word's beginning ranks before a name that only holds it; accents and case aside.
  assert.deepEqual(localResults(apps, "ACCÓ").items.map((one) => one.id), ["application:oxy"]);
  assert.deepEqual(localResults(apps, "lari").topHit, null);
  assert.deepEqual(localResults(apps, "  ").items, []);
});
