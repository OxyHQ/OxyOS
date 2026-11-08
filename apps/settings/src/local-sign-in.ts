/**
 * Settings › Oxy Account's sign-in on this computer (docs/adr/0020-oxy-account-login.mdx, decision
 * 2): the account's PIN for this computer (set up, changed, removed) and whether its password signs
 * in here, as the accounts' service tells them and changes them. The person proves each change
 * with the password or the current PIN; the service decides, and Settings says what came of it.
 */
import type { Messages } from "./messages";

/** What the service says of the account's local sign-in: never a PIN or a verifier. */
export type LocalSignIn = { pin: "none" | "set" | "locked"; password: boolean };

/** A change Settings asks for. */
export type LocalChange = "setPin" | "removePin" | "passwordOn" | "passwordOff";

/** What the person gives with it: the new PIN, and the proof (the password or the current PIN). */
export type LocalSecrets = { pin?: string; password?: string; currentPin?: string };

/** What came of a change: the local sign-in after it, or why it was refused (nothing changed). */
export type LocalOutcome =
  | { localSignIn: LocalSignIn }
  | { refused: string; retryAfter?: number }
  | { failed: string };

/** What Settings asks of the platform for it. */
export type LocalSignInActions = {
  read: () => Promise<LocalSignIn | null>;
  change: (change: LocalChange, secrets: LocalSecrets) => Promise<LocalOutcome>;
};

/** The service's local sign-in, as the platform hands it over: anything else is none. */
export function localSignInFrom(told: unknown): LocalSignIn | null {
  if (typeof told !== "object" || told === null) return null;
  const { pin, password } = told as Record<string, unknown>;
  if ((pin !== "none" && pin !== "set" && pin !== "locked") || typeof password !== "boolean") return null;
  return { pin, password };
}

/** A change's answer, as the platform hands it over: anything that is not one is a failure. */
export function localOutcomeFrom(told: unknown): LocalOutcome {
  if (typeof told === "object" && told !== null) {
    const answer = told as Record<string, unknown>;
    const local = localSignInFrom(answer.localSignIn);
    if (local) return { localSignIn: local };
    const refused = answer.refused as Record<string, unknown> | undefined;
    if (refused && typeof refused.reason === "string") {
      return typeof refused.retryAfter === "number"
        ? { refused: refused.reason, retryAfter: refused.retryAfter }
        : { refused: refused.reason };
    }
    const failed = answer.failed as Record<string, unknown> | undefined;
    if (failed && typeof failed.reason === "string") return { failed: failed.reason };
  }
  return { failed: "invalidAnswer" };
}

/** Whether `pin` can be a PIN: 4 to 16 digits. */
export function usablePin(pin: string): boolean {
  return /^[0-9]{4,16}$/.test(pin);
}

/** Which proof a change asks for: the password while it signs in here (turning it on, always:
    Oxy checks it), else the current PIN. */
export function proofFor(change: LocalChange, local: LocalSignIn): "password" | "pin" {
  if (change === "passwordOn" || change === "removePin") return "password";
  if (change === "passwordOff") return "pin";
  return local.password ? "password" : "pin";
}

/** What a refusal says, in the page's words. */
export function localRefusal(outcome: Exclude<LocalOutcome, { localSignIn: LocalSignIn }>, messages: Messages): string {
  const words = messages.localSignIn;
  if ("failed" in outcome) return words.failed;
  switch (outcome.refused) {
    case "wrongPassword":
      return words.wrongPassword;
    case "wrongPin":
      return words.wrongPin;
    case "throttled":
      return words.throttled(Math.max(1, Math.ceil(outcome.retryAfter ?? 1)));
    case "pinLocked":
      return words.pinLockedRefused;
    case "unusablePin":
      return words.unusablePin;
    case "offline":
      return words.offline;
    case "secondFactorRequired":
      return words.secondFactorRequired;
    case "noPin":
      return words.noPin;
    case "passwordSignInOff":
      return words.passwordSignInOff;
    default:
      return words.failed;
  }
}
