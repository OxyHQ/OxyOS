/**
 * What the login screen (docs/adr/0020-oxy-account-login.mdx) shows, apart from how: the enrolled
 * accounts as the accounts' service lists them, which one the password is for, and where adding
 * an account stands. Nothing here decides who signs in: the login program does, with PAM.
 */

/** An enrolled account: its Unix user (what the password is for), its name and its picture. */
export type LoginAccount = {
  user: string;
  name: string;
  /** The picture's file (`file://...`), or none. */
  picture?: string | null;
};

/** The accounts as the platform gives them: a list, or null while they cannot be read. */
export type Accounts = LoginAccount[] | null;

/** The accounts' service's `list`, as the platform hands it over: anything else is no account. */
export function accountsFrom(listed: unknown): Accounts {
  if (!Array.isArray(listed)) return null;
  return listed.flatMap((entry): LoginAccount[] => {
    if (typeof entry !== "object" || entry === null) return [];
    const { user, name, picture } = entry as Record<string, unknown>;
    if (typeof user !== "string" || !user) return [];
    return [
      {
        user,
        name: typeof name === "string" && name.trim() ? name : user,
        picture: typeof picture === "string" && picture ? `file://${picture}` : null,
      },
    ];
  });
}

/**
 * The account the password field is for: the one chosen while it is still listed, else the
 * first. None without accounts.
 */
export function chosenAccount(accounts: Accounts, chosen: string | null): LoginAccount | null {
  if (!accounts || accounts.length === 0) return null;
  return accounts.find((account) => account.user === chosen) ?? accounts[0]!;
}

/**
 * Adding an account (ADR 0020, decision 4): setup is online only. `checking` the network,
 * `offline` (it says it needs the network and does nothing else), or `online`: the SDK's own
 * sign-in comes next, which this step of the login screen does not have yet (`unavailable`).
 */
export type Setup = "closed" | "checking" | "offline" | "unavailable";

/** Where setup goes once the network was looked at. */
export function setupAfterCheck(online: boolean): Setup {
  return online ? "unavailable" : "offline";
}

/** The login screen opens on setup when no account is enrolled: the first boot. */
export function opensOnSetup(accounts: Accounts): boolean {
  return accounts !== null && accounts.length === 0;
}
