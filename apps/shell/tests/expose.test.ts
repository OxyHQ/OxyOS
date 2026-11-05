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
