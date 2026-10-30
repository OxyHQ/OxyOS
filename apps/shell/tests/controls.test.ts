import { test } from "node:test";
import assert from "node:assert/strict";
import {
  announcement,
  batteryLabel,
  bluetoothLabel,
  percent,
  segments,
  silent,
  wifiBars,
  wifiLabel,
  wifiOn,
} from "../src/controls.ts";

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
  assert.equal(announcement("output", { level: 0.5625, available: true }), "Volumen, 56 %");
  assert.equal(announcement("display", { level: 0.25, available: true }), "Brillo, 25 %");
  assert.equal(announcement("output", { level: 0.5, muted: true, available: true }), "Sonido silenciado");
  assert.equal(announcement("input", { level: 1, muted: true, available: true }), "Micrófono silenciado");
});

test("the battery in words, and none without one", () => {
  assert.equal(batteryLabel(undefined), undefined);
  assert.equal(batteryLabel({ battery: "absent", percent: 0, onAC: true, minutes: 0 }), undefined);
  assert.equal(
    batteryLabel({ battery: "charging", percent: 78, onAC: true, minutes: 181 }),
    "78 %, cargando (3 h 1 min para cargarse)",
  );
  assert.equal(batteryLabel({ battery: "discharging", percent: 64, onAC: false, minutes: 45 }), "64 %, 45 min restantes");
  assert.equal(batteryLabel({ battery: "discharging", percent: 64, onAC: false, minutes: 0 }), "64 %");
  assert.equal(batteryLabel({ battery: "full", percent: 100, onAC: true, minutes: 0 }), "100 %, cargada");
  assert.equal(batteryLabel({ battery: "not-charging", percent: 80, onAC: true, minutes: 0 }), "80 %, conectada, sin cargar");
});

test("Wi-Fi and Bluetooth in words", () => {
  const network = { wifi: "connected", signal: 73, ssid: "LukeSkyRouter", wired: false } as const;
  assert.equal(wifiLabel(network), "LukeSkyRouter");
  assert.equal(wifiLabel({ ...network, wifi: "off", ssid: "" }), "Desactivada");
  assert.equal(wifiLabel({ ...network, wifi: "unavailable" }), undefined);
  assert.equal(wifiLabel({ ...network, wifi: "connecting" }), "Conectando a LukeSkyRouter");
  assert.ok(wifiOn(network));
  assert.ok(wifiOn({ ...network, wifi: "disconnected" }));
  assert.ok(!wifiOn({ ...network, wifi: "off" }));
  assert.ok(!wifiOn(undefined));
  assert.deepEqual([wifiBars(10), wifiBars(50), wifiBars(73)], [1, 2, 3]);
  assert.equal(bluetoothLabel({ state: "on", connected: 0 }), "Activado");
  assert.equal(bluetoothLabel({ state: "on", connected: 2 }), "2 dispositivos");
  assert.equal(bluetoothLabel({ state: "off", connected: 0 }), "Desactivado");
  assert.equal(bluetoothLabel({ state: "unavailable", connected: 0 }), undefined);
});
