import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BACKUP_REASONS,
  backupBytes,
  backupDisksOf,
  backupFailureOf,
  backupPasswordProblem,
  backupReasonLabel,
  backupStateOf,
  backupWhen,
  canBackUp,
  destinationSummary,
  diskName,
  fileSystemName,
  pathName,
  progressFraction,
  progressKnown,
  progressLine,
  remainingLabel,
  shareOf,
  withExcludedPath,
  type BackupDestination,
  type BackupRunning,
  type BackupState,
} from "../src/backups.ts";
import { messagesFor } from "../src/messages.ts";
import { BACKUP_DEFAULTS, PREFERENCE_KEYS } from "../src/model.ts";

const en = messagesFor("en").backups;
const es = messagesFor("es").backups;

/** Moments in the local time zone, as the page's clock reads them, in seconds since the epoch. */
const local = (month: number, day: number, hour: number, minute: number) =>
  new Date(2026, month - 1, day, hour, minute).getTime() / 1000;
/** Six in the evening of 8 November 2026. */
const NOW = local(11, 8, 18, 0);
/** Intl's spaces (a 12-hour time's narrow no-break space) as plain ones. */
const plain = (text: string) => text.replace(/\s/g, " ");

const destination = (extra: Partial<BackupDestination> = {}): BackupDestination => ({
  id: "d1",
  name: "Backup",
  kind: "disk",
  url: null,
  available: true,
  lastBackup: local(11, 8, 14, 5),
  oldestBackup: local(11, 3, 10, 0),
  backups: 12,
  usedBytes: 1_234_567_890,
  freeBytes: 245_000_000_000,
  lastFailure: null,
  ...extra,
});

const running = (extra: Partial<BackupRunning> = {}): BackupRunning => ({
  destination: "d1",
  phase: "backingUp",
  asked: true,
  percent: 0.05,
  bytesDone: 12_345_678,
  bytesTotal: 240_000_000,
  filesDone: 120,
  filesTotal: 4000,
  secondsRemaining: 125,
  ...extra,
});

const state = (extra: Partial<BackupState> = {}): BackupState => ({
  automatic: true,
  destinations: [destination()],
  excluded: ["/home/ana/Videos"],
  running: null,
  lastBackup: local(11, 8, 14, 5),
  nextBackup: local(11, 8, 15, 5),
  held: false,
  administrator: true,
  inMenuBar: true,
  ...extra,
});

test("the session's state is read strictly", () => {
  const told = state({ running: running(), destinations: [destination({ lastFailure: { reason: "full", at: NOW } })] });
  assert.deepEqual(backupStateOf(JSON.parse(JSON.stringify(told))), told);
  // Unknown fields are not carried along.
  assert.deepEqual(backupStateOf({ ...told, extra: 1 }), told);
  // `{"backup": null}`: no service.
  assert.equal(backupStateOf(null), null);
  // The value of `backup`, not the message around it.
  assert.equal(backupStateOf({ backup: told }), null);
  for (const bad of [
    { ...told, automatic: "yes" },
    { ...told, excluded: ["/home", 3] },
    { ...told, held: undefined },
    { ...told, nextBackup: "soon" },
    { ...told, inMenuBar: null },
    { ...told, running: { ...running(), phase: "copying" } },
    { ...told, running: { ...running(), percent: "5" } },
    { ...told, running: { ...running(), secondsRemaining: undefined } },
    { ...told, destinations: [{ ...destination(), kind: "cloud" }] },
    { ...told, destinations: [{ ...destination(), url: undefined }] },
    { ...told, destinations: [{ ...destination(), usedBytes: Number.NaN }] },
    { ...told, destinations: [{ ...destination(), lastFailure: { reason: "full" } }] },
    { ...told, destinations: "d1" },
    [],
    "backup",
  ]) {
    assert.equal(backupStateOf(bad), null, JSON.stringify(bad));
  }
});

test("the disks are read strictly, and a request's answer says why it failed", () => {
  const disk = { uuid: "1234-ABCD", label: "Backup", fsType: "exfat", size: 500_107_862_016, added: false };
  assert.deepEqual(backupDisksOf({ backupDisks: [disk] }), [disk]);
  assert.deepEqual(backupDisksOf({ backupDisks: [] }), []);
  assert.equal(backupDisksOf({ backupDisks: [{ ...disk, added: "no" }] }), null);
  assert.equal(backupDisksOf({ backupDisks: [{ ...disk, label: null }] }), null);
  assert.equal(backupDisksOf({ refused: { reason: "notPermitted" } }), null);
  assert.equal(backupDisksOf(null), null);

  assert.equal(backupFailureOf({ backupNow: {} }, "backupNow"), null);
  assert.equal(backupFailureOf({ backupAddDisk: { id: "d2" } }, "backupAddDisk"), null);
  assert.equal(backupFailureOf({ backupStop: {} }, "backupNow"), "failed");
  assert.equal(backupFailureOf({ refused: { reason: "notPermitted" } }, "backupRemove"), "notPermitted");
  assert.equal(backupFailureOf({ refused: { reason: "needsInput" } }, "backupExcluded"), "needsInput");
  assert.equal(backupFailureOf({ failed: { reason: "unavailable" } }, "backupAutomatic"), "unavailable");
  assert.equal(backupFailureOf({ failed: { reason: "wrongPassword" } }, "backupAddShare"), "wrongPassword");
  assert.equal(backupFailureOf({ failed: {} }, "backupNow"), "failed");
  assert.equal(backupFailureOf(null, "backupNow"), "failed");
});

test("every reason has its words in English and Spanish, and one not known is a failure", () => {
  for (const words of [en, es]) {
    for (const reason of BACKUP_REASONS) assert.ok(words.reasons[reason], reason);
    assert.equal(Object.keys(words.reasons).length, BACKUP_REASONS.length);
  }
  assert.equal(backupReasonLabel("notConnected", en), "The backup disk is not connected.");
  assert.equal(backupReasonLabel("unreachable", en), "The computer with the share could not be reached.");
  assert.equal(backupReasonLabel("full", en), "The backup disk is full.");
  assert.equal(backupReasonLabel("shareRefused", en), "The share refused the user name or password.");
  assert.equal(backupReasonLabel("unavailable", en), "The backup service is not running.");
  assert.equal(backupReasonLabel("full", es), "El disco de copia de seguridad está lleno.");
  assert.equal(backupReasonLabel("notConnected", es), "El disco de copia de seguridad no está conectado.");
  assert.equal(backupReasonLabel("somethingNew", en), en.reasons.failed);
});

test("a backup's moment is said as Time Machine says it, in the locale's date and hour cycle", () => {
  assert.equal(backupWhen(local(11, 8, 14, 5), NOW, "en-GB", en), "Today, 14:05");
  assert.equal(backupWhen(local(11, 8, 14, 5), NOW, "es-ES", es), "Hoy, 14:05");
  assert.equal(plain(backupWhen(local(11, 8, 14, 5), NOW, "en-US", en)), "Today, 2:05 PM");
  assert.equal(backupWhen(local(11, 7, 9, 30), NOW, "en-GB", en), "Yesterday, 09:30");
  assert.equal(backupWhen(local(11, 7, 9, 30), NOW, "es-ES", es), "Ayer, 09:30");
  assert.equal(backupWhen(local(11, 9, 8, 0), NOW, "es-ES", es), "Mañana, 08:00");
  assert.equal(backupWhen(local(11, 3, 10, 0), NOW, "en-GB", en, false), "3 November 2026");
  assert.equal(backupWhen(local(11, 3, 10, 0), NOW, "es-ES", es, false), "3 de noviembre de 2026");
  assert.equal(backupWhen(local(11, 3, 10, 0), NOW, "en-GB", en), "3 November 2026 at 10:00");
  assert.equal(backupWhen(local(11, 3, 10, 0), NOW, "es-ES", es), "3 de noviembre de 2026, 10:00");
  // A twelve-hour clock chosen in a 24-hour region.
  assert.equal(plain(backupWhen(local(11, 8, 14, 5), NOW, "es-ES-u-hc-h12", es)).startsWith("Hoy, 2:05"), true);
});

test("sizes are decimal, three figures at most, in the locale's numbers", () => {
  assert.equal(backupBytes(1_234_567_890, "en", en), "1.2 GB");
  assert.equal(backupBytes(1_234_567_890, "es", es), "1,2 GB");
  assert.equal(backupBytes(12_345_678, "en", en), "12.3 MB");
  assert.equal(backupBytes(240_000_000, "en", en), "240 MB");
  assert.equal(backupBytes(245_000_000_000, "en", en), "245 GB");
  assert.equal(backupBytes(2_000_000_000_000, "en", en), "2 TB");
  // Rounding up to the next unit says the next unit.
  assert.equal(backupBytes(999_960, "en", en), "1 MB");
  assert.equal(backupBytes(1500, "en", en), "1.5 KB");
  assert.equal(backupBytes(512, "en", en), "512 bytes");
  assert.equal(backupBytes(1, "en", en), "1 byte");
  assert.equal(backupBytes(0, "en", en), "Zero KB");
  assert.equal(backupBytes(0, "es", es), "Cero KB");
});

test("a destination's lines: its latest and oldest backups, its space, and why it failed", () => {
  assert.deepEqual(destinationSummary(destination(), NOW, "en-GB", en), {
    status: null,
    latest: "Latest backup: Today, 14:05",
    oldest: "Oldest backup: 3 November 2026",
    space: "1.2 GB used, 245 GB available",
    failure: null,
  });
  assert.deepEqual(destinationSummary(destination(), NOW, "es-ES", es), {
    status: null,
    latest: "Última copia de seguridad: Hoy, 14:05",
    oldest: "Copia de seguridad más antigua: 3 de noviembre de 2026",
    space: "1,2 GB usados, 245 GB disponibles",
    failure: null,
  });
  // A new disk: nothing backed up yet, nothing known of its space but what is free; not connected.
  const fresh = destination({
    available: false,
    lastBackup: null,
    oldestBackup: null,
    backups: 0,
    usedBytes: null,
    lastFailure: { reason: "notConnected", at: NOW - 60 },
  });
  assert.deepEqual(destinationSummary(fresh, NOW, "en-GB", en), {
    status: "Not connected",
    latest: "No backups yet",
    oldest: null,
    space: "245 GB available",
    failure: "Backup failed. The backup disk is not connected.",
  });
  // One backup: its oldest is its latest, said once.
  assert.equal(destinationSummary(destination({ backups: 1, oldestBackup: local(11, 8, 14, 5) }), NOW, "en-GB", en).oldest, null);
  // A failure before the latest backup is over.
  const recovered = destination({ lastFailure: { reason: "full", at: local(11, 8, 13, 0) } });
  assert.equal(destinationSummary(recovered, NOW, "en-GB", en).failure, null);
  const failed = destination({ lastFailure: { reason: "full", at: local(11, 8, 15, 5) } });
  assert.equal(
    destinationSummary(failed, NOW, "es-ES", es).failure,
    "No se ha podido realizar la copia de seguridad. El disco de copia de seguridad está lleno.",
  );
  // A share that cannot be reached.
  assert.equal(destinationSummary(destination({ kind: "share", url: "smb://nas/b", available: false }), NOW, "en", en).status, "Can't be reached");
});

test("a running backup says how far and how long is left, and each phase", () => {
  assert.equal(progressLine(running(), "en", en), "Backing up: 12.3 MB of 240 MB – About 2 minutes remaining");
  assert.equal(
    progressLine(running(), "es", es),
    "Realizando copia de seguridad: 12,3 MB de 240 MB – Quedan aproximadamente 2 minutos",
  );
  assert.equal(progressLine(running({ secondsRemaining: null }), "en", en), "Backing up: 12.3 MB of 240 MB");
  assert.equal(progressLine(running({ phase: "preparing", bytesTotal: 0 }), "en", en), "Preparing Backup…");
  assert.equal(progressLine(running({ phase: "cleaningUp" }), "en", en), "Cleaning Up…");
  assert.equal(progressLine(running({ phase: "checking" }), "en", en), "Verifying Backups…");
  assert.equal(progressLine(running({ phase: "checking" }), "es", es), "Verificando copias de seguridad…");
  assert.equal(progressLine(running({ phase: "preparing" }), "es", es), "Preparando copia de seguridad…");
  // Copying before its size is known.
  assert.equal(progressLine(running({ bytesTotal: 0 }), "es", es), "Realizando copia de seguridad…");
  assert.equal(remainingLabel(30, en), "Less than a minute remaining");
  assert.equal(remainingLabel(60, en), "About a minute remaining");
  assert.equal(remainingLabel(3600, en), "About 1 hour remaining");
  assert.equal(remainingLabel(4800, en), "About 1 hour, 20 minutes remaining");
  assert.equal(remainingLabel(30, es), "Queda menos de un minuto");
  assert.equal(remainingLabel(60, es), "Queda aproximadamente 1 minuto");
  assert.equal(remainingLabel(7200, es), "Quedan aproximadamente 2 horas");
  assert.equal(remainingLabel(4800, es), "Quedan aproximadamente 1 hora y 20 minutos");
  // The bar: a fraction, indeterminate while it prepares, cleans up or verifies.
  assert.equal(progressKnown(running()), true);
  assert.equal(progressKnown(running({ phase: "preparing" })), false);
  assert.equal(progressFraction(running({ percent: 1.2 })), 1);
  assert.equal(progressFraction(running({ percent: -0.1 })), 0);
});

test("a backup password has eight characters at least and is typed twice the same", () => {
  assert.equal(backupPasswordProblem("", ""), "tooShort");
  assert.equal(backupPasswordProblem("1234567", "1234567"), "tooShort");
  assert.equal(backupPasswordProblem("12345678", "1234567"), "mismatch");
  assert.equal(backupPasswordProblem("12345678", "12345678"), null);
  // Counted as the service counts them: characters, not UTF-16 units.
  assert.equal(backupPasswordProblem("ñandúçé", "ñandúçé"), "tooShort");
  assert.equal(backupPasswordProblem("😀".repeat(7), "😀".repeat(7)), "tooShort");
  assert.equal(backupPasswordProblem("😀".repeat(8), "😀".repeat(8)), null);
  assert.equal(en.passwordRule, "At least 8 characters.");
  assert.equal(es.passwordRule, "Al menos 8 caracteres.");
});

test("a share is what the form names, and a disk is named by its label or its size", () => {
  assert.deepEqual(shareOf({ host: " nas.local ", share: "Backups", user: "ana", domain: "" }), {
    host: "nas.local",
    share: "Backups",
    user: "ana",
  });
  assert.deepEqual(shareOf({ host: "nas", share: "B", user: "ana", domain: "WORK" }), {
    host: "nas",
    share: "B",
    user: "ana",
    domain: "WORK",
  });
  // Typed as a server's address.
  assert.deepEqual(shareOf({ host: "smb://nas.local/Backups/", share: "", user: "ana", domain: "" }), {
    host: "nas.local",
    share: "Backups",
    user: "ana",
  });
  assert.equal(shareOf({ host: "nas", share: "", user: "ana", domain: "" }), null);
  assert.equal(shareOf({ host: "nas", share: "B", user: " ", domain: "" }), null);
  const disk = { uuid: "u", label: "", fsType: "exfat", size: 500_107_862_016, added: false };
  assert.equal(diskName(disk, "en", en), "500 GB Disk");
  assert.equal(diskName(disk, "es", es), "Disco de 500 GB");
  assert.equal(diskName({ ...disk, label: "Backup" }, "en", en), "Backup");
  assert.equal(fileSystemName("exfat"), "exFAT");
  assert.equal(fileSystemName("btrfs"), "Btrfs");
  assert.equal(fileSystemName("ntfs"), "ntfs");
});

test("Back Up Now needs a destination it can reach, and excluded items are kept once", () => {
  assert.equal(canBackUp(state()), true);
  assert.equal(canBackUp(state({ destinations: [destination({ available: false })] })), false);
  assert.equal(canBackUp(state({ destinations: [] })), false);
  assert.deepEqual(withExcludedPath(["/home/ana/Videos"], "/home/ana/Videos/"), ["/home/ana/Videos"]);
  assert.deepEqual(withExcludedPath(["/home/ana/Videos"], "/srv/vm"), ["/home/ana/Videos", "/srv/vm"]);
  assert.equal(pathName("/home/ana/Mis cosas/"), "Mis cosas");
  assert.equal(pathName("/"), "/");
});

test("the menu bar's backup item is a preference of the session's, on by default", () => {
  assert.equal(PREFERENCE_KEYS.backupInMenuBar, "backup_in_menu_bar");
  assert.deepEqual(BACKUP_DEFAULTS, { backupInMenuBar: true });
  assert.equal(en.inMenuBar, "Show Backups in menu bar");
  assert.equal(es.inMenuBar, "Mostrar las copias de seguridad en la barra de menús");
  assert.equal(messagesFor("en").pages.backups, "Backups");
  assert.equal(messagesFor("es").pages.backups, "Copias de seguridad");
});
