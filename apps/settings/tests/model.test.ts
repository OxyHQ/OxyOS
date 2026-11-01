import { test } from "node:test";
import assert from "node:assert/strict";
import {
  besideOthers,
  diagonalInches,
  formatMemory,
  formatRefresh,
  LOCK_AFTER_CHOICES,
  looksLike,
  PREFERENCE_KEYS,
  preferenceText,
  prefersSteadyCaret,
  refreshRates,
  resolutions,
  type Display,
} from "../src/model.ts";
import { stringsFor } from "../src/strings.ts";

test("preferences are named as the session's file names them", () => {
  assert.deepEqual(Object.values(PREFERENCE_KEYS).sort(), [
    "accent",
    "appearance",
    "caret_blink",
    "clock_24h",
    "do_not_disturb",
    "lock_after",
    "notification_previews",
    "reduce_motion",
    "reduce_transparency",
    "screen_reader",
    "wallpaper",
  ]);
  assert.equal(preferenceText<"wallpaper">(null), "");
  assert.equal(preferenceText<"reduceMotion">(true), "true");
  assert.equal(preferenceText<"lockAfter">(15), "15");
  assert.equal(preferenceText<"notificationPreviews">("always"), "always");
  assert.equal(preferenceText<"doNotDisturb">(false), "false");
});

test("Prefer non-blinking cursor is the session's caret_blink turned around", () => {
  assert.equal(PREFERENCE_KEYS.caretBlink, "caret_blink");
  // Carets blink by default (crates/oxyos-preferences): the switch is off.
  assert.equal(prefersSteadyCaret({ caretBlink: true }), false);
  assert.equal(prefersSteadyCaret({ caretBlink: false }), true);
  // As the session's file writes it.
  assert.equal(preferenceText<"caretBlink">(false), "false");
  assert.equal(preferenceText<"caretBlink">(true), "true");
  // Named as macOS names it, in Accessibility, in both languages.
  assert.equal(stringsFor("en").accessibility.steadyCaret, "Prefer non-blinking cursor");
  assert.equal(stringsFor("es").accessibility.steadyCaret, "Preferir cursor sin parpadeo");
  assert.ok(stringsFor("es").accessibility.steadyCaretDescription.length > 0);
});

test("lock after offers never, and a day at most", () => {
  assert.ok(LOCK_AFTER_CHOICES.includes(0));
  assert.ok(LOCK_AFTER_CHOICES.every((minutes) => minutes <= 24 * 60));
});

test("memory reads as people read it", () => {
  assert.equal(formatMemory(16 * 1024 ** 3), "16 GB");
  assert.equal(formatMemory(15.6 * 1024 ** 3), "16 GB");
  assert.equal(formatMemory(512 * 1024 ** 2), "512 MB");
});

test("Spanish for Spanish, English otherwise", () => {
  assert.equal(stringsFor("es-ES").title, "Ajustes");
  assert.equal(stringsFor("es_MX").title, "Ajustes");
  assert.equal(stringsFor("en-US").title, "Settings");
  assert.equal(stringsFor("fr").title, "Settings");
  assert.equal(stringsFor("es").lockScreen.minutes(1), "1 minuto");
  assert.equal(stringsFor("en").lockScreen.minutes(60), "1 hour");
});

/** A 24-inch 1080p monitor right of a 14-inch panel at 1.25, as the session tells Settings them. */
const panel: Display = {
  output: 1,
  name: "eDP-1",
  builtin: true,
  primary: true,
  x: 0,
  y: 0,
  width: 1536,
  height: 960,
  scale: 150,
  mode: 0,
  modes: [{ width: 1920, height: 1200, refreshMhz: 60000, preferred: true }],
  scales: [120, 150, 180, 210],
  recommended: 150,
  widthMm: 301,
  heightMm: 188,
};
const monitor: Display = {
  ...panel,
  output: 2,
  name: "OxyOS Monitor",
  builtin: false,
  primary: false,
  x: 1536,
  width: 1920,
  height: 1080,
  scale: 120,
  modes: [
    { width: 1920, height: 1080, refreshMhz: 60000, preferred: true },
    { width: 1920, height: 1080, refreshMhz: 50000, preferred: false },
    { width: 1280, height: 720, refreshMhz: 59940, preferred: false },
  ],
  scales: [120, 150, 180],
  recommended: 120,
  widthMm: 531,
  heightMm: 299,
};

test("a display's resolutions each once, its refresh rates the fastest first", () => {
  assert.deepEqual(resolutions(monitor), [
    { width: 1920, height: 1080, native: true },
    { width: 1280, height: 720, native: false },
  ]);
  assert.deepEqual(
    refreshRates(monitor, 1920, 1080).map((mode) => mode.refreshMhz),
    [60000, 50000],
  );
  assert.equal(formatRefresh(60000), "60 Hz");
  assert.equal(formatRefresh(59940), "59.94 Hz");
  assert.equal(formatRefresh(143981), "143.98 Hz");
});

test("a scale looks like the mode's pixels at it, rounded up as the session rounds them", () => {
  assert.deepEqual(looksLike(panel, 150), { width: 1536, height: 960 });
  assert.deepEqual(looksLike(panel, 210), { width: 1098, height: 686 });
  assert.equal(diagonalInches(301, 188), 14);
  assert.equal(diagonalInches(531, 299), 24);
  assert.equal(diagonalInches(16, 9), null);
});

test("a display moved to a side of the others is wanted just beyond them", () => {
  const displays = [panel, monitor];
  assert.deepEqual(besideOthers(displays, 2, "above"), { x: 0, y: -1080 });
  assert.deepEqual(besideOthers(displays, 2, "left"), { x: -1920, y: 0 });
  assert.deepEqual(besideOthers(displays, 1, "below"), { x: 1536, y: 1080 });
  assert.deepEqual(besideOthers([panel], 1, "right"), { x: 0, y: 0 });
});

test("the Displays page's words in both languages", () => {
  assert.equal(stringsFor("es").pages.displays, "Pantallas");
  assert.equal(stringsFor("en").displays.keepDescription(1), "The display goes back to its previous settings in 1 second.");
  assert.equal(stringsFor("es").displays.size(14, "1920 × 1200"), "14 pulgadas, 1920 × 1200");
  assert.equal(stringsFor("en").displays.looksLike("1536 × 960"), "Looks like 1536 × 960");
});
