/**
 * The menus of the top bar after the Oxy menu (docs/adr/0021-menu-bar.mdx): the active
 * application's own menu, in bold, then the menus it declared; the desktop's while no window is
 * active. Pure, so their order, words, chords and states are tested without drawing them.
 *
 * Every application has its own menu, as on a Mac, whether it declared menus or not: About *App*,
 * Settings… (Ctrl+,), Hide *App* (Super+H), Hide Others (Alt+Super+H), Show All and Quit *App*
 * (Super+Q). The application's own About and Settings… (Bloom's `MenubarItem action`, told with
 * their role) are hoisted there from where it declared them; without its own About the system's
 * standard About panel stands in, and without Settings… the row is disabled. There is no Services
 * menu.
 */
import { chordLabel } from "./chord.ts";
import type { ShellApplication, ShellMenu, ShellMenuItem, ShellWindow, ShellWorkspace } from "./model";
import type { ShellMessages } from "./messages";

/** One entry of the active window's menus, as the platform told it. */
export type ToldMenuEntry = {
  /** The index of the menu or submenu it is in; -1 for a menu of the bar. */
  parent: number;
  kind: "menu" | "item" | "submenu" | "separator";
  enabled: boolean;
  checkable: boolean;
  checked: boolean;
  label: string;
  /** Its chord in Bloom's spelling (`Mod+N`); empty for none. */
  shortcut: string;
  /** The application's About or Settings…; absent or null for any other. */
  role?: "about" | "settings" | null;
};

/** The active window's menus as the platform told them: window 0 when none is active, no
    entries when it declared none. */
export type ToldMenuBar = {
  window: number;
  serial: number;
  items: readonly ToldMenuEntry[];
};

/** What choosing a row of these menus asks. */
export type AppMenuCommand =
  /** A command the application declared, its About and Settings… included: run in it. */
  | { kind: "declared"; window: number; serial: number; index: number }
  /** The system's standard About panel for the window's application. */
  | { kind: "about"; window: number }
  | { kind: "hide"; window: number }
  /** Every application but the window's; window 0 (the desktop's menu): every one. */
  | { kind: "hideOthers"; window: number }
  | { kind: "showAll" }
  | { kind: "quit"; window: number }
  /** The desktop's Empty Trash…, which the platform asks the person about first. */
  | { kind: "emptyTrash" }
  /** The Window menu's: the compositor minimizes, zooms or lays out the window, or moves it to
      another desktop (by the desktop's id as the snapshot has it). */
  | { kind: "minimize"; window: number }
  | { kind: "zoom"; window: number }
  | { kind: "layout"; window: number; layout: WindowLayout }
  | { kind: "moveToDesktop"; window: number; desktop: string };

/** Where the Window menu asks the compositor to put the window (scene/tiling.rs's `Layout`):
    the whole work area within its margins, its middle at the window's size, a half or quarter,
    an arrangement with the windows behind it, or the frame it had before it was first tiled. */
export type WindowLayout =
  | "fill"
  | "center"
  | "left"
  | "right"
  | "top"
  | "bottom"
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight"
  | "leftAndRight"
  | "rightAndLeft"
  | "topAndBottom"
  | "bottomAndTop"
  | "quarters"
  | "returnToPreviousSize";

const LAYOUTS: readonly WindowLayout[] = [
  "fill",
  "center",
  "left",
  "right",
  "top",
  "bottom",
  "topLeft",
  "topRight",
  "bottomLeft",
  "bottomRight",
  "leftAndRight",
  "rightAndLeft",
  "topAndBottom",
  "bottomAndTop",
  "quarters",
  "returnToPreviousSize",
];

const DECLARED = /^window-menu:(\d+):(\d+):(\d+)$/;
const WINDOW = /^window-command:(minimize|zoom):(\d+)$/;
const LAYOUT = /^window-command:layout:([a-zA-Z]+):(\d+)$/;
const MOVE = /^window-command:move:([^:]+):(\d+)$/;
const OWN = /^app-menu:(about|settings|hide|hide-others|quit):(\d+)$/;

/** The command a row's id asks, or null when it is none of these menus'. */
export function appMenuCommand(id: string): AppMenuCommand | null {
  const declared = DECLARED.exec(id);
  if (declared) {
    return {
      kind: "declared",
      window: Number(declared[1]),
      serial: Number(declared[2]),
      index: Number(declared[3]),
    };
  }
  const windowCommand = WINDOW.exec(id);
  if (windowCommand) return { kind: windowCommand[1] as "minimize" | "zoom", window: Number(windowCommand[2]) };
  const layout = LAYOUT.exec(id);
  if (layout && (LAYOUTS as readonly string[]).includes(layout[1])) {
    return { kind: "layout", window: Number(layout[2]), layout: layout[1] as WindowLayout };
  }
  const move = MOVE.exec(id);
  if (move) return { kind: "moveToDesktop", window: Number(move[2]), desktop: move[1] };
  if (id === "app-menu:show-all") return { kind: "showAll" };
  if (id === "app-menu:empty-trash") return { kind: "emptyTrash" };
  const own = OWN.exec(id);
  if (!own) return null;
  const window = Number(own[2]);
  switch (own[1]) {
    case "about":
      return { kind: "about", window };
    case "hide":
      return { kind: "hide", window };
    case "hide-others":
      return { kind: "hideOthers", window };
    case "quit":
      return { kind: "quit", window };
    default:
      // Settings… with none declared is disabled: never chosen.
      return null;
  }
}

/** What the application menu's Hide Others and Show All can do now: whether another
    application has a window shown, and whether any application is hidden. */
export type ApplicationsShown = { others: boolean; hidden: boolean };

/** Whether an application other than `appId`'s has a window on the screen, and whether any is
    hidden with its application, among `windows`. */
export function applicationsShown(windows: readonly ShellWindow[], appId?: string): ApplicationsShown {
  return {
    others: windows.some((window) => window.appId !== appId && !window.minimized && !window.hidden),
    hidden: windows.some((window) => window.hidden),
  };
}

/** The name of a window's application, as its own menu and its About panel say it: the
    registry's for one the session started, else the window's title, else the id it claims
    (docs/adr/0013-launching-applications.mdx). */
export function applicationName(
  window: Pick<ShellWindow, "appId" | "title" | "launched"> | undefined,
  applications: readonly Pick<ShellApplication, "id" | "name">[],
): string {
  if (!window) return "";
  const registered = window.launched ? applications.find((app) => app.id === window.appId) : undefined;
  return registered?.name || window.title || window.appId || "";
}

/** The desktops the Window menu can move the window to: every desktop, and the one it is on. */
export type WindowDesktops = { desktops: readonly ShellWorkspace[]; current?: string };

/**
 * The system's Window menu (docs/adr/0016-windows.mdx, decision 12), as AppKit adds to every
 * application's: Minimize and Zoom, Fill and Center, Move & Resize with the halves, the quarters,
 * the arrangements and Return to Previous Size, then Move to each other desktop. Each is the
 * compositor's, by the window's number, with the chord the compositor itself takes for it (ADR
 * 0024: Super+M, Super+Alt with F, C, R and the arrows, and with Shift the arrangements). With no
 * window (window 0, the desktop's bar), every row is there and none is offered, as a Mac's Finder
 * shows its Window menu with no window open.
 */
export function windowMenuRows(window: number, messages: ShellMessages, where?: WindowDesktops): ShellMenuItem[] {
  const words = messages.menus;
  const enabled = window !== 0;
  const chord = (keys: readonly string[]) => chordLabel(keys, messages);
  const layout = (name: WindowLayout, label: string, keys?: readonly string[]): ShellMenuItem => ({
    id: `window-command:layout:${name}:${window}`,
    label,
    enabled,
    shortcut: keys ? chord(keys) : undefined,
  });
  const separator = (name: string): ShellMenuItem => ({ kind: "separator", id: `window-command:${name}-separator` });
  const others = (where?.desktops ?? []).filter((desktop) => desktop.id !== where?.current);
  return [
    { id: `window-command:minimize:${window}`, label: words.minimize, enabled, shortcut: chord(["Super", "M"]) },
    { id: `window-command:zoom:${window}`, label: words.zoom, enabled },
    separator("zoom"),
    layout("fill", words.fill, ["Alt", "Super", "F"]),
    layout("center", words.center, ["Alt", "Super", "C"]),
    {
      kind: "submenu",
      id: "window-command:move-and-resize",
      label: words.moveAndResize,
      enabled,
      items: [
        layout("left", words.leftHalf, ["Alt", "Super", "ArrowLeft"]),
        layout("right", words.rightHalf, ["Alt", "Super", "ArrowRight"]),
        layout("top", words.topHalf, ["Alt", "Super", "ArrowUp"]),
        layout("bottom", words.bottomHalf, ["Alt", "Super", "ArrowDown"]),
        separator("halves"),
        layout("topLeft", words.topLeftQuarter),
        layout("topRight", words.topRightQuarter),
        layout("bottomLeft", words.bottomLeftQuarter),
        layout("bottomRight", words.bottomRightQuarter),
        separator("quarters"),
        layout("leftAndRight", words.leftAndRight, ["Alt", "Shift", "Super", "ArrowLeft"]),
        layout("rightAndLeft", words.rightAndLeft, ["Alt", "Shift", "Super", "ArrowRight"]),
        layout("topAndBottom", words.topAndBottom, ["Alt", "Shift", "Super", "ArrowUp"]),
        layout("bottomAndTop", words.bottomAndTop, ["Alt", "Shift", "Super", "ArrowDown"]),
        layout("quarters", words.quarters),
        separator("arrange"),
        layout("returnToPreviousSize", words.returnToPreviousSize, ["Alt", "Super", "R"]),
      ],
    },
    ...(others.length > 0
      ? [
          separator("desktops"),
          ...others.map(
            (desktop): ShellMenuItem => ({
              id: `window-command:move:${desktop.id}:${window}`,
              label: words.moveTo(desktop.name),
              enabled,
            }),
          ),
        ]
      : []),
  ];
}

/** The menus with the system's Window menu: its rows first in a Window menu the application
    declared (as AppKit merges them), else a Window menu of its own after the others. */
function withWindowMenu(menus: ShellMenu[], rows: ShellMenuItem[], messages: ShellMessages): ShellMenu[] {
  const label = messages.menus.window;
  const at = menus.findIndex((menu, index) => index > 0 && menu.label === label);
  if (at < 0) return [...menus, { label, items: rows }];
  const declared = menus[at].items;
  const merged: ShellMenuItem[] =
    declared.length > 0 ? [...rows, { kind: "separator", id: "window-command:declared-separator" }, ...declared] : rows;
  return menus.map((menu, index) => (index === at ? { label, items: merged } : menu));
}

/** The rows of the declared entries in `parent`, without those hoisted into the application's
    own menu, and without the separators their going left at an end or doubled. */
function declaredRows(bar: ToldMenuBar, parent: number, messages: ShellMessages): ShellMenuItem[] {
  const rows = bar.items.flatMap((item, index): ShellMenuItem[] => {
    if (item.parent !== parent || item.role) return [];
    const id = `window-menu:${bar.window}:${bar.serial}:${index}`;
    if (item.kind === "separator") return [{ kind: "separator", id }];
    if (item.kind === "submenu") {
      return [
        { kind: "submenu", id, label: item.label, enabled: item.enabled, items: declaredRows(bar, index, messages) },
      ];
    }
    return [
      {
        id,
        label: item.label,
        enabled: item.enabled,
        checked: item.checkable ? item.checked : undefined,
        shortcut: item.shortcut ? chordLabel(item.shortcut, messages) : undefined,
      },
    ];
  });
  const tidy: ShellMenuItem[] = [];
  for (const row of rows) {
    if (row.kind === "separator" && (tidy.length === 0 || tidy[tidy.length - 1].kind === "separator")) continue;
    tidy.push(row);
  }
  if (tidy[tidy.length - 1]?.kind === "separator") tidy.pop();
  return tidy;
}

/**
 * The bar for the active window (window 0: none, and the caller shows the desktop's): its
 * application's own menu, named `name`, then the menus it declared, in order. A declared menu
 * that held only what was hoisted is left out; one the application declared empty is kept.
 */
export function applicationMenus(
  bar: ToldMenuBar,
  given: string,
  messages: ShellMessages,
  shown: ApplicationsShown,
  where?: WindowDesktops,
): ShellMenu[] {
  const words = messages.menus;
  // An application nothing names still has its own menu, under a word of the catalogue: a menu
  // titled with nothing would be a title no one can see or press.
  const name = given || words.unnamedApp;
  const window = bar.window;
  const command = (index: number) => `window-menu:${window}:${bar.serial}:${index}`;
  const about = bar.items.findIndex((item) => item.kind === "item" && item.role === "about");
  const settings = bar.items.findIndex((item) => item.kind === "item" && item.role === "settings");
  const declaredSettings = settings >= 0 ? bar.items[settings] : undefined;
  const own: ShellMenu = {
    label: name,
    items: [
      about >= 0
        ? { id: command(about), label: words.aboutApp(name), enabled: bar.items[about].enabled }
        : { id: `app-menu:about:${window}`, label: words.aboutApp(name) },
      { kind: "separator", id: "app-menu:about-separator" },
      declaredSettings
        ? {
            id: command(settings),
            label: words.settings,
            enabled: declaredSettings.enabled,
            shortcut: declaredSettings.shortcut ? chordLabel(declaredSettings.shortcut, messages) : undefined,
          }
        : // None declared: shown, as a Mac shows it, and not offered.
          { id: `app-menu:settings:${window}`, label: words.settings, enabled: false, shortcut: chordLabel("Mod+,", messages) },
      { kind: "separator", id: "app-menu:settings-separator" },
      { id: `app-menu:hide:${window}`, label: words.hideApp(name), shortcut: chordLabel(["Super", "H"], messages) },
      {
        id: `app-menu:hide-others:${window}`,
        label: words.hideOthers,
        enabled: shown.others,
        shortcut: chordLabel(["Alt", "Super", "H"], messages),
      },
      { id: "app-menu:show-all", label: words.showAll, enabled: shown.hidden },
      { kind: "separator", id: "app-menu:quit-separator" },
      { id: `app-menu:quit:${window}`, label: words.quitApp(name), shortcut: chordLabel(["Super", "Q"], messages) },
    ],
  };
  const declared = bar.items.flatMap((item, index): ShellMenu[] => {
    if (item.parent !== -1 || item.kind !== "menu") return [];
    const items = declaredRows(bar, index, messages);
    const hoisted = bar.items.some((entry) => entry.parent === index && entry.role);
    return items.length === 0 && hoisted ? [] : [{ label: item.label, items }];
  });
  return withWindowMenu([own, ...declared], windowMenuRows(bar.window, messages, where), messages);
}

/**
 * The bar while no window is active: the desktop's own menu, in bold, as Finder's is on a Mac
 * when no other application is (there is no About here: the computer's is the Oxy menu's), then
 * View. The desktop's menu empties the Trash (where the platform has one; the platform asks
 * first) and hides every application or shows them all again.
 */
export function desktopMenus(
  messages: ShellMessages,
  { trashItems, shown }: { trashItems?: number; shown: ApplicationsShown },
): ShellMenu[] {
  const words = messages.menus;
  const trash: ShellMenuItem[] =
    trashItems === undefined
      ? []
      : [
          { id: "app-menu:empty-trash", label: words.emptyTrash, enabled: trashItems > 0 },
          { kind: "separator", id: "app-menu:trash-separator" },
        ];
  return [
    {
      label: words.desktop,
      items: [
        ...trash,
        {
          id: "app-menu:hide-others:0",
          label: words.hideOthers,
          enabled: shown.others,
          shortcut: chordLabel(["Alt", "Super", "H"], messages),
        },
        { id: "app-menu:show-all", label: words.showAll, enabled: shown.hidden },
      ],
    },
    {
      label: words.view,
      items: [
        { id: "view.overview", label: messages.common.showDesktops },
        { id: "view.launcher", label: words.showApplications },
      ],
    },
    { label: words.window, items: windowMenuRows(0, messages) },
  ];
}
