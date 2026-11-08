import { test } from "node:test";
import assert from "node:assert/strict";
import {
  closesAccess,
  codeFrom,
  enrollmentFrom,
  formatCode,
  initialOf,
  itemFrom,
  itemsIn,
  keychainClient,
  memberFrom,
  otpauthFrom,
  outcomeMessage,
  recoveryKeyGroups,
  search,
  stageOf,
  staysOpen,
  statusFrom,
  summaryFrom,
  urlsFromText,
  warningsOf,
  type ItemSummary,
  type KeychainPlatform,
  type KeychainRequest,
} from "../src/passwords.ts";
import { CATALOGUES, messagesFor } from "../src/messages.ts";

// Fixture values only: no secret of anyone's.
const login = {
  itemId: "i1",
  type: "login",
  title: "Example",
  username: "bea@example.com",
  domains: ["example.com"],
  hasCode: true,
  modifiedAt: "2026-11-08T10:00:00.000Z",
};

test("the status is the service's, read strictly", () => {
  assert.deepEqual(statusFrom({ state: "member", vault: true, verified: false, items: 3 }), {
    state: "member",
    vault: true,
    verified: false,
    items: 3,
  });
  assert.deepEqual(statusFrom({ state: "joining", vault: true, verified: true, code: "123456" })?.code, "123456");
  assert.equal(statusFrom({ state: "open", vault: true, verified: true }), null);
  assert.equal(statusFrom({ state: "member", vault: "yes", verified: true }), null);
  assert.equal(statusFrom({ state: "member", vault: null }), null);
  assert.equal(statusFrom({ state: "joining", vault: true, verified: true, code: "12345" }), null);
});

test("an item is listed without a secret, and shown by its type", () => {
  assert.deepEqual(summaryFrom(login), login);
  assert.deepEqual(summaryFrom({ ...login, username: undefined, hasCode: undefined }), {
    itemId: "i1",
    type: "login",
    title: "Example",
    domains: ["example.com"],
    hasCode: false,
    modifiedAt: "2026-11-08T10:00:00.000Z",
  });
  assert.equal(summaryFrom({ ...login, type: "card" }), null);
  assert.equal(summaryFrom({ ...login, itemId: "" }), null);
  const shown = itemFrom({
    itemId: "i1",
    createdAt: "2026-11-01T10:00:00.000Z",
    modifiedAt: "2026-11-08T10:00:00.000Z",
    type: "login",
    title: "Example",
    username: "bea@example.com",
    password: "fixture-password",
    urls: ["https://example.com"],
    apps: ["so.oxy.dropsea"],
  });
  assert.equal(shown?.type, "login");
  assert.deepEqual(shown?.type === "login" ? [shown.apps, shown.totp, shown.notes] : null, [
    ["so.oxy.dropsea"],
    null,
    null,
  ]);
  assert.deepEqual(
    itemFrom({
      itemId: "p1",
      createdAt: "2026-11-01T10:00:00.000Z",
      modifiedAt: "2026-11-01T10:00:00.000Z",
      type: "passkey",
      rpId: "example.com",
      rpName: "Example",
      userName: "bea",
      userDisplayName: "Bea",
    })?.type,
    "passkey",
  );
  assert.equal(itemFrom({ itemId: "n1", createdAt: "x", modifiedAt: "x", type: "note", title: "T" }), null);
  assert.equal(itemFrom({ itemId: "w1", createdAt: "x", modifiedAt: "x", type: "wifi", ssid: "Home", security: "wpa2-psk" }), null);
});

test("enrollments, members and codes are read strictly", () => {
  assert.deepEqual(enrollmentFrom({ enrollmentId: "e1", status: "code", code: "042917", name: "Phone" }), {
    enrollmentId: "e1",
    status: "code",
    code: "042917",
    name: "Phone",
  });
  assert.equal(enrollmentFrom({ enrollmentId: "e1", status: "code" }), null);
  assert.equal(enrollmentFrom({ enrollmentId: "e1", status: "waiting", code: "42" }), null);
  assert.deepEqual(memberFrom({ memberId: "m1", name: "Laptop", platform: "oxyos", self: true, orphaned: false }), {
    memberId: "m1",
    name: "Laptop",
    platform: "oxyos",
    self: true,
    orphaned: false,
  });
  assert.equal(memberFrom({ memberId: "m1", name: "Laptop", platform: "oxyos", self: "yes", orphaned: false }), null);
  assert.deepEqual(codeFrom({ code: "287082", period: 30, remaining: 12 }), { code: "287082", period: 30, remaining: 12 });
  assert.equal(codeFrom({ code: "287082", period: 30, remaining: 31 }), null);
  assert.equal(codeFrom({ code: "28708", period: 30, remaining: 3 }), null);
});

/** A platform answering each request from a table, recording what it was asked. */
function played(answers: Record<string, unknown>) {
  const asked: KeychainRequest[] = [];
  const copied: string[] = [];
  const platform: KeychainPlatform = {
    ask: async (request) => {
      asked.push(request);
      return answers[Object.keys(request)[0]!];
    },
    copySensitive: async (text) => {
      copied.push(text);
      return true;
    },
  };
  return { client: keychainClient(platform), asked, copied };
}

test("each request is Settings' message, and each answer only the one asked for", async () => {
  const { client, asked } = played({
    keychainList: { keychainItems: [login] },
    keychainVerify: { refused: { reason: "throttled", retryAfter: 29.2 } },
    keychainGenerate: { generatedPassword: "abcde-FGHIJ-12345-kLmNo" },
    keychainCode: { keychainCode: { code: "287082", period: 30, remaining: 12 } },
    keychainDelete: "keychainDeleted",
    keychainReveal: { keychainItems: [] },
    keychainCreate: { keychainRecoveryKey: { recoveryKey: "ABCD-EFGH-JKMN-PQRS-TVWX-YZ01-2345-6789" } },
    keychainAudit: { refused: { reason: "verificationRequired", retryAfter: 30 } },
    keychainMembers: { failed: { reason: "unavailable" } },
  });
  assert.deepEqual(await client.list(), { ok: [login] });
  assert.deepEqual(await client.verify("fixture-password"), { refused: "throttled", retryAfter: 30 });
  assert.deepEqual(await client.generate(), { ok: "abcde-FGHIJ-12345-kLmNo" });
  assert.deepEqual(await client.code("i1"), { ok: { code: "287082", period: 30, remaining: 12 } });
  assert.deepEqual(await client.remove("i1"), { ok: true });
  // Another answer than the one asked for is no answer.
  assert.deepEqual(await client.reveal("i1"), { failed: "not keychainItem" });
  assert.deepEqual(await client.create(), { ok: "ABCD-EFGH-JKMN-PQRS-TVWX-YZ01-2345-6789" });
  // A wait is kept only when throttled.
  assert.deepEqual(await client.audit(), { refused: "verificationRequired" });
  assert.deepEqual(await client.members(), { failed: "unavailable" });
  assert.deepEqual(asked, [
    { keychainList: {} },
    { keychainVerify: { password: "fixture-password" } },
    { keychainGenerate: {} },
    { keychainCode: { itemId: "i1" } },
    { keychainDelete: { itemId: "i1" } },
    { keychainReveal: { itemId: "i1" } },
    { keychainCreate: {} },
    { keychainAudit: {} },
    { keychainMembers: {} },
  ]);
});

test("joining and approving send the vault's messages", async () => {
  const { client, asked } = played({
    keychainJoin: { keychainEnrollment: { enrollmentId: "e1", status: "waiting" } },
    keychainApprove: "keychainApproved",
    keychainDeny: "keychainDenied",
    keychainConfirmRecoveryKey: { keychainStatus: { state: "member", vault: true, verified: true } },
  });
  assert.deepEqual(await client.joinByApproval(), { ok: { enrollmentId: "e1", status: "waiting" } });
  assert.deepEqual(await client.joinWithRecoveryKey("abcd efgh"), { failed: "not keychainStatus" });
  assert.deepEqual(await client.confirmApproval("e1"), { ok: true });
  assert.deepEqual(await client.deny("e1"), { ok: true });
  assert.equal((await client.confirmRecoveryKey("abcd")).hasOwnProperty("ok"), true);
  assert.deepEqual(asked, [
    { keychainJoin: { method: "approval" } },
    { keychainJoin: { method: "recoveryKey", recoveryKey: "abcd efgh" } },
    { keychainApprove: { enrollmentId: "e1", confirm: true } },
    { keychainDeny: { enrollmentId: "e1" } },
    { keychainConfirmRecoveryKey: { recoveryKey: "abcd" } },
  ]);
});

test("the page opens behind the password, but for a locked keychain and a lost password", () => {
  const status = (state: string, verified: boolean, vault: boolean | null = true) =>
    statusFrom({ state, vault, verified });
  assert.equal(stageOf(null), "unavailable");
  assert.equal(stageOf(status("locked", false, null)), "locked");
  assert.equal(stageOf(status("needsPreviousPassword", false)), "needsPreviousPassword");
  for (const state of ["off", "creating", "joining", "member", "removed", "unbound"]) {
    assert.equal(stageOf(status(state, false)), "gate", state);
  }
  assert.equal(stageOf(status("off", true, false)), "turnOn");
  // Not known yet at Oxy: offered, and making it asks Oxy first.
  assert.equal(stageOf(status("off", true, null)), "turnOn");
  assert.equal(stageOf(status("off", true, true)), "join");
  assert.equal(stageOf(status("creating", true)), "creating");
  assert.equal(stageOf(status("joining", true)), "joining");
  assert.equal(stageOf(status("removed", true)), "removed");
  assert.equal(stageOf(status("unbound", true)), "unbound");
  assert.equal(stageOf(status("member", true)), "open");
  // Asked again as the window's activation changes (the lock): open only while verified.
  assert.equal(staysOpen(status("member", true)), true);
  assert.equal(staysOpen(status("member", false)), false);
  assert.equal(staysOpen(status("locked", false, null)), false);
  assert.equal(staysOpen(null), false);
  assert.equal(closesAccess({ refused: "verificationRequired" }), true);
  assert.equal(closesAccess({ refused: "locked" }), true);
  assert.equal(closesAccess({ refused: "wrongPassword" }), false);
  assert.equal(closesAccess({ ok: [] }), false);
});

const items: ItemSummary[] = [
  { itemId: "a", type: "login", title: "Árbol", username: "bea@example.com", domains: ["arbol.example"], hasCode: false, modifiedAt: "" },
  { itemId: "b", type: "login", title: "Bank", username: "bea", domains: ["bank.example"], hasCode: true, modifiedAt: "" },
  { itemId: "c", type: "passkey", title: "Example", domains: ["example.com"], hasCode: false, modifiedAt: "" },
  { itemId: "d", type: "totp", title: "Mail", domains: [], hasCode: true, modifiedAt: "" },
  { itemId: "e", type: "wifi", title: "Home", domains: [], hasCode: false, modifiedAt: "" },
  { itemId: "f", type: "note", title: "Door", domains: [], hasCode: false, modifiedAt: "" },
];

test("each category lists its items; Security those warned of; search ignores case and accents", () => {
  const ids = (list: ItemSummary[]) => list.map((item) => item.itemId);
  const audit = { reusedCount: 1, weakCount: 1, reused: [["a", "b"]], weak: ["b"] };
  assert.deepEqual(ids(itemsIn("all", items, audit)), ["a", "b", "c", "d", "e", "f"]);
  assert.deepEqual(ids(itemsIn("passwords", items, audit)), ["a", "b"]);
  assert.deepEqual(ids(itemsIn("passkeys", items, audit)), ["c"]);
  assert.deepEqual(ids(itemsIn("codes", items, audit)), ["b", "d"]);
  assert.deepEqual(ids(itemsIn("wifi", items, audit)), ["e"]);
  assert.deepEqual(ids(itemsIn("notes", items, audit)), ["f"]);
  assert.deepEqual(ids(itemsIn("security", items, audit)), ["a", "b"]);
  assert.deepEqual(ids(itemsIn("security", items, null)), []);
  assert.deepEqual(ids(itemsIn("devices", items, audit)), []);
  assert.deepEqual(ids(search(items, "ARBOL")), ["a"]);
  assert.deepEqual(ids(search(items, "bea")), ["a", "b"]);
  assert.deepEqual(ids(search(items, "example.com")), ["a", "c"]);
  assert.deepEqual(ids(search(items, "  ")), ["a", "b", "c", "d", "e", "f"]);
  assert.deepEqual(warningsOf("b", audit), { reusedWith: 2, weak: true });
  assert.deepEqual(warningsOf("c", audit), { reusedWith: 0, weak: false });
  assert.equal(initialOf(items[0]!), "Á");
  assert.equal(initialOf({ title: " ", domains: ["example.com"] }), "E");
});

test("the recovery key in eight groups of four, a code in halves, a setup key as a URI", () => {
  assert.deepEqual(recoveryKeyGroups("abcd-efgh-jkmn-pqrs-tvwx-yz01-2345-6789"), [
    "ABCD",
    "EFGH",
    "JKMN",
    "PQRS",
    "TVWX",
    "YZ01",
    "2345",
    "6789",
  ]);
  assert.equal(formatCode("287082"), "287 082");
  assert.equal(formatCode("28708212"), "2870 8212");
  assert.equal(
    otpauthFrom("jbsw y3dp ehpk 3pxp", "Example", "bea@example.com"),
    "otpauth://totp/Example:bea%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Example",
  );
  assert.equal(otpauthFrom("otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP", "", ""), "otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP");
  assert.equal(otpauthFrom("not a key", "Example", "bea"), null);
  assert.equal(otpauthFrom("", "Example", "bea"), null);
  assert.deepEqual(urlsFromText("example.com\n\n https://bank.example/login \n"), [
    "https://example.com",
    "https://bank.example/login",
  ]);
});

test("refusals are said in the person's language, never as the service names them", () => {
  for (const language of ["en", "es"] as const) {
    const words = messagesFor(language).passwords;
    assert.equal(outcomeMessage({ refused: "wrongPassword" }, words), words.wrongPassword);
    assert.equal(outcomeMessage({ refused: "throttled", retryAfter: 30 }, words), words.tryAgainIn(30));
    assert.equal(outcomeMessage({ refused: "throttled" }, words), words.tryAgainLater);
    assert.equal(outcomeMessage({ refused: "offline" }, words), words.offline);
    assert.equal(outcomeMessage({ refused: "recoveryKeyMismatch" }, words), words.recoveryMismatch);
    assert.equal(outcomeMessage({ refused: "notPermitted" }, words), words.failed);
    assert.equal(outcomeMessage({ failed: "unavailable" }, words), words.failed);
  }
  assert.equal(CATALOGUES.es.passwords.categories.passkeys, "Llaves de acceso");
  assert.equal(CATALOGUES.es.passwords.generate, "Generar contraseña segura");
  assert.equal(CATALOGUES.es.pages.passwords, "Contraseñas");
});

test("a copy goes to the platform's sensitive clipboard", async () => {
  const { client, copied } = played({});
  assert.equal(await client.copySensitive("fixture-password"), true);
  assert.deepEqual(copied, ["fixture-password"]);
});
