import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CROSSFADE_MS,
  crossfadeMs,
  resizeModeOf,
  sameWallpaper,
  shownLayers,
  SLOW_CROSSFADE_MS,
  withIncoming,
  type ShellWallpaper,
} from "../src/wallpaper.ts";

const day: ShellWallpaper = { key: "solar-gradients/midday", source: { uri: "file:///w/solar-gradients/midday.jpg" }, fit: "fill", color: "#000000" };
const night: ShellWallpaper = { key: "solar-gradients/night", source: { uri: "file:///w/solar-gradients/night.jpg" }, fit: "fill", color: "#000000", slow: true };
const blue: ShellWallpaper = { key: "color:1f4fd8", fit: "fill", color: "#1f4fd8" };

test("each fit is the resize mode that draws it", () => {
  assert.equal(resizeModeOf("fill"), "cover");
  assert.equal(resizeModeOf("fit"), "contain");
  assert.equal(resizeModeOf("stretch"), "stretch");
  assert.equal(resizeModeOf("center"), "center");
  assert.equal(resizeModeOf("tile"), "repeat");
});

test("the same picture, fit and colour is the same wallpaper, however it came", () => {
  assert.ok(sameWallpaper(day, { ...day, slow: true }));
  assert.ok(!sameWallpaper(day, { ...day, fit: "fit" }));
  assert.ok(!sameWallpaper(day, { ...day, color: "#ffffff" }));
  assert.ok(!sameWallpaper(day, { ...day, source: { uri: "file:///w/other.svg" } }));
  assert.ok(!sameWallpaper(day, night));
});

test("the first wallpaper is shown at once; a change comes in over it and the old goes once it is shown", () => {
  let layers = withIncoming([], day, 1);
  assert.deepEqual(layers, [{ id: 1, wallpaper: day, shown: true }]);
  // Told again the same: nothing comes in.
  assert.equal(withIncoming(layers, { ...day }, 2), layers);
  layers = withIncoming(layers, night, 2);
  assert.deepEqual(layers.map((layer) => [layer.id, layer.shown]), [[1, true], [2, false]]);
  layers = shownLayers(layers, 2);
  assert.deepEqual(layers, [{ id: 2, wallpaper: night, shown: true }]);
});

test("a change while one fades in comes in over both, and only what it covers goes", () => {
  let layers = withIncoming(withIncoming([], day, 1), night, 2);
  layers = withIncoming(layers, blue, 3);
  // The second finishes first: the first goes, the third still comes in over it.
  layers = shownLayers(layers, 2);
  assert.deepEqual(layers.map((layer) => [layer.id, layer.shown]), [[2, true], [3, false]]);
  layers = shownLayers(layers, 3);
  assert.deepEqual(layers.map((layer) => layer.id), [3]);
  // A layer already gone changes nothing.
  assert.equal(shownLayers(layers, 1), layers);
});

test("a change the clock brought fades more slowly", () => {
  assert.equal(crossfadeMs(day), CROSSFADE_MS);
  assert.equal(crossfadeMs(night), SLOW_CROSSFADE_MS);
});
