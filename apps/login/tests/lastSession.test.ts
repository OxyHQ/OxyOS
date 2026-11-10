import assert from "node:assert/strict";
import { test } from "node:test";

import { endedBecauseOfAProblem, LAST_SESSION_REQUEST } from "../src/lastSession.ts";
import { loginMessages } from "../src/messages.ts";

test("the login screen asks its session how the last session ended", () => {
  assert.deepEqual(LAST_SESSION_REQUEST, { lastSession: {} });
});

test("only a problem the session says is said", () => {
  assert.ok(endedBecauseOfAProblem({ lastSession: { problem: true } }));
  assert.ok(!endedBecauseOfAProblem({ lastSession: { problem: false } }));
  assert.ok(!endedBecauseOfAProblem({ lastSession: {} }));
  assert.ok(!endedBecauseOfAProblem({ refused: { reason: "notPermitted" } }));
  assert.ok(!endedBecauseOfAProblem({ failed: { reason: "unavailable" } }));
  assert.ok(!endedBecauseOfAProblem(null));
});

test("its line in English and Spanish", () => {
  assert.equal(loginMessages("en").sessionEnded, "Your session ended because of a problem.");
  assert.equal(loginMessages("es").sessionEnded, "Tu sesión se cerró por un problema.");
});
