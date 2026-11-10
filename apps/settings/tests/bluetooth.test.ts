import { test } from "node:test";
import assert from "node:assert/strict";
import {
  batteryText,
  bluetoothView,
  connectionAction,
  connectionChanges,
  deviceLabel,
  deviceStatus,
  failureNeedsNote,
  failureOf,
  isOn,
  kindOf,
  myDevices,
  nearbyDevices,
  renameTo,
  statusLabel,
  type BluetoothAdapter,
  type BluetoothDevice,
  type BluetoothState,
} from "../src/bluetooth.ts";
import { CATALOGUES, messagesFor } from "../src/messages.ts";

const device = (name: string, extra: Partial<BluetoothDevice> = {}): BluetoothDevice => ({
  id: `00:11:22:33:44:${name.length.toString(16).padStart(2, "0").toUpperCase()}`,
  address: "00:11:22:33:44:55",
  name,
  kind: "other",
  paired: true,
  connected: false,
  busy: null,
  battery: null,
  audio: false,
  input: false,
  ...extra,
});

const adapter = (extra: Partial<BluetoothAdapter> = {}): BluetoothAdapter => ({
  name: "ThinkPad",
  address: "3C:58:C2:1A:7E:91",
  powered: true,
  blocked: false,
  discovering: false,
  discoverable: false,
  ...extra,
});

const words = messagesFor("en").bluetooth;

test("the page: unavailable without BlueZ or a state, no hardware without an adapter, else the adapter", () => {
  assert.equal(bluetoothView(undefined).kind, "unavailable");
  assert.equal(bluetoothView(null).kind, "unavailable");
  assert.equal(bluetoothView({ available: false, adapter: adapter(), devices: [] }).kind, "unavailable");
  assert.equal(bluetoothView({ available: true, adapter: null, devices: [] }).kind, "noAdapter");
  const state: BluetoothState = { available: true, adapter: adapter(), devices: [] };
  const view = bluetoothView(state);
  assert.equal(view.kind, "adapter");
  assert.equal(view.kind === "adapter" && view.adapter.name, "ThinkPad");
});

test("on: powered and not blocked by a switch", () => {
  assert.equal(isOn(adapter()), true);
  assert.equal(isOn(adapter({ powered: false })), false);
  assert.equal(isOn(adapter({ blocked: true })), false);
  assert.equal(isOn(null), false);
});

test("My Devices are the paired ones, Nearby the others, each in the session's order", () => {
  const devices = [
    device("AirPods Pro", { connected: true }),
    device("JBL Flip 6", { paired: false }),
    device("Magic Keyboard"),
    device("Pixel 8", { paired: false }),
  ];
  assert.deepEqual(
    myDevices(devices).map((d) => d.name),
    ["AirPods Pro", "Magic Keyboard"],
  );
  assert.deepEqual(
    nearbyDevices(devices).map((d) => d.name),
    ["JBL Flip 6", "Pixel 8"],
  );
});

test("a device's status: the session's call under way first, then whether it is connected", () => {
  assert.equal(deviceStatus(device("A", { connected: true })), "connected");
  assert.equal(deviceStatus(device("A")), "notConnected");
  assert.equal(deviceStatus(device("A", { busy: "connecting" })), "connecting");
  assert.equal(deviceStatus(device("A", { connected: true, busy: "disconnecting" })), "disconnecting");
  assert.equal(statusLabel(device("A", { connected: true }), words), "Connected");
  assert.equal(statusLabel(device("A"), words), "Not Connected");
  assert.equal(statusLabel(device("A", { busy: "connecting" }), words), "Connecting…");
});

test("a battery only when connected and reported, as the language writes a percentage", () => {
  assert.equal(batteryText({ battery: 80, connected: true }, "en"), "80%");
  assert.equal(batteryText({ battery: 55.4, connected: true }, "en-US"), "55%");
  assert.equal(batteryText({ battery: 80, connected: true }, "es").replace(/\s/u, " "), "80 %");
  assert.equal(batteryText({ battery: 140, connected: true }, "en"), "100%");
  assert.equal(batteryText({ battery: null, connected: true }, "en"), "");
  assert.equal(batteryText({ battery: 80, connected: false }, "en"), "");
});

test("a device said to the screen reader: name, status, battery when known", () => {
  assert.equal(
    deviceLabel(device("Magic Keyboard", { connected: true, battery: 80 }), words, "en"),
    "Magic Keyboard, Connected, Battery 80%",
  );
  assert.equal(deviceLabel(device("WH-1000XM4", { battery: 30 }), words, "en"), "WH-1000XM4, Not Connected");
  assert.equal(deviceLabel(device("MX Master 3S", { busy: "connecting" }), words, "en"), "MX Master 3S, Connecting…");
  const es = messagesFor("es").bluetooth;
  assert.match(deviceLabel(device("AirPods Pro", { connected: true, battery: 80 }), es, "es"), /^AirPods Pro, Conectado, Batería 80\s%$/u);
});

test("a row's button: connect, disconnect, or none while the session's call is under way", () => {
  assert.equal(connectionAction(device("A")), "connect");
  assert.equal(connectionAction(device("A", { connected: true })), "disconnect");
  assert.equal(connectionAction(device("A", { busy: "connecting" })), null);
  assert.equal(connectionAction(device("A", { connected: true, busy: "disconnecting" })), null);
});

test("a rename sends the trimmed name, empty to give back its own, nothing when unchanged", () => {
  const keyboard = device("Magic Keyboard");
  assert.equal(renameTo("  Teclado de Nate ", keyboard), "Teclado de Nate");
  assert.equal(renameTo("", keyboard), "");
  assert.equal(renameTo("   ", keyboard), "");
  assert.equal(renameTo("Magic Keyboard", keyboard), null);
  assert.equal(renameTo(" Magic Keyboard ", keyboard), null);
});

test("an unknown kind is drawn as a Bluetooth device", () => {
  assert.equal(kindOf("headphones"), "headphones");
  assert.equal(kindOf("trackpad"), "trackpad");
  assert.equal(kindOf("toaster"), "other");
  for (const catalogue of Object.values(CATALOGUES)) {
    assert.equal(typeof catalogue.bluetooth.kinds[kindOf("toaster")], "string");
  }
});

test("failures: known reasons kept, a refusal is not permitted, anything else failed", () => {
  // As the platform API rejects: a code, and the session's own reason when it said one.
  assert.equal(failureOf({ code: "busy", reason: "busy" }), "busy");
  assert.equal(failureOf({ code: "failed", reason: "rejected" }), "rejected");
  assert.equal(failureOf({ code: "failed", reason: "somethingNew" }), "failed");
  assert.equal(failureOf({ code: "notPermitted", reason: "notPermitted" }), "notPermitted");
  assert.equal(failureOf({ code: "needsInput", reason: "needsInput" }), "notPermitted");
  assert.equal(failureOf({ code: "cancelled" }), "cancelled");
  assert.equal(failureOf(new Error("anything")), "failed");
  assert.equal(failureOf(null), "failed");
});

test("Settings says a forget's and a rename's failure; a connection's is the session's sheet", () => {
  assert.equal(failureNeedsNote("forget", "busy"), true);
  assert.equal(failureNeedsNote("rename", "notFound"), true);
  assert.equal(failureNeedsNote("forget", "cancelled"), false);
  assert.equal(failureNeedsNote("forget", null), false);
  assert.equal(failureNeedsNote("connect", "rejected"), false);
  assert.equal(failureNeedsNote("connect", "timeout"), false);
  assert.equal(failureNeedsNote("disconnect", "failed"), false);
});

test("connection changes heard: paired devices that connected or disconnected, none at first", () => {
  const before = [
    device("AirPods Pro", { id: "A", connected: false }),
    device("Magic Keyboard", { id: "B", connected: true }),
    device("MX Master 3S", { id: "C", connected: true }),
  ];
  const after = [
    device("AirPods Pro", { id: "A", connected: true }),
    device("Magic Keyboard", { id: "B", connected: false }),
    device("MX Master 3S", { id: "C", connected: true }),
    device("Pixel 8", { id: "D", connected: true }),
    device("JBL Flip 6", { id: "E", paired: false, connected: true }),
  ];
  assert.deepEqual(connectionChanges(before, after), [
    { name: "AirPods Pro", connected: true },
    { name: "Magic Keyboard", connected: false },
  ]);
  assert.deepEqual(connectionChanges(undefined, after), []);
  assert.deepEqual(connectionChanges(after, after), []);
});

test("Bluetooth's words in every language, as a Mac says them", () => {
  assert.equal(words.discoverable("ThinkPad"), "Now discoverable as “ThinkPad”.");
  assert.equal(words.forgetTitle("AirPods Pro"), "Forget “AirPods Pro”?");
  assert.equal(words.forgetDescription, "You’ll need to pair it again to use it.");
  assert.equal(words.forget, "Forget Device");
  assert.equal(words.connectLabel("JBL Flip 6"), "Connect “JBL Flip 6”");
  assert.equal(words.disconnectLabel("AirPods Pro"), "Disconnect “AirPods Pro”");
  assert.equal(words.info("AirPods Pro"), "Details of “AirPods Pro”");
  assert.equal(messagesFor("en").pages.bluetooth, "Bluetooth");
  assert.equal(messagesFor("es").bluetooth.myDevices, "Mis dispositivos");
});
