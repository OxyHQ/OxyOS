/**
 * Login Items & Extensions (docs/adr/0060-login-items-and-background-activity.mdx), as a Mac's
 * General › Login Items & Extensions: the person's one Open at Login list, the dock's too (its
 * menu's Options › Open at Login changes the same list, ADR 0034), and each installed
 * application's background activity, as its account decided it and as the session tells what runs
 * in the background now. Settings keeps no list of its own: what opens at login is the session's
 * `open_at_login`, what may run in the background the install service's decisions.
 */
import type { PrivacyApplication } from "./updates";

/** An application of the session's registry, by id and the name the person knows it by. */
export type RegisteredApplication = { id: string; name: string };

/** A row of Open at Login: an application the session opens at login, named as the registry names
    it (an id it no longer lists, as itself: the session passes over it). */
export type LoginItemRow = { id: string; name: string };

/** The rows of Open at Login, in the list's order. */
export function loginItemRows(
  openAtLogin: readonly string[],
  applications: readonly RegisteredApplication[],
): LoginItemRow[] {
  return openAtLogin.map((id) => ({ id, name: applications.find((app) => app.id === id)?.name ?? id }));
}

/** What + offers: the registry's applications not opened at login yet, by name. */
export function addableApplications(
  openAtLogin: readonly string[],
  applications: readonly RegisteredApplication[],
  language: string,
): RegisteredApplication[] {
  return applications
    .filter((app) => !openAtLogin.includes(app.id))
    .slice()
    .sort((a, b) => a.name.localeCompare(b.name, language));
}

/** The list with `id` added at its end (once). */
export function withLoginItem(openAtLogin: readonly string[], id: string): string[] {
  return openAtLogin.includes(id) ? [...openAtLogin] : [...openAtLogin, id];
}

/** The list without `id`. */
export function withoutLoginItem(openAtLogin: readonly string[], id: string): string[] {
  return openAtLogin.filter((listed) => listed !== id);
}

/** The row selected once `id` is removed: the one after it, else the one before, else none. */
export function selectedAfterRemoving(openAtLogin: readonly string[], id: string): string | null {
  const at = openAtLogin.indexOf(id);
  if (at < 0) return openAtLogin[0] ?? null;
  return openAtLogin[at + 1] ?? openAtLogin[at - 1] ?? null;
}

/** A row of App Background Activity: an installed application that registered its background
    activity (its `background` capability decided), whether the person allows it, and whether its
    process runs in the background now. */
export type BackgroundRow = { id: string; name: string; allowed: boolean; running: boolean };

/** The rows of App Background Activity, by name: each installed application whose background
    activity was decided (granted as it registered, or turned off since). One never registered is
    not there: nothing of it can run in the background. */
export function backgroundRows(apps: readonly PrivacyApplication[] | null | undefined, language: string): BackgroundRow[] {
  return (apps ?? [])
    .filter((app) => app.capabilities.background === "granted" || app.capabilities.background === "denied")
    .map((app) => ({
      id: app.id,
      name: app.name,
      allowed: app.capabilities.background === "granted",
      running: app.capabilities.background === "granted" && app.runningInBackground === true,
    }))
    .sort((a, b) => a.name.localeCompare(b.name, language));
}
