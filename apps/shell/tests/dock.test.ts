import { test } from "node:test";
import assert from "node:assert/strict";
import {
  dockApps,
  dockMenu,
  keepInDock,
  openAtLogin,
  trashMenu,
  type DockAbilities,
  type DockMenuItem,
} from "../src/dock.ts";
import type { ShellApplication, ShellSnapshot, ShellWindow } from "../src/model.ts";

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
  applications: [app("stash", true), app("inbox", true), app("allo"), app("settings")],
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
    ["stash", "inbox", "allo"],
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
  );
  assert.deepEqual(shown(menu).slice(0, 2), ["inbox 1", "[x] inbox 2"]);
});

test("Option held, or an application not responding, makes Quit Force Quit", () => {
  const running = snapshot([window("1", "allo")]);
  assert.equal(shown(dockMenu(running, "allo", ALL, true)).at(-1), "Forzar salida");
  const stuck = snapshot([window("1", "allo", { responding: false })]);
  const menu = shown(dockMenu(stuck, "allo", ALL));
  assert.equal(menu[0], "(La aplicación no responde)");
  assert.equal(menu.at(-1), "Forzar salida");
  // Where the platform cannot force-quit, Quit stays.
  assert.equal(shown(dockMenu(stuck, "allo", { ...ALL, forceQuit: false })).at(-1), "Salir");
});

test("an application not running offers Open and its options, unchecked when it is not kept", () => {
  assert.deepEqual(shown(dockMenu(snapshot([]), "inbox", ALL)), [
    "Abrir",
    "Opciones > [x] Mantener en el Dock | Abrir al iniciar sesión",
  ]);
  // Nothing the platform cannot do is offered.
  assert.deepEqual(shown(dockMenu(snapshot([]), "inbox", { ...ALL, keepInDock: false, openAtLogin: false })), [
    "Abrir",
  ]);
});

test("keeping in the dock and opening at login change the lists in their order", () => {
  const shownSnapshot = snapshot([window("1", "allo")], { openAtLogin: ["stash"] });
  assert.deepEqual(keepInDock(shownSnapshot, "allo", true), ["stash", "inbox", "allo"]);
  assert.deepEqual(keepInDock(shownSnapshot, "inbox", false), ["stash"]);
  assert.deepEqual(openAtLogin(shownSnapshot, "allo", true), ["stash", "allo"]);
  assert.deepEqual(openAtLogin(shownSnapshot, "stash", false), []);
  // The commands say what changes: unkeeping a kept one, keeping one that is not.
  const options = dockMenu(shownSnapshot, "allo", ALL).find((item) => item.kind === "submenu");
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
  assert.deepEqual(trashMenu(0, true), [
    { id: "open", label: "Abrir", enabled: true },
    { id: "empty", label: "Vaciar Papelera", enabled: false },
  ]);
  assert.equal(trashMenu(3, true)[1]?.enabled, true);
  assert.deepEqual(
    trashMenu(3, false).map((entry) => entry.id),
    ["open"],
  );
});
