/**
 * Software Update and Privacy (docs/adr/0030-installing-applications.mdx, decisions 8 and 16): the
 * account's update state and its applications' capabilities, as the install service keeps them and
 * the session tells Settings (the `updates` and `privacy` grants). Settings keeps no list of its
 * own: what is installed, available and decided is the service's; Atlas's Updates shows the same.
 */

/** One application update, as the install service gives it. */
export type AvailableUpdate = {
  id: string;
  name: string;
  names: Record<string, string>;
  fromVersion: string;
  toVersion: string;
  toVersionCode: number;
  /** Bytes to fetch: only what the machine lacks. */
  download: number;
  /** What is new, by language (the store's text). */
  releaseNotes: Record<string, string>;
  status: "available" | "awaitingConsent" | "atNextLaunch";
  consent: { capabilities: string[]; keyRotation: boolean } | null;
};

export type UpdateState = {
  checking: boolean;
  checkedAt: string | null;
  checkFailed: string | null;
  automatic: boolean;
  updates: AvailableUpdate[];
};

/** The OS's own update (ADR 0023, decision 6): how this install updates. */
export type SystemUpdate =
  /** A development install updates with apt, which Settings never runs. */
  | { kind: "apt" }
  /** No service says (a machine without one). */
  | { kind: "unknown" };

/** An installed application's capabilities, as its account decided them. */
export type PrivacyApplication = {
  id: string;
  name: string;
  capabilities: Record<string, "granted" | "denied" | "undecided">;
  /** A process of it runs in the background now, as the session says
      (docs/adr/0060-login-items-and-background-activity.mdx). */
  runningInBackground?: boolean;
};

export type UpdatesSnapshot = {
  system: SystemUpdate;
  /** None until the service told it, or without the `updates` grant. */
  state: UpdateState | null;
  /** The last request's failure, said once. */
  failed: string | null;
};

/** How one application's update ended, as the install service answers it. */
export type UpdateOutcome = {
  id: string;
  outcome: "switched" | "atNextLaunch" | "awaitingConsent" | "upToDate" | "failed" | string;
  /** Failed for want of room: the bytes more the disk needs to download it (the install service
      checks before it fetches anything, docs/adr/0062-storage.mdx). */
  needed?: number;
};

/** The bytes more an outcome says the disk needs, if it failed for want of room. */
export function spaceNeeded(outcome: UpdateOutcome | undefined): number | null {
  return outcome?.outcome === "failed" && typeof outcome.needed === "number" && outcome.needed > 0
    ? outcome.needed
    : null;
}

/** An update asked for answers once it ended, with each application's outcome (none when the
    session refused it). */
export type UpdatesActions = {
  checkForUpdates: () => void;
  update: (id: string) => Promise<UpdateOutcome[]>;
  updateAll: () => Promise<UpdateOutcome[]>;
};

export type PrivacyActions = {
  decide: (id: string, capability: string, granted: boolean) => void;
};

/** A row of Software Update's list. */
export type UpdateRow = {
  id: string;
  name: string;
  version: string;
  size: number;
  notes: string;
  /** It waits for this account's consent: Update asks in the system's sheet. */
  needsConsent: boolean;
  /** Its running copy updates when it next opens. */
  atNextLaunch: boolean;
};

function localized(texts: Record<string, string>, language: string, fallback: string): string {
  return texts[language] ?? (fallback || texts.en || Object.values(texts)[0] || "");
}

/** The rows, in the service's order, named and described in `language`. */
export function updateRows(state: UpdateState | null, language: string): UpdateRow[] {
  return (state?.updates ?? []).map((u) => ({
    id: u.id,
    name: localized(u.names, language, u.name),
    version: u.toVersion,
    size: u.download,
    notes: localized(u.releaseNotes, language, ""),
    needsConsent: u.status === "awaitingConsent",
    atNextLaunch: u.status === "atNextLaunch",
  }));
}

/** Where an update the person asked for stands on the page: under way, or how it ended. */
export type UpdateProgress = "updating" | "updated" | "atNextLaunch";

/** What an update's outcome leaves its row as: kept and said done, or back to what the service
    says (declined in the consent sheet, or failed, which is said). */
export function progressOf(outcome: UpdateOutcome["outcome"] | undefined): UpdateProgress | "failed" | null {
  switch (outcome) {
    case "switched":
    case "upToDate":
      return "updated";
    case "atNextLaunch":
      return "atNextLaunch";
    case "failed":
      return "failed";
    default:
      return null;
  }
}

/** What can be updated now: those not already waiting for their next launch. */
export function updatable(rows: UpdateRow[]): UpdateRow[] {
  return rows.filter((row) => !row.atNextLaunch);
}

/** Bytes the person reads. */
export function formatSize(bytes: number, language: string): string {
  const units = ["bytes", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit === 0 ? String(value) : value.toFixed(value < 10 ? 1 : 0);
  return `${language === "es" ? shown.replace(".", ",") : shown} ${units[unit]}`;
}

/** The capabilities Privacy lists for an application: those asked at first use, or any decided
    otherwise than at install (a denial), in a stable order. */
const PRIVACY_ORDER = ["notifications", "files", "camera", "microphone", "location", "background", "login-item"];

export function privacyRows(app: PrivacyApplication): { capability: string; granted: boolean; decided: boolean }[] {
  return PRIVACY_ORDER.filter((name) => name in app.capabilities).map((name) => ({
    capability: name,
    granted: app.capabilities[name] === "granted",
    decided: app.capabilities[name] !== "undecided",
  }));
}
