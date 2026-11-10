/**
 * The system's words follow one glossary (docs/wording.mdx), whichever app says them: the shell,
 * Settings, the login window and Activity Monitor. Each catalogue is rendered whole (every
 * function called with sample arguments) and checked for what the glossary rules out.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages, type Language } from "../src/messages.ts";
import { CATALOGUES as SETTINGS } from "../../settings/src/messages.ts";
import { LOGIN_CATALOGUES as LOGIN } from "../../login/src/messages.ts";
import { CATALOGUES as ACTIVITY_MONITOR } from "../../activity-monitor/src/messages.ts";

const LANGUAGES: Language[] = ["en", "es"];

/** Every string a catalogue can say, with where it is: functions called with "Ada", 1 and 2. */
function said(value: unknown, path: string, out: [string, string][] = []): [string, string][] {
  if (typeof value === "string") out.push([path, value]);
  else if (typeof value === "function") {
    for (const sample of ["Ada", 1, 2]) {
      const args = Array.from({ length: Math.max(1, value.length) }, () => sample);
      try {
        said(value(...args), path, out);
      } catch {
        // A function that wants another kind of argument (a list): its other samples cover it.
      }
    }
  } else if (Array.isArray(value)) {
    for (const [i, item] of value.entries()) said(item, `${path}[${i}]`, out);
  } else if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      if (key === "format" || key === "locale" || key === "language") continue;
      said(item, `${path}.${key}`, out);
    }
  }
  return out;
}

function everything(language: Language): [string, string][] {
  return [
    ...said(shellMessages(language === "es" ? "es-ES" : "en-US", language), "shell"),
    ...said(SETTINGS[language], "settings"),
    ...said(LOGIN[language], "login"),
    ...said(ACTIVITY_MONITOR[language], "activityMonitor"),
  ];
}

function offending(language: Language, wrong: RegExp): string[] {
  return everything(language)
    .filter(([, text]) => wrong.test(text))
    .map(([path, text]) => `${path}: ${text}`);
}

test("every catalogue writes apostrophes and quotes as typeset: ’ and “ ”, never ' or « »", () => {
  for (const language of LANGUAGES) {
    assert.deepEqual(offending(language, /'/), [], language);
    assert.deepEqual(offending(language, /[«»]/), [], language);
  }
});

test("English is US English", () => {
  assert.deepEqual(offending("en", /\b(colour|grey|centre|cancelled|favourite|any more)\b/i), []);
});

test("English: Log Out ends the session; sign in and sign out are the Oxy account's; login only in Apple's own names", () => {
  // No "Login" as a verb or a screen, no "sign-in" as a noun, no "login screen".
  assert.deepEqual(offending("en", /\bsign-in\b|\blogin screen\b|\blog(ging)? (in|back in)\b/i), []);
  // "Login" capitalized only in Apple's names: Open at Login, Login Items & Extensions.
  assert.deepEqual(offending("en", /(?<!at )\bLogin\b(?! Items)/), []);
});

test("English: a count is never written with a fixed plural", () => {
  assert.deepEqual(offending("en", /\b1 (seconds|minutes|hours|days|items|apps|accounts|devices|photos)\b/), []);
});

test("English: plain words, not the system's parts", () => {
  assert.deepEqual(offending("en", /NetworkManager|\bapt\b|\bpings?\b|otpauth:|power service|systemd|logind/), []);
});

test("Spanish: Ajustes del Sistema, the login window and the Trash by their names, ajustes and not configuración", () => {
  assert.deepEqual(offending("es", /Ajustes del sistema|pantalla de inicio|papelera|configuración de pantalla|configuración anterior/), []);
  assert.deepEqual(offending("es", /cuenta Oxy/), []);
  assert.deepEqual(offending("es", /Inténtalo de nuevo/), []);
  assert.deepEqual(offending("es", /\b1 (segundos|minutos|horas|días|ítems|apps|cuentas|dispositivos|fotos)\b/), []);
});

test("Spanish: a pane named in Ajustes de X… is named as the sidebar names it", () => {
  const shell = shellMessages("es-ES", "es");
  assert.equal(shell.battery.settings, "Ajustes de Batería…");
  assert.equal(shell.sound.settings, "Ajustes de Sonido…");
  assert.equal(shell.wifi.settings, "Ajustes de Wi-Fi…");
  assert.equal(shell.bluetooth.settings, "Ajustes de Bluetooth…");
  assert.equal(shell.inputSources.openKeyboardSettings, "Abrir ajustes de Teclado…");
  assert.equal(SETTINGS.es.wifi.networkSettings, "Ajustes de Red…");
});

test("one wrong-password message in each language, wherever a password is asked", () => {
  for (const language of LANGUAGES) {
    const shell = shellMessages(language === "es" ? "es-ES" : "en-US", language);
    const settings = SETTINGS[language];
    const wrong = [
      shell.password.incorrect,
      shell.sheet.wifiIncorrect,
      shell.sheet.passkeyIncorrect,
      settings.passwords.wrongPassword,
      settings.localSignIn.wrongPassword,
      settings.transferReset.wrongPassword,
      settings.wifi.failures.wrongPassword,
      LOGIN[language].setup.passwordIncorrect,
    ];
    assert.deepEqual(new Set(wrong), new Set([language === "es" ? "Contraseña incorrecta. Vuelve a intentarlo." : "Incorrect password. Try again."]));
  }
});

test("Done and OK by their role: Spanish says OK for Done and Aceptar for OK", () => {
  const shell = shellMessages("es-ES", "es");
  assert.equal(shell.sheet.done, "OK");
  assert.equal(shell.sheet.ok, "Aceptar");
  assert.equal(SETTINGS.es.passwords.done, "OK");
  assert.equal(SETTINGS.es.bluetooth.done, "OK");
  assert.equal(SETTINGS.es.bluetooth.ok, "Aceptar");
  assert.equal(SETTINGS.es.storage.ok, "Aceptar");
  assert.equal(ACTIVITY_MONITOR.es.inspector.done, "OK");
});

test("counts go through the plural rules: one and other, in both languages", () => {
  assert.equal(SETTINGS.en.passwords.tryAgainIn(1), "Too many attempts. Try again in 1 second.");
  assert.equal(SETTINGS.en.passwords.tryAgainIn(30), "Too many attempts. Try again in 30 seconds.");
  assert.equal(SETTINGS.es.passwords.tryAgainIn(1), "Demasiados intentos. Vuelve a intentarlo en 1 segundo.");
  assert.equal(SETTINGS.en.softwareUpdate.updatingCount(1), "Updating 1 app");
  assert.equal(SETTINGS.en.softwareUpdate.updatingCount(3), "Updating 3 apps");
  assert.equal(SETTINGS.es.softwareUpdate.updatingCount(1), "Actualizando 1 app");
  assert.equal(SETTINGS.en.passwords.reused(1), "Reused on 1 item");
  assert.equal(SETTINGS.es.passwords.reused(2), "Reutilizada en 2 ítems");
  assert.equal(SETTINGS.en.passwords.codeExpiresIn("123 456", 1), "123 456, expires in 1 second");
  assert.equal(LOGIN.en.setup.passwordThrottled(1), "Too many attempts. Wait 1 second, then try again.");
  assert.equal(LOGIN.es.setup.passwordThrottled(1), "Demasiados intentos. Espera 1 segundo y vuelve a intentarlo.");
  assert.equal(LOGIN.en.setup.waiting(1), "Oxy is busy right now. Setup goes on by itself in 1 second.");
  assert.equal(LOGIN.en.migrate.migrated(2), "2 accounts were brought over");
});

test("Empty Trash… asks first, so it ends in an ellipsis", () => {
  assert.equal(shellMessages("en-US", "en").dock.emptyTrash, "Empty Trash…");
  assert.equal(shellMessages("es-ES", "es").dock.emptyTrash, "Vaciar la Papelera…");
});
