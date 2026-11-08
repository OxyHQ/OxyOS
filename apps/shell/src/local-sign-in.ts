/**
 * The account's local sign-in (docs/adr/0020-oxy-account-login.mdx, decision 2), as the accounts'
 * service tells it to the lock screen and Settings: its PIN for this computer (`set`, or `locked`
 * after too many wrong ones) and whether its password signs in here. Never a PIN or a verifier.
 */
export type LocalSignIn = {
  pin?: "set" | "locked";
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
