import { test } from "node:test";
import assert from "node:assert/strict";
import {
  announcement,
  batteryLabel,
  bluetoothLabel,
  percent,
  segments,
  silent,
  wifiLabel,
  wifiOn,
} from "../src/controls.ts";
import { barClock, clockSaid } from "../src/clock.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");

test("a level fills the bezel's sixteen segments, none when muted", () => {
  assert.equal(segments({ level: 0.5625, available: true }), 9);
  assert.equal(segments({ level: 1, available: true }), 16);
  assert.equal(segments({ level: 0, available: true }), 0);
  assert.equal(segments({ level: 0.75, muted: true, available: true }), 0);
  assert.equal(percent(0.5625), 56);
  assert.equal(percent(1.5), 100);
  assert.ok(silent({ level: 0, available: true }));
  assert.ok(silent({ level: 0.5, muted: true, available: true }));
  assert.ok(!silent({ level: 0.1, available: true }));
});

test("the bezel says the level, or that it is muted", () => {
  assert.equal(announcement("output", { level: 0.5625, available: true }, ES), "Volumen, 56 %");
  assert.equal(announcement("display", { level: 0.25, available: true }, ES), "Brillo, 25 %");
  assert.equal(announcement("output", { level: 0.5, muted: true, available: true }, ES), "Sonido silenciado");
  assert.equal(announcement("input", { level: 1, muted: true, available: true }, ES), "Micrófono silenciado");
});

test("the battery in words, and none without one", () => {
  assert.equal(batteryLabel(undefined, ES), undefined);
  assert.equal(batteryLabel({ battery: "absent", percent: 0, onAC: true, minutes: 0 }, ES), undefined);
  assert.equal(
    batteryLabel({ battery: "charging", percent: 78, onAC: true, minutes: 181 }, ES),
    "78 %, cargando (3 h 1 min para cargarse)",
  );
  assert.equal(batteryLabel({ battery: "discharging", percent: 64, onAC: false, minutes: 45 }, ES), "64 %, 45 min restantes");
  assert.equal(batteryLabel({ battery: "discharging", percent: 64, onAC: false, minutes: 0 }, ES), "64 %");
  assert.equal(batteryLabel({ battery: "full", percent: 100, onAC: true, minutes: 0 }, ES), "100 %, cargada");
  assert.equal(batteryLabel({ battery: "not-charging", percent: 80, onAC: true, minutes: 0 }, ES), "80 %, conectada, sin cargar");
});

test("Wi-Fi and Bluetooth in words", () => {
  const network = { wifi: "connected", signal: 73, ssid: "LukeSkyRouter", wired: false } as const;
  assert.equal(wifiLabel(network, ES), "LukeSkyRouter");
  assert.equal(wifiLabel({ ...network, wifi: "off", ssid: "" }, ES), "Desactivada");
  assert.equal(wifiLabel({ ...network, wifi: "unavailable" }, ES), undefined);
  assert.equal(wifiLabel({ ...network, wifi: "connecting" }, ES), "Conectando a LukeSkyRouter");
  assert.ok(wifiOn(network));
  assert.ok(wifiOn({ ...network, wifi: "disconnected" }));
  assert.ok(!wifiOn({ ...network, wifi: "off" }));
  assert.ok(!wifiOn(undefined));
  assert.equal(bluetoothLabel({ state: "on", connected: 0 }, ES), "Activado");
  assert.equal(bluetoothLabel({ state: "on", connected: 2 }, ES), "2 dispositivos");
  assert.equal(bluetoothLabel({ state: "off", connected: 0 }, ES), "Desactivado");
  assert.equal(bluetoothLabel({ state: "unavailable", connected: 0 }, ES), undefined);
});

test("the clock is said by the date and time it shows, in whole words, in the session's language", () => {
  const now = new Date(2026, 10, 3, 14, 5);
  assert.equal(barClock(now, "es-ES"), `${now.toLocaleDateString("es", { weekday: "short", day: "numeric", month: "short" })} 14:05`);
  assert.equal(clockSaid(now, "es-ES"), "martes, 3 de noviembre, 14:05");
  // The locale's own short time, as the bar shows it (CLDR 48 pads Spanish's hour, an older CLDR
  // does not; the shell does neither itself).
  const morning = new Date(2026, 0, 12, 9, 30);
  const short = new Intl.DateTimeFormat("es-ES", { timeStyle: "short" }).format(morning);
  assert.equal(clockSaid(morning, "es-ES"), `lunes, 12 de enero, ${short}`);
  assert.equal(clockSaid(morning, "en-GB"), "Monday 12 January, 09:30");
  assert.match(clockSaid(now, "en-US"), /^Tuesday, November 3, 2:05[\s\u202f]PM$/);
});
