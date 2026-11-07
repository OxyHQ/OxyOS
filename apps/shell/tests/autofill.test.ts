import { test } from "node:test";
import assert from "node:assert/strict";
import { autofillLabel, autofillRow, autofillSaid, readAutofill, type AutofillList } from "../src/autofill.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");
const EN = shellMessages("en-US");

const logins: AutofillList = {
  content: "username",
  highlighted: 0,
  items: [
    { kind: "login", title: "example.com", detail: "bea@example.com" },
    { kind: "login", title: "example.com", detail: "ana@example.com" },
  ],
};

test("the list the session tells is read strictly", () => {
  assert.deepEqual(readAutofill(JSON.parse(JSON.stringify(logins))), logins);
  assert.equal(readAutofill(null), null);
  assert.equal(readAutofill({ ...logins, items: [] }), null);
  assert.equal(readAutofill({ ...logins, highlighted: 2 }), null);
  assert.equal(readAutofill({ ...logins, items: [{ kind: "card", title: "", detail: "" }] }), null);
  assert.equal(readAutofill({ ...logins, items: [{ kind: "login", title: 1, detail: "" }] }), null);
});

test("a login shows its username over its title, a strong password and a code their words", () => {
  assert.deepEqual(autofillRow(logins.items[0], EN), { primary: "bea@example.com", secondary: "example.com" });
  assert.deepEqual(autofillRow({ kind: "strongPassword", title: "", detail: "fixtr-Strng-Pass0-wordX" }, ES), {
    primary: "Usar contraseña segura",
    secondary: "fixtr-Strng-Pass0-wordX",
  });
  assert.deepEqual(autofillRow({ kind: "strongPassword", title: "", detail: "x" }, EN).primary, "Use Strong Password");
  assert.deepEqual(autofillRow({ kind: "code", title: "example.com", detail: "bea@example.com" }, ES), {
    primary: "example.com",
    secondary: "Código de verificación de bea@example.com",
  });
  assert.equal(autofillRow({ kind: "code", title: "example.com", detail: "" }, EN).secondary, "Verification code");
});

test("the reader says the highlighted item with its place and the count", () => {
  assert.equal(autofillSaid(logins, EN), "bea@example.com, example.com, 1 of 2");
  assert.equal(autofillSaid({ ...logins, highlighted: 1 }, ES), "ana@example.com, example.com, 2 de 2");
  assert.equal(autofillLabel(logins, 5, ES), "Contraseñas");
});
