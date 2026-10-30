import { test } from "node:test";
import assert from "node:assert/strict";
import { readSheet, sheetArrived, sheetGone, sheetView, size } from "../src/sheet.ts";

const plan = {
  id: "so.oxy.notes",
  name: "Notes",
  names: { es: "Notas" },
  version: "1.12.0",
  download: 71_000,
  capabilities: [
    { name: "network", purpose: { en: "Sync your notes", es: "Sincronizar tus notas" }, askedAtFirstUse: false },
    { name: "notifications", purpose: { en: "Reminders" }, askedAtFirstUse: true },
  ],
  newCapabilities: [],
  keyRotation: false,
  update: false,
};

test("an install sheet names the app, its size and each capability with its purpose", () => {
  const es = sheetView({ kind: "install", plan }, "es");
  assert.equal(es.title, "¿Instalar Notas?");
  assert.equal(es.body, "Versión 1.12.0 · 71 kB");
  assert.deepEqual(es.lines[0], { label: "Internet", detail: "Sincronizar tus notas" });
  assert.equal(es.lines[1].label, "Notificaciones");
  assert.match(es.lines[1].detail!, /Se preguntará cuando la app lo use/);
  assert.equal(es.accept, "Instalar");
  const en = sheetView({ kind: "install", plan }, "en");
  assert.equal(en.title, "Install Notes?");
  assert.equal(en.lines[1].detail, "Reminders · Asked when the app uses it");
});

test("removing says the data goes, and its button is destructive", () => {
  const view = sheetView({ kind: "uninstall", id: "so.oxy.notes", name: "Notes", version: "1.12.0" }, "en");
  assert.equal(view.title, "Delete Notes and its data?");
  assert.equal(view.accept, "Delete");
  assert.equal(view.destructive, true);
});

test("a consent lists what the update adds, and a rotated key", () => {
  const update = {
    id: "so.oxy.notes",
    name: "Notes",
    names: {},
    toVersion: "1.13.0",
    download: 40,
    releaseNotes: {},
    consent: { capabilities: ["oxy-account"], keyRotation: true },
  };
  const view = sheetView({ kind: "consent", update }, "en");
  assert.equal(view.title, "Notes asks for more access");
  assert.deepEqual(view.lines.map((l) => l.label), [
    "Your Oxy account",
    "The developer signs this version with a new key of its lineage.",
  ]);
});

test("a permission asks in the registry's name for the app", () => {
  const view = sheetView({ kind: "permission", id: "so.oxy.notes", capability: "notifications" }, "es", {
    "so.oxy.notes": "Notas",
  });
  assert.equal(view.title, "¿Permitir que Notas enviarte notificaciones?");
  assert.equal(view.accept, "Permitir");
  assert.equal(view.refuse, "No permitir");
});

test("sheets queue in order, once each, and go", () => {
  let queue = sheetArrived([], 1, '{"kind":"uninstall","id":"a","name":"A","version":"1"}');
  queue = sheetArrived(queue, 2, '{"kind":"permission","id":"b","capability":"camera"}');
  queue = sheetArrived(queue, 1, '{"kind":"uninstall","id":"a","name":"A","version":"1"}');
  queue = sheetArrived(queue, 3, "not json");
  assert.deepEqual(queue.map((s) => s.sheet), [1, 2]);
  assert.deepEqual(sheetGone(queue, 1).map((s) => s.sheet), [2]);
  assert.equal(readSheet("{}"), null);
});

test("sizes read as a person reads them", () => {
  assert.equal(size(71, "en"), "71 bytes");
  assert.equal(size(1_500_000, "es"), "1,5 MB");
  assert.equal(size(15_000_000, "en"), "15 MB");
});
