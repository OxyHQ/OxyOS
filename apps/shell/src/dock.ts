/**
 * The dock's applications and each one's menu (docs/adr/0034-the-dock.mdx), as a Mac's Dock has
 * them: the applications kept in it, in its order, then the others running; and the menu a long
 * press, a secondary click or a Control-click opens on an icon. What the menu offers is worked out
 * here from the platform's snapshot; the platform does what is chosen.
 */
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

const WORDS = {
  open: "Abrir",
  showAllWindows: "Mostrar todas las ventanas",
  hide: "Ocultar",
  options: "Opciones",
  keepInDock: "Mantener en el Dock",
  openAtLogin: "Abrir al iniciar sesión",
  quit: "Salir",
  forceQuit: "Forzar salida",
  notResponding: "La aplicación no responde",
  untitled: "Sin título",
};

/** The windows of the application `id`, as its menu lists them: its own, oldest first. */
export function appWindows(snapshot: DockState, id: string): ShellWindow[] {
  return snapshot.windows.filter((win) => belongsTo(win, id));
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
  option = false,
): DockMenuItem[] {
  const app = snapshot.applications.find((candidate) => candidate.id === id);
  const windows = appWindows(snapshot, id);
  const running = windows.length > 0;
  const kept = !!app?.pinned;
  const options: DockMenuItem[] = [];
  if (abilities.keepInDock) {
    options.push({
      kind: "item",
      id: "keep-in-dock",
      label: WORDS.keepInDock,
      command: { kind: "keep-in-dock", keep: !kept },
      checked: kept,
    });
  }
  if (abilities.openAtLogin) {
    const atLogin = (snapshot.openAtLogin ?? []).includes(id);
    options.push({
      kind: "item",
      id: "open-at-login",
      label: WORDS.openAtLogin,
      command: { kind: "open-at-login", open: !atLogin },
      checked: atLogin,
    });
  }
  const optionsMenu: DockMenuItem[] = options.length
    ? [{ kind: "submenu", id: "options", label: WORDS.options, items: options }]
    : [];
  if (!running) {
    return [{ kind: "item", id: "open", label: WORDS.open, command: { kind: "open" } }, ...optionsMenu];
  }
  const responding = windows.every((win) => win.responding !== false);
  const items: DockMenuItem[] = [];
  if (!responding) {
    items.push({ kind: "label", id: "not-responding", label: WORDS.notResponding });
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
      label: win.title || app?.name || WORDS.untitled,
      command: { kind: "window", window: win.id },
      checked: win === current,
    });
  }
  items.push({ kind: "separator", id: "windows-separator" });
  if (abilities.showAllWindows) {
    items.push({
      kind: "item",
      id: "show-all-windows",
      label: WORDS.showAllWindows,
      command: { kind: "show-all-windows" },
    });
  }
  if (abilities.hide) {
    items.push({ kind: "item", id: "hide", label: WORDS.hide, command: { kind: "hide" } });
  }
  items.push(...optionsMenu);
  const force = (option || !responding) && abilities.forceQuit;
  if (force || abilities.quit) {
    items.push({ kind: "separator", id: "quit-separator" });
    items.push(
      force
        ? { kind: "item", id: "force-quit", label: WORDS.forceQuit, command: { kind: "force-quit" } }
        : { kind: "item", id: "quit", label: WORDS.quit, command: { kind: "quit" } },
    );
  }
  return items;
}

/** The Trash's menu: Open, and Empty Trash while it holds anything (and the platform can). */
export type TrashCommand = "open" | "empty";
export function trashMenu(
  items: number | undefined,
  canEmpty: boolean,
): { id: TrashCommand; label: string; enabled: boolean }[] {
  return [
    { id: "open", label: WORDS.open, enabled: true },
    ...(canEmpty ? [{ id: "empty" as const, label: "Vaciar Papelera", enabled: (items ?? 0) > 0 }] : []),
  ];
}
