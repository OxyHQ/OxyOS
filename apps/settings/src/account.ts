/**
 * Settings' Oxy account (docs/adr/0020-oxy-account-login.mdx, decisions 5 and 6), as a Mac's System
 * Settings shows its Apple Account at the top of its sidebar: the machine's Oxy session as the
 * platform tells every application (the Oxy SDK's session host, `OxyHostSnapshot`), and what the
 * person may do with it here: manage the account, sign out (the session ends, back to the login
 * screen) and remove it from this computer. Settings decides none of it: the platform does.
 */

/** The session's account, as the platform's snapshot names it. */
export type OxyAccount = {
  id: string;
  handle: string;
  displayName?: string;
  avatar?: string;
};

/** The platform's snapshot of the machine's Oxy session (the SDK's `OxyHostSnapshot`). */
export type AccountSnapshot = {
  status: "signed_in" | "signed_out" | "unavailable";
  revision: number;
  account: OxyAccount | null;
  online: boolean;
};

/** What came of a request to the platform: done, or why not. */
export type AccountOutcome = { done: true } | { failed: string };

/** What Settings asks of the platform for the account. */
export type AccountActions = {
  /** The account's own management (the Oxy SDK's): absent where Settings cannot offer it. */
  manage?: () => void;
  /** Ends the session, back to the login screen. */
  signOut: () => Promise<AccountOutcome>;
  /** Removes the account from this computer: its sessions end, its credential, verifier, user
      and home are forgotten. Asked only once the person confirmed. */
  remove: () => Promise<AccountOutcome>;
  /** The account's sign-in on this computer: its PIN, and whether its password signs in
      (docs/adr/0020-oxy-account-login.mdx decision 2); none where the platform offers none. */
  local?: import("./local-sign-in").LocalSignInActions;
};

/** What the sidebar's account item and its page show, from the snapshot. */
export type AccountView =
  | { kind: "signed_in"; name: string; handle: string; picture: string | null; initials: string; online: boolean }
  | { kind: "signed_out" }
  | { kind: "unavailable" };

/**
 * The account item, if Settings shows one: only where the platform has an account to show (a
 * session signed in with an Oxy account, or one whose account was signed out elsewhere). A
 * session the platform cannot speak for (none to ask, as a development session started from a
 * terminal has none) shows no account item rather than one that says nothing.
 */
export function accountView(snapshot: AccountSnapshot | null | undefined): AccountView | null {
  if (!snapshot || snapshot.status === "unavailable") return null;
  if (snapshot.status === "signed_out" || !snapshot.account) return { kind: "signed_out" };
  const { account } = snapshot;
  const name = accountName(account);
  return {
    kind: "signed_in",
    name,
    handle: account.handle,
    picture: pictureOf(account.avatar),
    initials: initialsOf(name),
    online: snapshot.online,
  };
}

/** What the account is called: its display name, else its handle. */
export function accountName(account: Pick<OxyAccount, "handle" | "displayName">): string {
  return account.displayName?.trim() || account.handle;
}

/** The account's picture when it is one an image can show (an http(s) or file URL); else none, and
    its initials are shown. */
export function pictureOf(avatar: string | undefined): string | null {
  if (!avatar) return null;
  return /^(https?|file):\/\//i.test(avatar) ? avatar : null;
}

/** Up to two initials of a name, the first letters of its first and last words, as people write
    them ("Ada Lovelace" → "AL", "ada" → "A"). */
export function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/u).filter(Boolean);
  if (words.length === 0) return "";
  const first = [...words[0]!][0] ?? "";
  const last = words.length > 1 ? ([...words[words.length - 1]!][0] ?? "") : "";
  return (first + last).toLocaleUpperCase();
}

/** The snapshot as the platform hands it over: anything that is not one is none. */
export function accountSnapshotFrom(told: unknown): AccountSnapshot | null {
  if (typeof told !== "object" || told === null) return null;
  const { status, revision, account, online } = told as Record<string, unknown>;
  if (status !== "signed_in" && status !== "signed_out" && status !== "unavailable") return null;
  if (typeof revision !== "number" || typeof online !== "boolean") return null;
  let kept: OxyAccount | null = null;
  if (account != null) {
    const { id, handle, displayName, avatar } = account as Record<string, unknown>;
    if (typeof id !== "string" || typeof handle !== "string" || !id || !handle) return null;
    kept = {
      id,
      handle,
      ...(typeof displayName === "string" ? { displayName } : {}),
      ...(typeof avatar === "string" ? { avatar } : {}),
    };
  }
  if ((status === "signed_in") !== (kept !== null)) return null;
  return { status, revision, account: kept, online };
}
