import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ago,
  countLabel,
  groupNotifications,
  lockSummary,
  nextBanner,
  spoken,
} from "../src/notifications.ts";
import type { ShellNotification } from "../src/model.ts";

const applications = [
  { id: "so.oxy.allo", name: "Allo", icon: { svg: "" } },
  { id: "so.oxy.inbox", name: "Inbox", icon: { svg: "" } },
];
const notification = (
  id: string,
  appId: string,
  postedAt: number,
  extra: Partial<ShellNotification> = {},
): ShellNotification => ({ id, appId, title: `t${id}`, body: `b${id}`, actions: [], postedAt, ...extra });

test("the Notification Center groups by application, the newest group and notification first", () => {
  const groups = groupNotifications(
    [
      notification("1", "so.oxy.allo", 10),
      notification("2", "so.oxy.inbox", 20),
      notification("3", "so.oxy.allo", 30),
      notification("4", "so.oxy.unknown", 5),
    ],
    applications,
  );
  assert.deepEqual(
    groups.map((group) => [group.name, group.notifications.map((n) => n.id)]),
    [
      ["Allo", ["3", "1"]],
      ["Inbox", ["2"]],
      // An application the registry does not list is named by its id, with no icon.
      ["so.oxy.unknown", ["4"]],
    ],
  );
  assert.equal(groups[2].app, undefined);
});

test("a banner is the newest that arrived as one, shown once, never a withheld one", () => {
  const list = [
    notification("1", "so.oxy.allo", 10, { banner: true }),
    notification("2", "so.oxy.allo", 20, { banner: true }),
    notification("3", "so.oxy.allo", 30, { banner: false }),
    notification("4", "so.oxy.allo", 40, { banner: true, withheld: true }),
  ];
  assert.equal(nextBanner(list, new Set())?.id, "2");
  assert.equal(nextBanner(list, new Set(["2"]))?.id, "1");
  assert.equal(nextBanner(list, new Set(["1", "2"])), undefined);
});

test("the lock screen says how many each application has, and what they say only with previews", () => {
  const list = [
    notification("1", "so.oxy.allo", 10),
    notification("2", "so.oxy.allo", 20, { title: "Ana", body: "la clave" }),
    notification("3", "so.oxy.inbox", 5, { withheld: true, title: "", body: "" }),
  ];
  const without = lockSummary(list, applications, false);
  assert.deepEqual(
    without.map((entry) => [entry.name, entry.count, entry.preview]),
    [
      ["Allo", "2 notificaciones", undefined],
      ["Inbox", "1 notificación", undefined],
    ],
  );
  const withPreviews = lockSummary(list, applications, true);
  assert.deepEqual(withPreviews[0].preview, { title: "Ana", body: "la clave" });
  // One the session withheld has nothing to preview.
  assert.equal(withPreviews[1].preview, undefined);
  assert.equal(countLabel(1), "1 notificación");
});

test("when, as the Notification Center says it, and what a screen reader says of a banner", () => {
  const now = 1_000_000_000;
  assert.equal(ago(now - 30_000, now), "ahora");
  assert.equal(ago(now - 5 * 60_000, now), "hace 5 min");
  assert.equal(ago(now - 3 * 3_600_000, now), "hace 3 h");
  assert.equal(ago(now - 30 * 3_600_000, now), "ayer");
  assert.equal(ago(now - 72 * 3_600_000, now), "hace 3 días");
  assert.equal(ago(now + 5000, now), "ahora");
  assert.equal(
    spoken(notification("1", "so.oxy.allo", 0, { title: "Ana", body: "¿Comemos?" }), "Allo"),
    "Allo. Ana. ¿Comemos?",
  );
  assert.equal(
    spoken(notification("1", "so.oxy.allo", 0, { title: "Ana", subtitle: "Grupo", body: "" }), "Allo"),
    "Allo. Ana. Grupo",
  );
});
