import { test } from "node:test";
import assert from "node:assert/strict";
import { CATALOGUES, languageOf, shellMessages, type Formats } from "../src/messages.ts";
import { barClock, lockDate, lockTime, twelveHour } from "../src/clock.ts";
import { sheetView, size } from "../src/sheet.ts";
import { announcement, batteryLabel, bluetoothLabel } from "../src/controls.ts";
import { dockMenu, trashMenu } from "../src/dock.ts";
import { ago, countLabel } from "../src/notifications.ts";
import { waitSaid } from "../src/lock-wait.ts";

/** Every path of a catalogue to the kind of what is there: a string, a function, a list of n. */
function shape(value: unknown, path = ""): string[] {
  if (typeof value === "string") return [`${path}: string`];
  if (typeof value === "function") return [`${path}: function`];
  if (Array.isArray(value)) return [`${path}: list of ${value.length}`, ...value.flatMap((item, i) => shape(item, `${path}[${i}]`))];
  if (value && typeof value === "object") {
    return Object.keys(value)
      .sort()
      .flatMap((key) => shape((value as Record<string, unknown>)[key], path ? `${path}.${key}` : key));
  }
  return [`${path}: ${typeof value}`];
}

/** Every string a catalogue says, its functions called with a sample for each argument. */
function said(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (typeof value === "function") return [String(value(...Array.from({ length: value.length }, () => "2")))];
  if (Array.isArray(value)) return value.flatMap(said);
  if (value && typeof value === "object") return Object.values(value).flatMap(said);
  return [];
}

const formats: Formats = { number: (value) => String(value), plural: (count, forms) => (count === 1 ? forms.one : forms.other) };

test("the shell's Spanish and English have the same strings and functions, key for key", () => {
  assert.deepEqual(shape(CATALOGUES.es(formats)), shape(CATALOGUES.en(formats)));
});

test("no Spanish is English pasted in, nor empty", () => {
  const en = said(CATALOGUES.en(formats));
  const es = said(CATALOGUES.es(formats));
  assert.ok(es.every((text) => text.length > 0));
  // Words alike in both are names (Wi-Fi, the OxyOS menu), units or a chord's key.
  const same = es.filter((text, i) => text === en[i]);
  assert.ok(same.length < es.length / 10, `too many alike: ${same.join(" | ")}`);
});

test("a locale's language is its primary subtag, English for any OxyOS does not speak", () => {
  assert.equal(languageOf("es-ES"), "es");
  assert.equal(languageOf("es_MX"), "es");
  assert.equal(languageOf("es-MX-u-hc-h12"), "es");
  assert.equal(languageOf("en-US"), "en");
  assert.equal(languageOf("fr-FR"), "en");
  assert.equal(languageOf(undefined), "en");
  assert.equal(shellMessages("en-ES").language, "en");
  assert.equal(shellMessages("es-ES").language, "es");
  // A malformed tag still draws, in its language's defaults.
  assert.equal(shellMessages("es_ES").locale, "es");
});

test("the top bar's clock follows the locale's hour cycle, Spanish as before", () => {
  const morning = new Date(2026, 10, 2, 9, 41);
  assert.equal(barClock(morning, "es-ES"), "lun, 2 nov 09:41");
  assert.equal(barClock(morning, "es"), "lun, 2 nov 09:41");
  assert.match(barClock(morning, "en-US"), /^Mon, Nov 2 9:41\sAM$/u);
  assert.equal(barClock(morning, "en-US-u-hc-h23"), "Mon, Nov 2 09:41");
  assert.match(barClock(morning, "es-ES-u-hc-h12"), /^lun, 2 nov 9:41\sa\.\sm\.$/u);
  assert.equal(twelveHour("en-US"), true);
  assert.equal(twelveHour("es-ES"), false);
  assert.equal(twelveHour("en-GB"), false);
});

test("the lock screen's date and time, the day period said but not drawn", () => {
  const morning = new Date(2026, 10, 2, 9, 41);
  assert.equal(lockDate(morning, "es-ES"), "Lunes, 2 de noviembre");
  assert.equal(lockDate(morning, "en-US"), "Monday, November 2");
  assert.deepEqual(lockTime(morning, "es-ES"), { shown: "09:41", said: "09:41" });
  const twelve = lockTime(new Date(2026, 10, 2, 21, 5), "en-US");
  assert.equal(twelve.shown, "9:05");
  assert.match(twelve.said, /^9:05\sPM$/u);
});

test("the sheets speak the locale's language, and their sizes its digits", () => {
  const es = shellMessages("es-ES");
  const en = shellMessages("en-US");
  assert.equal(sheetView({ kind: "empty-trash", items: 3 }, es).body, "Se eliminarán 3 ítems. No puedes deshacer esta acción.");
  assert.equal(sheetView({ kind: "empty-trash", items: 3 }, en).body, "3 items will be erased. You can't undo this action.");
  assert.equal(sheetView({ kind: "empty-trash", items: 1 }, es).title, "¿Seguro que quieres eliminar permanentemente los ítems de la Papelera?");
  assert.equal(sheetView({ kind: "wifi-failed", ssid: "Casa" }, en).accept, "OK");
  assert.equal(size(1_500_000, es), "1,5 MB");
  assert.equal(size(1_500_000, en), "1.5 MB");
  assert.equal(size(71_000, es), "71 kB");
  assert.equal(size(71_000, en), "71 KB");
  // English formatted for Spain: English words, Spanish digits.
  assert.equal(size(1_500_000, shellMessages("en-ES")), "1,5 MB");
});

test("counts are the locale's plurals, levels its percent", () => {
  const es = shellMessages("es-ES");
  const en = shellMessages("en-US");
  assert.equal(countLabel(1, es), "1 notificación");
  assert.equal(countLabel(0, es), "0 notificaciones");
  assert.equal(countLabel(1, en), "1 notification");
  assert.equal(countLabel(4, en), "4 notifications");
  assert.equal(waitSaid(1, en), "1 second");
  assert.equal(waitSaid(120, en), "2 minutes");
  assert.equal(bluetoothLabel({ state: "on", connected: 1 }, en), "1 device");
  assert.equal(announcement("output", { level: 0.5625, available: true }, en), "Volume, 56%");
  assert.equal(announcement("output", { level: 0.5625, available: true }, es), "Volumen, 56 %");
  assert.equal(
    batteryLabel({ battery: "charging", percent: 78, onAC: true, minutes: 181 }, en),
    "78%, charging (3 h 1 min until full)",
  );
  assert.equal(ago(0, 5 * 60_000, en), "5 min ago");
  assert.equal(ago(0, 30 * 3_600_000, en), "yesterday");
});

test("the dock's menus in English are a Mac's", () => {
  const en = shellMessages("en-US");
  const snapshot = {
    applications: [{ id: "allo", name: "Allo", icon: { svg: "" }, pinned: true }],
    windows: [],
    activeAppId: undefined,
    openAtLogin: [],
  };
  const all = { hide: true, quit: true, forceQuit: true, keepInDock: true, openAtLogin: true, showAllWindows: true };
  const menu = dockMenu(snapshot, "allo", all, en);
  assert.deepEqual(
    menu.map((item) => (item.kind === "submenu" ? `${item.label} > ${item.items.map((i) => ("label" in i ? i.label : "")).join(" | ")}` : "label" in item ? item.label : "---")),
    ["Open", "Options > Keep in Dock | Open at Login"],
  );
  assert.deepEqual(
    trashMenu(2, true, en).map((entry) => entry.label),
    ["Open", "Empty Trash"],
  );
});

test("Safe Mode is named as a Mac's menu bar names Safe Boot, in each language", () => {
  // docs/adr/0054-recovery-and-erasing.mdx, decision 10.
  assert.equal(shellMessages("en-US").safeMode, "Safe Mode");
  assert.equal(shellMessages("es-ES").safeMode, "Modo seguro");
});
