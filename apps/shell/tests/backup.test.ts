import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages } from "../src/messages.ts";
import {
  backupAvailable,
  backupBytes,
  backupHandAngle,
  backupMenuLines,
  backupSaid,
  backupStateOf,
  showsBackupItem,
} from "../src/backup.ts";
import { statusItemOrder } from "../src/controls.ts";
import { clockTime } from "../src/clock.ts";
import type { ShellBackupDestination, ShellBackupRunning, ShellBackupState } from "../src/model.ts";

const en = shellMessages("en-GB");
const es = shellMessages("es-ES");

/** Seconds since the epoch of a local time, as the session tells it. */
const at = (year: number, month: number, day: number, hour: number, minute: number) =>
  new Date(year, month - 1, day, hour, minute).getTime() / 1000;
/** Sunday, 8 November 2026, 16:00, local. */
const NOW = new Date(2026, 10, 8, 16, 0);

const disk = (change: Partial<ShellBackupDestination> = {}): ShellBackupDestination => ({
  id: "d1",
  name: "Archivo",
  kind: "disk",
  url: null,
  available: true,
  lastBackup: at(2026, 11, 8, 14, 5),
  oldestBackup: at(2026, 10, 1, 9, 0),
  backups: 40,
  usedBytes: 120_000_000_000,
  freeBytes: 380_000_000_000,
  lastFailure: null,
  ...change,
});
const state = (change: Partial<ShellBackupState> = {}): ShellBackupState => ({
  automatic: true,
  destinations: [disk()],
  excluded: [],
  running: null,
  lastBackup: at(2026, 11, 8, 14, 5),
  nextBackup: at(2026, 11, 8, 15, 5),
  held: false,
  administrator: true,
  inMenuBar: true,
  ...change,
});
const running = (change: Partial<ShellBackupRunning> = {}): ShellBackupRunning => ({
  destination: "d1",
  phase: "backingUp",
  asked: false,
  percent: 0.45,
  bytesDone: 12_300_000,
  bytesTotal: 240_000_000,
  filesDone: 120,
  filesTotal: 900,
  secondsRemaining: 125,
  ...change,
});

test("the state the session tells is read strictly", () => {
  const told = JSON.parse(JSON.stringify(state({ running: running(), excluded: ["/home/ana/Películas"] })));
  assert.deepEqual(backupStateOf(told), state({ running: running(), excluded: ["/home/ana/Películas"] }));
  // A share with its URL and a failure.
  const share = disk({ id: "s1", kind: "share", url: "smb://nas/backups", lastFailure: { reason: "unreachable", at: 1 } });
  assert.deepEqual(backupStateOf({ ...told, destinations: [share] })?.destinations, [share]);
  // Fields the shell does not read are left behind.
  assert.equal("extra" in (backupStateOf({ ...told, extra: 1 }) as object), false);
  // No service, or not a state.
  assert.equal(backupStateOf(null), null);
  assert.equal(backupStateOf([]), null);
  assert.equal(backupStateOf("backup"), null);
  // Each field missing or of another type.
  for (const field of Object.keys(told)) {
    const without = { ...told };
    delete without[field];
    assert.equal(backupStateOf(without), null, `without ${field}`);
    assert.equal(backupStateOf({ ...told, [field]: "x" }), null, `${field} a string`);
  }
  const bad = (destination: Record<string, unknown>) => backupStateOf({ ...told, destinations: [destination] });
  assert.equal(bad({ ...disk(), kind: "cloud" }), null);
  assert.equal(bad({ ...disk(), lastBackup: "today" }), null);
  assert.equal(bad({ ...disk(), backups: -1 }), null);
  assert.equal(bad({ ...disk(), usedBytes: Number.NaN }), null);
  assert.equal(bad({ ...disk(), lastFailure: { reason: "full" } }), null);
  assert.equal(bad({ ...disk(), url: 3 }), null);
  const now = (run: Record<string, unknown>) => backupStateOf({ ...told, running: run });
  assert.equal(now({ ...running(), phase: "copying" }), null);
  assert.equal(now({ ...running(), percent: 45 }), null);
  assert.equal(now({ ...running(), secondsRemaining: "soon" }), null);
  assert.equal(now({ ...running(), asked: 1 }), null);
  assert.equal(backupStateOf({ ...told, excluded: [1] }), null);
});

test("the item is shown once a destination exists, while the person keeps it in the menu bar", () => {
  assert.equal(showsBackupItem(state()), true);
  assert.equal(showsBackupItem(state({ inMenuBar: false })), false);
  assert.equal(showsBackupItem(state({ destinations: [] })), false);
  assert.equal(showsBackupItem(null), false);
  assert.equal(showsBackupItem(undefined), false);
});

test("the icon's minute hand is still while idle and shows a running backup's progress", () => {
  assert.equal(backupHandAngle(null), 0);
  assert.equal(backupHandAngle(running({ percent: 0 })), 0);
  assert.equal(backupHandAngle(running({ percent: 0.25 })), 90);
  assert.equal(backupHandAngle(running({ percent: 0.45 })), 162);
  assert.equal(backupHandAngle(running({ percent: 1 })), 360);
});

test("sizes are said in decimal units, as a Mac says them", () => {
  assert.equal(backupBytes(0, en), "0 bytes");
  assert.equal(backupBytes(1, en), "1 byte");
  assert.equal(backupBytes(999, en), "999 bytes");
  assert.equal(backupBytes(1_000, en), "1 KB");
  assert.equal(backupBytes(45_600, en), "46 KB");
  assert.equal(backupBytes(999_960, en), "1 MB");
  assert.equal(backupBytes(12_300_000, en), "12.3 MB");
  assert.equal(backupBytes(12_300_000, es), "12,3 MB");
  assert.equal(backupBytes(240_000_000, en), "240 MB");
  assert.equal(backupBytes(2_000_000_000, en), "2 GB");
  assert.equal(backupBytes(1_234_000_000_000, es), "1,2 TB");
});

test("a running backup's menu says its phase and how far it is", () => {
  const lines = (run: Partial<ShellBackupRunning>, messages = en) =>
    backupMenuLines(state({ running: running(run) }), NOW, messages);
  assert.deepEqual(lines({}), ["Backing Up…", "12.3 MB of 240 MB – About 2 minutes remaining"]);
  assert.deepEqual(lines({}, es), ["Realizando copia de seguridad…", "12,3 MB de 240 MB – Quedan unos 2 minutos"]);
  assert.deepEqual(lines({ secondsRemaining: null }), ["Backing Up…", "12.3 MB of 240 MB"]);
  assert.deepEqual(lines({ bytesTotal: 0, bytesDone: 0 }), ["Backing Up…"]);
  assert.equal(lines({ secondsRemaining: 30 })[1], "12.3 MB of 240 MB – Less than a minute remaining");
  assert.equal(lines({ secondsRemaining: 60 })[1], "12.3 MB of 240 MB – About 1 minute remaining");
  assert.equal(lines({ secondsRemaining: 3_700 }, es)[1], "12,3 MB de 240 MB – Queda 1 hora aproximadamente");
  assert.equal(lines({ secondsRemaining: 5 * 3_600 })[1], "12.3 MB of 240 MB – About 5 hours remaining");
  assert.equal(lines({ secondsRemaining: 3 * 86_400 }, es)[1], "12,3 MB de 240 MB – Quedan unos 3 días");
  assert.deepEqual(lines({ phase: "preparing" }), ["Preparing Backup…"]);
  assert.deepEqual(lines({ phase: "preparing" }, es), ["Preparando copia de seguridad…"]);
  assert.deepEqual(lines({ phase: "cleaningUp" }), ["Cleaning Up…"]);
  assert.deepEqual(lines({ phase: "cleaningUp" }, es), ["Limpiando…"]);
  assert.deepEqual(lines({ phase: "checking" }), ["Verifying Backups…"]);
  assert.deepEqual(lines({ phase: "checking" }, es), ["Verificando copias de seguridad…"]);
  // Its value.
  assert.equal(backupSaid(state({ running: running() }), NOW, en), "Backing up, 45%");
  assert.equal(backupSaid(state({ running: running() }), NOW, es), "Realizando copia de seguridad, 45 %");
  assert.equal(backupSaid(state({ running: running({ phase: "checking" }) }), NOW, es), "Verificando copias de seguridad");
});

test("when nothing runs, the menu says the latest backup and when, in the shell's clock", () => {
  const time = (hour: number, minute: number, locale: string) => clockTime(new Date(2026, 10, 8, hour, minute), locale);
  assert.deepEqual(backupMenuLines(state(), NOW, en), ["Latest Backup to “Archivo”:", `Today, ${time(14, 5, "en-GB")}`]);
  assert.deepEqual(backupMenuLines(state(), NOW, es), [
    "Última copia de seguridad en “Archivo”:",
    `Hoy, ${time(14, 5, "es-ES")}`,
  ]);
  assert.equal(time(14, 5, "es-ES"), "14:05");
  const yesterday = state({ destinations: [disk({ lastBackup: at(2026, 11, 7, 9, 30) })] });
  assert.deepEqual(backupMenuLines(yesterday, NOW, en)[1], `Yesterday, ${time(9, 30, "en-GB")}`);
  assert.deepEqual(backupMenuLines(yesterday, NOW, es)[1], `Ayer, ${time(9, 30, "es-ES")}`);
  const older = state({ destinations: [disk({ lastBackup: at(2026, 11, 3, 14, 5) })] });
  assert.equal(backupMenuLines(older, NOW, en)[1], "3 November 2026, 14:05");
  assert.equal(backupMenuLines(older, NOW, es)[1], "3 de noviembre de 2026, 14:05");
  // The destination backed up last, of several.
  const two = state({ destinations: [disk({ lastBackup: at(2026, 11, 7, 9, 30) }), disk({ id: "d2", name: "NAS", kind: "share" })] });
  assert.equal(backupMenuLines(two, NOW, en)[0], "Latest Backup to “NAS”:");
  // Its value.
  assert.equal(backupSaid(state(), NOW, en), `Latest backup today, ${time(14, 5, "en-GB")}`);
  assert.equal(backupSaid(yesterday, NOW, es), `Última copia de seguridad ayer, ${time(9, 30, "es-ES")}`);
  assert.equal(backupSaid(older, NOW, en), "Latest backup on 3 November 2026, 14:05");
  assert.equal(backupSaid(older, NOW, es), "Última copia de seguridad el 3 de noviembre de 2026, 14:05");
});

test("a destination never backed up, a failure, a disk not connected and Low Power Mode", () => {
  const never = state({ destinations: [disk({ lastBackup: null, oldestBackup: null, backups: 0 })], lastBackup: null });
  assert.deepEqual(backupMenuLines(never, NOW, en), ["No Backups Yet"]);
  assert.deepEqual(backupMenuLines(never, NOW, es), ["Aún no hay copias de seguridad"]);
  assert.equal(backupSaid(never, NOW, en), "No backups yet");

  // The latest attempt failed after the latest backup.
  const full = state({ destinations: [disk({ lastFailure: { reason: "full", at: at(2026, 11, 8, 15, 5) } })] });
  assert.deepEqual(backupMenuLines(full, NOW, en).slice(2), ["Latest Backup Failed", "The backup disk is full."]);
  assert.deepEqual(backupMenuLines(full, NOW, es).slice(2), [
    "La última copia de seguridad ha fallado",
    "El disco de copia de seguridad está lleno.",
  ]);
  assert.equal(backupSaid(full, NOW, en), "Latest backup failed");
  // A failure older than the latest backup is past.
  const past = state({ destinations: [disk({ lastFailure: { reason: "full", at: at(2026, 11, 8, 13, 5) } })] });
  assert.equal(backupMenuLines(past, NOW, en).length, 2);
  // Never backed up, failed: why, alone.
  const share = state({
    destinations: [disk({ kind: "share", lastBackup: null, lastFailure: { reason: "unreachable", at: at(2026, 11, 8, 15, 5) } })],
  });
  assert.deepEqual(backupMenuLines(share, NOW, en), [
    "Latest Backup Failed",
    "The computer with the share could not be reached.",
  ]);
  // A reason the shell does not know, and one that is no reason, are said as a failure.
  for (const reason of ["somethingNew", "constructor"]) {
    const odd = state({ destinations: [disk({ lastFailure: { reason, at: at(2026, 11, 8, 15, 5) } })] });
    assert.equal(backupMenuLines(odd, NOW, en)[3], "The backup could not be completed.");
  }

  // The disk taken away: said, and nothing to back up now.
  const away = state({ destinations: [disk({ available: false })] });
  assert.deepEqual(backupMenuLines(away, NOW, en).slice(2), ["Backup Disk Not Connected"]);
  assert.deepEqual(backupMenuLines(away, NOW, es).slice(2), ["El disco de copia de seguridad no está conectado"]);
  assert.equal(backupAvailable(away), false);
  assert.equal(backupAvailable(state()), true);
  const shareAway = state({ destinations: [disk({ kind: "share", available: false })] });
  assert.deepEqual(backupMenuLines(shareAway, NOW, en).slice(2), ["Backup Not Available"]);

  // Low Power Mode holding the schedule back.
  assert.deepEqual(backupMenuLines(state({ held: true }), NOW, en).slice(2), ["Waiting for Low Power Mode to Turn Off"]);
  assert.deepEqual(backupMenuLines(state({ held: true }), NOW, es).slice(2), [
    "Esperando a que se desactive el modo de bajo consumo",
  ]);
  assert.equal(backupMenuLines(state({ held: true, automatic: false }), NOW, en).length, 2);
});

test("the status items go in the bar's order, the backup item first where it is shown", () => {
  assert.deepEqual(statusItemOrder({ backup: true, battery: true, network: "shell-wifi", inputSource: true }), [
    "shell-backup",
    "shell-battery",
    "shell-wifi",
    "shell-input-source",
    "shell-search",
    "shell-control-center",
    "shell-clock",
  ]);
  assert.equal(statusItemOrder({ backup: true })[0], "shell-backup");
  assert.equal(statusItemOrder({ battery: true, network: "shell-ethernet" })[0], "shell-battery");
  assert.equal(statusItemOrder({ network: "shell-wifi" })[0], "shell-wifi");
  assert.deepEqual(statusItemOrder({}), ["shell-search", "shell-control-center", "shell-clock"]);
});
