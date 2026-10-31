import { test } from "node:test";
import assert from "node:assert/strict";
import { barKey } from "../src/menu-bar-keys.ts";

test("Left and Right move across the menus, open or not", () => {
  for (const open of [false, true]) {
    assert.equal(barKey({ key: "ArrowLeft" }, open), "previous");
    assert.equal(barKey({ key: "ArrowRight" }, open), "next");
  }
});

test("with no menu open, Down, Return and Space open it and Escape leaves the bar", () => {
  for (const key of ["ArrowDown", "Enter", " "]) assert.equal(barKey({ key }, false), "open");
  assert.equal(barKey({ key: "Escape" }, false), "leave");
  assert.equal(barKey({ key: "ArrowUp" }, false), null);
  assert.equal(barKey({ key: "a" }, false), null);
});

test("with a menu open, its keys are the menu's", () => {
  for (const key of ["ArrowDown", "ArrowUp", "Home", "End", "Enter", " ", "Escape", "d"]) {
    assert.equal(barKey({ key }, true), null);
  }
});

test("a key with Ctrl, Alt or Super is never the bar's", () => {
  for (const modifier of ["ctrlKey", "altKey", "metaKey"] as const) {
    assert.equal(barKey({ key: "ArrowRight", [modifier]: true }, false), null);
    assert.equal(barKey({ key: "Escape", [modifier]: true }, false), null);
  }
});
