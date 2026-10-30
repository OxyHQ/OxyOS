import { test } from "node:test";
import assert from "node:assert/strict";
import { launcherLayout, moveSelection, pageOf } from "../src/launcher.ts";

test("a computer's screen has Launchpad's seven columns and five rows, icons a little over half a column", () => {
  // The ThinkPad's 1600 × 1000 at 1.25, less the dock: 1280 × 744 logical pixels.
  const layout = launcherLayout(1280, 744);
  assert.equal(layout.columns, 7);
  assert.equal(layout.rows, 5);
  assert.equal(layout.gridWidth, 998);
  assert.equal(layout.iconSize, 78);
  assert.equal(layout.rowHeight, 120);
  // The grid starts below the search, and its rows end above the dots.
  assert.ok(layout.gridTop >= layout.searchTop + 32 + 24);
  assert.ok(layout.gridTop + layout.rows * layout.rowHeight <= 744 - layout.dotsBottom);
  // A larger screen: larger icons, at most a ninth of the grid, the grid at most 1160 wide.
  const large = launcherLayout(2560, 1540);
  assert.equal(large.gridWidth, 1160);
  assert.ok(large.iconSize > layout.iconSize && large.iconSize <= 112);
  assert.equal(large.labelSize, 13);
  // A small one: fewer columns and rows, the icon never under 48.
  const small = launcherLayout(640, 420);
  assert.deepEqual([small.columns, small.rows], [4, 3]);
  assert.equal(launcherLayout(800, 560).rows, 4);
  assert.ok(small.iconSize >= 48);
});

test("every row holds its icon, its label and the highlight around them", () => {
  for (const [width, height] of [
    [1280, 664],
    [1280, 744],
    [1440, 844],
    [1920, 1144],
    [800, 560],
  ]) {
    const { rowHeight, iconSize, labelSize } = launcherLayout(width, height);
    assert.ok(iconSize + 8 + labelSize + 6 + 16 <= rowHeight, `${width}×${height}`);
  }
});

test("the arrows move the highlight through the icons and across pages", () => {
  const [columns, rows] = [7, 5];
  // None yet: any arrow highlights the first icon of the page shown.
  assert.equal(moveSelection(null, "ArrowRight", 50, columns, rows, 0), 0);
  assert.equal(moveSelection(null, "ArrowDown", 50, columns, rows, 1), 35);
  // Left and right go through every icon, past a page's end to the next.
  assert.equal(moveSelection(0, "ArrowLeft", 50, columns, rows, 0), 0);
  assert.equal(moveSelection(3, "ArrowRight", 50, columns, rows, 0), 4);
  assert.equal(moveSelection(34, "ArrowRight", 50, columns, rows, 0), 35);
  assert.equal(pageOf(35, columns, rows), 1);
  assert.equal(moveSelection(35, "ArrowLeft", 50, columns, rows, 1), 34);
  assert.equal(moveSelection(49, "ArrowRight", 50, columns, rows, 1), 49);
  // Up and down a row, within the page.
  assert.equal(moveSelection(9, "ArrowUp", 50, columns, rows, 0), 2);
  assert.equal(moveSelection(2, "ArrowUp", 50, columns, rows, 0), 2);
  assert.equal(moveSelection(30, "ArrowDown", 50, columns, rows, 0), 30);
  assert.equal(moveSelection(36, "ArrowDown", 50, columns, rows, 1), 43);
  assert.equal(moveSelection(44, "ArrowDown", 50, columns, rows, 1), 44, "no icon below");
  // Nothing to highlight.
  assert.equal(moveSelection(null, "ArrowRight", 0, columns, rows, 0), null);
  // A highlight past what a search left: back to the first.
  assert.equal(moveSelection(12, "ArrowRight", 3, columns, rows, 0), 0);
});
