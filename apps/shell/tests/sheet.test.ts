import { test } from "node:test";
import assert from "node:assert/strict";
import {
  bluetoothCodeTyped,
  readSheet,
  sheetArrived,
  sheetGone,
  sheetView,
  size,
  spokenCode,
  submittablePassword,
  validBluetoothCode,
  validWifiPassword,
  type AvailableUpdate,
  type Plan,
} from "../src/sheet.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");
const EN = shellMessages("en-US");

const plan: Plan = {
  id: "so.oxy.notes",
  name: "Notes",
  names: { es: "Notas" },
  version: "1.12.0",
  download: 71_000,
  capabilities: [
    { name: "network", purpose: { en: "Sync your notes", es: "Sincronizar tus notas" }, askedAtFirstUse: false },
    { name: "notifications", purpose: { en: "Reminders" }, askedAtFirstUse: true },
  ],
  newCapabilities: [],
  keyRotation: false,
  update: false,
};

test("an install sheet names the app, its size and each capability with its purpose", () => {
  const es = sheetView({ kind: "install", plan }, ES);
  assert.equal(es.title, "¿Instalar Notas?");
  assert.equal(es.body, "Versión 1.12.0 · 71 kB");
  assert.deepEqual(es.lines[0], { label: "Internet", detail: "Sincronizar tus notas" });
  assert.equal(es.lines[1].label, "Notificaciones");
  assert.match(es.lines[1].detail!, /Se preguntará cuando la app lo use/);
  assert.equal(es.accept, "Instalar");
  const en = sheetView({ kind: "install", plan }, EN);
  assert.equal(en.title, "Install Notes?");
  assert.equal(en.lines[1].detail, "Reminders · Asked when the app uses it");
});

test("without the install service, the sheet says so with one button and nothing to confirm", () => {
  const es = sheetView({ kind: "unavailable", action: "install" }, ES);
  assert.equal(es.title, "No se puede instalar la app");
  assert.match(es.body, /El servicio que instala las apps en este ordenador no está funcionando/);
  assert.equal(es.accept, "Aceptar");
  assert.equal(es.refuse, "");
  assert.equal(es.destructive, false);
  const en = sheetView({ kind: "unavailable", action: "update" }, EN);
  assert.equal(en.title, "Can’t Update");
  assert.equal(en.accept, "OK");
  assert.deepEqual(readSheet('{"kind":"unavailable","action":"uninstall"}'), { kind: "unavailable", action: "uninstall" });
});

test("removing says the data goes, and its button is destructive", () => {
  const view = sheetView({ kind: "uninstall", id: "so.oxy.notes", name: "Notes", version: "1.12.0" }, EN);
  assert.equal(view.title, "Delete Notes and its data?");
  assert.equal(view.accept, "Delete");
  assert.equal(view.destructive, true);
});

test("a consent lists what the update adds, and a rotated key", () => {
  const update: AvailableUpdate = {
    id: "so.oxy.notes",
    name: "Notes",
    names: {},
    toVersion: "1.13.0",
    download: 40,
    releaseNotes: {},
    consent: { capabilities: ["oxy-account"], keyRotation: true },
  };
  const view = sheetView({ kind: "consent", update }, EN);
  assert.equal(view.title, "Notes asks for more access");
  assert.deepEqual(view.lines.map((l) => l.label), [
    "Your Oxy account",
    "The developer signs this version with a new key of its lineage.",
  ]);
});

test("a permission asks in the registry's name for the app", () => {
  const view = sheetView({ kind: "permission", id: "so.oxy.notes", capability: "notifications" }, ES, {
    "so.oxy.notes": "Notas",
  });
  assert.equal(view.title, "¿Permitir que Notas enviarte notificaciones?");
  assert.equal(view.accept, "Permitir");
  assert.equal(view.refuse, "No permitir");
});

test("an app asking to save a password: its registry name, the username, never the password", () => {
  const sheet = readSheet(
    '{"kind":"save-password","id":"so.oxy.mail","username":"bea@example.com","domain":"example.com"}',
  );
  assert.ok(sheet);
  const es = sheetView(sheet, ES, { "so.oxy.mail": "Correo" });
  assert.equal(es.title, "¿Guardar la contraseña para Correo?");
  assert.equal(es.body, "bea@example.com · example.com");
  assert.equal(es.accept, "Guardar");
  assert.equal(es.refuse, "Ahora no");
  assert.equal(es.destructive, false);
  assert.equal(es.lines.length, 1);
  const en = sheetView({ kind: "save-password", id: "so.oxy.mail", username: "bea" }, EN);
  // Without the registry's name, its id: never a name the application gives itself.
  assert.equal(en.title, "Save password for so.oxy.mail?");
  assert.equal(en.body, "bea");
  assert.equal(en.accept, "Save");
  assert.equal(en.refuse, "Not Now");
  assert.match(en.lines[0].label, /keychain/);
});

test("sheets queue in order, once each, and go", () => {
  let queue = sheetArrived([], 1, '{"kind":"uninstall","id":"a","name":"A","version":"1"}');
  queue = sheetArrived(queue, 2, '{"kind":"permission","id":"b","capability":"camera"}');
  queue = sheetArrived(queue, 1, '{"kind":"uninstall","id":"a","name":"A","version":"1"}');
  queue = sheetArrived(queue, 3, "not json");
  assert.deepEqual(queue.map((s) => s.sheet), [1, 2]);
  assert.deepEqual(sheetGone(queue, 1).map((s) => s.sheet), [2]);
  assert.equal(readSheet("{}"), null);
});

test("sizes read as a person reads them", () => {
  assert.equal(size(71, EN), "71 bytes");
  assert.equal(size(1_500_000, ES), "1,5 MB");
  assert.equal(size(15_000_000, EN), "15 MB");
});

test("emptying the Trash says how many items go for good, and its button is destructive", () => {
  const sheet = readSheet('{"items":3,"kind":"empty-trash"}');
  assert.ok(sheet);
  const view = sheetView(sheet, ES);
  assert.equal(view.title, "¿Seguro que quieres eliminar permanentemente los ítems de la Papelera?");
  assert.equal(view.body, "Se eliminarán 3 ítems. No puedes deshacer esta acción.");
  assert.equal(view.accept, "Vaciar");
  assert.equal(view.refuse, "Cancelar");
  assert.equal(view.destructive, true);
  assert.equal(sheetView({ kind: "empty-trash", items: 1 }, EN).body, "1 item will be erased. You can’t undo this action.");
});

test("a Wi-Fi network's password sheet names it and its kind, with a password field and Join", () => {
  const es = sheetView({ kind: "wifi-password", ssid: "Casa", security: "wpa", incorrect: false }, ES);
  assert.equal(es.title, 'La red Wi-Fi “Casa” requiere una contraseña WPA2.');
  assert.equal(es.error, undefined);
  assert.equal(es.accept, "Conectarse");
  assert.equal(es.refuse, "Cancelar");
  assert.deepEqual(es.password, { label: "Contraseña", show: "Mostrar contraseña", hide: "Ocultar contraseña" });
  const wpa3 = sheetView({ kind: "wifi-password", ssid: "Casa", security: "wpa3", incorrect: true }, ES);
  assert.equal(wpa3.title, 'La red Wi-Fi “Casa” requiere una contraseña WPA3.');
  assert.equal(wpa3.error, "Contraseña incorrecta. Vuelve a intentarlo.");
  const en = sheetView({ kind: "wifi-password", ssid: "Casa", security: "wpa", incorrect: true }, EN);
  assert.equal(en.title, 'The Wi-Fi network “Casa” requires a WPA2 password.');
  assert.equal(en.error, "Incorrect password. Try again.");
  assert.equal(en.accept, "Join");
  assert.equal(en.refuse, "Cancel");
});

test("a join that failed, and a network OxyOS cannot join, only tell, with one button", () => {
  const failed = sheetView({ kind: "wifi-failed", ssid: "Casa" }, ES);
  assert.equal(failed.title, 'No se ha podido conectar a la red “Casa”.');
  assert.equal(failed.accept, "Aceptar");
  assert.equal(failed.refuse, "");
  assert.equal(failed.password, undefined);
  assert.equal(sheetView({ kind: "wifi-failed", ssid: "Casa" }, EN).title, "Couldn’t join the network “Casa”.");
  const enterprise = sheetView({ kind: "wifi-unsupported", ssid: "Oficina", security: "enterprise" }, ES);
  assert.equal(enterprise.title, 'OxyOS todavía no puede conectarse a “Oficina”.');
  assert.equal(enterprise.body, "Las redes empresariales (802.1X) aún no son compatibles.");
  assert.equal(enterprise.accept, "Aceptar");
  assert.equal(enterprise.refuse, "");
  const wep = sheetView({ kind: "wifi-unsupported", ssid: "Antiguo", security: "wep" }, ES);
  assert.equal(wep.body, "Las redes WEP no son compatibles: no son seguras.");
  const en = sheetView({ kind: "wifi-unsupported", ssid: "Antiguo", security: "wep" }, EN);
  assert.equal(en.title, "OxyOS can’t join “Antiguo” yet.");
  assert.equal(en.body, "WEP networks aren’t supported: they aren’t secure.");
});

test("a Wi-Fi password is 8 to 63 bytes, or 64 hexadecimal digits", () => {
  assert.equal(validWifiPassword(""), false);
  assert.equal(validWifiPassword("1234567"), false);
  assert.equal(validWifiPassword("12345678"), true);
  assert.equal(validWifiPassword("a".repeat(63)), true);
  assert.equal(validWifiPassword("a".repeat(64)), true);
  assert.equal(validWifiPassword("g".repeat(64)), false);
  assert.equal(validWifiPassword("a".repeat(65)), false);
  // Counted in bytes, as the daemon counts them: 32 "ñ" are 64 bytes.
  assert.equal(validWifiPassword("ñ".repeat(31)), true);
  assert.equal(validWifiPassword("ñ".repeat(32)), false);
  assert.equal(validWifiPassword("ññññ"), true);
});

test("a numeric comparison shows the code large, with Cancel and Pair", () => {
  const sheet = readSheet('{"kind":"bluetooth-confirm","name":"iPhone","device":"phone","code":"123456"}');
  assert.ok(sheet);
  const en = sheetView(sheet, EN);
  assert.equal(en.title, "Confirm that “iPhone” shows this code.");
  assert.equal(en.code, "123456");
  assert.equal(en.accept, "Pair");
  assert.equal(en.refuse, "Cancel");
  assert.equal(en.codeField, undefined);
  const es = sheetView(sheet, ES);
  assert.equal(es.title, "Confirma que “iPhone” muestra este código.");
  assert.equal(es.accept, "Enlazar");
  assert.equal(es.refuse, "Cancelar");
});

test("a device pairing with nothing to compare, and one asking for a service", () => {
  const authorize = sheetView({ kind: "bluetooth-authorize", name: "Pixel", device: "phone" }, EN);
  assert.equal(authorize.title, "“Pixel” wants to pair with this computer.");
  assert.equal(authorize.accept, "Pair");
  assert.equal(authorize.refuse, "Cancel");
  assert.equal(authorize.code, undefined);
  const service = sheetView({ kind: "bluetooth-service", name: "AirPods", device: "headphones", service: "audio" }, EN);
  assert.equal(service.title, "Allow “AirPods” to play and record audio?");
  assert.equal(service.accept, "Allow");
  assert.equal(service.refuse, "Don’t Allow");
  const es = sheetView({ kind: "bluetooth-service", name: "Pixel", device: "phone", service: "files" }, ES);
  assert.equal(es.title, "¿Permitir que “Pixel” envíe y reciba archivos?");
  assert.equal(es.accept, "Permitir");
  assert.equal(es.refuse, "No permitir");
  // A service OxyOS does not name is "connect to this computer".
  const other = sheetView({ kind: "bluetooth-service", name: "X", device: "other", service: "midi" as "other" }, EN);
  assert.equal(other.title, "Allow “X” to connect to this computer?");
});

test("a code to type on the device is shown large, and only Cancel answers it", () => {
  const view = sheetView({ kind: "bluetooth-display", name: "Magic Keyboard", device: "keyboard", code: "048213" }, EN);
  assert.equal(view.title, "Type this code on “Magic Keyboard”, then press Return.");
  assert.equal(view.code, "048213");
  assert.equal(view.accept, "");
  assert.equal(view.refuse, "Cancel");
  assert.equal(
    sheetView({ kind: "bluetooth-display", name: "Magic Keyboard", device: "keyboard", code: "0000" }, ES).title,
    "Escribe este código en “Magic Keyboard” y pulsa Retorno.",
  );
});

test("a code to type here has its field: the passkey the device shows, or a PIN with a hint", () => {
  const passkey = sheetView({ kind: "bluetooth-code", name: "Galaxy", device: "phone", code: "passkey" }, EN);
  assert.equal(passkey.title, "Enter the code shown on “Galaxy”.");
  assert.equal(passkey.body, "");
  assert.deepEqual(passkey.codeField, { label: "Code", kind: "passkey" });
  assert.equal(passkey.accept, "Pair");
  assert.equal(passkey.refuse, "Cancel");
  assert.equal(passkey.password, undefined);
  const pin = sheetView({ kind: "bluetooth-code", name: "Teclado antiguo", device: "keyboard", code: "pin" }, ES);
  assert.equal(pin.title, "Introduce el PIN de “Teclado antiguo”.");
  assert.equal(pin.body, "Si no lo sabes, prueba con 0000 o 1234.");
  assert.deepEqual(pin.codeField, { label: "PIN", kind: "pin" });
});

test("a connection that failed says why, with one button", () => {
  const rejected = sheetView({ kind: "bluetooth-failed", name: "AirPods", device: "headphones", reason: "rejected" }, EN);
  assert.equal(rejected.title, "Couldn’t connect to “AirPods”.");
  assert.equal(rejected.body, "The code didn’t match, or the device refused the connection.");
  assert.equal(rejected.accept, "OK");
  assert.equal(rejected.refuse, "");
  const timeout = sheetView({ kind: "bluetooth-failed", name: "AirPods", device: "headphones", reason: "timeout" }, EN);
  assert.equal(timeout.body, "Make sure it’s turned on, nearby and in pairing mode.");
  const failed = sheetView({ kind: "bluetooth-failed", name: "AirPods", device: "headphones", reason: "failed" }, ES);
  assert.equal(failed.title, "No se ha podido conectar a “AirPods”.");
  assert.equal(failed.body, "Asegúrate de que está encendido y cerca.");
  assert.equal(failed.accept, "Aceptar");
});

test("a passkey is one to six digits; a PIN one to 16 bytes", () => {
  assert.equal(validBluetoothCode("", "passkey"), false);
  assert.equal(validBluetoothCode("123456", "passkey"), true);
  assert.equal(validBluetoothCode("42", "passkey"), true);
  assert.equal(validBluetoothCode("1234567", "passkey"), false);
  assert.equal(validBluetoothCode("12a456", "passkey"), false);
  assert.equal(validBluetoothCode("", "pin"), false);
  assert.equal(validBluetoothCode("0000", "pin"), true);
  assert.equal(validBluetoothCode("a".repeat(16), "pin"), true);
  assert.equal(validBluetoothCode("a".repeat(17), "pin"), false);
  assert.equal(validBluetoothCode("ñ".repeat(9), "pin"), false);
  // The field keeps a passkey's digits, six at most, and a PIN's first 16 bytes.
  assert.equal(bluetoothCodeTyped("12-34 5678", "passkey"), "123456");
  assert.equal(bluetoothCodeTyped("a".repeat(20), "pin"), "a".repeat(16));
  assert.equal(bluetoothCodeTyped("ñ".repeat(9), "pin"), "ñ".repeat(8));
  // Said digit by digit.
  assert.equal(spokenCode("048213"), "0 4 8 2 1 3");
});

test("a passkey's sheet asks to use one for the relying party, by the app, with the Oxy account's password", () => {
  const get = {
    kind: "passkey" as const,
    mode: "get" as const,
    id: "so.oxy.astro",
    rpId: "github.com",
    choices: [{ userName: "ada", displayName: "Ada Lovelace" }],
    incorrect: false,
  };
  const es = sheetView(get, ES, { "so.oxy.astro": "Astro" });
  assert.equal(es.title, "¿Usar tu llave de acceso para github.com?");
  assert.equal(es.body, "Cuenta: ada. Lo pide Astro.");
  assert.equal(es.accept, "Continuar");
  assert.equal(es.refuse, "Cancelar");
  assert.deepEqual(es.password, { label: "Contraseña de la cuenta de Oxy", show: "Mostrar contraseña", hide: "Ocultar contraseña" });
  assert.equal(es.passwordKind, "account");
  assert.equal(es.choices, undefined);
  assert.equal(es.error, undefined);
  const en = sheetView(get, EN, {});
  assert.equal(en.title, "Use your passkey for github.com?");
  assert.equal(en.body, "Account: ada. Requested by so.oxy.astro.");
  assert.equal(en.accept, "Continue");
  assert.deepEqual(readSheet(JSON.stringify(get)), get);
});

test("several passkeys are a chooser; a wrong password or too many say so", () => {
  const sheet = {
    kind: "passkey" as const,
    mode: "get" as const,
    id: "so.oxy.mention",
    rpId: "mention.earth",
    choices: [
      { userName: "ada@example.com", displayName: "Ada" },
      { userName: "bea@example.com", displayName: "Bea" },
    ],
    incorrect: true,
  };
  const es = sheetView(sheet, ES, { "so.oxy.mention": "Mention" });
  assert.equal(es.body, "Lo pide Mention.");
  assert.deepEqual(es.choices, {
    label: "Elige una cuenta",
    rows: [
      { title: "ada@example.com", detail: "Ada" },
      { title: "bea@example.com", detail: "Bea" },
    ],
  });
  assert.equal(es.error, "Contraseña incorrecta. Vuelve a intentarlo.");
  const throttled = sheetView({ ...sheet, incorrect: false, retryAfter: 30 }, EN, {});
  assert.equal(throttled.error, "Too many attempts. Try again in 30 seconds.");
  assert.equal(sheetView({ ...sheet, incorrect: false, retryAfter: 1 }, ES, {}).error, "Demasiados intentos. Vuelve a intentarlo en 1 segundo.");
});

test("creating a passkey names the account and where it is kept", () => {
  const sheet = {
    kind: "passkey" as const,
    mode: "create" as const,
    id: "so.oxy.astro",
    rpId: "github.com",
    userName: "ada",
    choices: [],
    incorrect: false,
  };
  const es = sheetView(sheet, ES, { "so.oxy.astro": "Astro" });
  assert.equal(es.title, "¿Crear una llave de acceso para github.com?");
  assert.equal(
    es.body,
    "Cuenta: ada. Se guarda en el llavero de tu cuenta de Oxy, cifrada de extremo a extremo, en todos tus dispositivos. Lo pide Astro.",
  );
  assert.equal(sheetView(sheet, EN, {}).title, "Create a passkey for github.com?");
});

test("a passkey already there, or none, is only said, with one button", () => {
  const base = { kind: "passkey" as const, id: "so.oxy.astro", rpId: "github.com", choices: [], incorrect: false };
  const excluded = sheetView({ ...base, mode: "excluded" }, ES, { "so.oxy.astro": "Astro" });
  assert.equal(excluded.title, "Ya tienes una llave de acceso para github.com.");
  assert.equal(excluded.body, "No hace falta crear otra. Lo pide Astro.");
  assert.equal(excluded.accept, "Aceptar");
  assert.equal(excluded.refuse, "");
  assert.equal(excluded.password, undefined);
  const none = sheetView({ ...base, mode: "none" }, EN, {});
  assert.equal(none.title, "You don’t have a passkey for github.com.");
  assert.equal(none.accept, "OK");
});

test("the account's password is submitted once anything is typed; a Wi-Fi password once it can be one", () => {
  assert.equal(submittablePassword("", "account"), false);
  assert.equal(submittablePassword("x", "account"), true);
  assert.equal(submittablePassword("x"), false);
  assert.equal(submittablePassword("12345678", "wifi"), true);
});

// An application that quit unexpectedly, and resetting one (docs/adr/0061).
const report = "Mention quit unexpectedly.\n\nApplication:  Mention\n";

test("an application that keeps quitting is told with Reopen as its answer, Ignore, Reset… and Report…", () => {
  const text = readSheet(
    JSON.stringify({ kind: "crash", id: "so.oxy.mention", name: "Mention", times: 2, report, show: "notice" }),
  );
  assert.ok(text);
  const en = sheetView(text, EN);
  assert.equal(en.title, "Mention keeps quitting unexpectedly");
  assert.equal(en.body, "It quit 2 times in the last 10 minutes. If it keeps quitting, resetting it may help.");
  assert.equal(en.accept, "Reopen");
  assert.equal(en.refuse, "Ignore");
  assert.equal(en.destructive, false);
  assert.equal(en.focusAnswer, true);
  assert.deepEqual(en.actions, [
    { id: "reset", label: "Reset…" },
    { id: "report", label: "Report…" },
  ]);
  assert.deepEqual(en.report, {
    title: "What Mention reported",
    text: report,
    copy: "Copy",
    copied: "Copied",
    done: "Done",
    first: false,
  });
  const es = sheetView(text, ES);
  assert.equal(es.title, "Mention sigue cerrándose inesperadamente");
  assert.equal(es.body, "Se ha cerrado 2 veces en los últimos 10 minutos. Si sigue cerrándose, restablecerla puede ayudar.");
  assert.equal(es.accept, "Volver a abrir");
  assert.equal(es.refuse, "Ignorar");
  assert.deepEqual(
    es.actions?.map((action) => action.label),
    ["Restablecer…", "Informe…"],
  );
  assert.equal(es.report?.done, "OK");
});

test("a report asked from the notification opens at the report, of a quit that happened once", () => {
  const once = sheetView(
    { kind: "crash", id: "so.oxy.mention", name: "Mention", times: 1, report, show: "report" },
    EN,
  );
  assert.equal(once.title, "Mention quit unexpectedly");
  assert.equal(once.body, "You can reopen it, or see what it reported.");
  assert.equal(once.report?.first, true);
  const es = sheetView({ kind: "crash", id: "so.oxy.mention", name: "Mention", times: 1, report, show: "report" }, ES);
  assert.equal(es.title, "Mention se cerró inesperadamente");
});

test("resetting says what is erased and what is kept, destructively, and that a running app quits", () => {
  const en = sheetView({ kind: "reset", id: "so.oxy.mention", name: "Mention", running: false }, EN);
  assert.equal(en.title, "Reset “Mention”?");
  assert.equal(
    en.body,
    "Mention’s data and settings on this computer will be erased, and it will open as new. Your documents and files, and your passwords, are kept.",
  );
  assert.equal(en.accept, "Reset");
  assert.equal(en.refuse, "Cancel");
  assert.equal(en.destructive, true);
  assert.equal(en.focusAnswer, undefined, "no key resets");
  const running = sheetView({ kind: "reset", id: "so.oxy.mention", name: "Mention", running: true }, ES);
  assert.equal(running.title, "¿Restablecer “Mention”?");
  assert.match(running.body, /se conservan\. Antes se cerrará Mention\.$/);
  assert.equal(running.accept, "Restablecer");
});
