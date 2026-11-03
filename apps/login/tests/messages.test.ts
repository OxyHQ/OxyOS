import { test } from "node:test";
import assert from "node:assert/strict";
import { LOGIN_CATALOGUES, loginMessages } from "../src/messages.ts";

/** Every path of a catalogue to the kind of what is there. */
function shape(value: unknown, path = ""): string[] {
  if (value && typeof value === "object") {
    return Object.keys(value)
      .sort()
      .flatMap((key) => shape((value as Record<string, unknown>)[key], path ? `${path}.${key}` : key));
  }
  return [`${path}: ${typeof value}`];
}

function said(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (value && typeof value === "object") return Object.values(value).flatMap(said);
  return [];
}

test("the login screen's Spanish and English have the same strings, key for key", () => {
  assert.deepEqual(shape(LOGIN_CATALOGUES.es), shape(LOGIN_CATALOGUES.en));
});

test("every Spanish string is translated, none English pasted in", () => {
  const en = said(LOGIN_CATALOGUES.en);
  const es = said(LOGIN_CATALOGUES.es);
  assert.equal(es.length, en.length);
  assert.ok(es.every((text, i) => text.length > 0 && text !== en[i]));
});

test("the login screen speaks the language it is given, Spanish as before", () => {
  assert.equal(loginMessages("es").addAccount, "Añadir cuenta");
  assert.equal(loginMessages("es").setup.title, "Añadir una cuenta de Oxy");
  assert.equal(loginMessages("en").addAccount, "Add Account");
  assert.equal(loginMessages("en").setup.retry, "Try Again");
});
