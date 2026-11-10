import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages } from "../src/messages.ts";
import { systemMenuRows, type SystemMenuRow } from "../src/system-menu.ts";
import { chordLabel } from "../src/chord.ts";
import { sheetView, startsOnAnswer, type SheetText } from "../src/sheet.ts";

const en = shellMessages("en-US");
const es = shellMessages("es-ES");

/** A menu's rows as the person reads them: each label with its chord, separators as "—". */
function read(rows: SystemMenuRow[]): string[] {
  return rows.map((row) => (row.kind === "separator" ? "—" : row.chord ? `${row.label} ${row.chord}` : row.label));
}

test("the Oxy menu is a Mac's Apple menu: its rows, separators and chords in order", () => {
  const rows = systemMenuRows(en, { canLock: true, power: true, name: "Nate Isern" });
  assert.deepEqual(read(rows), [
    "System Settings",
    "Applications",
    "Show Desktops",
    "—",
    // Force Quit… in its own part, before the session's ends, as a Mac's (docs/adr/0067).
    "Force Quit… Alt+Super+Esc",
    "—",
    "Sleep",
    "Restart…",
    "Shut Down…",
    "—",
    "Lock Screen Ctrl+Super+Q",
    "Log Out Nate Isern… Shift+Super+Q",
  ]);
  // Each end asks its question.
  assert.ok(rows.every((row) => row.kind === "separator" || row.confirm !== false));
});

test("in Spanish, as a Mac's: Reposo, Reiniciar…, Apagar equipo…, Bloquear pantalla, Cerrar sesión de…", () => {
  const rows = systemMenuRows(es, { canLock: true, power: true, name: "Nate Isern" });
  assert.deepEqual(read(rows).slice(4, 5), ["Forzar salida… Alt+Super+Esc"]);
  assert.deepEqual(read(rows).slice(6), [
    "Reposo",
    "Reiniciar…",
    "Apagar equipo…",
    "—",
    "Bloquear pantalla Ctrl+Super+Q",
    "Cerrar sesión de Nate Isern… Mayús+Super+Q",
  ]);
});

test("Option held shows the alternates: no ellipsis, no question, and ⌥⇧⌘Q", () => {
  const rows = systemMenuRows(en, { canLock: true, power: true, name: "Nate", option: true });
  assert.deepEqual(read(rows).slice(6), [
    "Sleep",
    "Restart",
    "Shut Down",
    "—",
    "Lock Screen Ctrl+Super+Q",
    "Log Out Nate Alt+Shift+Super+Q",
  ]);
  const ends = rows.filter((row) => row.kind === "item" && row.id !== "power.sleep" && row.id.startsWith("power"));
  assert.ok(ends.length === 2 && ends.every((row) => row.kind === "item" && row.confirm === false));
  const logOut = rows.find((row) => row.kind === "item" && row.id === "session.logOut");
  assert.equal(logOut?.kind === "item" && logOut.confirm, false);
  assert.deepEqual(read(systemMenuRows(es, { canLock: true, power: true, name: "Nate", option: true })).slice(6), [
    "Reposo",
    "Reiniciar",
    "Apagar equipo",
    "—",
    "Bloquear pantalla Ctrl+Super+Q",
    "Cerrar sesión de Nate Alt+Mayús+Super+Q",
  ]);
});

test("Log Out always names the person, as a Mac's: an account with no name is the Oxy Account, never the Unix user", () => {
  for (const name of [undefined, "", "  "]) {
    assert.equal(
      read(systemMenuRows(en, { canLock: true, power: true, name })).at(-1),
      "Log Out Oxy Account… Shift+Super+Q",
    );
    assert.equal(
      read(systemMenuRows(en, { canLock: true, power: true, name, option: true })).at(-1),
      "Log Out Oxy Account Alt+Shift+Super+Q",
    );
    assert.equal(
      read(systemMenuRows(es, { canLock: true, power: true, name })).at(-1),
      "Cerrar sesión de Cuenta de Oxy… Mayús+Super+Q",
    );
  }
});

test("without the session's power or lock the menu offers neither, and no stray separator", () => {
  assert.deepEqual(read(systemMenuRows(en, { canLock: false, power: false })), [
    "System Settings",
    "Applications",
    "Show Desktops",
  ]);
  assert.deepEqual(read(systemMenuRows(en, { canLock: true, power: false })), [
    "System Settings",
    "Applications",
    "Show Desktops",
    "—",
    "Lock Screen Ctrl+Super+Q",
  ]);
  assert.equal(chordLabel(["Ctrl", "Shift", "Q"], es), "Ctrl+Mayús+Q");
});

test("the question to shut down, restart and log out, its countdown and its default button", () => {
  const shutDown = sheetView({ kind: "end-session", action: "shutDown", seconds: 60, reopenWindows: true }, en);
  assert.equal(shutDown.title, "Are you sure you want to shut down your computer now?");
  assert.equal(shutDown.body, "If you do nothing, the computer will shut down automatically in 60 seconds.");
  assert.equal(shutDown.countdown?.text(1), "If you do nothing, the computer will shut down automatically in 1 second.");
  assert.equal(shutDown.accept, "Shut Down");
  assert.equal(shutDown.refuse, "Cancel");
  assert.deepEqual(
    [shutDown.defaultAnswer, shutDown.focusAnswer, shutDown.destructive, shutDown.checkbox?.checked],
    [true, true, false, true],
  );
  assert.equal(shutDown.checkbox?.label, "Reopen windows when signing back in");
  const restart = sheetView({ kind: "end-session", action: "restart", seconds: 59, reopenWindows: false }, en);
  assert.equal(restart.title, "Are you sure you want to restart your computer now?");
  assert.equal(restart.body, "If you do nothing, the computer will restart automatically in 59 seconds.");
  assert.equal(restart.accept, "Restart");
  assert.equal(restart.checkbox?.checked, false);
  const logOut = sheetView({ kind: "end-session", action: "logOut", seconds: 60, reopenWindows: true }, en);
  assert.equal(logOut.title, "Are you sure you want to quit all apps and log out now?");
  assert.equal(logOut.body, "If you do nothing, you will be logged out automatically in 60 seconds.");
  assert.equal(logOut.accept, "Log Out");
});

test("the question in Spanish", () => {
  const shutDown = sheetView({ kind: "end-session", action: "shutDown", seconds: 59, reopenWindows: true }, es);
  assert.equal(shutDown.title, "¿Seguro que quieres apagar el ordenador ahora?");
  assert.equal(shutDown.body, "Si no haces nada, el ordenador se apagará automáticamente en 59 segundos.");
  assert.equal(shutDown.countdown?.text(1), "Si no haces nada, el ordenador se apagará automáticamente en 1 segundo.");
  assert.deepEqual([shutDown.accept, shutDown.refuse], ["Apagar", "Cancelar"]);
  assert.equal(shutDown.checkbox?.label, "Volver a abrir las ventanas al volver a iniciar sesión");
  const restart = sheetView({ kind: "end-session", action: "restart", seconds: 60, reopenWindows: true }, es);
  assert.equal(restart.title, "¿Seguro que quieres reiniciar el ordenador ahora?");
  assert.equal(restart.accept, "Reiniciar");
  const logOut = sheetView({ kind: "end-session", action: "logOut", seconds: 60, reopenWindows: true }, es);
  assert.equal(logOut.title, "¿Seguro que quieres salir de todas las apps y cerrar la sesión ahora?");
  assert.equal(logOut.body, "Si no haces nada, la sesión se cerrará automáticamente en 60 segundos.");
  assert.equal(logOut.accept, "Cerrar sesión");
});

test("the power key's choice: Restart and Sleep at its foot's start, Cancel, Shut Down the default", () => {
  const view = sheetView({ kind: "power-key", reopenWindows: true }, en);
  assert.equal(view.title, "Are you sure you want to shut down your computer now?");
  assert.deepEqual(view.actions, [
    { id: "restart", label: "Restart" },
    { id: "sleep", label: "Sleep" },
  ]);
  assert.deepEqual([view.refuse, view.accept, view.defaultAnswer, view.countdown], ["Cancel", "Shut Down", true, undefined]);
  assert.deepEqual(
    sheetView({ kind: "power-key", reopenWindows: false }, es).actions?.map((action) => action.label),
    ["Reiniciar", "Reposo"],
  );
});

test("what kept an end from happening: the application named, as a Mac says it", () => {
  const cancelled = sheetView({ kind: "end-session-cancelled", action: "restart", application: "Pages" }, en);
  assert.deepEqual([cancelled.title, cancelled.accept, cancelled.refuse], ["“Pages” canceled restart.", "OK", ""]);
  assert.equal(sheetView({ kind: "end-session-cancelled", action: "shutDown", application: "Pages" }, en).title, "“Pages” canceled shut down.");
  assert.equal(sheetView({ kind: "end-session-cancelled", action: "logOut", application: "Pages" }, en).title, "“Pages” canceled log out.");
  assert.equal(sheetView({ kind: "end-session-cancelled", action: "restart", application: "Pages" }, es).title, "“Pages” ha cancelado el reinicio.");
  assert.equal(sheetView({ kind: "end-session-cancelled", action: "shutDown", application: "Pages" }, es).title, "“Pages” ha cancelado el apagado.");
  assert.equal(sheetView({ kind: "end-session-cancelled", action: "logOut", application: "Pages" }, es).title, "“Pages” ha cancelado el cierre de sesión.");
  const timedOut = sheetView({ kind: "end-session-timed-out", action: "restart", application: "Pages" }, en);
  assert.equal(timedOut.title, "Restart has timed out because the app “Pages” failed to quit.");
  assert.equal(timedOut.body, "To try again, quit Pages and restart again.");
  const failed = sheetView({ kind: "end-session-failed", action: "shutDown", reason: "notPermitted" }, en);
  assert.equal(failed.title, "Your computer can’t shut down now.");
  assert.equal(
    failed.body,
    "Someone else is signed in to this computer, or an app is keeping it on. An administrator must shut it down.",
  );
  assert.equal(sheetView({ kind: "end-session-failed", action: "sleep", reason: "unavailable" }, en).title, "Your computer can’t sleep now.");
  assert.equal(
    sheetView({ kind: "end-session-failed", action: "restart", reason: "failed" }, es).body,
    "El sistema no ha respondido. Vuelve a intentarlo en un momento.",
  );
});

test("the keyboard starts on the default button of every power sheet, and on Cancel elsewhere", () => {
  assert.ok(startsOnAnswer({ kind: "end-session", action: "restart", seconds: 60, reopenWindows: true }));
  assert.ok(startsOnAnswer({ kind: "power-key", reopenWindows: true }));
  assert.ok(startsOnAnswer({ kind: "end-session-cancelled", action: "restart", application: "Pages" }));
  assert.ok(!startsOnAnswer({ kind: "empty-trash", items: 2 }));
  assert.ok(startsOnAnswer({ kind: "crash", id: "a", name: "A", times: 1, report: "", show: "notice" }));
});

// Force Quit Applications (docs/adr/0067-hangs-and-force-quit.mdx), as a Mac's ⌥⌘⎋ window: the
// session's launched apps, the hung one marked in red and said, the frontmost chosen, Force Quit the
// default button (Return), Cancel (Escape).
test("Force Quit Applications: its apps, the hung one marked, the frontmost chosen", () => {
  const sheet: SheetText = {
    kind: "force-quit",
    applications: [
      { id: "so.oxy.inbox", name: "Inbox", responding: true },
      { id: "so.oxy.hangprobe", name: "Hang Probe", responding: false },
    ],
    chosen: 1,
  };
  const view = sheetView(sheet, en);
  assert.equal(view.title, "Force Quit Applications");
  assert.equal(view.body, "If an app doesn’t respond for a while, select its name and click Force Quit.");
  assert.deepEqual(view.choices?.rows, [
    { title: "Inbox", detail: "", alert: false },
    { title: "Hang Probe", detail: "Not Responding", alert: true },
  ]);
  assert.equal(view.accept, "Force Quit");
  assert.equal(view.refuse, "Cancel");
  assert.ok(view.defaultAnswer && startsOnAnswer(sheet));
  const spanish = sheetView(sheet, es);
  assert.equal(spanish.title, "Forzar salida de las aplicaciones");
  assert.equal(spanish.accept, "Forzar salida");
  assert.equal(spanish.choices?.rows[1].detail, "No responde");
  // None open: said, and nothing to force quit.
  const none = sheetView({ kind: "force-quit", applications: [] }, en);
  assert.equal(none.body, "No apps are open.");
  assert.equal(none.accept, "");
  assert.equal(none.choices, undefined);
});
