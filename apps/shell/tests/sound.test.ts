import { test } from "node:test";
import assert from "node:assert/strict";
import { hasSoundView, selectedOutput, soundDeviceName, type SoundDevice, type SoundState } from "../src/sound.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");
const EN = shellMessages("en-US");

const device = (id: string, more: Partial<SoundDevice> = {}): SoundDevice => ({
  id,
  name: id,
  kind: "speakers",
  transport: "builtIn",
  selected: false,
  ...more,
});

const state = (devices: SoundDevice[], more: Partial<SoundState> = {}): SoundState => ({
  available: true,
  output: { devices, volume: 0.5, muted: false, balance: 0 },
  input: { devices: [], volume: null, muted: false, balance: null },
  ...more,
});

test("a built-in output is named by what it is, in the session's language", () => {
  const raw = "Built-in Audio Analog Stereo";
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "speakers" }), ES), "Altavoces integrados");
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "speakers" }), EN), "Built-in Speakers");
  // The jack, headphones or a headset in it.
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "headphones" }), ES), "Auriculares externos");
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "headset" }), EN), "External Headphones");
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "line" }), ES), "Salida de línea");
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "line" }), EN), "Line Out");
  // Anything else built in keeps the system's name.
  assert.equal(soundDeviceName(device("a", { name: raw, kind: "other" }), ES), raw);
  assert.equal(soundDeviceName(device("a", { name: "Micrófono", kind: "microphone" }), ES), "Micrófono");
});

test("any other output is named as the system names it", () => {
  assert.equal(soundDeviceName(device("d", { name: "DELL U2720Q", kind: "hdmi", transport: "hdmi" }), ES), "DELL U2720Q");
  assert.equal(
    soundDeviceName(device("b", { name: "AirPods Pro", kind: "headphones", transport: "bluetooth" }), ES),
    "AirPods Pro",
  );
  assert.equal(
    soundDeviceName(device("u", { name: "Jabra Evolve 40", kind: "headset", transport: "usb" }), EN),
    "Jabra Evolve 40",
  );
  assert.equal(soundDeviceName(device("v", { name: "Null Output", kind: "speakers", transport: "virtual" }), ES), "Null Output");
});

test("the output chosen is the one the session marks selected", () => {
  assert.equal(selectedOutput(null), undefined);
  assert.equal(selectedOutput(undefined), undefined);
  assert.equal(selectedOutput(state([device("a"), device("b")])), undefined);
  assert.equal(selectedOutput(state([device("a"), device("b", { selected: true })]))?.id, "b");
});

test("the Control Center has a Sound view only with the sound told, its server up and an output", () => {
  assert.equal(hasSoundView(undefined), false);
  assert.equal(hasSoundView(null), false);
  assert.equal(hasSoundView(state([device("a")], { available: false })), false);
  assert.equal(hasSoundView(state([])), false);
  assert.equal(hasSoundView(state([device("a", { selected: true })])), true);
});
