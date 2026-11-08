/**
 * Setting an account up at the login screen, the setup assistant (docs/adr/0020-oxy-account-login.mdx,
 * decisions 2 and 4, amended by decision 7): where it stands and where each answer leads, apart
 * from how it is shown. Nothing here decides who may sign in.
 *
 * The steps, only those that apply, in order, as a Mac's Setup Assistant shows them:
 *
 * 1. **Welcome**, on the computer's first account only.
 * 1a. **Transfer Your Data**, on the computer's first account only and where the platform migrates
 *    (docs/adr/0059-backups.mdx, decision 7): another computer's accounts brought over from its
 *    backup, as a Mac's Setup Assistant offers Migration Assistant before its first user; or Not
 *    Now, to set up as new. Migrating ends the assistant: the accounts brought over are listed,
 *    each to sign in again.
 * 2. **Connect to the internet**, only while offline: setup is online only.
 * 3. **Sign in with Oxy**: the SDK's own sign-in (the platform's, with OxyOS's client id from the
 *    accounts' service), the QR a phone scans or a way typed in; it obtains a device credential,
 *    which the platform takes over at once and holds.
 * 4. **The Oxy password once more**, only for a sign-in that typed none (the email's code) of an
 *    account that has one.
 * 5. **A PIN for this computer**: required for a sign-in by the QR and for an account without a
 *    password, optional ("Not Now") where a password becomes the local credential.
 * 6. **Appearance**: light, dark or automatic, the new account's first.
 * 7. **Setting up**: the accounts' service sets the account up, saying each stage as it reaches
 *    it, asking a second factor on the same conversation when Oxy does; then the account signs
 *    in with what was just chosen, straight to its desktop, never asked a second time.
 *
 * Back goes to the step before; going back past the sign-in gives it up. A failure says what
 * happened and what to do next, and keeps whatever can be kept: the sign-in, the PIN and the
 * choices survive a transient failure, and only a sign-in that can no longer be used is done again.
 */

import type { MigrateStep } from "./migrate";

/** Where the SDK's sign-in runs: Oxy's API and OxyOS's public client id (neither secret). */
export type SetupPlatform = { api: string; clientId: string };

/** The accounts' service's `platform`, as the platform hands it over: anything else is none. */
export function platformFrom(told: unknown): SetupPlatform | null {
  if (typeof told !== "object" || told === null) return null;
  const { api, clientId } = told as Record<string, unknown>;
  if (typeof api !== "string" || !/^https?:\/\//.test(api)) return null;
  if (typeof clientId !== "string" || !clientId) return null;
  return { api, clientId };
}

/**
 * The account the SDK's sign-in signed in, and how (docs/adr/0020-oxy-account-login.mdx, decision 2
 * as amended): `qr`, the QR approved on a phone (a Commons approval: no password involved, the PIN
 * alone); `password`, the account's password typed in the sign-in, which the setup takes and never
 * asks again (the PIN optional); `other`, the email's code or a new account, where a password the
 * account has is asked once (the PIN optional) and an account without one sets a PIN. Its name is
 * what the sign-in said of it, empty when it said nothing.
 */
export type SignedInAccount = { id: string; name: string; hasPassword: boolean; via: "qr" | "password" | "other" };

/** Whether the setup asks for the account's password once: only a sign-in that typed none, of an
    account that has one. */
export function asksPassword(account: SignedInAccount): boolean {
  return account.via === "other" && account.hasPassword;
}

/** Whether the PIN may be left for later: only where a password becomes the local credential. */
export function pinOptional(account: SignedInAccount): boolean {
  return account.via === "password" || asksPassword(account);
}

/** Whether `pin` can be a PIN: 4 to 16 digits, as the accounts' service takes one. */
export function usablePin(pin: string): boolean {
  return /^[0-9]{4,16}$/.test(pin);
}

/** The new account's appearance (docs/adr/0019-settings.mdx): `auto` follows the sun. */
export type Appearance = "light" | "dark" | "auto";
export const APPEARANCES: readonly Appearance[] = ["light", "dark", "auto"];

/**
 * The stages the accounts' service reports as it sets the account up, in its order (any skipped
 * where it has nothing to do: `securing`, the PIN's verifier, without a PIN; a retry with what the
 * service kept starts at `preparing`), and the last, the login screen's own: the new account's
 * desktop opening, signed in with what was just chosen.
 */
export const SETUP_STAGES = ["securing", "verifying", "preparing", "signingIn", "finishing", "starting"] as const;
export type SetupStage = (typeof SETUP_STAGES)[number];

/** A stage as the platform reports it: anything else is none (the progress does not move). */
export function stageFrom(told: unknown): SetupStage | null {
  return typeof told === "string" && (SETUP_STAGES as readonly string[]).includes(told) ? (told as SetupStage) : null;
}

/**
 * Why the setup stopped, each with its way forward:
 * - `unreachable`: Oxy could not be reached. The sign-in, the PIN and the choices are kept: Try
 *   Again once connected, no new sign-in.
 * - `machine`: this computer could not set the account up; nothing was changed and the sign-in is
 *   kept: Try Again.
 * - `expired`: the sign-in can no longer be used (its time ran out, or Oxy no longer takes it):
 *   Sign In Again.
 * - `failed`: anything else: Try Again, or start over.
 */
export type SetupProblem = "unreachable" | "machine" | "expired" | "failed";

/** The accounts' service's answer to a setup's turn, or why it could not be asked. */
export type EnrollAnswer =
  | { enrolled: { accountId: string; user: string } }
  | "secondFactorRequired"
  | { refused: { reason: string; retryAfterSeconds?: number | null } }
  | { failed: { reason: string } };

/** Where setup stands. */
export type SetupStep =
  /** The first account's welcome. */
  | { step: "welcome" }
  /** Transfer Your Data: from a backup, or set up as new. */
  | { step: "transfer" }
  /** Migrating from a backup, where it stands. */
  | { step: "migrate"; migrate: MigrateStep }
  /** The connection being checked before the sign-in. */
  | { step: "checking" }
  | { step: "offline" }
  /** The machine sets up no account: the accounts' service names no client id, or cannot be read. */
  | { step: "notConfigured" }
  | { step: "signIn"; platform: SetupPlatform }
  /** The password once more, for the account signed in; `wait` seconds before another try. */
  | { step: "password"; account: SignedInAccount; problem?: "incorrect" | "throttled" | "unusable"; wait?: number }
  /** A PIN for this computer; `first`: the one typed, to be typed again; `problem`: why the last
      was not taken. */
  | { step: "pin"; account: SignedInAccount; first?: string; problem?: "mismatch" | "unusable" }
  | { step: "appearance"; account: SignedInAccount }
  /** The accounts' service at work: the stage it last reported, or why it stopped. */
  | { step: "settingUp"; account: SignedInAccount; stage: SetupStage | null; problem?: SetupProblem }
  | { step: "secondFactor"; account: SignedInAccount; invalid: boolean }
  /** Set up: the new account signing in with what was chosen; `accepted` once the login program
      took it, its desktop taking the screen. */
  | { step: "signingIn"; account: SignedInAccount; user: string; accepted?: boolean };

/** The assistant's pages: a setup step is shown on one. */
export type PageId = "welcome" | "transfer" | "migrate" | "network" | "signIn" | "password" | "pin" | "appearance" | "settingUp";

/** The page a step is shown on. */
export function pageOf(step: SetupStep): PageId {
  switch (step.step) {
    case "welcome":
      return "welcome";
    case "transfer":
      return "transfer";
    case "migrate":
      return "migrate";
    case "offline":
    case "notConfigured":
      return "network";
    case "checking":
    case "signIn":
      return "signIn";
    case "password":
      return "password";
    case "pin":
      return "pin";
    case "appearance":
      return "appearance";
    case "settingUp":
    case "secondFactor":
    case "signingIn":
      return "settingUp";
  }
}

/**
 * The pages the assistant shows for where it stands, in order: only as many as needed. The
 * welcome on the first account alone; the network's page only while it is the problem; the
 * password's only for an account that is asked it.
 */
export function pagesFor(step: SetupStep, first: boolean, transfer = false): PageId[] {
  const pages: PageId[] = first ? ["welcome"] : [];
  // Transfer Your Data on the first account where the platform migrates; migrating, the
  // assistant ends with it.
  if (first && transfer) pages.push("transfer");
  if (step.step === "migrate") {
    pages.push("migrate");
    return pages;
  }
  if (step.step === "offline" || step.step === "notConfigured") pages.push("network");
  pages.push("signIn");
  if ("account" in step && asksPassword(step.account)) pages.push("password");
  pages.push("pin", "appearance", "settingUp");
  return pages;
}

/**
 * Where Back leads from a step, or null where there is no going back (setting up, signing in,
 * migrating). `transfer`: the first account's assistant offers Transfer Your Data.
 * `first`: the computer's first account, whose assistant begins at the welcome; otherwise Back
 * from the beginning closes the setup (`"close"`). Going back past the sign-in gives it up.
 */
export function backFrom(step: SetupStep, first: boolean, transfer = false): SetupStep | "close" | "signIn" | null {
  const start = first ? ({ step: transfer ? "transfer" : "welcome" } as const) : "close";
  switch (step.step) {
    case "welcome":
      return first ? null : "close";
    case "transfer":
      return { step: "welcome" };
    case "migrate": {
      const to = migrateBack(step.migrate);
      return to === "transfer" ? { step: "transfer" } : to === null ? null : { step: "migrate", migrate: to };
    }
    case "checking":
    case "offline":
    case "notConfigured":
    case "signIn":
      return start;
    case "password":
      return "signIn";
    case "pin":
      if (step.first !== undefined) return { step: "pin", account: step.account };
      return asksPassword(step.account) ? { step: "password", account: step.account } : "signIn";
    case "appearance":
      return { step: "pin", account: step.account };
    case "settingUp":
      // Only a setup that ended may be changed and sent again: one the service keeps
      // (`unreachable`, `machine`) is tried again as it is, never abandoned for a new one, which
      // would spend the sign-in.
      return step.problem === "failed" ? { step: "appearance", account: step.account } : null;
    case "secondFactor":
      return { step: "appearance", account: step.account };
    case "signingIn":
      return null;
  }
}

/** Where an answer of the accounts' service leads the setup of `account`. */
export type AfterEnroll =
  | { next: "enrolled"; user: string }
  | { next: "secondFactor"; invalid: boolean }
  /** The password again (only an account that is asked it): `wait` seconds before another try. */
  | { next: "password"; problem: "incorrect" | "throttled" | "unusable"; wait: number }
  /** The PIN again: not one the service takes (not digits, or the password itself). */
  | { next: "pin" }
  | { next: "problem"; problem: SetupProblem };

/** Where an answer of the accounts' service leads the setup of `account`. */
export function afterEnroll(account: SignedInAccount, answer: EnrollAnswer): AfterEnroll {
  if (answer === "secondFactorRequired") return { next: "secondFactor", invalid: false };
  if ("enrolled" in answer) return { next: "enrolled", user: answer.enrolled.user };
  const reason = "failed" in answer ? answer.failed.reason : answer.refused.reason;
  // A password the setup sent without asking (typed in the sign-in) that Oxy refuses is a
  // sign-in to do again: the password field is only for an account that was asked it.
  const password = (problem: "incorrect" | "throttled" | "unusable", wait = 0): AfterEnroll =>
    asksPassword(account) ? { next: "password", problem, wait } : { next: "problem", problem: "expired" };
  switch (reason) {
    case "secondFactorInvalid":
      return { next: "secondFactor", invalid: true };
    case "wrongPassword":
      return password("incorrect");
    case "locked": {
      const wait = "refused" in answer ? answer.refused.retryAfterSeconds : 0;
      return password("throttled", Math.max(0, Math.ceil(wait ?? 0)));
    }
    case "unusablePassword":
      return password("unusable");
    case "unusablePin":
    case "pinRequired":
      return { next: "pin" };
    case "offline":
    case "unreachable":
      return { next: "problem", problem: "unreachable" };
    case "localFailure":
      return { next: "problem", problem: "machine" };
    case "credentialRefused":
    case "accountNotOnDevice":
    case "accountMismatch":
    case "invalidAccount":
    case "signInAgain":
      return { next: "problem", problem: "expired" };
    default:
      return { next: "problem", problem: "failed" };
  }
}

/** An answer as the platform hands it over: anything that is not one is a failure. */
export function enrollAnswerFrom(told: unknown): EnrollAnswer {
  if (told === "secondFactorRequired") return told;
  if (typeof told === "object" && told !== null) {
    const answer = told as Record<string, Record<string, unknown> | undefined>;
    const enrolled = answer.enrolled;
    if (enrolled && typeof enrolled.accountId === "string" && typeof enrolled.user === "string" && enrolled.user) {
      return { enrolled: { accountId: enrolled.accountId, user: enrolled.user } };
    }
    const refused = answer.refused;
    if (refused && typeof refused.reason === "string") {
      const wait = refused.retryAfterSeconds;
      return { refused: { reason: refused.reason, retryAfterSeconds: typeof wait === "number" ? wait : null } };
    }
    const failed = answer.failed;
    if (failed && typeof failed.reason === "string") return { failed: { reason: failed.reason } };
  }
  return { failed: { reason: "invalidAnswer" } };
}

/** Where Back leads from a step of the migration: the step before, `transfer` (the assistant's
    Transfer Your Data, giving the backups up), or null while it runs. */
export function migrateBack(step: MigrateStep): MigrateStep | "transfer" | null {
  const source: MigrateStep = { step: "source", disks: null, problem: null };
  switch (step.step) {
    case "source":
    case "failed":
      return "transfer";
    case "share":
    case "password":
    case "computer":
      return source;
    case "backup":
      return step.several ? { step: "computer", computers: step.several } : source;
    case "confirm":
      return { step: "backup", computer: step.computer, several: step.several };
    case "migrating":
      return null;
  }
}
