import { test } from "node:test";
import assert from "node:assert/strict";
import {
  besideOthers,
  diagonalInches,
  formatMemory,
  formatRefresh,
  LOCK_AFTER_CHOICES,
  looksLike,
  notificationSoundsWith,
  placeOf,
  PREFERENCE_KEYS,
  preferenceText,
  prefersSteadyCaret,
  refreshRates,
  resolutions,
  sideOfKey,
  type Display,
} from "../src/model.ts";
import { messagesFor } from "../src/messages.ts";

test("preferences are named as the session's file names them", () => {
  assert.deepEqual(Object.values(PREFERENCE_KEYS).sort(), [
    "accent",
    "appearance",
    "battery_percentage",
    "caret_blink",
    "dim_on_battery",
    "display_off_adapter",
    "display_off_battery",
    "do_not_disturb",
    "hour_cycle",
    "input_source",
    "input_sources",
    "language",
    "lock_after",
    "low_power_mode",
    "mouse_natural_scrolling",
    "notification_previews",
    "notification_sounds_off",
    "reduce_motion",
    "reduce_transparency",
    "region",
    "screen_reader",
    "sleep_adapter",
    "sleep_battery",
    "trackpad_natural_scrolling",
    "ui_sounds",
    "wallpaper",
    "wallpaper_all_desktops",
  ]);
  assert.equal(preferenceText<"wallpaper">(""), "");
  assert.equal(preferenceText<"reduceMotion">(true), "true");
  assert.equal(preferenceText<"lockAfter">(15), "15");
  assert.equal(preferenceText<"notificationPreviews">("always"), "always");
  // A list as the session's file keeps it: its names joined by commas, none for an empty one.
  assert.equal(preferenceText<"inputSources">(["pinyin", "korean"]), "pinyin,korean");
  assert.equal(preferenceText<"inputSources">([]), "");
  assert.equal(preferenceText<"doNotDisturb">(false), "false");
});

test("Reduce motion is said as the crossfade it is", () => {
  // A window minimized fades where it is (docs/adr/0043-the-genie.mdx, decision 7): nothing is at
  // once. The genie is the only other way a window goes into the dock, so Settings has no choice of
  // it (5 November 2026) and no Dock page.
  assert.doesNotMatch(messagesFor("en").accessibility.reduceMotionDescription, /at once/);
  assert.match(messagesFor("es").accessibility.reduceMotionDescription, /fundido/);
  assert.equal("dock" in messagesFor("en").pages, false);
  assert.equal("minimizeEffect" in PREFERENCE_KEYS, false);
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
  assert.equal(messagesFor("en").accessibility.steadyCaret, "Prefer non-blinking cursor");
  assert.equal(messagesFor("es").accessibility.steadyCaret, "Preferir cursor sin parpadeo");
  assert.ok(messagesFor("es").accessibility.steadyCaretDescription.length > 0);
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
  assert.equal(messagesFor("es-ES").title, "Ajustes");
  assert.equal(messagesFor("es_MX").title, "Ajustes");
  assert.equal(messagesFor("en-US").title, "Settings");
  assert.equal(messagesFor("fr").title, "Settings");
  assert.equal(messagesFor("es").lockScreen.minutes(1), "1 minuto");
  assert.equal(messagesFor("en").lockScreen.minutes(60), "1 hour");
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

test("a display's place is the side of the display it shares the longest edge with", () => {
  // The panel at 0,0 (1536 × 960), the monitor right of it.
  assert.deepEqual(placeOf([panel, monitor], 2), { side: "right", of: panel });
  assert.deepEqual(placeOf([panel, monitor], 1), { side: "left", of: monitor });
  // The monitor above the panel, then below it.
  const above = { ...monitor, x: 0, y: -1080 };
  assert.deepEqual(placeOf([panel, above], 2), { side: "above", of: panel });
  assert.deepEqual(placeOf([panel, above], 1), { side: "below", of: above });
  // A third display under the panel's right end and the monitor's left end: the panel's is longer.
  const short = { ...monitor, height: 960 };
  const third = { ...monitor, output: 3, x: 1000, y: 960, width: 1000, height: 600 };
  assert.deepEqual(placeOf([panel, short, third], 3), { side: "below", of: panel });
  // Alone, or corner to corner, it stands beside none.
  assert.equal(placeOf([panel], 1), null);
  assert.equal(placeOf([panel, { ...monitor, x: 1536, y: 960 }], 2), null);
  assert.equal(placeOf([panel], 9), null);
});

test("the arrows are the sides a display moves to", () => {
  assert.equal(sideOfKey("ArrowLeft"), "left");
  assert.equal(sideOfKey("ArrowRight"), "right");
  assert.equal(sideOfKey("ArrowUp"), "above");
  assert.equal(sideOfKey("ArrowDown"), "below");
  assert.equal(sideOfKey("Enter"), null);
});

test("the Displays page's words in both languages", () => {
  assert.equal(messagesFor("es").pages.displays, "Pantallas");
  assert.equal(messagesFor("en").displays.keepDescription(1), "The display goes back to its previous settings in 1 second.");
  assert.equal(messagesFor("es").displays.size(14, "1920 × 1200"), "14 pulgadas, 1920 × 1200");
  assert.equal(messagesFor("en").displays.looksLike("1536 × 960"), "Looks like 1536 × 960");
  assert.equal(messagesFor("en").displays.place("right", "Built-in Display"), "right of Built-in Display");
  assert.equal(messagesFor("es").displays.place("right", "Pantalla 1"), "a la derecha de Pantalla 1");
  assert.equal(messagesFor("es").displays.place("above", "Pantalla 1"), "encima de Pantalla 1");
});

test("interface sounds and each application's notification sound are named as a Mac's", () => {
  // Sound's switch (docs/adr/0044-system-sounds.mdx), on by default, as the file writes it.
  assert.equal(PREFERENCE_KEYS.uiSounds, "ui_sounds");
  assert.equal(preferenceText<"uiSounds">(false), "false");
  assert.equal(messagesFor("en").sound.uiSounds, "Play user interface sound effects");
  assert.equal(messagesFor("es").sound.uiSounds, "Reproducir efectos de sonido de la interfaz de usuario");
  assert.equal(messagesFor("es").pages.sound, "Sonido");
  // The applications whose notifications are silent: one turned off joins the list at its end, on
  // again leaves it; the list as the file keeps it.
  assert.deepEqual(notificationSoundsWith([], "so.oxy.allo", false), ["so.oxy.allo"]);
  assert.deepEqual(notificationSoundsWith(["so.oxy.allo"], "so.oxy.inbox", false), ["so.oxy.allo", "so.oxy.inbox"]);
  assert.deepEqual(notificationSoundsWith(["so.oxy.allo", "so.oxy.inbox"], "so.oxy.allo", true), ["so.oxy.inbox"]);
  assert.deepEqual(notificationSoundsWith(["so.oxy.allo"], "so.oxy.allo", false), ["so.oxy.allo"]);
  assert.equal(preferenceText<"notificationSoundsOff">(["so.oxy.allo", "so.oxy.inbox"]), "so.oxy.allo,so.oxy.inbox");
  assert.equal(messagesFor("en").notifications.playSound, "Play sound for notifications");
  assert.equal(messagesFor("es").notifications.playSound, "Reproducir sonido para las notificaciones");
  assert.equal(messagesFor("en").notifications.playSoundFor("Allo"), "Play sound for Allo's notifications");
});

test("natural scrolling is a Mac's switch on its Mouse and Trackpad pages, each its own preference", () => {
  // docs/adr/0042-scrolling.mdx, decision 6: the session's two keys, as the file writes them.
  assert.equal(PREFERENCE_KEYS.trackpadNaturalScrolling, "trackpad_natural_scrolling");
  assert.equal(PREFERENCE_KEYS.mouseNaturalScrolling, "mouse_natural_scrolling");
  assert.equal(preferenceText<"trackpadNaturalScrolling">(false), "false");
  assert.equal(preferenceText<"mouseNaturalScrolling">(true), "true");
  // macOS's words, in English and in Spanish.
  assert.equal(messagesFor("en").pages.mouse, "Mouse");
  assert.equal(messagesFor("en").pages.trackpad, "Trackpad");
  assert.equal(messagesFor("es").pages.mouse, "Ratón");
  assert.equal(messagesFor("es").pages.trackpad, "Trackpad");
  assert.equal(messagesFor("en").scrolling.naturalScrolling, "Natural scrolling");
  assert.equal(messagesFor("en").scrolling.naturalScrollingDescription, "Content tracks finger movement");
  assert.equal(messagesFor("es").scrolling.naturalScrolling, "Desplazamiento natural");
  assert.equal(
    messagesFor("es").scrolling.naturalScrollingDescription,
    "El contenido sigue el movimiento de los dedos",
  );
});
