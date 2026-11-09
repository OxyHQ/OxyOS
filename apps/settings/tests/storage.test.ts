import { test } from "node:test";
import assert from "node:assert/strict";
import {
  appTotal,
  indexedOf,
  largeFiles,
  measuredOf,
  removeOutcomeOf,
  resetOutcomeOf,
  SETTINGS_ID,
  STORAGE_CATEGORIES,
  storageBar,
  unusedApps,
  UNUSED_AFTER_DAYS,
  type StorageApp,
  type StorageMeasured,
} from "../src/storage.ts";
import { spaceNeeded } from "../src/updates.ts";
import { messagesFor } from "../src/messages.ts";
import { PREFERENCE_KEYS } from "../src/model.ts";

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.UTC(2026, 10, 8);

const app = (id: string, over: Partial<StorageApp> = {}): StorageApp => ({
  id,
  name: id,
  origin: "installed",
  package: 1_000,
  data: 2_000,
  caches: 500,
  lastUsed: NOW - DAY,
  removable: true,
  ...over,
});

const measured: StorageMeasured = {
  name: "OxyOS",
  capacity: 1_000_000,
  available: 300_000,
  used: 650_000,
  reserved: 50_000,
  system: 200_000,
  trash: 10_000,
  apps: [app("so.oxy.mention", { package: 40_000, data: 50_000, caches: 10_000 })],
};

const indexed = {
  kinds: {
    image: { files: 3, bytes: 60_000 },
    movie: { files: 1, bytes: 40_000 },
    pdf: { files: 2, bytes: 30_000 },
    text: { files: 9, bytes: 20_000 },
  },
  indexing: false,
  partial: false,
};

test("the bar's categories add up to what is used and the reserve, and with what is free to the volume", () => {
  const bar = storageBar(measured, indexed);
  const bytes = Object.fromEntries(bar.segments.map(({ key, bytes }) => [key, bytes]));
  assert.deepEqual(bar.segments.map(({ key }) => key), [...STORAGE_CATEGORIES]);
  assert.deepEqual(bytes, {
    apps: 100_000,
    documents: 50_000,
    // The system's image and the administrator's reserve.
    system: 250_000,
    photos: 100_000,
    trash: 10_000,
    // What no category names: 650 000 used − 460 000 named.
    other: 190_000,
  });
  const total = bar.segments.reduce((sum, { bytes }) => sum + bytes, 0);
  assert.equal(total, measured.used + measured.reserved);
  assert.equal(total + bar.available, bar.capacity);
});

test("what is measured never says more than the volume holds", () => {
  // Files sharing blocks: the index's sums exceed what is used. Taken in order, each up to what is
  // left: the Trash, the apps, the photos, then documents and the system's estimate get the rest.
  const shared = { ...measured, used: 150_000 };
  const bar = storageBar(shared, indexed);
  const bytes = Object.fromEntries(bar.segments.map(({ key, bytes }) => [key, bytes]));
  assert.deepEqual(bytes, { trash: 10_000, apps: 100_000, photos: 40_000, documents: 0, system: 50_000, other: 0 });
  assert.equal(
    bar.segments.reduce((sum, { bytes }) => sum + bytes, 0),
    shared.used + shared.reserved,
  );
  // Without the index, its files are the rest's.
  const unindexed = Object.fromEntries(storageBar(measured, null).segments.map(({ key, bytes }) => [key, bytes]));
  assert.equal(unindexed.documents, 0);
  assert.equal(unindexed.photos, 0);
  assert.equal(unindexed.other, 340_000);
});

test("an app is its package and its own data, and unused once not opened in 90 days", () => {
  assert.equal(appTotal(app("a")), 3_500);
  assert.equal(appTotal(app("a", { package: null })), 2_500);
  const apps = [
    app("recent"),
    app("old-small", { lastUsed: NOW - (UNUSED_AFTER_DAYS + 1) * DAY, data: 10 }),
    app("old-large", { lastUsed: NOW - 400 * DAY, data: 900_000 }),
    // Exactly 90 days: unused.
    app("ninety", { lastUsed: NOW - UNUSED_AFTER_DAYS * DAY }),
    // Never removable (a system application), or of unknown use (a development entry): never.
    app("system", { lastUsed: NOW - 400 * DAY, removable: false, origin: "system" }),
    app("unknown", { lastUsed: null }),
  ];
  assert.deepEqual(
    unusedApps(apps, NOW).map(({ id }) => id),
    ["old-large", "ninety", "old-small"],
  );
});

test("Review Files lists the index's files with a place and a size, the largest first, each once", () => {
  const files = largeFiles([
    { id: "a", uri: "file:///home/ana/a.iso", title: "a.iso", size: 10, location: "Downloads", modified: 5, category: "archive" },
    { id: "b", uri: "file:///home/ana/b.mov", title: "b.mov", size: 90, location: "Movies", category: "movie" },
    { id: "b", uri: "file:///home/ana/b.mov", title: "b.mov", size: 90, location: "Movies", category: "movie" },
    // A folder, or an item without a size, is no file to review.
    { id: "c", uri: "file:///home/ana/Old/", title: "Old", size: 1, category: "folder" },
    { id: "d", uri: "file:///home/ana/d", title: "d" },
  ]);
  assert.deepEqual(files, [
    { uri: "file:///home/ana/b.mov", name: "b.mov", location: "Movies", size: 90, modified: null, category: "movie" },
    { uri: "file:///home/ana/a.iso", name: "a.iso", location: "Downloads", size: 10, modified: 5, category: "archive" },
  ]);
});

test("the session's answers are read strictly", () => {
  const answer = {
    storage: {
      name: "OxyOS",
      capacity: 10,
      available: 4,
      used: 5,
      reserved: 1,
      system: 2,
      trash: 1,
      apps: [
        { id: "so.oxy.allo", name: "Allo", origin: "development", package: null, data: 1, caches: 0, lastUsed: null, removable: false },
        { name: "no id" },
      ],
    },
  };
  const read = measuredOf(answer);
  assert.equal(read?.apps.length, 1);
  assert.equal(read?.apps[0].origin, "development");
  assert.equal(read?.capacity, 10);
  assert.equal(measuredOf({ refused: { reason: "notPermitted" } }), null);
  assert.equal(measuredOf(null), null);
  assert.equal(indexedOf({ kinds: { image: { files: 1, bytes: 4096 } }, indexing: true })?.indexing, true);
  assert.equal(indexedOf({ refused: {} }), null);
  assert.equal(removeOutcomeOf({ uninstalled: { id: "x" } }), "removed");
  assert.equal(removeOutcomeOf({ cancelled: {} }), "cancelled");
  assert.equal(removeOutcomeOf({ refused: { reason: "needsInput" } }), "failed");
  assert.equal(removeOutcomeOf(null), "failed");
});

test("an update refused for want of room says how much more is needed", () => {
  assert.equal(spaceNeeded({ id: "a", outcome: "failed", needed: 2_100_000_000 }), 2_100_000_000);
  assert.equal(spaceNeeded({ id: "a", outcome: "failed" }), null);
  assert.equal(spaceNeeded({ id: "a", outcome: "switched", needed: 5 }), null);
  assert.equal(spaceNeeded(undefined), null);
  assert.equal(messagesFor("en").softwareUpdate.noSpace("2.1 GB"), "2.1 GB more is needed to download this update.");
  assert.equal(
    messagesFor("es").softwareUpdate.noSpace("2,1 GB"),
    "Se necesitan 2,1 GB más para descargar esta actualización.",
  );
});

test("Storage speaks English and Spanish, and Empty Trash Automatically is the session's preference", () => {
  assert.equal(PREFERENCE_KEYS.emptyTrashAutomatically, "empty_trash_automatically");
  assert.equal(messagesFor("en").pages.storage, "Storage");
  assert.equal(messagesFor("es").pages.storage, "Almacenamiento");
  assert.equal(messagesFor("en").storage.unusedAppsDescription(1, "3 GB"), "An app you haven't opened in 3 months uses 3 GB.");
  assert.equal(
    messagesFor("es").storage.unusedAppsDescription(2, "3 GB"),
    "2 apps que no has abierto en 3 meses ocupan 3 GB.",
  );
  assert.equal(messagesFor("en").storage.fileLine("Movies", "8.6 GB", ""), "Movies · 8.6 GB");
});

test("a reset's answer: done, cancelled in the system's sheet, or failed; never Settings' own", () => {
  // docs/adr/0061-crashes-and-resetting-an-application.mdx: the session's vocabulary.
  assert.equal(resetOutcomeOf({ reset: { id: "so.oxy.crashprobe" } }), "reset");
  assert.equal(resetOutcomeOf({ refused: { reason: "cancelled" } }), "cancelled");
  assert.equal(resetOutcomeOf({ refused: { reason: "notPermitted" } }), "failed");
  assert.equal(resetOutcomeOf({ failed: { reason: "it has no data directory" } }), "failed");
  assert.equal(resetOutcomeOf(null), "failed");
  assert.equal(SETTINGS_ID, "so.oxy.settings");
  const en = messagesFor("en").storage;
  const es = messagesFor("es").storage;
  assert.equal(en.resetApp, "Reset…");
  assert.equal(es.resetApp, "Restablecer…");
  assert.equal(en.resetFailed("Mention"), "Mention couldn't be reset.");
  assert.equal(es.resetDone("Mention"), "Mention restablecida");
});
