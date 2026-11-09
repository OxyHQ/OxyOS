/**
 * Migrate from a Backup (docs/adr/0059-backups.mdx, decision 7): at the first setup, while no
 * account is enrolled, the login screen brings another computer's accounts over from its backup,
 * on a disk or a share. The platform's migration service does the work as root; this reads what it
 * answers and says where the person is, apart from how it is shown. The backup password goes to
 * the service and is never kept here.
 */

/** A disk that may hold backups. */
export type BackupDisk = { uuid: string; label: string; size: number };

/** A share on a NAS or another computer. */
export type BackupShare = { host: string; share: string; user: string };

/** Where the backups are, as the service takes it. */
export type BackupSource = { disk: { uuid: string } } | { share: { share: BackupShare } };

/** A backup of a computer, `time` in seconds since the epoch. */
export type ComputerBackup = { snapshot: string; time: number };

/** A computer whose backups the password opens: its backups newest first, the accounts of its
    newest. */
export type BackupComputer = {
  repository: string;
  name: string;
  backups: ComputerBackup[];
  accounts: { user: string; name: string; administrator: boolean }[];
};

/** Why the service did not do what it was asked, as the person is told it. */
export type MigrateProblem =
  | "notConnected"
  | "noBackup"
  | "wrongPassword"
  | "shareRefused"
  | "unreachable"
  | "inUse"
  | "full"
  | "invalidBackup"
  | "clash"
  | "alreadySetUp"
  | "failed";

const PROBLEMS: readonly MigrateProblem[] = [
  "notConnected",
  "noBackup",
  "wrongPassword",
  "shareRefused",
  "unreachable",
  "inUse",
  "full",
  "invalidBackup",
  "clash",
  "alreadySetUp",
];

/** A refusal's reason (or a failure to ask) as a problem the person is told. */
export function problemOf(told: unknown): MigrateProblem {
  if (typeof told === "object" && told !== null) {
    const reason = (told as { refused?: { reason?: unknown } }).refused?.reason;
    if (typeof reason === "string" && (PROBLEMS as readonly string[]).includes(reason)) {
      return reason as MigrateProblem;
    }
  }
  return "failed";
}

const record = (value: unknown): Record<string, unknown> | null =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : null;

/** The service's disks, or the problem. */
export function disksFrom(told: unknown): BackupDisk[] | MigrateProblem {
  const disks = record(told)?.disks;
  if (!Array.isArray(disks)) return problemOf(told);
  return disks.flatMap((entry): BackupDisk[] => {
    const disk = record(entry);
    if (!disk || typeof disk.uuid !== "string" || !disk.uuid) return [];
    return [
      {
        uuid: disk.uuid,
        label: typeof disk.label === "string" ? disk.label : "",
        size: typeof disk.size === "number" && disk.size > 0 ? disk.size : 0,
      },
    ];
  });
}

/** The computers the password opened, the most recently backed up first, each with at least one
    backup; or the problem. None opened is the wrong password when there were others' there. */
export function computersFrom(told: unknown): BackupComputer[] | MigrateProblem {
  const listing = record(record(told)?.backups);
  if (!listing || !Array.isArray(listing.computers)) return problemOf(told);
  const computers = listing.computers.flatMap((entry): BackupComputer[] => {
    const computer = record(entry);
    if (!computer || typeof computer.repository !== "string" || typeof computer.name !== "string") return [];
    const backups = (Array.isArray(computer.backups) ? computer.backups : [])
      .flatMap((item): ComputerBackup[] => {
        const backup = record(item);
        return backup && typeof backup.snapshot === "string" && typeof backup.time === "number"
          ? [{ snapshot: backup.snapshot, time: backup.time }]
          : [];
      })
      .sort((a, b) => b.time - a.time);
    if (backups.length === 0) return [];
    const accounts = (Array.isArray(computer.accounts) ? computer.accounts : []).flatMap((item) => {
      const account = record(item);
      return account && typeof account.user === "string" && typeof account.name === "string"
        ? [{ user: account.user, name: account.name.trim(), administrator: account.administrator === true }]
        : [];
    });
    return [{ repository: computer.repository, name: computer.name || computer.repository, backups, accounts }];
  });
  if (computers.length === 0) {
    return typeof listing.unopened === "number" && listing.unopened > 0 ? "wrongPassword" : "noBackup";
  }
  return computers.sort((a, b) => b.backups[0]!.time - a.backups[0]!.time);
}

/** Where a migration is. */
export type MigrateProgress = { step: "accounts" | "files" | "finishing"; percent: number };

/** A migration's progress message, or null for anything else. */
export function progressFrom(told: unknown): MigrateProgress | null {
  const migrating = record(record(told)?.migrating);
  if (!migrating) return null;
  const { step, percent } = migrating;
  if (step !== "accounts" && step !== "files" && step !== "finishing") return null;
  return { step, percent: typeof percent === "number" ? Math.min(1, Math.max(0, percent)) : 0 };
}

/** The accounts a migration brought over, or the problem. */
export function migratedFrom(told: unknown): { computer: string; accounts: { user: string; name: string }[] } | MigrateProblem {
  const migrated = record(record(told)?.migrated);
  if (!migrated || !Array.isArray(migrated.accounts)) return problemOf(told);
  return {
    computer: typeof migrated.computer === "string" ? migrated.computer : "",
    accounts: migrated.accounts.flatMap((item) => {
      const account = record(item);
      return account && typeof account.user === "string"
        ? [{ user: account.user, name: typeof account.name === "string" ? account.name.trim() : "" }]
        : [];
    }),
  };
}

/** Where Migrate from a Backup stands. */
export type MigrateStep =
  /** Choosing where the backups are: the disks connected (null while they are asked for). */
  | { step: "source"; disks: BackupDisk[] | null; problem: MigrateProblem | null }
  /** A share's computer, name, user and password. */
  | { step: "share"; problem: MigrateProblem | null }
  /** The backup password, for the backups at `source` (`place` is what the person is shown). */
  | { step: "password"; source: BackupSource; place: string; problem: MigrateProblem | null }
  /** Choosing one of several computers. */
  | { step: "computer"; computers: BackupComputer[] }
  /** Choosing a backup of the computer. */
  | { step: "backup"; computer: BackupComputer; several: BackupComputer[] | null }
  /** Confirming. */
  | { step: "confirm"; computer: BackupComputer; backup: ComputerBackup; several: BackupComputer[] | null }
  | { step: "migrating"; computer: BackupComputer; progress: MigrateProgress }
  /** The migration did not finish: nothing more is done until the person starts again. */
  | { step: "failed"; problem: MigrateProblem };

/** Where the backups' answer leads from the password: on to the computer (straight to its backups
    when there is one), or back with the problem. */
export function afterBackups(told: unknown, at: Extract<MigrateStep, { step: "password" }>): MigrateStep {
  const computers = computersFrom(told);
  if (typeof computers === "string") {
    // The password again only when it was the password; anything else is the source's.
    return computers === "wrongPassword"
      ? { ...at, problem: computers }
      : { step: "source", disks: null, problem: computers };
  }
  if (computers.length === 1) return { step: "backup", computer: computers[0]!, several: null };
  return { step: "computer", computers };
}

/** What a disk is called: its label, else its size. */
export function diskName(disk: BackupDisk, language: string): string {
  if (disk.label.trim()) return disk.label;
  const gigabytes = disk.size / 1e9;
  return `${new Intl.NumberFormat(language, { maximumFractionDigits: gigabytes < 10 ? 1 : 0 }).format(gigabytes)} GB`;
}

/** When a backup was made, as the person reads dates. */
export function backupDate(time: number, language: string): string {
  return new Intl.DateTimeFormat(language, { dateStyle: "long", timeStyle: "short" }).format(new Date(time * 1000));
}

/** A share as typed, or null until it can be one (each part given, the computer one word). */
export function shareFrom(host: string, share: string, user: string): BackupShare | null {
  const parts = { host: host.trim(), share: share.trim(), user: user.trim() };
  if (!parts.host || !parts.share || !parts.user) return null;
  if (/\s|\//.test(parts.host) || /[/\\:]/.test(parts.share)) return null;
  return parts;
}
