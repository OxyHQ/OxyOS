import { test } from "node:test";
import assert from "node:assert/strict";
import { exposedWindows } from "../src/expose.ts";
import type { ShellWindow } from "../src/model.ts";

const window = (id: string, appId: string, used: number, extra: Partial<ShellWindow> = {}): ShellWindow => ({
  id,
  appId,
  title: id,
  workspaceId: "1",
  used,
  ...extra,
});

test("App Exposé: the application's windows, the one used last first, the minimized apart", () => {
  const windows = [
    window("a", "so.oxy.allo", 1),
    window("b", "so.oxy.inbox", 9),
    window("c", "so.oxy.allo", 5),
    window("d", "so.oxy.allo", 7, { minimized: true }),
    window("e", "so.oxy.allo", 3, { hidden: true, workspaceId: "2" }),
    // A window claiming the application the session did not start is not its.
    window("f", "so.oxy.allo", 8, { launched: false }),
  ];
  const { open, minimized } = exposedWindows({ windows }, "so.oxy.allo");
  assert.deepEqual(open.map((w) => w.id), ["c", "e", "a"]);
  assert.deepEqual(minimized.map((w) => w.id), ["d"]);
  assert.deepEqual(exposedWindows({ windows }, "so.oxy.settings"), { open: [], minimized: [] });
});

// A window whose application does not answer (docs/adr/0067-hangs-and-force-quit.mdx): Mission
// Control and App Exposé say so after its title, as a Mac's Force Quit and Windows' title bars do.
test("a hung application's window is titled (Not Responding)", async () => {
  const { shownTitle } = await import("../src/expose.ts");
  const { shellMessages } = await import("../src/messages.ts");
  const en = shellMessages("en-US").common;
  const es = shellMessages("es-ES").common;
  assert.equal(shownTitle({ title: "Hang Probe", responding: false }, en), "Hang Probe (Not Responding)");
  assert.equal(shownTitle({ title: "Hang Probe", responding: false }, es), "Hang Probe (No responde)");
  assert.equal(shownTitle({ title: "Inbox", responding: true }, en), "Inbox");
  assert.equal(shownTitle({ title: "Inbox" }, en), "Inbox");
});
