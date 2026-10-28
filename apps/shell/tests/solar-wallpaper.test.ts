import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { selectSolarFrame } from "../src/solar-wallpaper.ts";
const manifest = JSON.parse(
  readFileSync(
    new URL(
      "../../../assets/wallpapers/solar-gradients/manifest.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

test("every original solar anchor resolves to its source frame on the appropriate phase", () => {
  assert.equal(manifest.frames.length, 16);
  for (const anchor of manifest.solarAnchors) {
    assert.equal(
      selectSolarFrame(
        manifest.solarAnchors,
        anchor.elevationDegrees,
        anchor.azimuthDegrees < 180,
      ),
      anchor.frame,
    );
  }
});
test("morning and evening twilight use different frames; invalid positions do not silently select a wallpaper", () => {
  assert.equal(selectSolarFrame(manifest.solarAnchors, -5.4, true), 3);
  assert.equal(selectSolarFrame(manifest.solarAnchors, -5.4, false), 13);
  assert.equal(selectSolarFrame(manifest.solarAnchors, -90, true), 0);
  assert.equal(selectSolarFrame(manifest.solarAnchors, 89, false), 8);
  assert.throws(
    () => selectSolarFrame(manifest.solarAnchors, NaN, true),
    RangeError,
  );
  assert.throws(() => selectSolarFrame([], 10, true));
});
