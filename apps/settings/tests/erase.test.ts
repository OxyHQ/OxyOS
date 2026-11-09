import { test } from "node:test";
import assert from "node:assert/strict";
import {
  administratorOf,
  eraseAvailability,
  eraseOutcomeFrom,
  erasePreviewFrom,
  refusalMessage,
  signOutLine,
  type ErasePreview,
} from "../src/erase.ts";
import { messagesFor } from "../src/messages.ts";

const en = messagesFor("en-US").transferReset;
const es = messagesFor("es-ES").transferReset;

const told = {
  available: true,
  administrator: true,
  online: true,
  accounts: [
    { name: "Zoë Ada", handle: "zoe", signsOutOnline: true },
    { name: "Ada Lovelace", handle: "ada", signsOutOnline: true },
    // An account Oxy names nothing is unnamed: its user (`oxy-…`) is never a name.
    { name: "", handle: null, signsOutOnline: false },
  ],
};

test("what erasing would do is the platform's, read strictly", () => {
  assert.deepEqual(erasePreviewFrom(told), told);
  const broken: unknown[] = [
    null,
    "erasePreview",
    { ...told, available: "yes" },
    { ...told, accounts: [] },
    { ...told, accounts: "Ada" },
    { ...told, extra: true },
    { ...told, accounts: [{ name: 3, handle: null, signsOutOnline: false }] },
    { ...told, accounts: [{ name: "Ada", handle: "", signsOutOnline: false }] },
    { ...told, accounts: [{ name: "Ada", handle: 3, signsOutOnline: false }] },
    { ...told, accounts: [{ name: "Ada", signsOutOnline: false }] },
    { ...told, accounts: [{ name: "Ada", handle: null, signsOutOnline: false, uid: 1000 }] },
    // Signed out online while the computer is offline: no such thing.
    { ...told, online: false },
  ];
  for (const shape of broken) assert.equal(erasePreviewFrom(shape), null, JSON.stringify(shape));
});

test("the accounts keep the platform's order: the asking administrator first", () => {
  const preview = erasePreviewFrom(told)!;
  assert.deepEqual(
    preview.accounts.map((account) => account.name),
    ["Zoë Ada", "Ada Lovelace", ""],
  );
  assert.equal(administratorOf(preview).name, "Zoë Ada");
});

test("what the page offers follows what the platform told", () => {
  assert.equal(eraseAvailability(undefined), "none");
  assert.equal(eraseAvailability(null), "none", "a preview that could not be read offers nothing");
  assert.equal(eraseAvailability(erasePreviewFrom({ ...told, available: false })), "noRecovery");
  assert.equal(eraseAvailability(erasePreviewFrom({ ...told, administrator: false })), "notAdministrator");
  assert.equal(
    eraseAvailability(erasePreviewFrom({ ...told, available: false, administrator: false })),
    "noRecovery",
    "no recovery is said first: no administrator could erase it either",
  );
  assert.equal(eraseAvailability(erasePreviewFrom(told)), "ready");
});

test("each account's sign-in: signed out online, already signed out, or not while offline", () => {
  const online = erasePreviewFrom(told)!;
  assert.equal(signOutLine(online.accounts[0]!, online, en), en.signsOut);
  assert.equal(signOutLine(online.accounts[2]!, online, en), en.signedOutAlready);
  const offline: ErasePreview = {
    ...online,
    online: false,
    accounts: online.accounts.map((account) => ({ ...account, signsOutOnline: false })),
  };
  assert.equal(signOutLine(offline.accounts[0]!, offline, en), en.cannotSignOut);
});

test("an erase's answer is read strictly: anything else is a failure, never an erase", () => {
  assert.deepEqual(eraseOutcomeFrom({ erasing: { signedOut: ["Ada"], notSignedOut: [] } }), {
    erasing: { signedOut: ["Ada"], notSignedOut: [] },
  });
  assert.deepEqual(eraseOutcomeFrom({ erasing: { signedOut: "Ada" } }), { failed: "not an erase" });
  assert.deepEqual(eraseOutcomeFrom({ refused: { reason: "wrongPassword" } }), { refused: "wrongPassword" });
  assert.deepEqual(eraseOutcomeFrom({ refused: { reason: "throttled", retryAfter: 59.2 } }), {
    refused: "throttled",
    retryAfter: 60,
  });
  assert.deepEqual(eraseOutcomeFrom({ refused: { reason: "throttled", retryAfter: -1 } }), { refused: "throttled" });
  assert.deepEqual(eraseOutcomeFrom({ refused: { reason: "somethingNew" } }), { refused: "failed" });
  assert.deepEqual(eraseOutcomeFrom({ failed: { reason: "unreachable" } }), { failed: "unreachable" });
  assert.deepEqual(eraseOutcomeFrom(true), { failed: "no answer" });
  assert.deepEqual(eraseOutcomeFrom(null), { failed: "no answer" });
});

test("a refusal is said where it belongs: the field's, or the step's", () => {
  assert.deepEqual(refusalMessage({ refused: "wrongPassword" }, en), { field: "Incorrect password.", step: null });
  assert.deepEqual(refusalMessage({ refused: "throttled", retryAfter: 30 }, en), {
    field: "Try again in 30 seconds.",
    step: null,
  });
  assert.deepEqual(refusalMessage({ refused: "throttled", retryAfter: 1 }, en), {
    field: "Try again in 1 second.",
    step: null,
  });
  assert.deepEqual(refusalMessage({ refused: "throttled" }, en), { field: en.tryAgainLater, step: null });
  const nothingChanged = "OxyOS couldn't erase this computer. Nothing was changed.";
  for (const refused of ["failed", "notPermitted"] as const) {
    assert.deepEqual(refusalMessage({ refused }, en), { field: null, step: nothingChanged });
  }
  assert.deepEqual(refusalMessage({ failed: "unreachable" }, en), { field: null, step: nothingChanged });
  assert.deepEqual(refusalMessage({ refused: "notAdministrator" }, en), { field: null, step: en.notAdministrator });
  assert.deepEqual(refusalMessage({ refused: "unavailable" }, en), { field: null, step: en.noRecovery });
});

test("Transfer or Reset's words in both languages", () => {
  assert.equal(messagesFor("en-US").pages.transferReset, "Transfer or Reset");
  assert.equal(messagesFor("es-ES").pages.transferReset, "Transferir o restablecer");
  assert.equal(en.erase, "Erase All Content and Settings…");
  assert.equal(es.erase, "Borrar contenidos y ajustes…");
  assert.equal(en.passwordDescription("Ada"), "Enter the password for Ada to erase this computer.");
  assert.equal(es.passwordDescription("Ada"), "Introduce la contraseña de Ada para borrar este ordenador.");
  assert.equal(es.tryAgainIn(1), "Vuelve a intentarlo dentro de 1 segundo.");
  assert.equal(es.tryAgainIn(45), "Vuelve a intentarlo dentro de 45 segundos.");
  assert.equal(
    en.noRecovery,
    "This computer can't be erased from Settings because it has no recovery.",
  );
});
