import { test } from "node:test";
import assert from "node:assert/strict";
import { afterEnroll, enrollAnswerFrom, platformFrom, usablePin } from "../src/setup.ts";
import { loginMessages } from "../src/messages.ts";

test("the platform is the accounts' service's, read strictly", () => {
  assert.deepEqual(platformFrom({ api: "https://api.oxy.so", clientId: "oxy_pk_os" }), {
    api: "https://api.oxy.so",
    clientId: "oxy_pk_os",
  });
  assert.equal(platformFrom({ api: "https://api.oxy.so", clientId: "" }), null, "no client id, no setup");
  assert.equal(platformFrom({ api: "ftp://api.oxy.so", clientId: "x" }), null);
  assert.equal(platformFrom(null), null);
});

test("each answer of the accounts' service leads somewhere", () => {
  assert.deepEqual(afterEnroll({ enrolled: { accountId: "a", user: "oxy-a" } }), { next: "enrolled", user: "oxy-a" });
  assert.deepEqual(afterEnroll("secondFactorRequired"), { next: "secondFactor", invalid: false });
  assert.deepEqual(afterEnroll({ refused: { reason: "secondFactorInvalid" } }), { next: "secondFactor", invalid: true });
  assert.deepEqual(afterEnroll({ refused: { reason: "wrongPassword" } }), { next: "password", problem: "incorrect", wait: 0 });
  assert.deepEqual(afterEnroll({ refused: { reason: "locked", retryAfterSeconds: 29.2 } }), {
    next: "password",
    problem: "throttled",
    wait: 30,
  });
  assert.deepEqual(afterEnroll({ refused: { reason: "unusablePassword" } }), { next: "password", problem: "unusable", wait: 0 });
  assert.deepEqual(afterEnroll({ refused: { reason: "offline" } }), { next: "offline" });
  assert.deepEqual(afterEnroll({ refused: { reason: "credentialRefused" } }), { next: "signIn" });
  assert.deepEqual(afterEnroll({ refused: { reason: "accountMismatch" } }), { next: "signIn" });
  assert.deepEqual(afterEnroll({ refused: { reason: "failed" } }), { next: "password", problem: "failed", wait: 0 });
  assert.deepEqual(afterEnroll({ failed: { reason: "no service" } }), { next: "password", problem: "failed", wait: 0 });
  // A PIN the service does not take is chosen again; an account without a password is not set up.
  assert.deepEqual(afterEnroll({ refused: { reason: "unusablePin" } }), { next: "pin" });
  assert.deepEqual(afterEnroll({ refused: { reason: "passwordRequired" } }), { next: "noPassword" });
});

test("a PIN is four to sixteen digits, as the accounts' service takes one", () => {
  for (const pin of ["1234", "482916", "1234567890123456"]) assert.equal(usablePin(pin), true, pin);
  for (const pin of ["", "123", "12345678901234567", "12a4", "12 34", "١٢٣٤"]) assert.equal(usablePin(pin), false, pin);
});

test("an answer that is not one is a failure", () => {
  assert.equal(enrollAnswerFrom("secondFactorRequired"), "secondFactorRequired");
  assert.deepEqual(enrollAnswerFrom({ enrolled: { accountId: "a", user: "oxy-a", uid: 1001 } }), {
    enrolled: { accountId: "a", user: "oxy-a" },
  });
  assert.deepEqual(enrollAnswerFrom({ refused: { reason: "locked", retryAfterSeconds: 60 } }), {
    refused: { reason: "locked", retryAfterSeconds: 60 },
  });
  assert.deepEqual(enrollAnswerFrom({ enrolled: { user: "" } }), { failed: { reason: "invalidAnswer" } });
  assert.deepEqual(enrollAnswerFrom(true), { failed: { reason: "invalidAnswer" } });
});

test("setup's words in both languages", () => {
  assert.equal(loginMessages("en").setup.passwordTitle("Ada"), "Enter Ada's password");
  assert.equal(loginMessages("es").setup.passwordTitle("Ada"), "Introduce la contraseña de Ada");
  assert.equal(loginMessages("es").setup.notConfigured, "Este ordenador no está preparado para añadir cuentas de Oxy.");
});
