/**
 * Who the session is for, as every screen of OxyOS names them (docs/adr/0020-oxy-account-login.mdx,
 * decision 1, as amended on 2026-11-09): the person's Oxy account, from the one place the platform
 * keeps it, the accounts' service's snapshot of the session (`OxyOSAccountSession`, the SDK's
 * `OxyHostSnapshot`). As a Mac shows a person's full name and their short name only where a short
 * name is meant, the full name is the account's display name and the short name its username, and
 * what a screen calls the person is the first of them it has, as Oxy's own apps do. The account's
 * Unix user (`oxy-…`) is an identifier of the machine's: no screen ever shows or says it.
 */

/** The session's user: `name`, the short name (the account's username), and `fullName`, the
    display name; either may be empty. Neither is ever the Unix user's name. */
export type SessionUser = { name: string; fullName?: string };

/** What a screen calls the person: their full name, else their short name; empty when the
    platform names nobody. */
export function personName(user: { name?: string; fullName?: string } | null | undefined): string {
  return user?.fullName?.trim() || user?.name?.trim() || "";
}

/**
 * The session's user from the platform's snapshot of the session: a signed-in account's username
 * and display name. Without one (signed out elsewhere, the accounts' service away for a moment, a
 * development session with no accounts' service), the full name the system keeps for the user
 * (passwd's GECOS, which the accounts' service writes from the same profile), and no short name:
 * never the Unix user's name.
 */
export function sessionUserFrom(snapshot: unknown, systemFullName?: string | null): SessionUser {
  if (typeof snapshot === "object" && snapshot !== null) {
    const { status, account } = snapshot as { status?: unknown; account?: unknown };
    if (status === "signed_in" && typeof account === "object" && account !== null) {
      const { handle, displayName } = account as { handle?: unknown; displayName?: unknown };
      if (typeof handle === "string") {
        const fullName = typeof displayName === "string" ? displayName.trim() : "";
        return fullName ? { name: handle.trim(), fullName } : { name: handle.trim() };
      }
    }
  }
  const fullName = typeof systemFullName === "string" ? systemFullName.trim() : "";
  return fullName ? { name: "", fullName } : { name: "" };
}
