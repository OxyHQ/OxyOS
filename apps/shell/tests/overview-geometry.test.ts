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
