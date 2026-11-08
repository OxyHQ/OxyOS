import { test } from "node:test";
import assert from "node:assert/strict";
import {
  localOutcomeFrom,
  localRefusal,
  localSignInFrom,
  proofFor,
  usablePin,
} from "../src/local-sign-in.ts";
import { messagesFor } from "../src/messages.ts";

test("the local sign-in is the service's, read strictly", () => {
  assert.deepEqual(localSignInFrom({ pin: "set", password: true }), { pin: "set", password: true });
  assert.deepEqual(localSignInFrom({ pin: "locked", password: false }), { pin: "locked", password: false });
  assert.equal(localSignInFrom({ pin: "on", password: true }), null);
  assert.equal(localSignInFrom({ pin: "set" }), null);
  assert.equal(localSignInFrom(null), null);
});

test("a change's answer is the local sign-in after it, a refusal, or a failure", () => {
  assert.deepEqual(localOutcomeFrom({ localSignIn: { pin: "none", password: true } }), {
    localSignIn: { pin: "none", password: true },
  });
  assert.deepEqual(localOutcomeFrom({ refused: { reason: "throttled", retryAfter: 30 } }), {
    refused: "throttled",
    retryAfter: 30,
  });
  assert.deepEqual(localOutcomeFrom({ refused: { reason: "wrongPin" } }), { refused: "wrongPin" });
  assert.deepEqual(localOutcomeFrom({ failed: { reason: "unavailable" } }), { failed: "unavailable" });
  assert.deepEqual(localOutcomeFrom({ localSignIn: { pin: "x" } }), { failed: "invalidAnswer" });
  assert.deepEqual(localOutcomeFrom("nonsense"), { failed: "invalidAnswer" });
});

test("each change asks for the proof the service takes", () => {
  const both = { pin: "set", password: true } as const;
  const pinOnly = { pin: "set", password: false } as const;
  assert.equal(proofFor("setPin", both), "password");
  assert.equal(proofFor("setPin", pinOnly), "pin");
  assert.equal(proofFor("removePin", both), "password");
  assert.equal(proofFor("passwordOff", both), "pin");
  // Turning the password on is proved by the password itself, checked with Oxy.
  assert.equal(proofFor("passwordOn", pinOnly), "password");
});

test("a PIN is four to sixteen digits", () => {
  assert.equal(usablePin("1234"), true);
  assert.equal(usablePin("123"), false);
  assert.equal(usablePin("12345678901234567"), false);
  assert.equal(usablePin("12ab"), false);
});

test("every refusal is said, in both languages", () => {
  for (const language of ["en", "es"] as const) {
    const messages = messagesFor(language);
    const said = new Set<string>();
    for (const reason of [
      "wrongPassword",
      "wrongPin",
      "pinLocked",
      "unusablePin",
      "offline",
      "secondFactorRequired",
      "noPin",
      "passwordSignInOff",
      "anythingElse",
    ]) {
      const text = localRefusal({ refused: reason }, messages);
      assert.ok(text.length > 0);
      said.add(text);
    }
    assert.equal(said.size, 9);
    assert.match(localRefusal({ refused: "throttled", retryAfter: 29.5 }, messages), /30/);
    assert.equal(localRefusal({ failed: "x" }, messages), messages.localSignIn.failed);
  }
});
