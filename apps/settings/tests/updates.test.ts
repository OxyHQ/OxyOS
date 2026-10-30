import { test } from "node:test";
import assert from "node:assert/strict";
import { formatSize, privacyRows, updatable, updateRows, type UpdateState } from "../src/updates.ts";

const state: UpdateState = {
  checking: false,
  checkedAt: "2026-10-30T12:00:00Z",
  checkFailed: null,
  automatic: true,
  updates: [
    {
      id: "so.oxy.notes",
      name: "Notes",
      names: { es: "Notas" },
      fromVersion: "1.12.0",
      toVersion: "1.13.0",
      toVersionCode: 13,
      download: 1_500_000,
      releaseNotes: { en: "Faster sync", es: "Sincroniza más rápido" },
      status: "awaitingConsent",
      consent: { capabilities: ["oxy-account"], keyRotation: false },
    },
    {
      id: "so.oxy.allo",
      name: "Allo",
      names: {},
      fromVersion: "2.0.0",
      toVersion: "2.1.0",
      toVersionCode: 21,
      download: 71,
      releaseNotes: {},
      status: "atNextLaunch",
      consent: null,
    },
  ],
};

test("each update is a row in the person's language, saying what it waits for", () => {
  const rows = updateRows(state, "es");
  assert.deepEqual(rows[0], {
    id: "so.oxy.notes",
    name: "Notas",
    version: "1.13.0",
    size: 1_500_000,
    notes: "Sincroniza más rápido",
    needsConsent: true,
    atNextLaunch: false,
  });
  assert.equal(updateRows(state, "en")[0].name, "Notes");
  assert.equal(rows[1].atNextLaunch, true);
  assert.deepEqual(updatable(rows).map((r) => r.id), ["so.oxy.notes"]);
  assert.deepEqual(updateRows(null, "en"), []);
});

test("sizes and privacy rows", () => {
  assert.equal(formatSize(1_500_000, "es"), "1,5 MB");
  assert.equal(formatSize(71, "en"), "71 bytes");
  assert.deepEqual(
    privacyRows({
      id: "so.oxy.notes",
      name: "Notes",
      capabilities: { network: "granted", camera: "denied", notifications: "undecided" },
    }),
    [
      { capability: "notifications", granted: false, decided: false },
      { capability: "camera", granted: false, decided: true },
    ],
  );
});
