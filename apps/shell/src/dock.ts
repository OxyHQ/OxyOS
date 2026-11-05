/**
 * The dock's applications and each one's menu (docs/adr/0034-the-dock.mdx), as a Mac's Dock has
 * them: the applications kept in it, in its order, then the others running; and the menu a long
 * press, a secondary click or a Control-click opens on an icon. What the menu offers is worked out
 * here from the platform's snapshot; the platform does what is chosen.
 */
import type { ShellMessages } from "./messages";
import type { ShellApplication, ShellSnapshot, ShellWindow } from "./model";

/** What the dock's menus are worked out from: the snapshot's applications, windows, active
    application and those opened at login. */
export type DockState = Pick<ShellSnapshot, "applications" | "windows" | "activeAppId" | "openAtLogin">;

// model.ts's `belongsTo`, here so this module needs nothing at run time (its tests run in Node).
const belongsTo = (window: ShellWindow, appId: string) =>
  window.launched !== false && window.appId === appId;

/** The applications the dock shows: those kept in it, in its order, then the others running, as the
    registry orders them. A running application not kept stays until it quits, as on a Mac. */
export function dockApps(snapshot: DockState): ShellApplication[] {
  const running = (app: ShellApplication) => snapshot.windows.some((win) => belongsTo(win, app.id));
  return [
    ...snapshot.applications.filter((app) => app.pinned),
    ...snapshot.applications.filter((app) => !app.pinned && running(app)),
  ];
}

/** The applications kept in the dock, by id, in its order. */
export function keptIds(snapshot: DockState): string[] {
  return snapshot.applications.filter((app) => app.pinned).map((app) => app.id);
}

/** The dock's list with `id` kept (where it shows now: after those kept) or not. */
export function keepInDock(snapshot: DockState, id: string, keep: boolean): string[] {
  const kept = keptIds(snapshot).filter((kept) => kept !== id);
  return keep ? [...kept, id] : kept;
}

/** The applications opened at login with `id` among them or not, in their order. */
export function openAtLogin(snapshot: DockState, id: string, open: boolean): string[] {
  const listed = (snapshot.openAtLogin ?? []).filter((listed) => listed !== id);
  return open ? [...listed, id] : listed;
}

/** What an icon's menu can do: a command of the menu, or one of the application's windows. */
export type DockCommand =
  | { kind: "window"; window: string }
  | { kind: "open" }
  | { kind: "show-all-windows" }
  | { kind: "hide" }
  | { kind: "keep-in-dock"; keep: boolean }
  | { kind: "open-at-login"; open: boolean }
  | { kind: "quit" }
  | { kind: "force-quit" };

export type DockMenuItem =
  | { kind: "item"; id: string; label: string; command: DockCommand; checked?: boolean; enabled?: boolean }
  | { kind: "label"; id: string; label: string }
  | { kind: "separator"; id: string }
  | { kind: "submenu"; id: string; label: string; items: DockMenuItem[] };

/** What the platform can do, as its actions say: an entry it cannot do is not offered. */
export type DockAbilities = {
  hide: boolean;
  quit: boolean;
  forceQuit: boolean;
  keepInDock: boolean;
  openAtLogin: boolean;
  showAllWindows: boolean;
};


/** The windows of the application `id`, as its menu lists them: its own, oldest first. */
export function appWindows(snapshot: DockState, id: string): ShellWindow[] {
  return snapshot.windows.filter((win) => belongsTo(win, id));
}

/** The ids of the application `id`'s windows, oldest first, the minimized ones among them: those
    that minimize into its icon (docs/adr/0011-transitions.mdx, the genie). */
export function appWindowIds(windows: readonly ShellWindow[], id: string): string[] {
  return windows.filter((win) => belongsTo(win, id)).map((win) => win.id);
}

/** Each application's window ids (`appWindowIds`, one a line) by its id, as one string: the same
    while the same windows are open, whatever else of them changes (a size, a title), so that the
    dock renders an icon again only when its application's windows come or go. */
export function windowIdsKey(applications: readonly ShellApplication[], windows: readonly ShellWindow[]): string {
  return JSON.stringify(applications.map((app) => [app.id, appWindowIds(windows, app.id).join("\n")]));
}

/** `windowIdsKey`'s strings by application id; an application with no window has the empty string. */
export function windowIdsByApp(key: string): Map<string, string> {
  return new Map(key ? (JSON.parse(key) as [string, string][]) : []);
}

/**
 * The menu of the application `id`'s icon, as a Mac's Dock has it:
 * - a running application's windows, its current one checked; then Show All Windows and Hide;
 * - Options: Keep in Dock (checked while kept), Open at Login;
 * - Quit, or Force Quit while Option is held (`option`) or the application does not respond, which
 *   the menu then says first;
 * - an application not running: Open and Options.
 */
export function dockMenu(
  snapshot: DockState,
  id: string,
  abilities: DockAbilities,
  messages: ShellMessages,
  option = false,
): DockMenuItem[] {
  const words = messages.dock;
  const app = snapshot.applications.find((candidate) => candidate.id === id);
  const windows = appWindows(snapshot, id);
  const running = windows.length > 0;
  const kept = !!app?.pinned;
  const options: DockMenuItem[] = [];
  if (abilities.keepInDock) {
    options.push({
      kind: "item",
      id: "keep-in-dock",
      label: words.keepInDock,
      command: { kind: "keep-in-dock", keep: !kept },
      checked: kept,
    });
  }
  if (abilities.openAtLogin) {
    const atLogin = (snapshot.openAtLogin ?? []).includes(id);
    options.push({
      kind: "item",
      id: "open-at-login",
      label: words.openAtLogin,
      command: { kind: "open-at-login", open: !atLogin },
      checked: atLogin,
    });
  }
  const optionsMenu: DockMenuItem[] = options.length
    ? [{ kind: "submenu", id: "options", label: messages.common.options, items: options }]
    : [];
  if (!running) {
    return [{ kind: "item", id: "open", label: words.open, command: { kind: "open" } }, ...optionsMenu];
  }
  const responding = windows.every((win) => win.responding !== false);
  const items: DockMenuItem[] = [];
  if (!responding) {
    items.push({ kind: "label", id: "not-responding", label: words.notResponding });
    items.push({ kind: "separator", id: "not-responding-separator" });
  }
  // The application's current window, checked: the active one, else the one it used last (the
  // menu's own popup has the keyboard while it is open).
  const current =
    windows.find((win) => win.focused) ??
    windows.reduce<ShellWindow | undefined>((last, win) => ((win.used ?? 0) > (last?.used ?? 0) ? win : last), undefined);
  for (const win of windows) {
    items.push({
      kind: "item",
      id: `window-${win.id}`,
      label: win.title || app?.name || words.untitled,
      command: { kind: "window", window: win.id },
      checked: win === current,
    });
  }
  items.push({ kind: "separator", id: "windows-separator" });
  if (abilities.showAllWindows) {
    items.push({
      kind: "item",
      id: "show-all-windows",
      label: words.showAllWindows,
      command: { kind: "show-all-windows" },
    });
  }
  if (abilities.hide) {
    items.push({ kind: "item", id: "hide", label: words.hide, command: { kind: "hide" } });
  }
  items.push(...optionsMenu);
  const force = (option || !responding) && abilities.forceQuit;
  if (force || abilities.quit) {
    items.push({ kind: "separator", id: "quit-separator" });
    items.push(
      force
        ? { kind: "item", id: "force-quit", label: words.forceQuit, command: { kind: "force-quit" } }
        : { kind: "item", id: "quit", label: words.quit, command: { kind: "quit" } },
    );
  }
  return items;
}

/** What a drop on the Trash puts in it: the dropped files that have a `file://` URI, once each. */
export function trashedByDrop(files: readonly { uri?: string }[]): string[] {
  const uris = files.flatMap((file) => (file.uri?.toLowerCase().startsWith("file:") ? [file.uri] : []));
  return [...new Set(uris)];
}

/** The Trash's menu: Open, and Empty Trash while it holds anything (and the platform can). */
export type TrashCommand = "open" | "empty";
export function trashMenu(
  items: number | undefined,
  canEmpty: boolean,
  messages: ShellMessages,
): { id: TrashCommand; label: string; enabled: boolean }[] {
  return [
    { id: "open", label: messages.dock.open, enabled: true },
    ...(canEmpty ? [{ id: "empty" as const, label: messages.dock.emptyTrash, enabled: (items ?? 0) > 0 }] : []),
  ];
}

/** How far a touchpad's two fingers scroll on a dock icon before it acts (logical pixels). */
export const ICON_SCROLL = 12;

/** A scroll going on over a dock icon: how far it went, and whether it acted. */
export type IconScroll = { moved: number; acted: boolean };
export const NO_ICON_SCROLL: IconScroll = { moved: 0, acted: false };

/**
 * A scroll on a dock icon (docs/adr/0045-app-expose.mdx), as a Mac's Dock takes one: up (as a
 * wheel turned away, the content's view moving up) opens the application's App Exposé, down closes
 * it. A wheel's notch acts at once; a touchpad's fingers once they have gone `ICON_SCROLL` one way,
 * once until they lift. Across does nothing.
 */
export function iconScroll(
  state: IconScroll,
  event: { dx: number; dy: number; finger: boolean; end: boolean },
): { state: IconScroll; direction: "up" | "down" | null } {
  if (event.end) return { state: NO_ICON_SCROLL, direction: null };
  const vertical = Math.abs(event.dy) > Math.abs(event.dx);
  if (!event.finger) {
    return { state: NO_ICON_SCROLL, direction: vertical && event.dy !== 0 ? (event.dy < 0 ? "up" : "down") : null };
  }
  if (state.acted) return { state, direction: null };
  const moved = state.moved + (vertical ? event.dy : 0);
  if (Math.abs(moved) < ICON_SCROLL) return { state: { moved, acted: false }, direction: null };
  return { state: { moved, acted: true }, direction: moved < 0 ? "up" : "down" };
}
