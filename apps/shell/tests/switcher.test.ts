import { test } from "node:test";
import assert from "node:assert/strict";
import {
  chosen,
  followSwitcher,
  moveSwitcher,
  openSwitcher,
  runningApps,
} from "../src/switcher.ts";
import type { ShellSnapshot, ShellWindow } from "../src/model.ts";

const app = (id: string) => ({ id, name: id, icon: { svg: "" } });
const window = (id: string, appId: string, used: number, extra: Partial<ShellWindow> = {}) =>
  ({ id, appId, title: id, workspaceId: "0", used, ...extra }) as ShellWindow;
const snapshot = (windows: ShellWindow[], activeAppId?: string): ShellSnapshot => ({
  applications: ["inbox", "allo", "settings", "mention"].map(app),
  folders: [],
  windows,
  workspaces: [{ id: "0", name: "Escritorio 1" }],
  workspaceId: "0",
  activeAppId,
  menus: [],
});

test("running applications come the most recently used first, by their latest window", () => {
  const shown = snapshot([
    window("1", "inbox", 3),
    window("2", "allo", 5),
    window("3", "inbox", 7),
    window("4", "settings", 1),
  ]);
  assert.deepEqual(runningApps(shown), ["inbox", "allo", "settings"]);
});

test("a window the session did not start is no registry application's", () => {
  const shown = snapshot([window("1", "inbox", 9, { launched: false }), window("2", "allo", 1)]);
  assert.deepEqual(runningApps(shown), ["allo"]);
});

test("forwards it opens on the application used before the active one, backwards on the last", () => {
  const windows = [window("1", "inbox", 3), window("2", "allo", 5), window("3", "settings", 1)];
  const forwards = openSwitcher(snapshot(windows, "allo"), false)!;
  assert.deepEqual(forwards.order, ["allo", "inbox", "settings"]);
  assert.equal(chosen(forwards), "inbox");
  assert.equal(chosen(openSwitcher(snapshot(windows, "allo"), true)!), "settings");
  // With none active (the shell has the keyboard), the most recent.
  assert.equal(chosen(openSwitcher(snapshot(windows), false)!), "allo");
  // One application: itself. None: no switcher.
  assert.equal(chosen(openSwitcher(snapshot([window("1", "inbox", 1)], "inbox"), false)!), "inbox");
  assert.equal(openSwitcher(snapshot([]), false), null);
});

test("it moves round the ends both ways", () => {
  const windows = [window("1", "inbox", 3), window("2", "allo", 5), window("3", "settings", 1)];
  let switcher = openSwitcher(snapshot(windows, "allo"), false)!;
  switcher = moveSwitcher(switcher, 1);
  assert.equal(chosen(switcher), "settings");
  switcher = moveSwitcher(switcher, 1);
  assert.equal(chosen(switcher), "allo");
  switcher = moveSwitcher(switcher, -1);
  assert.equal(chosen(switcher), "settings");
});

test("an application quit leaves it, the choice moving on to the next one", () => {
  const windows = [window("1", "inbox", 3), window("2", "allo", 5), window("3", "settings", 1)];
  const switcher = openSwitcher(snapshot(windows, "allo"), false)!;
  // Inbox chosen and quit: Settings after it is chosen.
  const after = followSwitcher(switcher, snapshot([windows[1], windows[2]], "allo"))!;
  assert.deepEqual(after.order, ["allo", "settings"]);
  assert.equal(chosen(after), "settings");
  // Another one going keeps the choice where it was.
  const kept = followSwitcher(switcher, snapshot([windows[0], windows[1]], "allo"))!;
  assert.equal(chosen(kept), "inbox");
  assert.equal(followSwitcher(switcher, snapshot([])), null);
});
