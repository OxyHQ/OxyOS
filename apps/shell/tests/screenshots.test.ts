import { test } from "node:test";
import assert from "node:assert/strict";
import {
  defaultSelection,
  fileUrl,
  pixelSize,
  secondsLeft,
  sizeLabel,
  spanned,
  thumbnailRelease,
  thumbnailSize,
  toGlobal,
  toLocal,
  toolbarPick,
  within,
} from "../src/screenshots.ts";

const output = { id: 2, x: 1280, y: 0, scale: 1.25 };

test("a drag spans the same rectangle whichever way it goes, on whole pixels", () => {
  const forwards = spanned({ x: 10.4, y: 20 }, { x: 110.6, y: 70 });
  const backwards = spanned({ x: 110.6, y: 70 }, { x: 10.4, y: 20 });
  assert.deepEqual(forwards, { x: 10, y: 20, width: 101, height: 50 });
  assert.deepEqual(backwards, forwards);
  assert.deepEqual(spanned({ x: 5, y: 5 }, { x: 5, y: 9 }), { x: 5, y: 5, width: 0, height: 4 });
});

test("a rectangle is kept within its output, and one wholly outside is none", () => {
  assert.deepEqual(within({ x: -10, y: 700, width: 50, height: 50 }, 1280, 720), { x: 0, y: 700, width: 40, height: 20 });
  assert.equal(within({ x: 1300, y: 0, width: 10, height: 10 }, 1280, 720), null);
});

test("an output's rectangles go to the global space and back", () => {
  const local = { x: 100, y: 40, width: 300, height: 200 };
  const global = toGlobal(local, output);
  assert.deepEqual(global, { x: 1380, y: 40, width: 300, height: 200 });
  assert.deepEqual(toLocal(global, output), local);
});

test("the size shown is the display's pixels, each edge rounded as the compositor places it", () => {
  assert.deepEqual(pixelSize({ x: 0, y: 0, width: 100, height: 60 }, 1), { width: 100, height: 60 });
  assert.deepEqual(pixelSize({ x: 0, y: 0, width: 100, height: 60 }, 1.25), { width: 125, height: 75 });
  // 1 → 1.25 (1), 101 → 126.25 (126): 125 pixels; 3 → 3.75 (4), 5 → 6.25 (6): 2.
  assert.deepEqual(pixelSize({ x: 1, y: 3, width: 100, height: 2 }, 1.25), { width: 125, height: 2 });
  assert.equal(sizeLabel({ x: 0, y: 0, width: 800, height: 500 }, 1.25), "1000 × 625");
});

test("the toolbar captures every display, the window under the pointer, or its selection", () => {
  assert.deepEqual(toolbarPick("screen", null, null, output), { kind: "screens" });
  assert.deepEqual(toolbarPick("window", null, 7, output), { kind: "window", window: 7 });
  assert.equal(toolbarPick("window", null, null, output), null);
  assert.deepEqual(toolbarPick("selection", { x: 10, y: 10, width: 20, height: 30 }, null, output), {
    kind: "area",
    rect: { x: 1290, y: 10, width: 20, height: 30 },
  });
  assert.equal(toolbarPick("selection", { x: 10, y: 10, width: 0, height: 30 }, null, output), null);
  assert.deepEqual(defaultSelection(1280, 720), { x: 320, y: 180, width: 640, height: 360 });
});

test("a thumbnail keeps the screenshot's proportions", () => {
  assert.deepEqual(thumbnailSize(2000, 1250), { width: 180, height: 113 });
  assert.deepEqual(thumbnailSize(600, 900), { width: 120, height: 180 });
});

test("a thumbnail let go opens on a click, goes when swiped right, else stays", () => {
  assert.equal(thumbnailRelease(1, 2), "open");
  assert.equal(thumbnailRelease(80, 10), "dismiss");
  assert.equal(thumbnailRelease(-80, 0), "stay");
  assert.equal(thumbnailRelease(20, 30), "stay");
});

test("a saved file's path is read as a file URL, spaces and all", () => {
  assert.equal(
    fileUrl("/home/ada/Escritorio/Captura de pantalla 2026-10-31 a las 10.04.05.png"),
    "file:///home/ada/Escritorio/Captura%20de%20pantalla%202026-10-31%20a%20las%2010.04.05.png",
  );
});

test("a timer counts its whole seconds down", () => {
  assert.equal(secondsLeft(5, 1000, 1000), 5);
  assert.equal(secondsLeft(5, 1000, 1001), 5);
  assert.equal(secondsLeft(5, 1000, 5001), 1);
  assert.equal(secondsLeft(5, 1000, 6000), 0);
});
