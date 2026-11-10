/**
 * Backups (docs/adr/0059-backups.mdx, decision 9): what the session tells Settings of the backup
 * service (`oxyos-backupd`, the `settings` grant), what Settings may ask of it, and the words
 * Settings › General › Backups makes of them, as a Mac's Time Machine pane says them. Pure, so it
 * is tested without a renderer; the host maps the session's answers with the readers below.
 */
import type { Messages } from "./messages";

/** Why the last backup to a destination failed, and when (seconds since the epoch). */
export type BackupFailure = { reason: string; at: number };

/** A disk or a share backups go to, as the service keeps it. Times are seconds since the epoch,
    sizes bytes. */
export type BackupDestination = {
  id: string;
  name: string;
  kind: "disk" | "share";
  /** A share's `smb://host/share`; none for a disk. */
  url: string | null;
  /** A disk connected, a share worth trying. */
  available: boolean;
  lastBackup: number | null;
  oldestBackup: number | null;
  backups: number;
  usedBytes: number | null;
  freeBytes: number | null;
  lastFailure: BackupFailure | null;
};

/** What a running backup is doing. */
export type BackupPhase = "preparing" | "backingUp" | "cleaningUp" | "checking";

/** A backup running now. */
export type BackupRunning = {
  /** The destination's id. */
  destination: string;
  phase: BackupPhase;
  /** Started by *Back Up Now*, not by the schedule. */
  asked: boolean;
  /** How far, 0 to 1 (a fraction). */
  percent: number;
  bytesDone: number;
  bytesTotal: number;
  filesDone: number;
  filesTotal: number;
  secondsRemaining: number | null;
};

/** The backup service's state, as the session tells it. */
export type BackupState = {
  /** *Back Up Automatically*. */
  automatic: boolean;
  destinations: BackupDestination[];
  /** The items left out of backups, as absolute paths. */
  excluded: string[];
  running: BackupRunning | null;
  lastBackup: number | null;
  nextBackup: number | null;
  /** Scheduled backups held back (Low Power Mode). */
  held: boolean;
  /** The person may add and remove destinations, exclusions and *Back Up Automatically*. */
  administrator: boolean;
  /** The `backup_in_menu_bar` preference (the shell's). */
  inMenuBar: boolean;
};

/** A disk connected that can hold backups. Its label is empty where its file system has none. */
export type BackupDisk = { uuid: string; label: string; fsType: string; size: number; added: boolean };

/** A share on a NAS or another computer, reached over SMB. */
export type BackupShare = { host: string; port?: number; share: string; user: string; domain?: string };

/** What Settings asks of the session; each resolves with why it failed (a reason), or null once
    done. */
export type BackupActions = {
  backUpNow: () => Promise<string | null>;
  /** *Skip This Backup*. */
  stop: () => Promise<string | null>;
  /** The disks that can be added; null when the session could not say. */
  disks: () => Promise<BackupDisk[] | null>;
  addDisk: (uuid: string, password: string) => Promise<string | null>;
  addShare: (share: BackupShare, sharePassword: string, password: string) => Promise<string | null>;
  remove: (id: string) => Promise<string | null>;
  setExcluded: (paths: string[]) => Promise<string | null>;
  setAutomatic: (enabled: boolean) => Promise<string | null>;
};

/** The reasons put in words: the session's refusals, the service's and the failures it records. */
export const BACKUP_REASONS = [
  "notPermitted",
  "needsInput",
  "invalidRequest",
  "noDestination",
  "notConnected",
  "running",
  "notRunning",
  "notFound",
  "wrongPassword",
  "alreadyAdded",
  "unreachable",
  "full",
  "failed",
  "passwordTooShort",
  "invalidHost",
  "invalidShare",
  "invalidUser",
  "invalidDomain",
  "shareRefused",
  "inUse",
  "noBackup",
  "unavailable",
  "damaged",
  "noKey",
  "nothingToBackUp",
  "stopped",
] as const;
export type BackupReason = (typeof BACKUP_REASONS)[number];

export const BACKUP_PHASES: readonly BackupPhase[] = ["preparing", "backingUp", "cleaningUp", "checking"];

/** The fewest characters a backup password has, as the service counts them (Unicode scalar
    values, `oxyos_backup::service::MIN_PASSWORD`). */
export const MIN_BACKUP_PASSWORD = 8;

type Words = Messages["backups"];

// Reading the session's words, strictly.

type Fields = Record<string, unknown>;

function isObject(value: unknown): value is Fields {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isNumberOrNull(value: unknown): value is number | null {
  return value === null || isNumber(value);
}

function failureOf(value: unknown): BackupFailure | null | undefined {
  if (value === null) return null;
  if (!isObject(value) || typeof value.reason !== "string" || !isNumber(value.at)) return undefined;
  return { reason: value.reason, at: value.at };
}

function destinationOf(value: unknown): BackupDestination | null {
  if (!isObject(value)) return null;
  const d = value;
  const lastFailure = failureOf(d.lastFailure);
  if (
    typeof d.id !== "string" ||
    typeof d.name !== "string" ||
    (d.kind !== "disk" && d.kind !== "share") ||
    !(d.url === null || typeof d.url === "string") ||
    typeof d.available !== "boolean" ||
    !isNumberOrNull(d.lastBackup) ||
    !isNumberOrNull(d.oldestBackup) ||
    !isNumber(d.backups) ||
    !isNumberOrNull(d.usedBytes) ||
    !isNumberOrNull(d.freeBytes) ||
    lastFailure === undefined
  ) {
    return null;
  }
  return {
    id: d.id,
    name: d.name,
    kind: d.kind,
    url: d.url,
    available: d.available,
    lastBackup: d.lastBackup,
    oldestBackup: d.oldestBackup,
    backups: d.backups,
    usedBytes: d.usedBytes,
    freeBytes: d.freeBytes,
    lastFailure,
  };
}

function runningOf(value: unknown): BackupRunning | null | undefined {
  if (value === null) return null;
  if (!isObject(value)) return undefined;
  const r = value;
  if (
    typeof r.destination !== "string" ||
    !BACKUP_PHASES.includes(r.phase as BackupPhase) ||
    typeof r.asked !== "boolean" ||
    !isNumber(r.percent) ||
    !isNumber(r.bytesDone) ||
    !isNumber(r.bytesTotal) ||
    !isNumber(r.filesDone) ||
    !isNumber(r.filesTotal) ||
    !isNumberOrNull(r.secondsRemaining)
  ) {
    return undefined;
  }
  return {
    destination: r.destination,
    phase: r.phase as BackupPhase,
    asked: r.asked,
    percent: r.percent,
    bytesDone: r.bytesDone,
    bytesTotal: r.bytesTotal,
    filesDone: r.filesDone,
    filesTotal: r.filesTotal,
    secondsRemaining: r.secondsRemaining,
  };
}

/**
 * The backup state the session told (the value of its `backup`), read strictly: every field of
 * the right type, else null. Null is also what `{"backup": null}` holds: the service is not there.
 */
export function backupStateOf(told: unknown): BackupState | null {
  if (!isObject(told)) return null;
  const s = told;
  if (!Array.isArray(s.destinations) || !Array.isArray(s.excluded)) return null;
  const destinations = s.destinations.map(destinationOf);
  const running = runningOf(s.running);
  if (
    typeof s.automatic !== "boolean" ||
    destinations.some((d) => d === null) ||
    !s.excluded.every((path) => typeof path === "string") ||
    running === undefined ||
    !isNumberOrNull(s.lastBackup) ||
    !isNumberOrNull(s.nextBackup) ||
    typeof s.held !== "boolean" ||
    typeof s.administrator !== "boolean" ||
    typeof s.inMenuBar !== "boolean"
  ) {
    return null;
  }
  return {
    automatic: s.automatic,
    destinations: destinations as BackupDestination[],
    excluded: [...(s.excluded as string[])],
    running,
    lastBackup: s.lastBackup,
    nextBackup: s.nextBackup,
    held: s.held,
    administrator: s.administrator,
    inMenuBar: s.inMenuBar,
  };
}

/** The session's answer to the disks (`{"backupDisks": [Disk]}`), read strictly; null when it is
    not one (a refusal, a failure, or anything malformed). */
export function backupDisksOf(answer: unknown): BackupDisk[] | null {
  if (!isObject(answer) || !Array.isArray(answer.backupDisks)) return null;
  const disks: BackupDisk[] = [];
  for (const disk of answer.backupDisks) {
    if (
      !isObject(disk) ||
      typeof disk.uuid !== "string" ||
      typeof disk.label !== "string" ||
      typeof disk.fsType !== "string" ||
      !isNumber(disk.size) ||
      typeof disk.added !== "boolean"
    ) {
      return null;
    }
    disks.push({ uuid: disk.uuid, label: disk.label, fsType: disk.fsType, size: disk.size, added: disk.added });
  }
  return disks;
}

/**
 * The session's answer to a request: null when it answered with the request's own key (`done`:
 * `backupNow`, `backupStop`, `backupAddDisk`, `backupAddShare`, `backupRemove`, `backupExcluded`,
 * `backupAutomatic`), else why not: the reason of its `refused` or `failed`, or `failed`.
 */
export function backupFailureOf(answer: unknown, done: string): string | null {
  if (!isObject(answer)) return "failed";
  if (Object.prototype.hasOwnProperty.call(answer, done) && isObject(answer[done])) return null;
  const said = isObject(answer.refused) ? answer.refused : isObject(answer.failed) ? answer.failed : null;
  return said && typeof said.reason === "string" && said.reason !== "" ? said.reason : "failed";
}

// Words.

/** A reason in words; one the page does not know is said as a failure. */
export function backupReasonLabel(reason: string, words: Pick<Words, "reasons">): string {
  const known = (BACKUP_REASONS as readonly string[]).includes(reason) ? (reason as BackupReason) : "failed";
  return words.reasons[known];
}

/** A size as a Mac shows it: decimal units (1000 bytes a kilobyte), three figures at most ("1.2 GB",
    "12.3 MB", "245 GB"), the locale's decimal separator; bytes under a kilobyte as bytes. */
export function backupBytes(bytes: number, locale: string, words: Pick<Words, "zeroBytes" | "bytes">): string {
  const whole = Math.max(0, Math.round(bytes));
  if (whole === 0) return words.zeroBytes;
  if (whole < 1000) return words.bytes(whole);
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let unit = 0;
  let value = whole / 1000;
  for (;;) {
    const digits = value < 100 ? 1 : 0;
    const rounded = Math.round(value * 10 ** digits) / 10 ** digits;
    if (rounded >= 1000 && unit < units.length - 1) {
      value /= 1000;
      unit += 1;
      continue;
    }
    const number = new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(rounded);
    return `${number} ${units[unit]}`;
  }
}

/** The time as the shell's clock shows it (apps/shell/src/clock.ts): the locale's short time,
    "14:05", "2:05 PM". */
function timeOfDay(at: Date, locale: string): string {
  return new Intl.DateTimeFormat(locale, { timeStyle: "short" }).format(at);
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** The local day `days` after the one `at` is in (negative: before). */
function dayAfter(at: Date, days: number): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate() + days);
}

/**
 * When a backup was or will be, as Time Machine says it: "Today, 14:05", "Yesterday, 9:30 AM",
 * "Tomorrow, 08:00", else the date ("3 November 2026"), with its time where `withTime` asks for
 * it ("3 November 2026 at 14:05"). In the local time zone, the locale's date and hour cycle; `now`
 * and `seconds` are seconds since the epoch.
 */
export function backupWhen(
  seconds: number,
  now: number,
  locale: string,
  words: Pick<Words, "today" | "yesterday" | "tomorrow" | "dateAt">,
  withTime = true,
): string {
  const at = new Date(seconds * 1000);
  const today = new Date(now * 1000);
  const time = timeOfDay(at, locale);
  if (sameDay(at, today)) return words.today(time);
  if (sameDay(at, dayAfter(today, -1))) return words.yesterday(time);
  if (sameDay(at, dayAfter(today, 1))) return words.tomorrow(time);
  const date = new Intl.DateTimeFormat(locale, { day: "numeric", month: "long", year: "numeric" }).format(at);
  return withTime ? words.dateAt(date, time) : date;
}

/** What a destination's row says under its name, each line its own (absent where there is none). */
export type DestinationSummary = {
  /** Not connected, or not reachable. */
  status: string | null;
  latest: string;
  oldest: string | null;
  space: string | null;
  /** Why the last backup failed, where it failed since the last that did not. */
  failure: string | null;
};

/** A destination's lines: "Latest backup: Today, 14:05", "Oldest backup: 3 November 2026", "1.2 GB
    used, 245 GB available", and why its last backup failed. */
export function destinationSummary(
  destination: BackupDestination,
  now: number,
  locale: string,
  words: Words,
): DestinationSummary {
  const latest =
    destination.lastBackup === null
      ? words.noBackups
      : words.latest(backupWhen(destination.lastBackup, now, locale, words));
  // The oldest only where there is more than the latest.
  const oldest =
    destination.oldestBackup !== null && destination.backups > 1 && destination.oldestBackup !== destination.lastBackup
      ? words.oldest(backupWhen(destination.oldestBackup, now, locale, words, false))
      : null;
  const used = destination.usedBytes === null ? null : backupBytes(destination.usedBytes, locale, words);
  const free = destination.freeBytes === null ? null : backupBytes(destination.freeBytes, locale, words);
  const space =
    used !== null && free !== null
      ? words.usedAvailable(used, free)
      : used !== null
        ? words.used(used)
        : free !== null
          ? words.available(free)
          : null;
  const failed = destination.lastFailure;
  const failure =
    failed && (destination.lastBackup === null || failed.at >= destination.lastBackup)
      ? `${words.backupFailed} ${backupReasonLabel(failed.reason, words)}`
      : null;
  const status = destination.available ? null : destination.kind === "disk" ? words.notConnected : words.notReachable;
  return { status, latest, oldest, space, failure };
}

/** The time left in words: "Less than a minute remaining", "About 2 minutes remaining", "About 1
    hour, 20 minutes remaining". */
export function remainingLabel(seconds: number, words: Pick<Words, "lessThanMinute" | "aboutRemaining">): string {
  if (seconds < 60) return words.lessThanMinute;
  const minutes = Math.round(seconds / 60);
  return words.aboutRemaining(Math.floor(minutes / 60), minutes % 60);
}

/**
 * What a running backup is doing in a line: "Preparing Backup…", "Backing up: 12.3 MB of 240 MB –
 * About 2 minutes remaining", "Cleaning Up…", "Verifying Backups…". A copy whose size is not known
 * yet is "Backing Up…".
 */
export function progressLine(running: BackupRunning, locale: string, words: Words): string {
  if (running.phase !== "backingUp" || running.bytesTotal <= 0) return words.phases[running.phase];
  const copied = words.progress(
    backupBytes(running.bytesDone, locale, words),
    backupBytes(running.bytesTotal, locale, words),
  );
  return running.secondsRemaining === null
    ? copied
    : `${copied} – ${remainingLabel(running.secondsRemaining, words)}`;
}

/** Whether the progress bar can say how far (it copies, and knows how much), else it is
    indeterminate. */
export function progressKnown(running: BackupRunning): boolean {
  return running.phase === "backingUp" && running.bytesTotal > 0;
}

/** How far, 0 to 1, as the bar draws it. */
export function progressFraction(running: BackupRunning): number {
  return Math.min(1, Math.max(0, running.percent));
}

/** Why a backup password cannot be used yet: too short, or the two typed differ; null when it can. */
export function backupPasswordProblem(password: string, again: string): "tooShort" | "mismatch" | null {
  if ([...password].length < MIN_BACKUP_PASSWORD) return "tooShort";
  return password === again ? null : "mismatch";
}

/** A file system's name as people read it ("exFAT", "Btrfs"); one not known as udev names it. */
export function fileSystemName(fsType: string): string {
  const names: Record<string, string> = { ext4: "ext4", btrfs: "Btrfs", xfs: "XFS", exfat: "exFAT" };
  return names[fsType.toLowerCase()] ?? fsType;
}

/** A disk as the sheet names it: its label, else its size ("500 GB Disk"). */
export function diskName(disk: BackupDisk, locale: string, words: Pick<Words, "diskOfSize" | "zeroBytes" | "bytes">): string {
  return disk.label.trim() || words.diskOfSize(backupBytes(disk.size, locale, words));
}

/** What the share form holds, as typed. */
export type ShareFields = { host: string; share: string; user: string; domain: string };

/**
 * The share the form names, its words trimmed and an empty domain left out. A computer typed as a
 * Mac's *Connect to Server* takes it, `smb://host/share`, gives the share too where none was typed.
 * Null while the computer, the share or the user is missing.
 */
export function shareOf(fields: ShareFields): BackupShare | null {
  let host = fields.host.trim();
  let share = fields.share.trim();
  const url = /^smb:\/\/([^/]+)(?:\/([^/]+))?\/?$/i.exec(host);
  if (url) {
    host = url[1];
    if (share === "" && url[2]) share = url[2];
  }
  const user = fields.user.trim();
  const domain = fields.domain.trim();
  if (host === "" || share === "" || user === "") return null;
  return domain === "" ? { host, share, user } : { host, share, user, domain };
}

/** The destination a backup can go to now: one is available. */
export function canBackUp(state: BackupState): boolean {
  return state.destinations.some((destination) => destination.available);
}

/** An absolute path's last name ("Videos"), or "/" for the root. */
export function pathName(path: string): string {
  const trimmed = path.length > 1 ? path.replace(/\/+$/, "") : path;
  return trimmed === "/" || trimmed === "" ? "/" : (trimmed.split("/").pop() ?? trimmed);
}

/** The items excluded with `path` added once (an item already excluded, whatever its trailing
    slash, is not added again). */
export function withExcludedPath(excluded: readonly string[], path: string): string[] {
  const clean = path.length > 1 ? path.replace(/\/+$/, "") : path;
  return excluded.some((one) => (one.length > 1 ? one.replace(/\/+$/, "") : one) === clean)
    ? [...excluded]
    : [...excluded, clean];
}
