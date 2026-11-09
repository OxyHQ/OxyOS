import { test } from "node:test";
import assert from "node:assert/strict";
import { personName, sessionUserFrom } from "../src/session-user.ts";

const signedIn = (account: Record<string, unknown>) => ({ status: "signed_in", revision: 3, online: false, account });

test("the session's user is the account's: its display name, else its username", () => {
  const user = sessionUserFrom(signedIn({ id: "a1", handle: "nate", displayName: "Nate" }));
  assert.deepEqual(user, { name: "nate", fullName: "Nate" });
  assert.equal(personName(user), "Nate");
  // Without a display name (blank is none), the username, as Oxy's apps call the person.
  const handleOnly = sessionUserFrom(signedIn({ id: "a1", handle: "nate", displayName: "  " }));
  assert.deepEqual(handleOnly, { name: "nate" });
  assert.equal(personName(handleOnly), "nate");
});

test("without a signed-in account, no name at all: never one of the machine's", () => {
  for (const snapshot of [null, undefined, "snapshot", { status: "signed_out", account: null }, { status: "unavailable" }]) {
    assert.deepEqual(sessionUserFrom(snapshot), { name: "" }, JSON.stringify(snapshot));
    assert.equal(personName(sessionUserFrom(snapshot)), "");
  }
  // A malformed account is none.
  assert.deepEqual(sessionUserFrom(signedIn({ id: "a1" })), { name: "" });
});

test("what a screen calls the person: the full name, else the short name", () => {
  assert.equal(personName({ name: "nate", fullName: "Nate Ibáñez" }), "Nate Ibáñez");
  assert.equal(personName({ name: "nate", fullName: " " }), "nate");
  assert.equal(personName({ name: "" }), "");
  assert.equal(personName(null), "");
});

test("the session's user's picture is the account's, as the platform keeps it on this computer", () => {
  const user = sessionUserFrom(
    signedIn({ id: "a1", handle: "nate", displayName: "Nate", picture: "/var/lib/oxyos/pictures/oxy-n-01a1" }),
  );
  assert.deepEqual(user, { name: "nate", fullName: "Nate", picture: "file:///var/lib/oxyos/pictures/oxy-n-01a1" });
  // No picture, or one that is not a path of the machine's: the default picture.
  assert.equal(sessionUserFrom(signedIn({ id: "a1", handle: "nate" })).picture, undefined);
  assert.equal(sessionUserFrom(signedIn({ id: "a1", handle: "nate", picture: "https://x/y.png" })).picture, undefined);
  assert.equal(sessionUserFrom(null).picture, undefined);
});
