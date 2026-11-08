import { test } from "node:test";
import assert from "node:assert/strict";
import {
  afterBackups,
  backupDate,
  computersFrom,
  diskName,
  disksFrom,
  migratedFrom,
  problemOf,
  progressFrom,
  shareFrom,
} from "../src/migrate.ts";
import { loginMessages } from "../src/messages.ts";

const listing = {
  backups: {
    computers: [
      {
        repository: "0123456789abcdef",
        name: "ada-laptop",
        backups: [
          { snapshot: "aaaa1111", time: 1_793_000_000 },
          { snapshot: "bbbb2222", time: 1_794_000_000 },
        ],
        accounts: [
          { user: "oxy-a", name: "Ada Lovelace", administrator: true },
          { user: "oxy-b", name: " ", administrator: false },
        ],
      },
      { repository: "fedcba9876543210", name: "old-desktop", backups: [{ snapshot: "cccc3333", time: 1_700_000_000 }], accounts: [] },
      // A computer with no backup of a machine is none to choose.
      { repository: "1111111111111111", name: "empty", backups: [], accounts: [] },
    ],
    unopened: 1,
  },
};

test("the service's answers are read strictly", () => {
  assert.deepEqual(disksFrom({ disks: [{ uuid: "1234-ABCD", label: "Backups", fsType: "ext4", size: 2e12 }, { label: "no uuid" }] }), [
    { uuid: "1234-ABCD", label: "Backups", size: 2e12 },
  ]);
  assert.equal(disksFrom({ refused: { reason: "alreadySetUp" } }), "alreadySetUp");
  assert.equal(disksFrom({ failed: { reason: "unreachable socket" } }), "failed");
  assert.equal(disksFrom(null), "failed");
  assert.equal(problemOf({ refused: { reason: "somethingNew" } }), "failed");
  assert.deepEqual(progressFrom({ migrating: { step: "files", percent: 1.4 } }), { step: "files", percent: 1 });
  assert.equal(progressFrom({ migrating: { step: "erasing", percent: 0.1 } }), null);
  assert.equal(progressFrom({ migrated: { computer: "x", accounts: [] } }), null);
  assert.deepEqual(migratedFrom({ migrated: { computer: "ada-laptop", accounts: [{ user: "oxy-a", name: "" }] } }), {
    computer: "ada-laptop",
    accounts: [{ user: "oxy-a", name: "oxy-a" }],
  });
  assert.equal(migratedFrom({ refused: { reason: "clash" } }), "clash");
});

test("computers come the most recently backed up first, their backups newest first", () => {
  const computers = computersFrom(listing);
  assert.ok(Array.isArray(computers));
  assert.deepEqual(
    computers.map((computer) => computer.name),
    ["ada-laptop", "old-desktop"],
  );
  assert.deepEqual(
    computers[0]!.backups.map((backup) => backup.snapshot),
    ["bbbb2222", "aaaa1111"],
  );
  assert.equal(computers[0]!.accounts[1]!.name, "oxy-b");
  // None opened: the password, when others' were there; else no backups at all.
  assert.equal(computersFrom({ backups: { computers: [], unopened: 2 } }), "wrongPassword");
  assert.equal(computersFrom({ backups: { computers: [], unopened: 0 } }), "noBackup");
});

test("the backup password leads on to the computer, or back with what was wrong", () => {
  const at = { step: "password", source: { disk: { uuid: "1234-ABCD" } }, place: "Backups", problem: null } as const;
  assert.equal(afterBackups(listing, at).step, "computer");
  const one = { backups: { computers: [listing.backups.computers[1]], unopened: 0 } };
  const next = afterBackups(one, at);
  assert.equal(next.step, "backup");
  assert.equal(next.step === "backup" && next.several, null);
  assert.deepEqual(afterBackups({ refused: { reason: "wrongPassword" } }, at), { ...at, problem: "wrongPassword" });
  assert.deepEqual(afterBackups({ refused: { reason: "notConnected" } }, at), {
    step: "source",
    disks: null,
    problem: "notConnected",
  });
});

test("a disk is called by its label, else its size; a backup by its date", () => {
  assert.equal(diskName({ uuid: "x", label: "Time Capsule", size: 0 }, "en"), "Time Capsule");
  assert.equal(diskName({ uuid: "x", label: "", size: 2_000_000_000_000 }, "en"), "2,000 GB");
  assert.equal(diskName({ uuid: "x", label: "", size: 1_500_000_000 }, "es"), "1,5 GB");
  assert.match(backupDate(1_794_000_000, "en"), /2026/);
});

test("a share is taken once each part is given", () => {
  assert.deepEqual(shareFrom(" nas.local ", "Backups", "ada"), { host: "nas.local", share: "Backups", user: "ada" });
  assert.equal(shareFrom("nas local", "Backups", "ada"), null);
  assert.equal(shareFrom("nas", "a/b", "ada"), null);
  assert.equal(shareFrom("nas", "Backups", ""), null);
});

test("migration's words in both languages", () => {
  assert.equal(loginMessages("en").migrate.offer, "Migrate from a Backup");
  assert.equal(loginMessages("es").migrate.offer, "Migrar desde una copia de seguridad");
  assert.equal(loginMessages("en").migrate.list(["Ada", "Bea", "Carmen"]), "Ada, Bea and Carmen");
  assert.equal(loginMessages("es").migrate.list(["Ada", "Bea"]), "Ada y Bea");
  assert.equal(loginMessages("es").migrate.migrated(2), "Se han traído 2 cuentas");
});
