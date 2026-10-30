import type { ShellSnapshot, ShellWindow } from "./model";

// model.ts's `belongsTo`, here so this module needs nothing at run time (its tests run in Node).
const belongsTo = (window: ShellWindow, appId: string) =>
  window.launched !== false && window.appId === appId;

/**
 * The application switcher (docs/adr/0024-system-shortcuts.mdx), as a Mac's ⌘Tab: the running
 * applications, the most recently used first, and the one chosen. It opens on the application
 * used before the active one (going forwards) or on the last (backwards), moves while Super is
 * held, and activates its choice when Super is released.
 */
export type Switcher = {
  /** Application ids, the most recently used first. */
  order: readonly string[];
  index: number;
};

/** The registry's applications that have windows, the most recently used first. */
export function runningApps(snapshot: ShellSnapshot): string[] {
  const used = (id: string) =>
    Math.max(
      0,
      ...snapshot.windows
        .filter((window) => belongsTo(window, id))
        .map((window) => window.used ?? 0),
    );
  return snapshot.applications
    .filter((app) => snapshot.windows.some((window) => belongsTo(window, app.id)))
    .map((app, place) => ({ id: app.id, used: used(app.id), place }))
    .sort((a, b) => b.used - a.used || a.place - b.place)
    .map(({ id }) => id);
}

/** Opens the switcher: forwards, on the application used before the active one (the first when
    none is active); backwards, on the last. None while nothing runs. */
export function openSwitcher(
  snapshot: ShellSnapshot,
  backwards: boolean,
): Switcher | null {
  const order = runningApps(snapshot);
  if (order.length === 0) return null;
  if (backwards) return { order, index: order.length - 1 };
  const activeFirst = order[0] === snapshot.activeAppId;
  return { order, index: activeFirst && order.length > 1 ? 1 : 0 };
}

/** The next (`by` 1) or previous (-1) application, round the ends. */
export function moveSwitcher(switcher: Switcher, by: number): Switcher {
  const count = switcher.order.length;
  return { ...switcher, index: (((switcher.index + by) % count) + count) % count };
}

/** The applications still running, in the order the switcher opened with, the choice kept on
    the same application (or the one after it, when it went). None once nothing runs. */
export function followSwitcher(
  switcher: Switcher,
  snapshot: ShellSnapshot,
): Switcher | null {
  const running = new Set(runningApps(snapshot));
  const order = switcher.order.filter((id) => running.has(id));
  if (order.length === 0) return null;
  if (order.length === switcher.order.length) return switcher;
  const chosen = switcher.order[switcher.index];
  const kept = order.indexOf(chosen);
  if (kept >= 0) return { order, index: kept };
  // The chosen one went: the next one that is still there, else the first.
  const after = switcher.order
    .slice(switcher.index + 1)
    .find((id) => running.has(id));
  return { order, index: after ? order.indexOf(after) : 0 };
}

export function chosen(switcher: Switcher): string {
  return switcher.order[switcher.index];
}
