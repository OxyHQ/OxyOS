import { test } from "node:test";
import assert from "node:assert/strict";
import {
  bluetoothEmptyText,
  bluetoothSummary,
  bluetoothSwitchable,
  deviceBattery,
  deviceLabel,
  deviceStatus,
  hasBluetoothView,
  pairedDevices,
  type BluetoothAdapter,
  type BluetoothDevice,
  type BluetoothState,
} from "../src/bluetooth.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");
const EN = shellMessages("en-US");

const device = (name: string, more: Partial<BluetoothDevice> = {}): BluetoothDevice => ({
  id: `AA:BB:CC:DD:EE:${name.length.toString(16).padStart(2, "0")}`,
  address: "AA:BB:CC:DD:EE:FF",
  name,
  kind: "keyboard",
  paired: true,
  connected: false,
  busy: null,
  battery: null,
  audio: false,
  input: true,
  ...more,
});

const state = (adapter: Partial<BluetoothAdapter> | null = {}, more: Partial<BluetoothState> = {}): BluetoothState => ({
  available: true,
  adapter:
    adapter === null
      ? null
      : {
          name: "Portátil de Nate",
          address: "3C:A9:F4:00:00:01",
          powered: true,
          blocked: false,
          discovering: false,
          discoverable: false,
          ...adapter,
        },
  devices: [device("Magic Keyboard", { connected: true, battery: 80 })],
  ...more,
});

test("the view lists the paired devices only, in the session's order", () => {
  const listed = pairedDevices(
    state(
      {},
      {
        devices: [
          device("AirPods", { connected: true }),
          device("Altavoz", { paired: false }),
          device("Magic Mouse"),
        ],
      },
    ),
  );
  assert.deepEqual(
    listed.map((d) => d.name),
    ["AirPods", "Magic Mouse"],
  );
});

test("each device says its state to a screen reader, and its charge while connected", () => {
  assert.equal(deviceLabel(device("Magic Keyboard", { connected: true, battery: 80 }), EN), "Magic Keyboard, Connected, Battery 80%");
  assert.equal(deviceLabel(device("WH-1000XM4", { battery: 60 }), EN), "WH-1000XM4, Not Connected");
  assert.equal(deviceLabel(device("AirPods", { busy: "connecting" }), EN), "AirPods, Connecting");
  assert.equal(deviceLabel(device("AirPods", { connected: true, busy: "disconnecting" }), EN), "AirPods, Disconnecting");
  assert.equal(deviceLabel(device("Magic Keyboard", { connected: true, battery: 80 }), ES), "Magic Keyboard, Conectado, Batería 80 %");
  assert.equal(deviceLabel(device("Magic Mouse"), ES), "Magic Mouse, No conectado");
});

test("a device's state and charge, shown", () => {
  assert.equal(deviceStatus(device("A", { connected: true }), EN), "Connected");
  assert.equal(deviceStatus(device("A"), EN), "Not Connected");
  assert.equal(deviceStatus(device("A", { busy: "connecting" }), EN), "Connecting…");
  assert.equal(deviceStatus(device("A", { busy: "connecting" }), ES), "Conectando…");
  assert.equal(deviceBattery(device("A", { connected: true, battery: 42 }), EN), "42%");
  assert.equal(deviceBattery(device("A", { connected: true, battery: 42 }), ES), "42 %");
  // Not connected, or none reported: no charge.
  assert.equal(deviceBattery(device("A", { battery: 42 }), EN), undefined);
  assert.equal(deviceBattery(device("A", { connected: true }), EN), undefined);
});

test("the view says why it has no list: no BlueZ, the machine's switch, off, nothing paired", () => {
  assert.equal(bluetoothEmptyText(state({}, { available: false }), EN), "Bluetooth isn't available.");
  assert.equal(bluetoothEmptyText(state(null), EN), "Bluetooth isn't available.");
  assert.equal(
    bluetoothEmptyText(state({ powered: false, blocked: true }), EN),
    "Bluetooth is turned off by a hardware switch or airplane mode.",
  );
  assert.equal(bluetoothEmptyText(state({ powered: false }), EN), "Bluetooth is off.");
  assert.equal(bluetoothEmptyText(state({ powered: false }), ES), "Bluetooth está desactivado.");
  assert.equal(bluetoothEmptyText(state({}, { devices: [device("Altavoz", { paired: false })] }), EN), "No Devices");
  assert.equal(bluetoothEmptyText(state(), EN), undefined);
});

test("the switch moves only with BlueZ answering and the adapter not blocked", () => {
  assert.equal(bluetoothSwitchable(state()), true);
  assert.equal(bluetoothSwitchable(state({ powered: false })), true);
  assert.equal(bluetoothSwitchable(state({ blocked: true })), false);
  assert.equal(bluetoothSwitchable(state(null)), false);
  assert.equal(bluetoothSwitchable(state({}, { available: false })), false);
});

test("the Control Center has a Bluetooth view only with Bluetooth told and an adapter", () => {
  assert.equal(hasBluetoothView(undefined), false);
  assert.equal(hasBluetoothView(null), false);
  assert.equal(hasBluetoothView(state(null)), false);
  assert.equal(hasBluetoothView(state()), true);
  assert.equal(hasBluetoothView(state({ powered: false })), true);
});

test("the module's state from Bluetooth's own, where the session told no other", () => {
  assert.equal(bluetoothSummary(undefined), undefined);
  assert.deepEqual(bluetoothSummary(state(null)), { state: "unavailable", connected: 0 });
  assert.deepEqual(bluetoothSummary(state({ powered: false })), { state: "off", connected: 0 });
  assert.deepEqual(bluetoothSummary(state()), { state: "on", connected: 1 });
});
