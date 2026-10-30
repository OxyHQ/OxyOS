import { test } from "node:test";
import assert from "node:assert/strict";
import {
  accountsFrom,
  chosenAccount,
  opensOnSetup,
  setupAfterCheck,
} from "../src/model.ts";

test("the accounts' service's list becomes the screen's accounts", () => {
  assert.equal(accountsFrom(null), null);
  assert.equal(accountsFrom({ accounts: [] }), null);
  assert.deepEqual(accountsFrom([]), []);
  assert.deepEqual(
    accountsFrom([
      { user: "oxy-a", name: "Ada Lovelace", picture: "/var/lib/oxyos/pictures/oxy-a" },
      { user: "oxy-b", name: "  " },
      { name: "no user" },
      "nonsense",
    ]),
    [
      { user: "oxy-a", name: "Ada Lovelace", picture: "file:///var/lib/oxyos/pictures/oxy-a" },
      { user: "oxy-b", name: "oxy-b", picture: null },
    ],
  );
});

test("the password is for the account chosen, else the first", () => {
  const accounts = accountsFrom([{ user: "oxy-a", name: "Ada" }, { user: "oxy-b", name: "Bea" }]);
  assert.equal(chosenAccount(accounts, null)?.user, "oxy-a");
  assert.equal(chosenAccount(accounts, "oxy-b")?.user, "oxy-b");
  // One no longer listed (removed meanwhile) is not signed in to.
  assert.equal(chosenAccount(accounts, "oxy-gone")?.user, "oxy-a");
  assert.equal(chosenAccount([], "oxy-a"), null);
  assert.equal(chosenAccount(null, "oxy-a"), null);
});

test("setup is online only, and opens at the first boot", () => {
  assert.equal(setupAfterCheck(false), "offline");
  assert.equal(setupAfterCheck(true), "unavailable");
  assert.equal(opensOnSetup([]), true);
  assert.equal(opensOnSetup([{ user: "oxy-a", name: "Ada" }]), false);
  // Accounts that cannot be read are no first boot: nothing is offered until they can.
  assert.equal(opensOnSetup(null), false);
});
