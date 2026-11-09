import { test } from "node:test";
import assert from "node:assert/strict";
import { accountName, accountSnapshotFrom, accountView, initialsOf, localPicture, pictureOf } from "../src/account.ts";
import { messagesFor } from "../src/messages.ts";

const ada = { id: "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", handle: "ada", displayName: "Ada Lovelace" };

test("the account item shows only an account the platform has", () => {
  assert.equal(accountView(null), null, "outside a session: no item");
  assert.equal(accountView(undefined), null);
  assert.equal(
    accountView({ status: "unavailable", revision: 1, account: null, online: false }),
    null,
    "a platform that cannot say: no item that says nothing",
  );
  assert.deepEqual(accountView({ status: "signed_out", revision: 2, account: null, online: true }), {
    kind: "signed_out",
  });
  assert.deepEqual(accountView({ status: "signed_in", revision: 3, account: ada, online: false }), {
    kind: "signed_in",
    name: "Ada Lovelace",
    handle: "ada",
    picture: null,
    initials: "AL",
    online: false,
  });
});

test("a name is the display name, else the handle", () => {
  assert.equal(accountName({ handle: "ada", displayName: "  " }), "ada");
  assert.equal(accountName({ handle: "ada" }), "ada");
  assert.equal(accountName({ handle: "ada", displayName: "Ada" }), "Ada");
});

test("initials are the first and last words' first letters", () => {
  assert.equal(initialsOf("Ada Lovelace"), "AL");
  assert.equal(initialsOf("ada"), "A");
  assert.equal(initialsOf("Ada Augusta King Lovelace"), "AL");
  assert.equal(initialsOf("ñandú élan"), "ÑÉ");
  assert.equal(initialsOf("   "), "");
});

test("a picture is shown only from a URL an image can load", () => {
  assert.equal(pictureOf("https://cloud.oxy.so/a.png"), "https://cloud.oxy.so/a.png");
  assert.equal(pictureOf("file:///var/lib/oxyos/pictures/oxy-a"), "file:///var/lib/oxyos/pictures/oxy-a");
  assert.equal(pictureOf("65f0c3a1b2"), null, "a file id is not a picture");
  assert.equal(pictureOf("javascript:alert(1)"), null);
  assert.equal(pictureOf(undefined), null);
});

test("a snapshot is the platform's, read strictly", () => {
  assert.deepEqual(accountSnapshotFrom({ status: "signed_in", revision: 1, account: ada, online: true }), {
    status: "signed_in",
    revision: 1,
    account: ada,
    online: true,
  });
  assert.equal(accountSnapshotFrom({ status: "signed_in", revision: 1, account: null, online: true }), null);
  assert.equal(accountSnapshotFrom({ status: "signed_out", revision: 1, account: ada, online: true }), null);
  assert.equal(accountSnapshotFrom({ status: "signed_in", revision: "1", account: ada, online: true }), null);
  assert.equal(accountSnapshotFrom({ status: "elsewhere", revision: 1, account: null, online: true }), null);
  assert.equal(accountSnapshotFrom(null), null);
});

test("the account's words in both languages", () => {
  const en = messagesFor("en-US").account;
  const es = messagesFor("es-ES").account;
  assert.equal(en.group, "Oxy Account");
  assert.equal(es.group, "Cuenta de Oxy");
  assert.equal(en.removeTitle("Ada"), "Remove Ada from this computer?");
  assert.equal(es.removeTitle("Ada"), "¿Eliminar a Ada de este ordenador?");
  assert.equal(es.handle("ada"), "@ada");
});

test("the account's picture is the one the platform keeps on this computer, else none", () => {
  const snapshot = accountSnapshotFrom({
    status: "signed_in",
    revision: 2,
    online: false,
    account: { id: "a", handle: "ada", displayName: "Ada", avatar: "01a1-file", picture: "/var/lib/oxyos/pictures/oxy-a-01a1-file" },
  });
  const view = accountView(snapshot);
  assert.equal(view?.kind === "signed_in" ? view.picture : undefined, "file:///var/lib/oxyos/pictures/oxy-a-01a1-file");
  assert.equal(localPicture("relative/path"), null);
  assert.equal(localPicture(undefined), null);
  assert.equal(localPicture("/a b/c#d"), "file:///a%20b/c%23d");
});
