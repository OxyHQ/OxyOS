import { test } from "node:test";
import assert from "node:assert/strict";
import {
  arrange,
  arrowed,
  banded,
  cellNear,
  cellOrigin,
  copyName,
  DESKTOP_CELL,
  DESKTOP_EDGE,
  droppedAt,
  filesUrl,
  firstFree,
  folderDrop,
  freeNear,
  gridOf,
  inBand,
  inGridOrder,
  isSecondPress,
  itemAt,
  NO_PRESS,
  kindOf,
  movedTo,
  neighbour,
  openingOf,
  opensType,
  pressed,
  pressTime,
  readArrangement,
  remembered,
  sameArrangement,
  writeArrangement,
  type Cell,
  type DesktopItem,
} from "../src/desktop.ts";

// An area of 4 columns and 3 rows.
const WIDTH = DESKTOP_EDGE + 4 * DESKTOP_CELL.width + 20;
const HEIGHT = DESKTOP_EDGE + 3 * DESKTOP_CELL.height + 30;
const GRID = gridOf(WIDTH, HEIGHT);
const cell = (column: number, row: number): Cell => ({ column, row });
const middleOf = (at: Cell) => {
  const origin = cellOrigin(at, WIDTH);
  return { x: origin.x + DESKTOP_CELL.width / 2, y: origin.y + DESKTOP_CELL.height / 2 };
};

test("the grid is the cells that fit the area", () => {
  assert.deepEqual(GRID, { columns: 4, rows: 3 });
  assert.deepEqual(gridOf(10, 10), { columns: 1, rows: 1 });
});

test("cells are laid out from the top right, down and then leftward", () => {
  assert.deepEqual(cellOrigin(cell(0, 0), WIDTH), { x: WIDTH - DESKTOP_EDGE - DESKTOP_CELL.width, y: DESKTOP_EDGE });
  assert.deepEqual(cellOrigin(cell(1, 2), WIDTH), {
    x: WIDTH - DESKTOP_EDGE - 2 * DESKTOP_CELL.width,
    y: DESKTOP_EDGE + 2 * DESKTOP_CELL.height,
  });
  assert.deepEqual(firstFree(GRID, new Set()), cell(0, 0));
  assert.deepEqual(firstFree(GRID, new Set(["0,0", "0,1"])), cell(0, 2));
  assert.deepEqual(firstFree(GRID, new Set(["0,0", "0,1", "0,2"])), cell(1, 0));
});

test("a full grid goes on past its last column, so no icon is lost", () => {
  const taken = new Set<string>();
  for (let column = 0; column < 4; column++) for (let row = 0; row < 3; row++) taken.add(`${column},${row}`);
  assert.deepEqual(firstFree(GRID, taken), cell(4, 0));
});

test("the cell nearest a point, kept on the grid", () => {
  assert.deepEqual(cellNear(middleOf(cell(2, 1)), WIDTH, GRID), cell(2, 1));
  assert.deepEqual(cellNear({ x: WIDTH + 500, y: -40 }, WIDTH, GRID), cell(0, 0));
  assert.deepEqual(cellNear({ x: -500, y: HEIGHT + 500 }, WIDTH, GRID), cell(3, 2));
});

test("the free cell nearest", () => {
  assert.deepEqual(freeNear(cell(1, 1), GRID, new Set()), cell(1, 1));
  // A cell is wider than it is tall: the nearest are beside it, the one first in the grid's order
  // (the rightmost) winning a tie.
  assert.deepEqual(freeNear(cell(1, 1), GRID, new Set(["1,1"])), cell(0, 1));
});

test("kept cells are kept; new items take the first free cells", () => {
  const shown = arrange(["a", "b", "c"], { b: cell(2, 2) }, GRID);
  assert.deepEqual(shown.get("b"), cell(2, 2));
  assert.deepEqual(shown.get("a"), cell(0, 0));
  assert.deepEqual(shown.get("c"), cell(0, 1));
  assert.deepEqual([...shown.keys()], ["a", "b", "c"]);
});

test("two kept in one cell: the first keeps it, the other goes to the first free", () => {
  const shown = arrange(["a", "b"], { a: cell(1, 1), b: cell(1, 1) }, GRID);
  assert.deepEqual(shown.get("a"), cell(1, 1));
  assert.deepEqual(shown.get("b"), cell(0, 0));
});

test("a display too small for a kept cell shows the icon in the first free one and does not forget", () => {
  const kept = { far: cell(9, 9), near: cell(1, 0) };
  const small = arrange(["far", "near"], kept, GRID);
  assert.deepEqual(small.get("far"), cell(0, 0));
  const next = remembered(small, kept, GRID);
  assert.deepEqual(next.far, cell(9, 9));
  assert.deepEqual(next.near, cell(1, 0));
  // On a display large enough it is back where it was.
  assert.deepEqual(arrange(["far", "near"], next, { columns: 12, rows: 12 }).get("far"), cell(9, 9));
});

test("an item gone forgets its cell", () => {
  const shown = arrange(["a"], { a: cell(0, 1), gone: cell(2, 2) }, GRID);
  assert.deepEqual(remembered(shown, { a: cell(0, 1), gone: cell(2, 2) }, GRID), { a: cell(0, 1) });
});

test("arrangements compare by their cells", () => {
  assert.ok(sameArrangement({ a: cell(0, 1) }, { a: cell(0, 1) }));
  assert.ok(!sameArrangement({ a: cell(0, 1) }, { a: cell(1, 1) }));
  assert.ok(!sameArrangement({ a: cell(0, 1) }, { a: cell(0, 1), b: cell(0, 0) }));
});

test("icons dragged land on the cell nearest, keeping their arrangement", () => {
  const shown = new Map([
    ["a", cell(0, 0)],
    ["b", cell(0, 1)],
    ["c", cell(1, 0)],
  ]);
  const grip = { x: 30, y: 20 };
  // a and b dragged so a's cell lands on (2, 0): b follows to (2, 1).
  const origin = cellOrigin(cell(2, 0), WIDTH);
  const moved = movedTo(["a", "b"], "a", { x: origin.x + grip.x, y: origin.y + grip.y }, grip, shown, WIDTH, GRID);
  assert.deepEqual(moved.get("a"), cell(2, 0));
  assert.deepEqual(moved.get("b"), cell(2, 1));
});

test("an icon dragged onto another takes the free cell nearest", () => {
  const shown = new Map([
    ["a", cell(0, 0)],
    ["c", cell(1, 0)],
  ]);
  const grip = { x: 10, y: 10 };
  const origin = cellOrigin(cell(1, 0), WIDTH);
  const moved = movedTo(["a"], "a", { x: origin.x + grip.x, y: origin.y + grip.y }, grip, shown, WIDTH, GRID);
  // Its own cell is free again, and as near as any.
  assert.deepEqual(moved.get("a"), cell(0, 0));
});

test("icons dragged past the grid's edge are kept on it", () => {
  const shown = new Map([
    ["a", cell(0, 0)],
    ["b", cell(0, 1)],
  ]);
  const grip = { x: 10, y: 10 };
  const origin = cellOrigin(cell(3, 2), WIDTH);
  const moved = movedTo(["a", "b"], "a", { x: origin.x + grip.x, y: origin.y + grip.y }, grip, shown, WIDTH, GRID);
  assert.deepEqual(moved.get("a"), cell(3, 2));
  assert.deepEqual(moved.get("b"), cell(2, 2));
});

test("items dropped take the cell where they were dropped, or the nearest free", () => {
  const shown = new Map([["a", cell(2, 1)]]);
  assert.deepEqual(droppedAt(["x"], middleOf(cell(1, 2)), shown, WIDTH, GRID).get("x"), cell(1, 2));
  const placed = droppedAt(["x", "y"], middleOf(cell(2, 1)), shown, WIDTH, GRID);
  assert.deepEqual(placed.get("x"), cell(1, 1));
  assert.deepEqual(placed.get("y"), cell(3, 1));
});

test("what a point is on, and what a band touches", () => {
  const shown = new Map([
    ["a", cell(0, 0)],
    ["b", cell(0, 1)],
    ["c", cell(2, 2)],
  ]);
  assert.equal(itemAt({ x: middleOf(cell(0, 1)).x, y: cellOrigin(cell(0, 1), WIDTH).y + 10 }, shown, WIDTH), "b");
  assert.equal(itemAt(middleOf(cell(1, 1)), shown, WIDTH), null);
  const touched = inBand({ x: WIDTH, y: 0 }, middleOf(cell(1, 1)), shown, WIDTH);
  assert.deepEqual(touched.sort(), ["a", "b"]);
  assert.deepEqual(inGridOrder(["c", "b", "a"], shown), ["a", "b", "c"]);
});

test("presses choose as a Mac's desktop does", () => {
  assert.deepEqual(pressed(["a"], "b", false), ["b"]);
  // A press on a selected icon keeps the selection, to drag it together.
  assert.deepEqual(pressed(["a", "b"], "b", false), ["a", "b"]);
  assert.deepEqual(pressed(["a"], "b", true), ["a", "b"]);
  assert.deepEqual(pressed(["a", "b"], "a", true), ["b"]);
  assert.deepEqual(banded(["a", "b"], ["b", "c"], false), ["b", "c"]);
  assert.deepEqual(banded(["a", "b"], ["b", "c"], true), ["a", "c"]);
});

test("the arrows go from icon to icon", () => {
  const shown = new Map([
    ["a", cell(0, 0)],
    ["b", cell(0, 1)],
    ["c", cell(1, 0)],
    ["d", cell(2, 1)],
  ]);
  assert.equal(neighbour("a", "down", shown), "b");
  assert.equal(neighbour("a", "left", shown), "c");
  assert.equal(neighbour("c", "right", shown), "a");
  assert.equal(neighbour("b", "up", shown), "a");
  assert.equal(neighbour("a", "right", shown), null);
  // Across the nearest row first: from b leftward, d on its row before c.
  assert.equal(neighbour("b", "left", shown), "d");
  assert.deepEqual(arrowed([], null, "down", false, shown), { selection: ["a"], anchor: "a" });
  assert.deepEqual(arrowed(["a"], "a", "down", false, shown), { selection: ["b"], anchor: "b" });
  assert.deepEqual(arrowed(["a"], "a", "left", true, shown), { selection: ["a", "c"], anchor: "c" });
  assert.deepEqual(arrowed(["a"], "a", "right", false, shown), { selection: ["a"], anchor: "a" });
});

const item = (name: string, folder: boolean, type = ""): DesktopItem => ({
  name,
  uri: `file:///home/ana/Desktop/${encodeURIComponent(name)}`,
  folder,
  type,
});

test("kinds by media type", () => {
  assert.equal(kindOf(item("Fotos", true)), "folder");
  assert.equal(kindOf(item("a.png", false, "image/png")), "image");
  assert.equal(kindOf(item("a.txt", false, "text/plain")), "text");
  assert.equal(kindOf(item("a.pdf", false, "application/pdf")), "pdf");
  assert.equal(kindOf(item("a.zip", false, "application/zip")), "archive");
  assert.equal(kindOf(item("a.bin", false, "application/octet-stream")), "document");
});

test("which application opens a type: the type, or its type/*", () => {
  assert.ok(opensType(["text/plain"], "text/plain"));
  assert.ok(opensType(["image/*"], "image/png"));
  assert.ok(opensType(["Text/Plain; charset=utf-8"], "text/plain"));
  assert.ok(!opensType(["text/plain"], "image/png"));
  assert.ok(!opensType(undefined, "text/plain"));
  assert.ok(!opensType(["text/plain"], ""));
});

test("opening: folders in Dropsea, files with their application, the rest Dropsea at the desktop", () => {
  const desktop = "file:///home/ana/Desktop";
  const applications = [
    { id: "so.oxy.noted", opens: ["text/plain"] },
    { id: "so.oxy.photos", opens: ["image/*"] },
  ];
  const opening = openingOf(
    [
      item("Fotos", true),
      item("a.txt", false, "text/plain"),
      item("b.txt", false, "text/plain"),
      item("c.png", false, "image/png"),
      item("d.bin", false, "application/octet-stream"),
    ],
    applications,
    desktop,
  );
  assert.deepEqual(opening.urls, [filesUrl(item("Fotos", true).uri), filesUrl(desktop)]);
  assert.deepEqual(opening.files, [
    { application: "so.oxy.noted", uris: [item("a.txt", false).uri, item("b.txt", false).uri] },
    { application: "so.oxy.photos", uris: [item("c.png", false).uri] },
  ]);
  assert.equal(filesUrl("file:///home/ana/Desktop/Mis fotos"), "dropsea://files?uri=file%3A%2F%2F%2Fhome%2Fana%2FDesktop%2FMis%20fotos");
});

test("a drop into a folder: into itself, a place, already there, or going", () => {
  const home = "file:///home/ana";
  const desktop = `${home}/Desktop`;
  const places = [home, desktop, `${home}/Documents`];
  assert.deepEqual(folderDrop([`${home}/a.txt`], desktop, "move", places), { kind: "go", uris: [`${home}/a.txt`] });
  assert.deepEqual(folderDrop([`${desktop}/Fotos`], `${desktop}/Fotos/2026`, "move", places), { kind: "into-itself" });
  assert.deepEqual(folderDrop([`${desktop}`], `${home}/Documents`, "copy", places), { kind: "go", uris: [desktop] });
  assert.deepEqual(folderDrop([`${home}/Documents`], desktop, "move", places), {
    kind: "place",
    uri: `${home}/Documents`,
  });
  assert.deepEqual(folderDrop([`${desktop}/a.txt`], desktop, "move", places), { kind: "nothing" });
  assert.deepEqual(folderDrop([`${desktop}/a.txt`, `${home}/b.txt`], `${desktop}/`, "move", places), {
    kind: "go",
    uris: [`${home}/b.txt`],
  });
  // Escapes are the same path.
  assert.deepEqual(folderDrop([`${desktop}/Mis%20fotos`], `${desktop}/Mis fotos/x`, "move", places), {
    kind: "into-itself",
  });
});

test("desktop.json keeps each name's cell", () => {
  const text = writeArrangement({ "b.txt": cell(1, 2), "a b.png": cell(0, 0) });
  assert.equal(text, '{\n  "cells": {\n    "a b.png": [\n      0,\n      0\n    ],\n    "b.txt": [\n      1,\n      2\n    ]\n  }\n}\n');
  assert.deepEqual(readArrangement(text), { "b.txt": cell(1, 2), "a b.png": cell(0, 0) });
  assert.deepEqual(readArrangement("not json"), {});
  assert.deepEqual(readArrangement('{"cells": {"a": [1], "b": [-1, 0], "c": [1.5, 0], "d": [2, 3]}}'), { d: cell(2, 3) });
});

test("a double press: two presses on one icon within half a second, by their own times", () => {
  assert.ok(!isSecondPress(NO_PRESS, "a", 100));
  assert.ok(isSecondPress({ name: "a", at: 100 }, "a", 590));
  assert.ok(!isSecondPress({ name: "a", at: 100 }, "a", 600));
  assert.ok(!isSecondPress({ name: "a", at: 100 }, "b", 200));
  assert.equal(pressTime({ timeStamp: 5, nativeEvent: { timestamp: 42 } }), 42);
  assert.equal(pressTime({ timeStamp: 5, nativeEvent: {} }), 5);
});

test("copies made on the desktop are named as Finder names them", () => {
  const none = new Set<string>();
  assert.equal(copyName("notas.txt", false, none, "copy"), "notas copy.txt");
  assert.equal(copyName("notas.txt", false, new Set(["notas copy.txt"]), "copy"), "notas copy 2.txt");
  assert.equal(copyName("notas.txt", false, new Set(["notas copy.txt", "notas copy 2.txt"]), "copy"), "notas copy 3.txt");
  // A copy of a copy is numbered from its original's name.
  assert.equal(copyName("notas copy.txt", false, new Set(["notas copy.txt"]), "copy"), "notas copy 2.txt");
  assert.equal(copyName("notas copy 2.txt", false, new Set(["notas copy.txt", "notas copy 2.txt"]), "copy"), "notas copy 3.txt");
  // A folder's whole name, and a name whose only dot begins it.
  assert.equal(copyName("Fotos v1.2", true, none, "copia"), "Fotos v1.2 copia");
  assert.equal(copyName(".bashrc", false, none, "copy"), ".bashrc copy");
  assert.equal(copyName("archive.tar.gz", false, none, "copia"), "archive.tar copia.gz");
});

test("copies let go on the desktop keep their originals' cells taken", () => {
  const shown = new Map([
    ["a", cell(0, 0)],
    ["b", cell(0, 1)],
  ]);
  const grip = { x: 10, y: 10 };
  // Let go where they are: the copies take the free cells nearest, the originals stay.
  const origin = cellOrigin(cell(0, 0), WIDTH);
  const copied = movedTo(["a", "b"], "a", { x: origin.x + grip.x, y: origin.y + grip.y }, grip, shown, WIDTH, GRID, true);
  assert.deepEqual(copied.get("a"), cell(1, 0));
  assert.deepEqual(copied.get("b"), cell(1, 1));
});
