/**
 * Login Items & Extensions (docs/adr/0060-login-items-and-background-activity.mdx): Open at Login is
 * the session's one list, the dock's too, shown by the registry's names, changed whole; App
 * Background Activity is each installed application whose background activity was decided, as the
 * install service keeps it and the session says what runs in the background.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  addableApplications,
  backgroundRows,
  loginItemRows,
  selectedAfterRemoving,
  withLoginItem,
  withoutLoginItem,
} from "../src/login-items.ts";
import { CATALOGUES } from "../src/messages.ts";
import { PREFERENCE_KEYS, preferenceText } from "../src/model.ts";
import type { PrivacyApplication } from "../src/updates.ts";

const APPS = [
  { id: "so.oxy.inbox", name: "Inbox" },
  { id: "so.oxy.allo", name: "Allo" },
  { id: "so.oxy.dropsea", name: "Dropsea" },
];

test("Open at Login is the session's list, named as the registry names each, in its order", () => {
  assert.equal(PREFERENCE_KEYS.openAtLogin, "open_at_login");
  assert.equal(preferenceText<"openAtLogin">(["so.oxy.inbox", "so.oxy.allo"]), "so.oxy.inbox,so.oxy.allo");
  assert.deepEqual(loginItemRows(["so.oxy.dropsea", "so.oxy.inbox"], APPS), [
    { id: "so.oxy.dropsea", name: "Dropsea" },
    { id: "so.oxy.inbox", name: "Inbox" },
  ]);
  // An id the registry no longer lists is shown as itself: the session passes over it.
  assert.deepEqual(loginItemRows(["so.oxy.gone"], APPS), [{ id: "so.oxy.gone", name: "so.oxy.gone" }]);
  assert.deepEqual(loginItemRows([], APPS), []);
});

test("+ offers the registry's applications not opened at login, by name", () => {
  assert.deepEqual(
    addableApplications(["so.oxy.inbox"], APPS, "en").map((app) => app.id),
    ["so.oxy.allo", "so.oxy.dropsea"],
  );
  assert.deepEqual(addableApplications(APPS.map((app) => app.id), APPS, "en"), []);
});

test("a change is the whole list: one added at its end once, one taken out, the selection moving on", () => {
  assert.deepEqual(withLoginItem(["so.oxy.inbox"], "so.oxy.allo"), ["so.oxy.inbox", "so.oxy.allo"]);
  assert.deepEqual(withLoginItem(["so.oxy.inbox"], "so.oxy.inbox"), ["so.oxy.inbox"]);
  assert.deepEqual(withoutLoginItem(["so.oxy.inbox", "so.oxy.allo"], "so.oxy.inbox"), ["so.oxy.allo"]);
  assert.equal(selectedAfterRemoving(["a", "b", "c"], "b"), "c");
  assert.equal(selectedAfterRemoving(["a", "b", "c"], "c"), "b");
  assert.equal(selectedAfterRemoving(["a"], "a"), null);
});

function app(id: string, name: string, background?: "granted" | "denied" | "undecided", running = false): PrivacyApplication {
  return {
    id,
    name,
    capabilities: background ? { notifications: "granted", background } : { notifications: "granted" },
    runningInBackground: running,
  };
}

test("App Background Activity lists each application whose background activity was decided, by name", () => {
  const rows = backgroundRows(
    [
      app("so.oxy.chat", "Chat", "granted", true),
      app("so.oxy.notes", "Notes", "denied"),
      app("so.oxy.mail", "Mail", "undecided"),
      app("so.oxy.photos", "Photos"),
      app("so.oxy.backup", "Backup", "granted"),
    ],
    "en",
  );
  assert.deepEqual(rows, [
    { id: "so.oxy.backup", name: "Backup", allowed: true, running: false },
    { id: "so.oxy.chat", name: "Chat", allowed: true, running: true },
    { id: "so.oxy.notes", name: "Notes", allowed: false, running: false },
  ]);
  // Turned off, nothing of it runs there, whatever was last told.
  assert.deepEqual(backgroundRows([app("so.oxy.chat", "Chat", "denied", true)], "en"), [
    { id: "so.oxy.chat", name: "Chat", allowed: false, running: false },
  ]);
  assert.deepEqual(backgroundRows(null, "en"), []);
});

test("the page's words are in both languages, each switch named with its application", () => {
  for (const language of ["en", "es"] as const) {
    const words = CATALOGUES[language].loginItems;
    assert.ok(words.openAtLogin && words.background && words.running && words.backgroundFooter);
    assert.ok(words.allowFor("Chat").includes("Chat"));
    assert.ok(words.removeItem("Chat").includes("Chat"));
  }
  assert.equal(CATALOGUES.en.pages.loginItems, "Login Items & Extensions");
  assert.equal(CATALOGUES.es.pages.loginItems, "Ítems de inicio y extensiones");
});
