import type {
  ShellBackupDestination,
  ShellBackupRunning,
  ShellBackupState,
} from "./model";
import type { ShellMessages } from "./messages";

/**
 * The menu bar's backup item (docs/adr/0059-backups.mdx, decision 10), as a Mac's Time Machine
 * extra: the state the session tells, read strictly, and what the item and its menu say of it.
 */

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const isCount = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value) && value >= 0;
const isTime = (value: unknown): value is number | null =>
  value === null || (typeof value === "number" && Number.isFinite(value));
const isCountOrNull = (value: unknown): value is number | null => value === null || isCount(value);

const PHASES: readonly ShellBackupRunning["phase"][] = ["preparing", "backingUp", "cleaningUp", "checking"];

function destinationOf(told: unknown): ShellBackupDestination | null {
  if (!isObject(told)) return null;
  const { id, name, kind, url, available, lastBackup, oldestBackup, backups, usedBytes, freeBytes, lastFailure } = told;
  if (typeof id !== "string" || typeof name !== "string") return null;
  if (kind !== "disk" && kind !== "share") return null;
  if (url !== null && typeof url !== "string") return null;
  if (typeof available !== "boolean") return null;
  if (!isTime(lastBackup) || !isTime(oldestBackup) || !isCount(backups)) return null;
  if (!isCountOrNull(usedBytes) || !isCountOrNull(freeBytes)) return null;
  let failure: ShellBackupDestination["lastFailure"] = null;
  if (lastFailure !== null) {
    if (!isObject(lastFailure)) return null;
    if (typeof lastFailure.reason !== "string" || !isTime(lastFailure.at) || lastFailure.at === null) return null;
    failure = { reason: lastFailure.reason, at: lastFailure.at };
  }
  return {
    id,
    name,
    kind,
    url,
    available,
    lastBackup,
    oldestBackup,
    backups,
    usedBytes,
    freeBytes,
    lastFailure: failure,
  };
}

function runningOf(told: unknown): ShellBackupRunning | null | undefined {
  if (told === null) return null;
  if (!isObject(told)) return undefined;
  const { destination, phase, asked, percent, bytesDone, bytesTotal, filesDone, filesTotal, secondsRemaining } = told;
  if (typeof destination !== "string") return undefined;
  if (!PHASES.includes(phase as ShellBackupRunning["phase"])) return undefined;
  if (typeof asked !== "boolean") return undefined;
  if (!isCount(percent) || percent > 1) return undefined;
  if (!isCount(bytesDone) || !isCount(bytesTotal) || !isCount(filesDone) || !isCount(filesTotal)) return undefined;
  if (!isCountOrNull(secondsRemaining)) return undefined;
  return {
    destination,
    phase: phase as ShellBackupRunning["phase"],
    asked,
    percent,
    bytesDone,
    bytesTotal,
    filesDone,
    filesTotal,
    secondsRemaining,
  };
}

/** The backup state the session told (`{"backup": State}`), or null where it is not one: every
    field there with its type, or nothing at all (`{"backup": null}`: no backup service). */
export function backupStateOf(told: unknown): ShellBackupState | null {
  if (!isObject(told)) return null;
  const { automatic, destinations, excluded, running, lastBackup, nextBackup, held, administrator, inMenuBar } = told;
  if (typeof automatic !== "boolean" || typeof held !== "boolean") return null;
  if (typeof administrator !== "boolean" || typeof inMenuBar !== "boolean") return null;
  if (!isTime(lastBackup) || !isTime(nextBackup)) return null;
  if (!Array.isArray(excluded) || !excluded.every((path) => typeof path === "string")) return null;
  if (!Array.isArray(destinations)) return null;
  const read = destinations.map(destinationOf);
  if (read.some((destination) => destination === null)) return null;
  const now = runningOf(running);
  if (now === undefined) return null;
  return {
    automatic,
    destinations: read as ShellBackupDestination[],
    excluded: [...(excluded as string[])],
    running: now,
    lastBackup,
    nextBackup,
    held,
    administrator,
    inMenuBar,
  };
}

/** Whether the bar shows the item: once a destination exists, while the person's
    `backup_in_menu_bar` is on. */
export function showsBackupItem(state: ShellBackupState | null | undefined): state is ShellBackupState {
  return !!state && state.inMenuBar && state.destinations.length > 0;
}

/** The icon's minute hand, in degrees clockwise from twelve: still at twelve while idle; while a
    backup runs, at its progress (a full turn when done), moving only as the session tells it. */
export function backupHandAngle(running: ShellBackupRunning | null): number {
  if (!running) return 0;
  return Math.round(Math.min(1, Math.max(0, running.percent)) * 360);
}

/** A size in decimal units, as a Mac shows it: whole bytes and kilobytes, one decimal under 100 of
    a larger unit ("12.3 MB", "240 MB", "1.2 GB"; "12,3 MB" in Spanish), no ".0". */
export function backupBytes(bytes: number, messages: ShellMessages): string {
  const words = messages.backup;
  if (bytes < 1000) return words.bytes(Math.max(0, Math.round(bytes)));
  let unit = 0;
  let value = bytes / 1000;
  for (;;) {
    const decimals = unit > 0 && value < 100 ? 1 : 0;
    const scale = 10 ** decimals;
    const shown = Math.round(value * scale) / scale;
    if (shown >= 1000 && unit < words.units.length - 1) {
      unit += 1;
      value /= 1000;
      continue;
    }
    const whole = Number.isInteger(shown);
    return words.size(messages.format.number(shown, whole ? 0 : decimals), words.units[unit]);
  }
}

/** The time left, as Time Machine says it: about so many minutes, hours or days. */
function remaining(seconds: number, messages: ShellMessages): string {
  const words = messages.backup;
  if (seconds < 60) return words.lessThanAMinute;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return words.minutesRemaining(minutes);
  const hours = Math.round(minutes / 60);
  if (hours < 48) return words.hoursRemaining(hours);
  return words.daysRemaining(Math.round(hours / 24));
}

/** How far a backup is: "12.3 MB of 240 MB – About 2 minutes remaining"; nothing before its
    total is known. */
function progress(running: ShellBackupRunning, messages: ShellMessages): string | undefined {
  if (running.bytesTotal <= 0) return undefined;
  const words = messages.backup;
  const done = backupBytes(Math.min(running.bytesDone, running.bytesTotal), messages);
  const total = backupBytes(running.bytesTotal, messages);
  return running.secondsRemaining === null
    ? words.progress(done, total)
    : words.progressRemaining(done, total, remaining(running.secondsRemaining, messages));
}

const dateFormats = new Map<string, Intl.DateTimeFormat>();
function dateFormat(locale: string, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${locale}|${JSON.stringify(options)}`;
  let made = dateFormats.get(key);
  if (!made) {
    made = new Intl.DateTimeFormat(locale, options);
    dateFormats.set(key, made);
  }
  return made;
}

/** A time as the bar's clock shows it (clock.ts's `clockTime`, of which this pure module keeps
    its own copy): the locale's short time ("09:30" in en-GB, "9:30 AM" in en-US). */
function timeOf(at: Date, locale: string): string {
  return dateFormat(locale, { timeStyle: "short" }).format(at);
}

/** Calendar days from `at` to `now`, in the local time zone. */
function daysBefore(at: Date, now: Date): number {
  const day = (date: Date) => Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000;
  return day(now) - day(at);
}

type When = { day: "today" | "yesterday" | "on"; date: string; time: string };
function whenOf(seconds: number, now: Date, locale: string): When {
  const at = new Date(seconds * 1000);
  const days = daysBefore(at, now);
  const day = days === 0 ? "today" : days === 1 ? "yesterday" : "on";
  const date = dateFormat(locale, { day: "numeric", month: "long", year: "numeric" }).format(at);
  return { day, date, time: timeOf(at, locale) };
}

/** What the state says when nothing runs: the destination backed up last, and the latest failure
    if it came after every backup (the latest attempt failed). */
function latest(state: ShellBackupState): {
  backup: { name: string; at: number } | null;
  failure: { reason: string; at: number } | null;
} {
  let backup: { name: string; at: number } | null = null;
  let failure: { reason: string; at: number } | null = null;
  for (const destination of state.destinations) {
    if (destination.lastBackup !== null && (!backup || destination.lastBackup > backup.at)) {
      backup = { name: destination.name, at: destination.lastBackup };
    }
    const failed = destination.lastFailure;
    if (failed && (!failure || failed.at > failure.at)) failure = failed;
  }
  if (failure && backup && failure.at <= backup.at) failure = null;
  return { backup, failure };
}

/** A failure's reason in words; one the shell does not know is said as a failure. */
function reasonOf(reason: string, messages: ShellMessages): string {
  const reasons = messages.backup.reasons;
  return Object.prototype.hasOwnProperty.call(reasons, reason) ? reasons[reason as keyof typeof reasons] : reasons.failed;
}

/**
 * The menu's lines above its commands, as a Mac's Time Machine menu: while a backup runs, its
 * phase and (backing up) how far it is; else the latest backup's destination and when, or that
 * there is none, why the latest failed, that no destination is connected, and that Low Power Mode
 * holds the schedule back.
 */
export function backupMenuLines(state: ShellBackupState, now: Date, messages: ShellMessages): string[] {
  const words = messages.backup;
  const running = state.running;
  if (running) {
    switch (running.phase) {
      case "preparing":
        return [words.preparing];
      case "cleaningUp":
        return [words.cleaningUp];
      case "checking":
        return [words.checking];
      default: {
        const far = progress(running, messages);
        return far === undefined ? [words.backingUp] : [words.backingUp, far];
      }
    }
  }
  const lines: string[] = [];
  const { backup, failure } = latest(state);
  if (backup) {
    const when = whenOf(backup.at, now, messages.locale);
    lines.push(
      words.latestTo(backup.name),
      when.day === "today"
        ? words.today(when.time)
        : when.day === "yesterday"
          ? words.yesterday(when.time)
          : words.on(when.date, when.time),
    );
  } else if (!failure) {
    lines.push(words.noBackups);
  }
  if (failure) lines.push(words.failed, reasonOf(failure.reason, messages));
  if (!backupAvailable(state)) {
    lines.push(state.destinations.every((destination) => destination.kind === "disk") ? words.diskNotConnected : words.notAvailable);
  } else if (state.held && state.automatic) {
    lines.push(words.held);
  }
  return lines;
}

/** Whether a backup can be made now: a destination is connected (a share: worth trying). */
export function backupAvailable(state: ShellBackupState): boolean {
  return state.destinations.some((destination) => destination.available);
}

/** The item's value, as a screen reader says it: "Backing up, 45%", "Latest backup today, 14:05",
    "Latest backup failed", "No backups yet". */
export function backupSaid(state: ShellBackupState, now: Date, messages: ShellMessages): string {
  const words = messages.backup;
  const running = state.running;
  if (running) {
    switch (running.phase) {
      case "preparing":
        return words.saidPreparing;
      case "cleaningUp":
        return words.saidCleaningUp;
      case "checking":
        return words.saidChecking;
      default:
        return words.saidBackingUp(messages.common.percent(Math.floor(Math.min(1, Math.max(0, running.percent)) * 100)));
    }
  }
  const { backup, failure } = latest(state);
  if (failure) return words.saidFailed;
  if (!backup) return words.saidNone;
  const when = whenOf(backup.at, now, messages.locale);
  return when.day === "today"
    ? words.saidToday(when.time)
    : when.day === "yesterday"
      ? words.saidYesterday(when.time)
      : words.saidOn(when.date, when.time);
}
