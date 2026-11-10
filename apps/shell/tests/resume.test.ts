import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages } from "../src/messages.ts";
import { sheetView, startsOnAnswer, type SheetText } from "../src/sheet.ts";

const en = shellMessages("en-US");
const es = shellMessages("es-ES");

// The session ended within a minute of reopening the person's apps (docs/adr/0065, decision 4): it
// asks before reopening them again, as a Mac asks after an app quit while reopening its windows.
test("the question to reopen again, its apps listed, the keyboard on Reopen", () => {
  const sheet: SheetText = { kind: "reopen-again", applications: ["Allo", "Inbox"] };
  const view = sheetView(sheet, en);
  assert.equal(view.title, "The last time you logged in, OxyOS quit unexpectedly while reopening your apps.");
  assert.equal(view.body, "Do you want to try to reopen them again?");
  assert.deepEqual(view.lines, [{ label: "Allo" }, { label: "Inbox" }]);
  assert.equal(view.accept, "Reopen");
  assert.equal(view.refuse, "Don't Reopen");
  assert.equal(view.destructive, false);
  assert.ok(startsOnAnswer(sheet));
  const spanish = sheetView(sheet, es);
  assert.equal(
    spanish.title,
    "La última vez que iniciaste sesión, OxyOS se cerró inesperadamente al volver a abrir tus apps.",
  );
  assert.equal(spanish.body, "¿Quieres intentar volver a abrirlas?");
  assert.equal(spanish.accept, "Volver a abrir");
  assert.equal(spanish.refuse, "No volver a abrir");
});
