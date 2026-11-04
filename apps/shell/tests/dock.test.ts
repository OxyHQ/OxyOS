import { test } from "node:test";
import assert from "node:assert/strict";
import {
  appWindowIds,
  dockApps,
  dockMenu,
  keepInDock,
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
