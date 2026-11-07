/**
 * Settings › General › Transfer or Reset (docs/adr/0054-recovery-and-erasing.mdx), as a Mac's: what
 * *Erase All Content and Settings…* would do and the erase itself, as the platform tells them (the
 * accounts' service, which alone decides who may erase and arms the erase; the recovery erases the
 * disk at the next boot). Settings decides none of it: it reads what it is told strictly, and says
 * what came of it.
 */
import type { Messages } from "./messages";

/** An account the erase removes from this computer. */
export type ErasedAccount = {
  name: string;
  handle: string | null;
  /** Its device session is ended online: it has a credential and the computer is online. */
  signsOutOnline: boolean;
};

/** What erasing this computer would do, as the platform tells it: the asking account first. */
export type ErasePreview = {
  /** The computer has a recovery to erase it. */
  available: boolean;
  /** The asking account is one of the computer's administrators. */
  administrator: boolean;
  online: boolean;
  accounts: ErasedAccount[];
};

/** Why an erase was refused; nothing was changed. */
export type EraseRefusal =
  | "notPermitted"
  | "notAdministrator"
  | "wrongPassword"
  | "throttled"
  | "unavailable"
  | "failed";

/** What came of an erase: armed (the computer restarts), refused, or the platform not reached. */
export type EraseOutcome =
  | { erasing: { signedOut: string[]; notSignedOut: string[] } }
  | { refused: EraseRefusal; retryAfter?: number }
  | { failed: string };

/** What Settings asks of the platform to erase the computer. */
export type EraseActions = {
  /** What erasing would do; null when it cannot be told. */
  preview: () => Promise<ErasePreview | null>;
  /** The erase, with the administrator's password. Asked only once the person confirmed. */
  eraseAll: (password: string) => Promise<EraseOutcome>;
};

const REFUSALS: readonly EraseRefusal[] = [
  "notPermitted",
  "notAdministrator",
  "wrongPassword",
  "throttled",
  "unavailable",
  "failed",
];

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const hasOnly = (value: Record<string, unknown>, keys: readonly string[]) =>
  Object.keys(value).every((key) => keys.includes(key));

/** What the platform told of erasing, read strictly: anything that is not one is none, and the
    page then offers no erase. Every account is named, the asking one first, and none is signed out
    online while the computer is offline. */
export function erasePreviewFrom(told: unknown): ErasePreview | null {
  if (!isRecord(told) || !hasOnly(told, ["available", "administrator", "online", "accounts"])) return null;
  const { available, administrator, online, accounts } = told;
  if (typeof available !== "boolean" || typeof administrator !== "boolean" || typeof online !== "boolean") {
    return null;
  }
  if (!Array.isArray(accounts) || accounts.length === 0) return null;
  const kept: ErasedAccount[] = [];
  for (const account of accounts) {
    if (!isRecord(account) || !hasOnly(account, ["name", "handle", "signsOutOnline"])) return null;
    const { name, handle, signsOutOnline } = account;
    if (typeof name !== "string" || name.trim() === "") return null;
    if (handle !== null && (typeof handle !== "string" || handle === "")) return null;
    if (typeof signsOutOnline !== "boolean" || (signsOutOnline && !online)) return null;
    kept.push({ name, handle, signsOutOnline });
  }
  return { available, administrator, online, accounts: kept };
}

/** What the platform answered an erase, read strictly: anything else is a failure, never an erase. */
export function eraseOutcomeFrom(told: unknown): EraseOutcome {
  if (!isRecord(told)) return { failed: "no answer" };
  if ("erasing" in told) {
    const erasing = told.erasing;
    const names = (value: unknown): value is string[] =>
      Array.isArray(value) && value.every((name) => typeof name === "string");
    if (isRecord(erasing) && names(erasing.signedOut) && names(erasing.notSignedOut)) {
      return { erasing: { signedOut: [...erasing.signedOut], notSignedOut: [...erasing.notSignedOut] } };
    }
    return { failed: "not an erase" };
  }
  if ("refused" in told && isRecord(told.refused)) {
    const { reason, retryAfter } = told.refused;
    const refused = REFUSALS.find((known) => known === reason) ?? "failed";
    if (refused === "throttled") {
      return typeof retryAfter === "number" && Number.isFinite(retryAfter) && retryAfter > 0
        ? { refused, retryAfter: Math.ceil(retryAfter) }
        : { refused };
    }
    return { refused };
  }
  if ("failed" in told && isRecord(told.failed) && typeof told.failed.reason === "string") {
    return { failed: told.failed.reason };
  }
  return { failed: "no answer" };
}

/** What the page offers, from what the platform told: nothing where it has no account session or
    could not say; the erase where the computer has a recovery and the account is an
    administrator, else why not. */
export type EraseAvailability = "none" | "noRecovery" | "notAdministrator" | "ready";

export function eraseAvailability(preview: ErasePreview | null | undefined): EraseAvailability {
  if (!preview) return "none";
  if (!preview.available) return "noRecovery";
  if (!preview.administrator) return "notAdministrator";
  return "ready";
}

/** What the first step says of an account's sign-in on this computer: signed out online; already
    signed out (online, it has no credential left: Oxy signed it out); or, offline, that its sign-in
    here ends when it expires or is removed from the account's devices. */
export function signOutLine(
  account: ErasedAccount,
  preview: ErasePreview,
  words: Messages["transferReset"],
): string {
  if (account.signsOutOnline) return words.signsOut;
  return preview.online ? words.signedOutAlready : words.cannotSignOut;
}

/** The administrator whose password authorizes the erase: the asking account, the preview's first. */
export function administratorOf(preview: ErasePreview): ErasedAccount {
  return preview.accounts[0]!;
}

/** What the password step says of a refusal: the field's error, or the step's. Where the computer
    can no longer be erased from here (no recovery, no longer an administrator), the step says so. */
export function refusalMessage(
  outcome: Exclude<EraseOutcome, { erasing: unknown }>,
  words: Messages["transferReset"],
): { field: string | null; step: string | null } {
  if ("failed" in outcome) return { field: null, step: words.failed };
  switch (outcome.refused) {
    case "wrongPassword":
      return { field: words.wrongPassword, step: null };
    case "throttled":
      return {
        field: outcome.retryAfter === undefined ? words.tryAgainLater : words.tryAgainIn(outcome.retryAfter),
        step: null,
      };
    case "notAdministrator":
      return { field: null, step: words.notAdministrator };
    case "unavailable":
      return { field: null, step: words.noRecovery };
    case "notPermitted":
    case "failed":
      return { field: null, step: words.failed };
  }
}
