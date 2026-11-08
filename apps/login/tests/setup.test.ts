import { test } from "node:test";
import assert from "node:assert/strict";
import {
  afterEnroll,
  asksPassword,
  backFrom,
  enrollAnswerFrom,
  pageOf,
  pagesFor,
  pinOptional,
  platformFrom,
  stageFrom,
  usablePin,
} from "../src/setup.ts";
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


const qr = { id: "a", name: "Ada", hasPassword: true, via: "qr" as const };
const typed = { id: "a", name: "Ada", hasPassword: true, via: "password" as const };
const code = { id: "a", name: "Ada", hasPassword: true, via: "other" as const };
const codeNoPassword = { id: "a", name: "Ada", hasPassword: false, via: "other" as const };

test("each answer of the accounts' service leads somewhere, a failure to its way forward", () => {
  assert.deepEqual(afterEnroll(qr, { enrolled: { accountId: "a", user: "oxy-a" } }), { next: "enrolled", user: "oxy-a" });
  assert.deepEqual(afterEnroll(code, "secondFactorRequired"), { next: "secondFactor", invalid: false });
  assert.deepEqual(afterEnroll(code, { refused: { reason: "secondFactorInvalid" } }), { next: "secondFactor", invalid: true });
  // The password asked once: refused, asked again; throttled, its wait said.
  assert.deepEqual(afterEnroll(code, { refused: { reason: "wrongPassword" } }), { next: "password", problem: "incorrect", wait: 0 });
  assert.deepEqual(afterEnroll(code, { refused: { reason: "locked", retryAfterSeconds: 29.2 } }), {
    next: "password",
    problem: "throttled",
    wait: 30,
  });
  assert.deepEqual(afterEnroll(code, { refused: { reason: "unusablePassword" } }), { next: "password", problem: "unusable", wait: 0 });
  // A password typed in the sign-in that Oxy no longer takes: the sign-in again, never a field.
  assert.deepEqual(afterEnroll(typed, { refused: { reason: "wrongPassword" } }), { next: "problem", problem: "expired" });
  // Kept by the service: tried again with nothing typed again, never a new sign-in.
  assert.deepEqual(afterEnroll(qr, { failed: { reason: "unreachable" } }), { next: "problem", problem: "unreachable" });
  assert.deepEqual(afterEnroll(qr, { failed: { reason: "localFailure" } }), { next: "problem", problem: "machine" });
  // A sign-in that can no longer be used: signed in again.
  for (const reason of ["credentialRefused", "accountNotOnDevice", "accountMismatch", "invalidAccount"]) {
    assert.deepEqual(afterEnroll(qr, { refused: { reason } }), { next: "problem", problem: "expired" }, reason);
  }
  // Anything else: said, and tried again.
  assert.deepEqual(afterEnroll(qr, { refused: { reason: "conflict" } }), { next: "problem", problem: "failed" });
  assert.deepEqual(afterEnroll(qr, { failed: { reason: "unanswered" } }), { next: "problem", problem: "failed" });
  assert.deepEqual(afterEnroll(qr, { refused: { reason: "unusablePin" } }), { next: "pin" });
  assert.deepEqual(afterEnroll(qr, { refused: { reason: "pinRequired" } }), { next: "pin" });
});

test("only as many pages as needed", () => {
  assert.deepEqual(pagesFor({ step: "welcome" }, true), ["welcome", "signIn", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "checking" }, false), ["signIn", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "offline" }, true), ["welcome", "network", "signIn", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "pin", account: qr }, false), ["signIn", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "password", account: code }, false), ["signIn", "password", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "pin", account: codeNoPassword }, false), ["signIn", "pin", "appearance", "settingUp"]);
  assert.equal(pageOf({ step: "secondFactor", account: code, invalid: false }), "settingUp");
  assert.equal(pageOf({ step: "checking" }), "signIn");
});

test("Back goes to the page before, past the sign-in gives it up, and never while setting up", () => {
  assert.equal(backFrom({ step: "welcome" }, true), null);
  assert.deepEqual(backFrom({ step: "signIn", platform: { api: "https://api.oxy.so", clientId: "x" } }, true), { step: "welcome" });
  assert.equal(backFrom({ step: "offline" }, false), "close");
  assert.equal(backFrom({ step: "pin", account: qr }, false), "signIn");
  assert.deepEqual(backFrom({ step: "pin", account: qr, first: "1234" }, false), { step: "pin", account: qr });
  assert.deepEqual(backFrom({ step: "pin", account: code }, false), { step: "password", account: code });
  assert.equal(backFrom({ step: "password", account: code }, false), "signIn");
  assert.deepEqual(backFrom({ step: "appearance", account: qr }, false), { step: "pin", account: qr });
  assert.equal(backFrom({ step: "settingUp", account: qr, stage: "preparing" }, false), null);
  // Kept by the service: tried again as it is, never abandoned for a new setup.
  assert.equal(backFrom({ step: "settingUp", account: qr, stage: null, problem: "unreachable" }, false), null);
  assert.equal(backFrom({ step: "settingUp", account: qr, stage: null, problem: "machine" }, false), null);
  assert.deepEqual(backFrom({ step: "settingUp", account: qr, stage: null, problem: "failed" }, false), {
    step: "appearance",
    account: qr,
  });
  assert.equal(backFrom({ step: "settingUp", account: qr, stage: null, problem: "expired" }, false), null);
  assert.equal(backFrom({ step: "signingIn", account: qr, user: "oxy-a" }, false), null);
});

test("a stage is one the service names, else none", () => {
  assert.equal(stageFrom("preparing"), "preparing");
  assert.equal(stageFrom("starting"), "starting");
  assert.equal(stageFrom("redeeming"), null);
  assert.equal(stageFrom(3), null);
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
  assert.equal(loginMessages("en").setup.passwordTitle("Ada"), "Enter Ada's Oxy Password");
  assert.equal(loginMessages("es").setup.passwordTitle("Ada"), "Introduce la contraseña de Oxy de Ada");
  assert.equal(loginMessages("en").setup.passwordTitle(""), "Enter Your Oxy Password");
  assert.equal(loginMessages("es").setup.step("Crea un PIN", 3, 5), "Crea un PIN, paso 3 de 5");
  assert.equal(loginMessages("en").setup.passwordThrottled(30), "Too many tries. Wait 30 seconds, then try again.");
});

test("a QR sets a PIN alone, a typed password is taken, and only a sign-in that typed none asks it", () => {
  const account = (via: "qr" | "password" | "other", hasPassword: boolean) => ({ id: "a", name: "Ada", hasPassword, via });
  // The QR: the PIN is required and no password is asked, even of an account that has one.
  assert.equal(asksPassword(account("qr", true)), false);
  assert.equal(pinOptional(account("qr", true)), false);
  assert.equal(pinOptional(account("qr", false)), false);
  // A password typed in the sign-in: never asked again, the PIN optional.
  assert.equal(asksPassword(account("password", true)), false);
  assert.equal(pinOptional(account("password", true)), true);
  // The email's code: an account with a password gives it once, the PIN optional; one without
  // sets a PIN.
  assert.equal(asksPassword(account("other", true)), true);
  assert.equal(pinOptional(account("other", true)), true);
  assert.equal(asksPassword(account("other", false)), false);
  assert.equal(pinOptional(account("other", false)), false);
});

test("Transfer Your Data comes after the welcome on the first account, and migrating ends the assistant", () => {
  const source = { step: "source", disks: null, problem: null } as const;
  const computer = {
    repository: "0123456789abcdef",
    name: "ada-laptop",
    backups: [{ snapshot: "bbbb2222", time: 1_794_000_000 }],
    accounts: [{ user: "oxy-a", name: "Ada", administrator: true }],
  };
  // Offered on the first account where the platform migrates, never on another.
  assert.deepEqual(pagesFor({ step: "welcome" }, true, true), ["welcome", "transfer", "signIn", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "checking" }, false, true), ["signIn", "pin", "appearance", "settingUp"]);
  assert.deepEqual(pagesFor({ step: "migrate", migrate: source }, true, true), ["welcome", "transfer", "migrate"]);
  assert.equal(pageOf({ step: "transfer" }), "transfer");
  assert.equal(pageOf({ step: "migrate", migrate: source }), "migrate");
  // Back: from the sign-in to Transfer Your Data, from it to the welcome; through the migration
  // step by step, to Transfer Your Data from its first page; none while it runs.
  assert.deepEqual(backFrom({ step: "checking" }, true, true), { step: "transfer" });
  assert.deepEqual(backFrom({ step: "transfer" }, true, true), { step: "welcome" });
  assert.deepEqual(backFrom({ step: "migrate", migrate: source }, true, true), { step: "transfer" });
  assert.deepEqual(backFrom({ step: "migrate", migrate: { step: "share", problem: null } }, true, true), {
    step: "migrate",
    migrate: source,
  });
  assert.deepEqual(
    backFrom({ step: "migrate", migrate: { step: "confirm", computer, backup: computer.backups[0]!, several: null } }, true, true),
    { step: "migrate", migrate: { step: "backup", computer, several: null } },
  );
  assert.deepEqual(backFrom({ step: "migrate", migrate: { step: "backup", computer, several: null } }, true, true), {
    step: "migrate",
    migrate: source,
  });
  assert.equal(
    backFrom({ step: "migrate", migrate: { step: "migrating", computer, progress: { step: "files", percent: 0.4 } } }, true, true),
    null,
  );
  assert.deepEqual(backFrom({ step: "migrate", migrate: { step: "failed", problem: "clash" } }, true, true), { step: "transfer" });
});
