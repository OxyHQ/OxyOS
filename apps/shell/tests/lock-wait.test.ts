import { test } from "node:test";
import assert from "node:assert/strict";
import { waitReminder, waitSaid } from "../src/lock-wait.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");

test("a wait is said in words: seconds under a minute, minutes when whole", () => {
  assert.equal(waitSaid(30, ES), "30 segundos");
  assert.equal(waitSaid(1, ES), "1 segundo");
  assert.equal(waitSaid(60, ES), "1 minuto");
  assert.equal(waitSaid(900, ES), "15 minutos");
  assert.equal(waitSaid(90, ES), "90 segundos");
});

test("a wait counting down is said again only each minute, at half a minute and at ten seconds", () => {
  const said = (from: number) =>
    Array.from({ length: from }, (_, index) => from - 1 - index).filter(waitReminder);
  // The session's first wait, 30 s: its start is said with the answer, then 10 s before the end.
  assert.deepEqual(said(30), [10]);
  assert.deepEqual(said(60), [30, 10]);
  assert.deepEqual(said(120), [60, 30, 10]);
  // The longest, 15 minutes: once a minute, never each second.
  assert.equal(said(900).length, 16);
  assert.ok(!waitReminder(0));
});
