import { test } from "node:test";
import assert from "node:assert/strict";
import { localSignInAfter, localSignInFrom } from "../src/local-sign-in.ts";

test("the service's answer is read strictly", () => {
  assert.deepEqual(localSignInFrom({ pin: "set", password: false }), { pin: "set", password: false });
  assert.deepEqual(localSignInFrom({ pin: "locked", password: true }), { pin: "locked", password: true });
  // No PIN, and an account set up before PINs (nothing said of either): the password's alone.
  assert.deepEqual(localSignInFrom({ pin: "none", password: true }), { pin: undefined, password: true });
  assert.deepEqual(localSignInFrom({}), { pin: undefined, password: true });
});

test("an answer replaces what was known", () => {
  const known = { pin: "set" as const, password: true };
  assert.deepEqual(localSignInAfter(known, { pin: "none", password: true }), { pin: undefined, password: true });
  assert.deepEqual(localSignInAfter(known, { pin: "locked", password: false }), { pin: "locked", password: false });
  assert.deepEqual(localSignInAfter(undefined, { pin: "set", password: false }), { pin: "set", password: false });
});

test("a read that failed keeps what was said last: a known PIN is never traded for the password", () => {
  // The platform's failure (the service refused the session, restarting, unreachable), and the
  // call itself failing.
  for (const told of [{ failed: { reason: "notPermitted" } }, { failed: { reason: "unavailable" } }, undefined]) {
    assert.deepEqual(localSignInAfter({ pin: "set", password: false }, told), { pin: "set", password: false });
    assert.deepEqual(localSignInAfter({ pin: "set", password: true }, told), { pin: "set", password: true });
    assert.deepEqual(localSignInAfter({ pin: "locked", password: true }, told), { pin: "locked", password: true });
    assert.deepEqual(localSignInAfter({ pin: undefined, password: true }, told), { pin: undefined, password: true });
  }
});

test("a read that failed with nothing said yet offers both: the password's field, the PIN's beside it", () => {
  for (const told of [{ failed: { reason: "notPermitted" } }, undefined, null, "notPermitted"]) {
    assert.deepEqual(localSignInAfter(undefined, told), { pin: "unknown", password: true });
  }
});
