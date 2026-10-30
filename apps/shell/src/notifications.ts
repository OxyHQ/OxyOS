import type { ShellApplication, ShellNotification } from "./model";

/** How long a banner stays, as a Mac's: five seconds, unless the person is replying in it. */
export const BANNER_MS = 5000;

/** An application's notifications in the Notification Center: the newest first. */
export type NotificationGroup = {
  appId: string;
  name: string;
  app?: ShellApplication;
  notifications: ShellNotification[];
};

/** The application a notification is from, by the registry; its id when the registry has none. */
export function applicationOf(
  notification: ShellNotification,
  applications: readonly ShellApplication[],
): { name: string; app?: ShellApplication } {
  const app = applications.find((candidate) => candidate.id === notification.appId);
  return { name: app?.name ?? notification.appId, app };
}

/**
 * The Notification Center's groups, as a Mac's: one per application, the one with the newest
 * notification first, each newest first.
 */
export function groupNotifications(
  list: readonly ShellNotification[],
  applications: readonly ShellApplication[],
): NotificationGroup[] {
  const groups = new Map<string, NotificationGroup>();
  for (const notification of list) {
    let group = groups.get(notification.appId);
    if (!group) {
      group = { appId: notification.appId, ...applicationOf(notification, applications), notifications: [] };
      groups.set(notification.appId, group);
    }
    group.notifications.push(notification);
  }
  const newest = (group: NotificationGroup) => Math.max(...group.notifications.map((n) => n.postedAt));
  for (const group of groups.values()) group.notifications.sort((a, b) => b.postedAt - a.postedAt);
  return [...groups.values()].sort((a, b) => newest(b) - newest(a));
}

/**
 * The newest notification that arrived as a banner and was not shown yet, among `list`: the one
 * to show now. `seen` holds every one the shell has known (a banner is shown once, and one told
 * again, as the session unlocks or the shell starts again, is none).
 */
export function nextBanner(
  list: readonly ShellNotification[],
  seen: ReadonlySet<string>,
): ShellNotification | undefined {
  return list
    .filter((notification) => notification.banner && !notification.withheld && !seen.has(notification.id))
    .sort((a, b) => b.postedAt - a.postedAt)[0];
}

/** What the lock screen says of an application's notifications without their previews. */
export function countLabel(count: number): string {
  return count === 1 ? "1 notificación" : `${count} notificaciones`;
}

/**
 * The lock screen's summary: per application, newest first, how many it has, and, where the
 * preferences allow previews and the session told them, its newest one's title and body.
 */
export function lockSummary(
  list: readonly ShellNotification[],
  applications: readonly ShellApplication[],
  previews: boolean,
): { appId: string; name: string; app?: ShellApplication; count: string; preview?: { title: string; body: string } }[] {
  return groupNotifications(list, applications).map((group) => {
    const newest = group.notifications[0];
    return {
      appId: group.appId,
      name: group.name,
      app: group.app,
      count: countLabel(group.notifications.length),
      preview:
        previews && !newest.withheld ? { title: newest.title, body: newest.body } : undefined,
    };
  });
}

/** How long ago, as the Notification Center says it. */
export function ago(postedAt: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - postedAt) / 60000);
  if (minutes < 1) return "ahora";
  if (minutes < 60) return `hace ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `hace ${hours} h`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "ayer" : `hace ${days} días`;
}

/** What a screen reader says of a banner: whose it is, then what it says. */
export function spoken(notification: ShellNotification, name: string): string {
  return [name, notification.title, notification.subtitle, notification.body].filter(Boolean).join(". ");
}
