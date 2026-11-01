import { test } from "node:test";
import assert from "node:assert/strict";
import {
  hasWifiView,
  isProtected,
  networkJoining,
  networkLabel,
  signalArcs,
  wifiEmptyText,
  wifiSections,
  type NetworkState,
  type WifiNetwork,
} from "../src/network.ts";

const network = (ssid: string, signal: number, more: Partial<WifiNetwork> = {}): WifiNetwork => ({
  ssid,
  signal,
  security: "wpa",
  known: false,
  current: false,
  supported: true,
  ...more,
});

const state = (wifi: Partial<NetworkState["wifi"]> = {}, more: Partial<NetworkState> = {}): NetworkState => ({
  available: true,
  wifi: {
    device: true,
    enabled: true,
    hardwareBlocked: false,
    state: "connected",
    current: null,
    networks: [network("Casa", 70)],
    known: [],
    details: null,
    ...wifi,
  },
  ethernet: { device: false, state: "unavailable", speed: 0, details: null },
  joining: null,
  ...more,
});

test("the known networks nearby come first, the joined one before them, then the others by signal", () => {
  const { known, other } = wifiSections([
    network("Vecinos", 90),
    network("Casa", 40, { known: true }),
    network("Oxy", 30, { known: true, current: true }),
    network("Café", 60, { security: "open" }),
    network("Biblioteca", 80, { known: true }),
    network("Oficina", 60, { security: "enterprise", supported: false }),
  ]);
  assert.deepEqual(
    known.map((n) => n.ssid),
    ["Oxy", "Biblioteca", "Casa"],
  );
  // Equal signals by name, so the list does not jump between scans.
  assert.deepEqual(
    other.map((n) => n.ssid),
    ["Vecinos", "Café", "Oficina"],
  );
});

test("a signal fills none to three arcs", () => {
  assert.equal(signalArcs(0), 0);
  assert.equal(signalArcs(9), 0);
  assert.equal(signalArcs(10), 1);
  assert.equal(signalArcs(33), 1);
  assert.equal(signalArcs(34), 2);
  assert.equal(signalArcs(66), 2);
  assert.equal(signalArcs(67), 3);
  assert.equal(signalArcs(100), 3);
});

test("a lock shows on every network that asks for something: not open, not OWE", () => {
  assert.equal(isProtected({ security: "open" }), false);
  assert.equal(isProtected({ security: "owe" }), false);
  for (const security of ["wpa", "wpa3", "wep", "enterprise"] as const) assert.equal(isProtected({ security }), true);
});

test("each row says what it is to a screen reader", () => {
  const joined = network("Oxy", 80, { current: true, known: true });
  assert.equal(networkLabel(joined, state()), "Oxy, conectada, protegida, señal 3 de 3");
  assert.equal(networkLabel(network("Café", 20, { security: "open" }), state()), "Café, señal 1 de 3");
  const joining = state({}, { joining: "Casa" });
  assert.equal(networkLabel(network("Casa", 50), joining), "Casa, conectando, protegida, señal 2 de 3");
  assert.equal(
    networkLabel(network("Oficina", 50, { security: "enterprise", supported: false }), joining),
    "Oficina, protegida, no compatible, señal 2 de 3",
  );
  // The daemon connecting to the network it marks current: being joined, not joined yet.
  assert.equal(networkLabel(joined, state({ state: "connecting" })), "Oxy, conectando, protegida, señal 3 de 3");
});

test("a network is being joined while the session joins it or the daemon connects to it", () => {
  assert.equal(networkJoining(network("Casa", 50), state({}, { joining: "Casa" })), true);
  assert.equal(networkJoining(network("Casa", 50), state()), false);
  assert.equal(networkJoining(network("Oxy", 50, { current: true }), state({ state: "connected" })), false);
  assert.equal(networkJoining(network("Oxy", 50, { current: true }), state({ state: "connecting" })), true);
});

test("the view says why it has no list: no daemon, the switch, off, nothing found yet", () => {
  assert.equal(wifiEmptyText(state({}, { available: false })), "El servicio de red no responde.");
  assert.equal(
    wifiEmptyText(state({ enabled: false, hardwareBlocked: true })),
    "Wi-Fi está desactivada con el interruptor del ordenador.",
  );
  assert.equal(wifiEmptyText(state({ enabled: false, state: "off" })), "Wi-Fi desactivada");
  assert.equal(wifiEmptyText(state({ networks: [] })), "Buscando redes…");
  assert.equal(wifiEmptyText(state()), undefined);
});

test("the Control Center has a Wi-Fi view only with the networks told and a Wi-Fi device", () => {
  assert.equal(hasWifiView(undefined), false);
  assert.equal(hasWifiView(null), false);
  assert.equal(hasWifiView(state({ device: false })), false);
  assert.equal(hasWifiView(state()), true);
});
