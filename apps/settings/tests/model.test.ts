import { test } from "node:test";
import assert from "node:assert/strict";
import { formatMemory, LOCK_AFTER_CHOICES, PREFERENCE_KEYS, preferenceText } from "../src/model.ts";
import { stringsFor } from "../src/strings.ts";

test("preferences are named as the session's file names them", () => {
  assert.deepEqual(Object.values(PREFERENCE_KEYS).sort(), [
    "accent",
    "appearance",
    "clock_24h",
    "lock_after",
    "reduce_motion",
    "reduce_transparency",
    "screen_reader",
    "wallpaper",
  ]);
  assert.equal(preferenceText<"wallpaper">(null), "");
  assert.equal(preferenceText<"reduceMotion">(true), "true");
  assert.equal(preferenceText<"lockAfter">(15), "15");
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
