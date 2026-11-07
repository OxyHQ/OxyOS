import { test } from "node:test";
import assert from "node:assert/strict";
import {
  balanceLean,
  deviceName,
  litSegments,
  METER_SEGMENTS,
  percentOf,
  typeLabel,
  VOLUME_KEY_STEP,
  type SoundDevice,
  type SoundKind,
  type SoundTransport,
} from "../src/sound.ts";
import { CATALOGUES, messagesFor } from "../src/messages.ts";

const device = (kind: SoundKind, transport: SoundTransport, name = "ALC257 Analog"): SoundDevice => ({
  id: `${kind}-${transport}`,
  name,
  kind,
  transport,
  selected: false,
});

const es = messagesFor("es").sound;
const en = messagesFor("en").sound;

test("the machine's own outputs are named by their kind, in the person's language", () => {
  assert.equal(deviceName(device("speakers", "builtIn"), "output", es), "Altavoces integrados");
  assert.equal(deviceName(device("headphones", "builtIn"), "output", es), "Auriculares externos");
  // A headset in the jack plays through its headphones.
  assert.equal(deviceName(device("headset", "builtIn"), "output", es), "Auriculares externos");
  assert.equal(deviceName(device("line", "builtIn"), "output", es), "Salida de línea");
  assert.equal(deviceName(device("speakers", "builtIn"), "output", en), "Built-in Speakers");
  assert.equal(deviceName(device("headphones", "builtIn"), "output", en), "External Headphones");
  assert.equal(deviceName(device("line", "builtIn"), "output", en), "Line Out");
  // A kind it has no word for keeps the session's name.
  assert.equal(deviceName(device("other", "builtIn", "Analog Surround"), "output", es), "Analog Surround");
  assert.equal(deviceName(device("microphone", "builtIn", "Mic"), "output", es), "Mic");
});

test("the machine's own inputs are named by their kind", () => {
  assert.equal(deviceName(device("microphone", "builtIn"), "input", es), "Micrófono integrado");
  assert.equal(deviceName(device("headset", "builtIn"), "input", es), "Micrófono externo");
  assert.equal(deviceName(device("line", "builtIn"), "input", es), "Entrada de línea");
  assert.equal(deviceName(device("microphone", "builtIn"), "input", en), "Built-in Microphone");
  assert.equal(deviceName(device("headset", "builtIn"), "input", en), "External Microphone");
  assert.equal(deviceName(device("line", "builtIn"), "input", en), "Line In");
  assert.equal(deviceName(device("speakers", "builtIn", "Monitor"), "input", en), "Monitor");
});

test("every other device keeps its own name, whatever its kind", () => {
  assert.equal(deviceName(device("hdmi", "hdmi", "DELL U2720Q"), "output", es), "DELL U2720Q");
  assert.equal(deviceName(device("headphones", "bluetooth", "AirPods Pro"), "output", es), "AirPods Pro");
  assert.equal(deviceName(device("speakers", "usb", "Jabra Speak 510"), "output", en), "Jabra Speak 510");
  assert.equal(deviceName(device("microphone", "usb", "Blue Yeti"), "input", es), "Blue Yeti");
  assert.equal(deviceName(device("other", "virtual", "Null Output"), "output", en), "Null Output");
});

test("a device's type, as a Mac's Type column", () => {
  const types: SoundTransport[] = ["builtIn", "hdmi", "usb", "bluetooth", "virtual", "other"];
  assert.deepEqual(
    types.map((t) => typeLabel(t, es)),
    ["Integrado", "HDMI/DisplayPort", "USB", "Bluetooth", "Virtual", "Otro"],
  );
  assert.deepEqual(
    types.map((t) => typeLabel(t, en)),
    ["Built-in", "HDMI/DisplayPort", "USB", "Bluetooth", "Virtual", "Other"],
  );
  // A transport the session adds later reads as other.
  assert.equal(typeLabel("thunderbolt" as SoundTransport, es), "Otro");
});

test("a device is said by its name and its type", () => {
  assert.equal(es.device("DELL U2720Q", typeLabel("hdmi", es)), "DELL U2720Q, HDMI/DisplayPort");
  assert.equal(en.device("Built-in Speakers", typeLabel("builtIn", en)), "Built-in Speakers, Built-in");
});

test("the input level lights sixteen segments, rounded and clamped", () => {
  assert.equal(METER_SEGMENTS, 16);
  assert.equal(litSegments(0), 0);
  assert.equal(litSegments(1), 16);
  assert.equal(litSegments(0.5), 8);
  assert.equal(litSegments(0.03), 0);
  assert.equal(litSegments(0.04), 1);
  assert.equal(litSegments(1.5), 16);
  assert.equal(litSegments(-0.2), 0);
  assert.equal(litSegments(Number.NaN), 0);
});

test("a level in whole percent; the keys step a volume a sixteenth", () => {
  assert.equal(percentOf(0.5), 50);
  assert.equal(percentOf(0.123), 12);
  assert.equal(percentOf(2), 100);
  assert.equal(percentOf(-1), 0);
  assert.equal(VOLUME_KEY_STEP, 1 / 16);
  assert.equal(es.percent(50), "50 %");
  assert.equal(en.percent(50), "50%");
});

test("the balance says the side it leans to, or centred", () => {
  assert.deepEqual(balanceLean(0), { side: "center", percent: 0 });
  assert.deepEqual(balanceLean(0.004), { side: "center", percent: 0 });
  assert.deepEqual(balanceLean(-0.5), { side: "left", percent: 50 });
  assert.deepEqual(balanceLean(1), { side: "right", percent: 100 });
  assert.equal(es.leaning("left", es.percent(50)), "Izquierda 50 %");
  assert.equal(en.leaning("right", en.percent(25)), "Right 25%");
});

test("the sound's words are in both languages", () => {
  const keys = (o: object) => Object.keys(o).sort();
  assert.deepEqual(keys(CATALOGUES.es.sound), keys(CATALOGUES.en.sound));
  assert.deepEqual(keys(CATALOGUES.es.sound.builtIn), keys(CATALOGUES.en.sound.builtIn));
  assert.equal(es.output, "Salida");
  assert.equal(es.input, "Entrada");
  assert.equal(es.unavailable, "OxyOS no puede acceder al sonido.");
  assert.equal(en.unavailable, "OxyOS can't reach the sound.");
});
