/**
 * The account's local sign-in (docs/adr/0020-oxy-account-login.mdx, decision 2), as the accounts'
 * service tells it to the lock screen and Settings: its PIN for this computer (`set`, or `locked`
 * after too many wrong ones) and whether its password signs in here. Never a PIN or a verifier.
 */
export type LocalSignIn = {
  /** `unknown`: the service could not say, and never said before (see `localSignInUnread`). */
  pin?: "set" | "locked" | "unknown";
  /** False: the PIN is the account's one local credential. */
  password: boolean;
};

/** What the platform handed over, read strictly: anything else is the password's alone, as an
    account set up before PINs signs in. */
export function localSignInFrom(told: unknown): LocalSignIn {
  if (typeof told !== "object" || told === null) return { pin: undefined, password: true };
  const { pin, password } = told as Record<string, unknown>;
  return {
    pin: pin === "set" || pin === "locked" ? pin : undefined,
    password: typeof password === "boolean" ? password : true,
  };
}

/**
 * What the lock screen goes by after asking the service, given what it knew before (`known`) and the
 * platform's answer (`told`; `undefined` when the call itself failed). An answer is read as
 * `localSignInFrom` reads it. Anything else (`{failed: …}`: refused, restarting, unreachable) is no
 * news about the account: what was said last stands. A PIN it had is still checked by the
 * session's PAM (`oxyos-pin-check` asks the service on its own socket, whatever the session socket
 * answers), so the lock never trades it for a password field the account may not sign in with
 * (issue #477: a person was locked out with their PIN unoffered while the service refused the
 * session). Never said: the password's field, with the PIN's offered beside it (`unknown`), so
 * whichever the account has is there.
 */
export function localSignInAfter(known: LocalSignIn | undefined, told: unknown): LocalSignIn {
  const answered = typeof told === "object" && told !== null && !("failed" in told);
  if (answered) return localSignInFrom(told);
  return known ?? { pin: "unknown", password: true };
}
