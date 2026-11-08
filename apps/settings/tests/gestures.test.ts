import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CYCLE_MS,
  CYCLES,
  fingerCount,
  fingersOf,
  frameAt,
  marksOf,
  PAD,
  PINCH_CENTRE,
  PLAY_MS,
  REST_FRAME,
} from "../src/gestures.ts";

test("a gesture's picture plays twice over and comes to rest on its still", () => {
  assert.equal(CYCLES, 2);
  assert.equal(PLAY_MS, 2 * CYCLE_MS);
  // Before and after a play, and at both its ends, the still: nothing moved, fully drawn.
  for (const t of [-1, 0, CYCLES, CYCLES + 1]) assert.deepEqual(frameAt(t), REST_FRAME);
  // Its first and last frames are the still's, so it neither jumps in nor out.
  const first = frameAt(1e-6);
  assert.ok(first.along < 1e-3 && first.opacity > 0.999, JSON.stringify(first));
  const last = frameAt(CYCLES - 1e-6);
  assert.ok(last.along === 0 && last.opacity > 0.999, JSON.stringify(last));
  // Each go reaches the end of its way and fades there.
  for (const go of [0, 1]) {
    const peak = Math.max(...Array.from({ length: 100 }, (_, i) => frameAt(go + i / 100).along));
    assert.ok(peak > 0.999, `go ${go} reached ${peak}`);
  }
  assert.ok(frameAt(0.84).opacity < 0.05);
  // Across, the first go is to the left and the second to the right.
  assert.equal(frameAt(0.4).direction, -1);
  assert.equal(frameAt(1.4).direction, 1);
});

test("the fingers are those of the choice, on the pad and on it wherever they go", () => {
  assert.equal(fingerCount("three"), 3);
  assert.equal(fingerCount("four"), 4);
  assert.equal(fingerCount("off"), 3);
  for (const gesture of ["desktops", "missionControl", "appExpose"] as const) {
    assert.equal(fingersOf(gesture, 3).length, 3);
    assert.equal(fingersOf(gesture, 4).length, 4);
  }
  assert.equal(fingersOf("launchpad", 4).length, 4);
  const inside = (x: number, y: number) => x >= 3 && x <= PAD.width - 3 && y >= 3 && y <= PAD.height - 3;
  for (const gesture of ["desktops", "missionControl", "appExpose", "launchpad"] as const) {
    for (const { x, y, dx, dy } of fingersOf(gesture, 4)) {
      // The desktops' fingers go either way.
      const back = gesture === "desktops" ? inside(x - dx, y - dy) : true;
      assert.ok(inside(x, y) && inside(x + dx, y + dy) && back, `${gesture} ${x},${y}`);
    }
    assert.ok(marksOf(gesture, fingersOf(gesture, 3)).length > 0);
  }
  // Up, down, across, and the pinch drawn in towards one place.
  assert.ok(fingersOf("missionControl", 3).every((f) => f.dy < 0 && f.dx === 0));
  assert.ok(fingersOf("appExpose", 3).every((f) => f.dy > 0 && f.dx === 0));
  assert.ok(fingersOf("desktops", 3).every((f) => f.dy === 0 && f.dx !== 0));
  for (const { x, y, dx, dy } of fingersOf("launchpad", 4)) {
    const before = Math.hypot(PINCH_CENTRE.x - x, PINCH_CENTRE.y - y);
    const after = Math.hypot(PINCH_CENTRE.x - x - dx, PINCH_CENTRE.y - y - dy);
    assert.ok(after < before / 2);
  }
});
