import { test } from "node:test";
import assert from "node:assert/strict";
import { personName, sessionUserFrom } from "../src/session-user.ts";

const signedIn = (account: Record<string, unknown>) => ({ status: "signed_in", revision: 3, online: false, account });

test("the session's user is the account's: its display name, else its username", () => {
  const user = sessionUserFrom(signedIn({ id: "a1", handle: "nate", displayName: "Nate" }), "");
  assert.deepEqual(user, { name: "nate", fullName: "Nate" });
  assert.equal(personName(user), "Nate");
  // Without a display name (blank is none), the username, as Oxy's apps call the person.
  const handleOnly = sessionUserFrom(signedIn({ id: "a1", handle: "nate", displayName: "  " }), "Old Name");
  assert.deepEqual(handleOnly, { name: "nate" });
  assert.equal(personName(handleOnly), "nate");
});

test("without a signed-in account, the system's full name; never the Unix user's name", () => {
  for (const snapshot of [null, undefined, "snapshot", { status: "signed_out", account: null }, { status: "unavailable" }]) {
    assert.deepEqual(sessionUserFrom(snapshot, " Nate "), { name: "", fullName: "Nate" }, JSON.stringify(snapshot));
    assert.deepEqual(sessionUserFrom(snapshot, ""), { name: "" });
    assert.equal(personName(sessionUserFrom(snapshot, null)), "");
  }
  // A malformed account is none.
  assert.deepEqual(sessionUserFrom(signedIn({ id: "a1" }), "Nate"), { name: "", fullName: "Nate" });
});

test("what a screen calls the person: the full name, else the short name", () => {
  assert.equal(personName({ name: "nate", fullName: "Nate Ibáñez" }), "Nate Ibáñez");
  assert.equal(personName({ name: "nate", fullName: " " }), "nate");
  assert.equal(personName({ name: "" }), "");
  assert.equal(personName(null), "");
});
