import { test } from "node:test";
import assert from "node:assert/strict";
import {
  workspaceAtPoint,
  nearestWorkspace,
} from "../src/overview-geometry.ts";
const geometry = {
  x: 0,
  y: 28,
  width: 1280,
  cardWidth: 896,
  cardHeight: 504,
  gap: 32,
  inset: 192,
  top: 50,
  count: 2,
};
test("drop targets exclude panels, gaps, margins and the add-desktop control", () => {
  assert.equal(workspaceAtPoint(300, 100, 0, geometry), 0);
  assert.equal(workspaceAtPoint(1200, 100, 0, geometry), 1);
  for (const [x, y] of [
    [1100, 100],
    [100, 100],
    [300, 50],
    [300, 600],
    [-1, 100],
    [1281, 100],
  ])
    assert.equal(workspaceAtPoint(x!, y!, 0, geometry), -1);
  assert.equal(workspaceAtPoint(1200, 100, 1856, geometry), -1);
});
test("scroll and output offsets preserve the identity of the drop target", () => {
  assert.equal(workspaceAtPoint(300, 100, 928, geometry), 1);
  assert.equal(
    workspaceAtPoint(340, 150, 928, { ...geometry, x: 40, y: 78 }),
    1,
  );
  assert.equal(workspaceAtPoint(300, 100, 928, { ...geometry, count: 1 }), -1);
});
test("settling clamps overscroll and picks the nearest real workspace", () => {
  assert.equal(nearestWorkspace(-120, 928, 2), 0);
  assert.equal(nearestWorkspace(463, 928, 2), 0);
  assert.equal(nearestWorkspace(500, 928, 2), 1);
  assert.equal(nearestWorkspace(3000, 928, 2), 1);
});

import { exposeLayout, neighbour, windowRows, type ExposedWindow } from "../src/overview-geometry.ts";

const overlap = (a: { x: number; y: number; width: number; height: number }, b: typeof a) =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

test("the overview's rows: columns a row, one height a row fitting its width", () => {
  const rows = windowRows([1.6, 1.6, 1.6, 1.6], {
    width: 864,
    height: 600,
    gap: 16,
    columns: 3,
    chrome: 24,
    minRow: 104,
    maxRow: 310,
  });
  assert.deepEqual(rows.map((row) => row.indices), [[0, 1, 2], [3]]);
  // Three of 1.6 in 864 − 32: 173.3 high; the second row's one is as tall as its row lets it,
  // (600 − 16) / 2 less its title bar.
  assert.ok(Math.abs(rows[0]!.contentHeight - 832 / 4.8) < 1e-9);
  assert.equal(rows[1]!.contentHeight, (600 - 16) / 2 - 24);
});

test("App Exposé: as large as they fit, never past their size, apart and inside the area", () => {
  const area = { x: 0, y: 80, width: 1280, height: 480 };
  const cases: ExposedWindow[][] = [
    [{ ratio: 600 / 360, height: 360 }],
    [{ ratio: 600 / 360, height: 360 }, { ratio: 460 / 300, height: 300 }],
    [{ ratio: 1.6 }, { ratio: 1.6 }, { ratio: 0.8 }, { ratio: 2 }, { ratio: 1.6 }],
  ];
  for (const windows of cases) {
    const rects = exposeLayout(windows, area, 32, 28);
    assert.equal(rects.length, windows.length);
    rects.forEach((rect, i) => {
      assert.ok(rect.x >= area.x && rect.y >= area.y, JSON.stringify(rect));
      assert.ok(rect.x + rect.width <= area.x + area.width);
      assert.ok(rect.y + rect.height + 28 <= area.y + area.height);
      const own = windows[i]!.height;
      if (own) assert.ok(rect.height <= own);
      // Its proportions kept, to the pixel.
      assert.ok(Math.abs(rect.width - rect.height * windows[i]!.ratio) <= 1.5);
      rects.forEach((other, j) => {
        if (j !== i) assert.ok(!overlap(rect, other), `${i} and ${j} overlap`);
      });
    });
  }
  // One window: at its own size, centred.
  const [one] = exposeLayout([{ ratio: 600 / 360, height: 360 }], area, 32, 28);
  assert.deepEqual(one, { x: 340, y: 126, width: 600, height: 360 });
  // Two that fit side by side are side by side.
  const two = exposeLayout(
    [{ ratio: 600 / 360, height: 360 }, { ratio: 460 / 300, height: 300 }],
    area,
    32,
    28,
  );
  assert.ok(two[0]!.y < two[1]!.y + two[1]!.height && two[1]!.y < two[0]!.y + two[0]!.height);
  assert.ok(two[0]!.x < two[1]!.x);
});

test("App Exposé's arrows move to the window that way", () => {
  // Two rows: 0 1 2 above, 3 4 below.
  const rects = [
    { x: 0, y: 0, width: 100, height: 60 },
    { x: 120, y: 0, width: 100, height: 60 },
    { x: 240, y: 0, width: 100, height: 60 },
    { x: 60, y: 80, width: 100, height: 60 },
    { x: 180, y: 80, width: 100, height: 60 },
  ];
  assert.equal(neighbour(rects, 0, "right"), 1);
  assert.equal(neighbour(rects, 1, "left"), 0);
  assert.equal(neighbour(rects, 2, "right"), 3, "past a row's end, the next in order");
  assert.equal(neighbour(rects, 3, "left"), 2, "before a row's start, the one before");
  assert.equal(neighbour(rects, 1, "down"), 3);
  assert.equal(neighbour(rects, 2, "down"), 4);
  assert.equal(neighbour(rects, 4, "up"), 1);
  assert.equal(neighbour(rects, 0, "up"), 0, "nothing above");
  assert.equal(neighbour(rects, 4, "right"), 4, "nothing after the last");
});
