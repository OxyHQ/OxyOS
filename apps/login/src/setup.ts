/**
 * Adding an account at the login screen (docs/adr/0020-oxy-account-login.mdx, decisions 2 and 4,
 * step 2): online only. The SDK's own sign-in (the platform's, with OxyOS's client id from the
 * accounts' service) obtains a device credential, which the platform takes over at once and holds.
 * An account with a password gives it once more, then chooses a PIN for this computer (or not, for
 * now); an account without one chooses a PIN, and is never asked to make an Oxy password. A
 * sign-in by the QR sets a PIN alone, and a password typed in the sign-in is taken as it was typed
 * (`SignedInAccount`). The accounts' service sets the account up with them, asking a second factor
 * on the same conversation when Oxy does; a PIN alone is redeemed by Oxy's device-proof route.
 * What came of each answer, apart from how it is shown: nothing here decides who may sign in.
 */

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
 * account has is asked once (the PIN optional) and an account without one sets a PIN.
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

/** The accounts' service's answer to a setup's turn, or why it could not be asked. */
export type EnrollAnswer =
  | { enrolled: { accountId: string; user: string } }
  | "secondFactorRequired"
  | { refused: { reason: string; retryAfterSeconds?: number | null } }
  | { failed: { reason: string } };

/** Where setup stands. */
export type SetupStep =
  | { step: "checking" }
  | { step: "offline" }
  /** The machine sets up no account: the accounts' service names no client id, or cannot be read. */
  | { step: "notConfigured" }
  | { step: "signIn"; platform: SetupPlatform }
  /** The password once more, for the account signed in. */
  | { step: "password"; account: SignedInAccount }
  /** A PIN for this computer; `first`: the one typed, to be typed again; `problem`: why the last
      was not taken. */
  | { step: "pin"; account: SignedInAccount; first?: string; problem?: "mismatch" | "unusable" }
  | { step: "secondFactor"; account: SignedInAccount; invalid: boolean };

/** What a password's or a code's answer leads to. */
export type AfterEnroll =
  | { next: "enrolled"; user: string }
  | { next: "secondFactor"; invalid: boolean }
  /** The password again: `incorrect` shakes the field, `wait` seconds before another try. */
  | { next: "password"; problem: "incorrect" | "throttled" | "unusable" | "failed"; wait: number }
  /** The PIN again: not one the service takes (not digits, or the password itself). */
  | { next: "pin" }
  /** The sign-in no longer holds (its credential refused, another account's): sign in again. */
  | { next: "signIn" }
  | { next: "offline" };

/** Where an answer of the accounts' service leads, at the password or at the second factor. */
export function afterEnroll(answer: EnrollAnswer, _at?: "password" | "secondFactor"): AfterEnroll {
  if (answer === "secondFactorRequired") return { next: "secondFactor", invalid: false };
  if ("enrolled" in answer) return { next: "enrolled", user: answer.enrolled.user };
  if ("failed" in answer) return { next: "password", problem: "failed", wait: 0 };
  const { reason, retryAfterSeconds } = answer.refused;
  switch (reason) {
    case "secondFactorInvalid":
      return { next: "secondFactor", invalid: true };
    case "wrongPassword":
      return { next: "password", problem: "incorrect", wait: 0 };
    case "locked":
      return { next: "password", problem: "throttled", wait: Math.max(0, Math.ceil(retryAfterSeconds ?? 0)) };
    case "unusablePassword":
      return { next: "password", problem: "unusable", wait: 0 };
    case "unusablePin":
      return { next: "pin" };
    case "pinRequired":
      return { next: "pin" };
    case "offline":
      return { next: "offline" };
    case "credentialRefused":
    case "accountNotOnDevice":
    case "accountMismatch":
    case "invalidAccount":
      return { next: "signIn" };
    default:
      // Any other refusal ends the conversation: the password again starts a new one.
      return { next: "password", problem: "failed", wait: 0 };
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
