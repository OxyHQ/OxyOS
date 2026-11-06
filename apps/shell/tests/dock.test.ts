import { test } from "node:test";
import assert from "node:assert/strict";
import {
  appWindowIds,
  dockApps,
  dockMenu,
  dockPress,
  DOCK_SLOT,
  gapAt,
  keepInDock,
  keptWithAt,
  moveInDock,
  openAtLogin,
  trashMenu,
  trashedByDrop,
  windowIdsByApp,
  windowIdsKey,
  type DockAbilities,
  type DockMenuItem,
} from "../src/dock.ts";
import type { ShellApplication, ShellSnapshot, ShellWindow } from "../src/model.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");

const app = (id: string, pinned = false): ShellApplication => ({ id, name: id, icon: { svg: "" }, pinned });
const window = (id: string, appId: string, extra: Partial<ShellWindow> = {}): ShellWindow => ({
  id,
  appId,
  title: `${appId} ${id}`,
  workspaceId: "0",
  launched: true,
  ...extra,
});
const snapshot = (windows: ShellWindow[], extra: Partial<ShellSnapshot> = {}): ShellSnapshot => ({
  applications: [app("dropsea", true), app("inbox", true), app("allo"), app("settings")],
  folders: [],
  windows,
  workspaces: [{ id: "0", name: "Escritorio 1" }],
  workspaceId: "0",
  menus: [],
  ...extra,
});
const ALL: DockAbilities = {
  hide: true,
  quit: true,
  forceQuit: true,
  keepInDock: true,
  openAtLogin: true,
  showAllWindows: true,
};
/** Each entry as the menu shows it: a label, `[x] ` before a checked one, `---` for a separator. */
const shown = (items: readonly DockMenuItem[]): string[] =>
  items.map((item) => {
    if (item.kind === "separator") return "---";
    if (item.kind === "submenu") return `${item.label} > ${shown(item.items).join(" | ")}`;
    if (item.kind === "label") return `(${item.label})`;
    return `${item.checked ? "[x] " : ""}${item.label}`;
  });

test("the dock shows the applications kept, in its order, then the others running", () => {
  const shownApps = dockApps(snapshot([window("1", "allo"), window("2", "settings", { launched: false })]));
  // A window the session did not start is no registry application's.
  assert.deepEqual(
    shownApps.map((shownApp) => shownApp.id),
    ["dropsea", "inbox", "allo"],
  );
});

test("a running application's menu lists its windows, the one it used last checked, then its commands", () => {
  const menu = dockMenu(
    snapshot([window("1", "inbox", { used: 4 }), window("2", "inbox", { used: 2 }), window("3", "allo", { used: 5 })], {
      activeAppId: "inbox",
      openAtLogin: ["inbox"],
    }),
    "inbox",
    ALL,
    ES,
  );
  assert.deepEqual(shown(menu), [
    "[x] inbox 1",
    "inbox 2",
    "---",
    "Mostrar todas las ventanas",
    "Ocultar",
    "Opciones > [x] Mantener en el Dock | [x] Abrir al iniciar sesión",
    "---",
    "Salir",
  ]);
});

test("the active window is the one checked", () => {
  const menu = dockMenu(
    snapshot([window("1", "inbox", { used: 4 }), window("2", "inbox", { used: 2, focused: true })]),
    "inbox",
    ALL,
    ES,
  );
  assert.deepEqual(shown(menu).slice(0, 2), ["inbox 1", "[x] inbox 2"]);
});

test("Option held, or an application not responding, makes Quit Force Quit", () => {
  const running = snapshot([window("1", "allo")]);
  assert.equal(shown(dockMenu(running, "allo", ALL, ES, true)).at(-1), "Forzar salida");
  const stuck = snapshot([window("1", "allo", { responding: false })]);
  const menu = shown(dockMenu(stuck, "allo", ALL, ES));
  assert.equal(menu[0], "(La aplicación no responde)");
  assert.equal(menu.at(-1), "Forzar salida");
  // Where the platform cannot force-quit, Quit stays.
  assert.equal(shown(dockMenu(stuck, "allo", { ...ALL, forceQuit: false }, ES)).at(-1), "Salir");
});

test("an application not running offers Open and its options, unchecked when it is not kept", () => {
  assert.deepEqual(shown(dockMenu(snapshot([]), "inbox", ALL, ES)), [
    "Abrir",
    "Opciones > [x] Mantener en el Dock | Abrir al iniciar sesión",
  ]);
  // Nothing the platform cannot do is offered.
  assert.deepEqual(shown(dockMenu(snapshot([]), "inbox", { ...ALL, keepInDock: false, openAtLogin: false }, ES)), [
    "Abrir",
  ]);
});

test("keeping in the dock and opening at login change the lists in their order", () => {
  const shownSnapshot = snapshot([window("1", "allo")], { openAtLogin: ["dropsea"] });
  assert.deepEqual(keepInDock(shownSnapshot, "allo", true), ["dropsea", "inbox", "allo"]);
  assert.deepEqual(keepInDock(shownSnapshot, "inbox", false), ["dropsea"]);
  assert.deepEqual(openAtLogin(shownSnapshot, "allo", true), ["dropsea", "allo"]);
  assert.deepEqual(openAtLogin(shownSnapshot, "dropsea", false), []);
  // The commands say what changes: unkeeping a kept one, keeping one that is not.
  const options = dockMenu(shownSnapshot, "allo", ALL, ES).find((item) => item.kind === "submenu");
  assert.ok(options?.kind === "submenu");
  assert.deepEqual(
    options.items.map((item) => (item.kind === "item" ? item.command : null)),
    [
      { kind: "keep-in-dock", keep: true },
      { kind: "open-at-login", open: true },
    ],
  );
});

test("the Trash offers emptying only while it holds anything", () => {
  assert.deepEqual(trashMenu(0, true, ES), [
    { id: "open", label: "Abrir", enabled: true },
    { id: "empty", label: "Vaciar Papelera", enabled: false },
  ]);
  assert.equal(trashMenu(3, true, ES)[1]?.enabled, true);
  assert.deepEqual(
    trashMenu(3, false, ES).map((entry) => entry.id),
    ["open"],
  );
});

test("a drop on the Trash puts in it the dropped files with a file URI, once each", () => {
  assert.deepEqual(
    trashedByDrop([
      { uri: "file:///home/ana/informe.txt" },
      { uri: "file:///home/ana/informe.txt" },
      { uri: "https://oxy.so/os" },
      {},
      { uri: "FILE:///home/ana/Fotos/" },
    ]),
    ["file:///home/ana/informe.txt", "FILE:///home/ana/Fotos/"],
  );
  assert.deepEqual(trashedByDrop([]), []);
});

test("an application's icon is where its windows minimize into, the minimized ones among them", () => {
  const windows = [
    window("7", "allo"),
    window("8", "inbox"),
    window("9", "allo", { minimized: true }),
    window("10", "allo", { launched: false }),
  ];
  // A window the session did not start is no registry application's.
  assert.deepEqual(appWindowIds(windows, "allo"), ["7", "9"]);
  assert.deepEqual(appWindowIds(windows, "dropsea"), []);
  const applications = dockApps(snapshot(windows));
  const key = windowIdsKey(applications, windows);
  assert.deepEqual(
    [...windowIdsByApp(key)],
    [
      ["dropsea", ""],
      ["inbox", "8"],
      ["allo", "7\n9"],
    ],
  );
  assert.deepEqual([...windowIdsByApp("")], []);
  // The same while the same windows are open, whatever else of them changes; not once one opens.
  const retitled = windows.map((win) => ({ ...win, title: `${win.title} (2)`, minimized: !win.minimized }));
  assert.equal(windowIdsKey(applications, retitled), key);
  assert.notEqual(windowIdsKey(applications, [...windows, window("11", "inbox")]), key);
});

import { iconScroll, NO_ICON_SCROLL } from "../src/dock.ts";

test("a scroll on a dock icon: a wheel's notch up or down at once, a touchpad's once it went far enough", () => {
  const wheel = (dy: number) => iconScroll(NO_ICON_SCROLL, { dx: 0, dy, finger: false, end: false }).direction;
  assert.equal(wheel(-15), "up");
  assert.equal(wheel(15), "down");
  assert.equal(iconScroll(NO_ICON_SCROLL, { dx: 15, dy: 0, finger: false, end: false }).direction, null);
  // Two fingers: 5 and 5 up do nothing, 5 more opens it, and nothing more until they lift.
  let state = NO_ICON_SCROLL;
  const said: (string | null)[] = [];
  for (const dy of [-5, -5, -5, -5, -30]) {
    const next = iconScroll(state, { dx: 0, dy, finger: true, end: false });
    state = next.state;
    said.push(next.direction);
  }
  assert.deepEqual(said, [null, null, "up", null, null]);
  state = iconScroll(state, { dx: 0, dy: 0, finger: true, end: true }).state;
  assert.equal(iconScroll(state, { dx: 0, dy: 20, finger: true, end: false }).direction, "down");
  // Across, never.
  assert.equal(iconScroll(NO_ICON_SCROLL, { dx: 40, dy: 3, finger: true, end: false }).direction, null);
});

test("a press on the frontmost application's icon minimizes its windows shown here, the frontmost last", () => {
  const state = snapshot(
    [
      window("1", "allo", { used: 3, focused: true }),
      window("2", "allo", { used: 1 }),
      window("3", "allo", { used: 2, minimized: true }),
      window("4", "allo", { used: 4, workspaceId: "1" }),
      window("5", "inbox", { used: 0 }),
    ],
    { activeAppId: "allo" },
  );
  // The minimized one stays as it is, and the one on another desktop is not shown here.
  assert.deepEqual(dockPress(state, "allo"), { kind: "minimize", windows: ["2", "1"] });
  // Over Launchpad, Mission Control or App Exposé it comes forward: nothing the person cannot see
  // is minimized.
  assert.deepEqual(dockPress(state, "allo", { desktop: false }), { kind: "activate", hide: [] });
  // Option on the frontmost application hides it, as a Mac's Dock does.
  assert.deepEqual(dockPress(state, "allo", { option: true }), { kind: "hide" });
});

test("pressed again, the windows the dock minimized come back together; else the one used last alone", () => {
  const state = snapshot(
    [
      window("1", "allo", { used: 3, minimized: true }),
      window("2", "allo", { used: 1, minimized: true }),
      window("3", "allo", { used: 5, minimized: true }),
      window("5", "inbox", { used: 4, focused: true }),
    ],
    { activeAppId: "inbox" },
  );
  assert.deepEqual(dockPress(state, "allo", { minimizedByDock: ["2", "1"] }), {
    kind: "restore",
    windows: ["2", "1"],
    hide: [],
  });
  // One of them restored and gone since: the others still come back.
  assert.deepEqual(dockPress(state, "allo", { minimizedByDock: ["9", "1"] }), {
    kind: "restore",
    windows: ["1"],
    hide: [],
  });
  // Minimized on their own (Super+M, the title bar): the one used last, as a Mac restores.
  assert.deepEqual(dockPress(state, "allo"), { kind: "restore", windows: ["3"], hide: [] });
});

test("an application running and not frontmost comes forward; one not running opens", () => {
  const state = snapshot(
    [
      window("1", "allo", { used: 3, focused: true }),
      window("2", "inbox", { used: 1, minimized: true, hidden: true }),
      window("3", "dropsea", { used: 2, minimized: true }),
      window("4", "settings", { used: 0, launched: false }),
    ],
    { activeAppId: "allo" },
  );
  // Hidden, or with a window minimized among others shown: it comes forward.
  assert.deepEqual(dockPress(state, "inbox"), { kind: "activate", hide: [] });
  assert.deepEqual(dockPress(state, "atlas"), { kind: "open" });
  // A window the session did not start is no registry application's.
  assert.deepEqual(dockPress(state, "settings"), { kind: "open" });
});

test("Option on another application brings it forward and hides every other one shown", () => {
  const state = snapshot(
    [
      window("1", "allo", { used: 3, focused: true }),
      window("2", "inbox", { used: 1 }),
      window("3", "dropsea", { used: 2, minimized: true }),
      window("4", "atlas", { used: 0, minimized: true, hidden: true }),
      window("5", "settings", { used: 4 }),
    ],
    { activeAppId: "allo" },
  );
  // Only what shows hides: Dropsea's one window is minimized, Atlas is hidden already.
  assert.deepEqual(dockPress(state, "inbox", { option: true }), { kind: "activate", hide: ["allo", "settings"] });
  assert.deepEqual(dockPress(state, "dropsea", { option: true }), {
    kind: "restore",
    windows: ["3"],
    hide: ["allo", "inbox", "settings"],
  });
});

test("an application is put among those kept at a place, moved if kept already, kept if not", () => {
  assert.deepEqual(keptWithAt(["a", "b", "c"], "d", 1), ["a", "d", "b", "c"]);
  assert.deepEqual(keptWithAt(["a", "b", "c"], "a", 2), ["b", "c", "a"]);
  assert.deepEqual(keptWithAt(["a", "b", "c"], "c", 0), ["c", "a", "b"]);
  // Past either end: at that end.
  assert.deepEqual(keptWithAt(["a", "b"], "d", 9), ["a", "b", "d"]);
  assert.deepEqual(keptWithAt(["a", "b"], "d", -3), ["d", "a", "b"]);
});

test("a kept icon moves a place left or right, never past the ends; one not kept does not move", () => {
  // Dropsea and Inbox kept, in that order; Allo running, not kept.
  const state = snapshot([window("1", "allo")]);
  assert.deepEqual(moveInDock(state, "inbox", -1), ["inbox", "dropsea"]);
  assert.deepEqual(moveInDock(state, "dropsea", 1), ["inbox", "dropsea"]);
  assert.equal(moveInDock(state, "dropsea", -1), null);
  assert.equal(moveInDock(state, "inbox", 1), null);
  assert.equal(moveInDock(state, "allo", -1), null);
});

test("a drag over the dock opens its gap under the pointer, within those kept", () => {
  assert.equal(gapAt(-30, 3), 0);
  assert.equal(gapAt(0, 3), 0);
  assert.equal(gapAt(DOCK_SLOT - 1, 3), 0);
  assert.equal(gapAt(DOCK_SLOT, 3), 1);
  assert.equal(gapAt(DOCK_SLOT * 2.5, 3), 2);
  // Past those kept (over the applications running, the Trash): their end.
  assert.equal(gapAt(DOCK_SLOT * 7, 3), 3);
  assert.equal(gapAt(DOCK_SLOT * 7, 0), 0);
});
