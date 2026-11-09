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
    display name; either may be empty. Neither is ever the Unix user's name. `picture`, the
    account's picture (its Oxy avatar, which the accounts' service keeps on the machine, so it shows
    offline too) as a `file://` URI; none: the system's default picture. */
export type SessionUser = { name: string; fullName?: string; picture?: string };

/** The account's picture as the snapshot names it (an absolute path of the machine's), as an
    image's URI; anything else is none. */
export function pictureUri(picture: unknown): string | undefined {
  if (typeof picture !== "string" || !picture.startsWith("/") || picture.includes("\0")) return undefined;
  return `file://${picture.split("/").map(encodeURIComponent).join("/")}`;
}

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
      const { handle, displayName, picture } = account as {
        handle?: unknown;
        displayName?: unknown;
        picture?: unknown;
      };
      if (typeof handle === "string") {
        const fullName = typeof displayName === "string" ? displayName.trim() : "";
        const uri = pictureUri(picture);
        return {
          name: handle.trim(),
          ...(fullName ? { fullName } : {}),
          ...(uri ? { picture: uri } : {}),
        };
      }
    }
  }
  const fullName = typeof systemFullName === "string" ? systemFullName.trim() : "";
  return fullName ? { name: "", fullName } : { name: "" };
}
