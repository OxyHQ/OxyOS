import { test } from "node:test";
import assert from "node:assert/strict";
import {
  back,
  bandSelect,
  boxBetween,
  canDropInto,
  canGoBack,
  canGoForward,
  clickSelect,
  compareNames,
  currentLocation,
  focusEnd,
  formatDate,
  formatSize,
  forward,
  freeName,
  go,
  hasThumbnail,
  historyAt,
  isWithin,
  keepShown,
  kindOf,
  moveFocus,
  nameSelection,
  NO_SELECTION,
  parentOf,
  pathParts,
  replace,
  rootOf,
  selectAll,
  sortBy,
  sortEntries,
  typeSelect,
  validName,
  visibleEntries,
  type Box,
  type FileEntry,
  type Places,
} from "../src/model.ts";
import { MemoryFileSystem } from "../src/memory.ts";
import { stringsFor } from "../src/strings.ts";

function entry(name: string, extra: Partial<FileEntry> = {}): FileEntry {
  return {
    name,
    path: `/home/ada/${name}`,
    folder: false,
    link: false,
    size: 0,
    modified: 0,
    hidden: name.startsWith("."),
    ...extra,
  };
}

const PLACES: Places = {
  home: "/home/ada",
  favourites: [{ key: "documents", path: "/home/ada/Documents" }],
  volumes: [{ name: "USB Stick", path: "/media/ada/USB Stick" }],
  trash: 0,
};

test("names sort as Finder sorts them: case and accents aside, numbers by value", () => {
  const names = [
    "report 10.pdf",
    "Report 9.pdf",
    "apple",
    "Éclair",
    "eclair",
    "file02",
    "file1",
    "Zebra",
    "ábaco",
  ];
  assert.deepEqual(names.slice().sort(compareNames), [
    "ábaco",
    "apple",
    "eclair",
    "Éclair",
    "file1",
    "file02",
    "Report 9.pdf",
    "report 10.pdf",
    "Zebra",
  ]);
  // The order is total: equal names apart from case still have one order.
  assert.ok(compareNames("A", "a") !== 0);
  assert.equal(compareNames("same", "same"), 0);
});

test("items sort by each column, ties by name, and a header pressed again reverses", () => {
  const items = [
    entry("b.txt", { size: 10, modified: 3 }),
    entry("Pictures", { folder: true, modified: 1 }),
    entry("a.png", { size: 300, modified: 2 }),
    entry("c.txt", { size: 10, modified: 3 }),
  ];
  const names = (column: "name" | "modified" | "size" | "kind", direction: "ascending" | "descending") =>
    sortEntries(items, { column, direction }).map((e) => e.name);
  assert.deepEqual(names("name", "ascending"), ["a.png", "b.txt", "c.txt", "Pictures"]);
  assert.deepEqual(names("name", "descending"), ["Pictures", "c.txt", "b.txt", "a.png"]);
  assert.deepEqual(names("modified", "descending"), ["c.txt", "b.txt", "a.png", "Pictures"]);
  // A folder has no size: it comes first by size; equal sizes by name.
  assert.deepEqual(names("size", "ascending"), ["Pictures", "b.txt", "c.txt", "a.png"]);
  assert.deepEqual(names("kind", "ascending"), ["Pictures", "a.png", "b.txt", "c.txt"]);
  // Sorting never changes what it was given.
  assert.equal(items[0].name, "b.txt");

  assert.deepEqual(sortBy({ column: "name", direction: "ascending" }, "name"), {
    column: "name",
    direction: "descending",
  });
  assert.deepEqual(sortBy({ column: "name", direction: "ascending" }, "modified"), {
    column: "modified",
    direction: "descending",
  });
  assert.deepEqual(sortBy({ column: "size", direction: "descending" }, "kind"), {
    column: "kind",
    direction: "ascending",
  });
});

test("hidden items show only when asked for, and the search finds names case and accents aside", () => {
  const items = [entry(".bashrc"), entry("Año nuevo.txt"), entry("notes.md")];
  assert.deepEqual(
    visibleEntries(items, "", false).map((e) => e.name),
    ["Año nuevo.txt", "notes.md"],
  );
  assert.deepEqual(visibleEntries(items, "", true).length, 3);
  assert.deepEqual(
    visibleEntries(items, "ANO", false).map((e) => e.name),
    ["Año nuevo.txt"],
  );
  assert.deepEqual(visibleEntries(items, "  ", false).length, 2);
  assert.deepEqual(visibleEntries(items, "bash", false).length, 0);
});

test("a click selects one, Shift extends from the anchor and Ctrl adds or removes one", () => {
  const order = ["a", "b", "c", "d", "e"];
  let s = clickSelect(NO_SELECTION, order, "b");
  assert.deepEqual(s, { paths: ["b"], anchor: "b", focus: "b" });
  s = clickSelect(s, order, "d", { extend: true });
  assert.deepEqual(s.paths, ["b", "c", "d"]);
  assert.equal(s.anchor, "b");
  // Extending again goes from the same anchor, the other way.
  s = clickSelect(s, order, "a", { extend: true });
  assert.deepEqual(s.paths, ["a", "b"]);
  s = clickSelect(s, order, "e", { toggle: true });
  assert.deepEqual(s.paths, ["a", "b", "e"]);
  s = clickSelect(s, order, "a", { toggle: true });
  assert.deepEqual(s.paths, ["b", "e"]);
  // A click on a selected item keeps the others (it may be dragged with them).
  assert.deepEqual(clickSelect(s, order, "e").paths, ["b", "e"]);
  // A plain click on another selects it alone.
  assert.deepEqual(clickSelect(s, order, "c").paths, ["c"]);
  assert.deepEqual(selectAll(order).paths, order);
  // Items gone from what is shown leave the selection.
  assert.deepEqual(keepShown({ paths: ["b", "x"], anchor: "x", focus: "x" }, order), {
    paths: ["b"],
    anchor: "b",
    focus: "b",
  });
});

test("arrow keys move through a list and a grid, Shift extending, clamped at the ends", () => {
  const order = ["a", "b", "c", "d", "e", "f", "g"];
  let s = moveFocus(NO_SELECTION, order, 1);
  assert.deepEqual(s.paths, ["a"]);
  s = moveFocus(s, order, 1);
  assert.deepEqual(s.paths, ["b"]);
  s = moveFocus(s, order, 1, true);
  s = moveFocus(s, order, 1, true);
  assert.deepEqual(s.paths, ["b", "c", "d"]);
  assert.equal(s.anchor, "b");
  // Down a grid of three columns, and past the last row: the last item.
  s = moveFocus(selectAll(["a"]), order, 3);
  assert.deepEqual(s.paths, ["d"]);
  s = moveFocus(s, order, 3);
  assert.deepEqual(s.paths, ["g"]);
  s = moveFocus(s, order, 3);
  assert.deepEqual(s.paths, ["g"]);
  assert.deepEqual(moveFocus(NO_SELECTION, order, -1).paths, ["g"]);
  assert.deepEqual(focusEnd(moveFocus(NO_SELECTION, order, 1), order, "last").paths, ["g"]);
  assert.deepEqual(focusEnd(NO_SELECTION, order, "first").paths, ["a"]);
  assert.deepEqual(moveFocus(NO_SELECTION, [], 1), NO_SELECTION);
});

test("a rubber band selects what it touches, adding to the selection with a modifier", () => {
  const frames = new Map<string, Box>([
    ["a", { x: 0, y: 0, width: 100, height: 100 }],
    ["b", { x: 100, y: 0, width: 100, height: 100 }],
    ["c", { x: 0, y: 100, width: 100, height: 100 }],
  ]);
  const order = ["a", "b", "c"];
  const band = boxBetween({ x: 150, y: 50 }, { x: 50, y: 150 });
  assert.deepEqual(band, { x: 50, y: 50, width: 100, height: 100 });
  assert.deepEqual(bandSelect(NO_SELECTION, order, frames, band, false).paths, ["a", "b", "c"]);
  const thin = boxBetween({ x: 120, y: 10 }, { x: 180, y: 20 });
  assert.deepEqual(bandSelect({ paths: ["c"], anchor: "c", focus: "c" }, order, frames, thin, false).paths, [
    "b",
  ]);
  assert.deepEqual(bandSelect({ paths: ["c"], anchor: "c", focus: "c" }, order, frames, thin, true).paths, [
    "c",
    "b",
  ]);
  // A band that touches nothing selects nothing.
  assert.deepEqual(
    bandSelect(NO_SELECTION, order, frames, { x: 300, y: 300, width: 5, height: 5 }, false).paths,
    [],
  );
});

test("typing a name selects the first that begins so, or the next after it", () => {
  const shown = [entry("Budget.xlsx"), entry("notes.md"), entry("Notas"), entry("Zebra")];
  assert.equal(typeSelect(shown, "no"), "/home/ada/notes.md");
  assert.equal(typeSelect(shown, "NOTA"), "/home/ada/Notas");
  // Nothing begins with "c": the first name after it.
  assert.equal(typeSelect(shown, "c"), "/home/ada/Notas");
  assert.equal(typeSelect(shown, "zz"), "/home/ada/Zebra");
  assert.equal(typeSelect(shown, ""), null);
  assert.equal(typeSelect([], "a"), null);
});

test("history goes back and forward, and somewhere new forgets what was forward", () => {
  let h = historyAt({ kind: "folder", path: "/home/ada" });
  assert.equal(canGoBack(h), false);
  h = go(h, { kind: "folder", path: "/home/ada/Documents" });
  h = go(h, { kind: "trash" });
  assert.equal(go(h, { kind: "trash" }), h);
  h = back(h);
  assert.deepEqual(currentLocation(h), { kind: "folder", path: "/home/ada/Documents" });
  assert.equal(canGoForward(h), true);
  h = go(h, { kind: "computer" });
  assert.equal(canGoForward(h), false);
  assert.equal(h.entries.length, 3);
  h = back(back(h));
  assert.deepEqual(currentLocation(forward(h)), { kind: "folder", path: "/home/ada/Documents" });
  assert.deepEqual(currentLocation(replace(h, { kind: "trash" })), { kind: "trash" });
  assert.equal(back(historyAt({ kind: "trash" })).index, 0);
});

test("paths: parents, roots and the path bar stay inside the home and the volumes", () => {
  assert.equal(parentOf("/home/ada/Documents"), "/home/ada");
  assert.equal(parentOf("/home/ada/Documents/"), "/home/ada");
  assert.equal(parentOf("/home"), "/");
  assert.equal(isWithin("/home/ada/Documents", "/home/ada"), true);
  assert.equal(isWithin("/home/adam", "/home/ada"), false);
  assert.deepEqual(rootOf("/home/ada/Documents", PLACES), { kind: "home", path: "/home/ada", name: "ada" });
  assert.deepEqual(rootOf("/media/ada/USB Stick/DCIM", PLACES), {
    kind: "volume",
    path: "/media/ada/USB Stick",
    name: "USB Stick",
  });
  // Outside both: none, and the window never goes there.
  assert.equal(rootOf("/etc", PLACES), null);
  assert.equal(rootOf("/home", PLACES), null);
  assert.deepEqual(pathParts("/home/ada/Documents/Taxes 2026", PLACES), [
    { name: "ada", path: "/home/ada" },
    { name: "Documents", path: "/home/ada/Documents" },
    { name: "Taxes 2026", path: "/home/ada/Documents/Taxes 2026" },
  ]);
  assert.deepEqual(
    pathParts("/media/ada/USB Stick/DCIM", PLACES).map((p) => p.name),
    ["USB Stick", "DCIM"],
  );
  assert.deepEqual(pathParts("/etc/passwd", PLACES), []);
});

test("names are refused as the service refuses them, and a new folder's name is free", () => {
  for (const name of ["", "   ", ".", "..", "a/b", "nul\0", "x".repeat(256), "ñ".repeat(128)]) {
    assert.equal(validName(name), false, JSON.stringify(name));
  }
  for (const name of ["notes.md", ".hidden", "x".repeat(255), "ñ".repeat(127), "Año 2026"]) {
    assert.equal(validName(name), true, name);
  }
  assert.equal(freeName("untitled folder", ["Documents"]), "untitled folder");
  assert.equal(freeName("untitled folder", ["Untitled Folder", "untitled folder 2"]), "untitled folder 3");
  assert.deepEqual(nameSelection("report.final.pdf", false), { start: 0, end: 12 });
  assert.deepEqual(nameSelection(".bashrc", false), { start: 0, end: 7 });
  assert.deepEqual(nameSelection("v1.2", true), { start: 0, end: 4 });
});

test("items go only into a folder that is not one of them, inside one, or where they are", () => {
  const into = (path: string) => ({ ...entry(path.slice(path.lastIndexOf("/") + 1)), path, folder: true });
  assert.equal(canDropInto(["/home/ada/a.txt"], into("/home/ada/Documents")), true);
  assert.equal(canDropInto(["/home/ada/Documents"], into("/home/ada/Documents")), false);
  assert.equal(canDropInto(["/home/ada/Documents"], into("/home/ada/Documents/Taxes")), false);
  assert.equal(canDropInto(["/home/ada/Documents/a.txt"], into("/home/ada/Documents")), false);
  assert.equal(canDropInto(["/home/ada/a.txt"], entry("b.txt")), false);
  assert.equal(canDropInto([], into("/home/ada/Documents")), false);
});

test("an item is of the kind its extension says, and pictures the host decodes have thumbnails", () => {
  assert.equal(kindOf(entry("Docs", { folder: true })), "folder");
  assert.equal(kindOf(entry("a.JPG")), "image");
  assert.equal(kindOf(entry("song.flac")), "audio");
  assert.equal(kindOf(entry("main.rs")), "code");
  assert.equal(kindOf(entry(".bashrc")), "file");
  assert.equal(kindOf(entry("archive.tar.gz")), "archive");
  assert.equal(kindOf(entry("README")), "file");
  assert.equal(hasThumbnail(entry("a.png", { size: 2_000 })), true);
  assert.equal(hasThumbnail(entry("a.webp", { size: 2_000 })), false);
  assert.equal(hasThumbnail(entry("a.jpg", { size: 0 })), false);
  assert.equal(hasThumbnail(entry("huge.jpg", { size: 100 * 1024 * 1024 })), false);
});

test("sizes and dates read as Finder writes them", () => {
  const words = stringsFor("en");
  assert.equal(formatSize(1, words.units), "1 byte");
  assert.equal(formatSize(999, words.units), "999 bytes");
  assert.equal(formatSize(1_500, words.units), "1.5 KB");
  assert.equal(formatSize(38_912, words.units), "39 KB");
  assert.equal(formatSize(1_840_000_000, words.units), "1.8 GB");
  const now = new Date(2026, 9, 30, 15, 0).getTime();
  const today = new Date(2026, 9, 30, 9, 5).getTime();
  const yesterday = new Date(2026, 9, 29, 22, 0).getTime();
  assert.match(formatDate(today, now, "en-US", words), /^Today at 9:05/);
  assert.match(formatDate(yesterday, now, "en-US", words), /^Yesterday at 10:00/);
  assert.match(formatDate(new Date(2026, 0, 2, 10, 0).getTime(), now, "en-US", words), /Jan 2, 2026/);
  assert.match(formatDate(today, now, "es-ES", stringsFor("es-ES")), /^Hoy a las 9:05/);
});

test("the memory file system keeps the service rules: scope, no replacing, the Trash", async () => {
  const fs = new MemoryFileSystem("/home/ada", {
    "Documents/": null,
    "Documents/a.txt": { size: 3 },
    "b.txt": { size: 1 },
  });
  await assert.rejects(fs.list("/etc"), (error: { code: string }) => error.code === "refused");
  await assert.rejects(fs.list("/home/ada/../etc"), (error: { code: string }) => error.code === "refused");
  await assert.rejects(
    fs.createFolder("/home/ada", "Documents"),
    (error: { code: string }) => error.code === "exists",
  );
  await assert.rejects(
    fs.rename("/home/ada", "other"),
    (error: { code: string }) => error.code === "refused",
  );
  const moved = await fs.move(["/home/ada/b.txt", "/home/ada/Documents"], "/home/ada/Documents");
  assert.deepEqual(moved, [
    { item: "/home/ada/b.txt", ok: true, to: "/home/ada/Documents/b.txt" },
    { item: "/home/ada/Documents", ok: false, error: "refused" },
  ]);
  let told = 0;
  const stop = await fs.watch({ trash: true }, () => told++);
  const [trashed] = await fs.trash(["/home/ada/Documents/a.txt"]);
  assert.equal(trashed.ok, true);
  assert.equal(told, 1);
  const inTrash = await fs.trashList();
  assert.equal(inTrash[0].original, "/home/ada/Documents/a.txt");
  assert.deepEqual(
    (await fs.list("/home/ada/Documents")).entries.map((e) => e.name),
    ["b.txt"],
  );
  await fs.restore([inTrash[0].id]);
  assert.deepEqual((await fs.list("/home/ada/Documents")).entries.map((e) => e.name).sort(), [
    "a.txt",
    "b.txt",
  ]);
  stop();
  await fs.trash(["/home/ada/Documents/a.txt"]);
  assert.equal(told, 2);
  assert.equal(await fs.emptyTrash(), 1);
});

test("the words are English or Spanish, every key in both", () => {
  const en = stringsFor("en-US");
  const es = stringsFor("es_ES.UTF-8");
  assert.equal(en.title, "Files");
  assert.equal(es.title, "Archivos");
  const keys = (value: object): string[] =>
    Object.entries(value).flatMap(([key, inner]) =>
      inner && typeof inner === "object" ? keys(inner).map((k) => `${key}.${k}`) : [key],
    );
  assert.deepEqual(keys(es).sort(), keys(en).sort());
  assert.equal(es.items(1), "1 elemento");
  assert.equal(en.selected(2, 5), "2 of 5 selected");
});
