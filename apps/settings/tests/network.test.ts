import { test } from "node:test";
import assert from "node:assert/strict";
import {
  currentNetwork,
  detailRows,
  ethernetStatus,
  failureLabel,
  failureOf,
  formatSpeed,
  isLocked,
  joinFailureNeedsNote,
  otherNetworks,
  signalBars,
  type EthernetState,
  type NearbyNetwork,
  type WifiState,
} from "../src/network.ts";
import { stringsFor } from "../src/strings.ts";

const nearby = (ssid: string, signal: number, extra: Partial<NearbyNetwork> = {}): NearbyNetwork => ({
  ssid,
  signal,
  security: "wpa",
  known: false,
  current: false,
  supported: true,
  ...extra,
});

const wifi = (networks: NearbyNetwork[], current: NearbyNetwork | null): WifiState => ({
  device: true,
  enabled: true,
  hardwareBlocked: false,
  state: current ? "connected" : "disconnected",
  current,
  networks,
  known: [],
  details: null,
});

test("other networks: the joined and the known left out (Known Networks has them), each by signal", () => {
  const networks = [
    nearby("LukeSkyRouter", 73, { known: true, current: true }),
    nearby("Vecinos", 90),
    nearby("Oficina", 40, { known: true }),
    nearby("Café Libre", 55, { security: "open" }),
    nearby("Casa", 62, { known: true }),
    nearby("Bar", 55),
  ];
  assert.deepEqual(
    otherNetworks(networks).map((n) => n.ssid),
    ["Vecinos", "Bar", "Café Libre"],
  );
  // The list given is not reordered.
  assert.equal(networks[0].ssid, "LukeSkyRouter");
});

test("the network joined: marked current among those nearby, else the state's", () => {
  const joined = nearby("LukeSkyRouter", 73, { current: true });
  assert.equal(currentNetwork(wifi([nearby("Bar", 20), joined], joined)), joined);
  const told = nearby("LukeSkyRouter", 73, { current: true });
  assert.equal(currentNetwork(wifi([], told)), told);
  assert.equal(currentNetwork(wifi([nearby("Bar", 20)], null)), null);
});

test("signal in three arcs, as the top bar draws it", () => {
  assert.equal(signalBars(0), 0);
  assert.equal(signalBars(1), 1);
  assert.equal(signalBars(33), 1);
  assert.equal(signalBars(34), 2);
  assert.equal(signalBars(66), 2);
  assert.equal(signalBars(67), 3);
  assert.equal(signalBars(73), 3);
  assert.equal(signalBars(100), 3);
});

test("a lock for every network that asks a password, none for open and OWE ones", () => {
  assert.equal(isLocked("open"), false);
  assert.equal(isLocked("owe"), false);
  for (const security of ["wpa", "wpa3", "wep", "enterprise"] as const) assert.equal(isLocked(security), true);
});

test("failures: the session's reasons, an unknown one as a failure, a refusal as not permitted", () => {
  assert.equal(failureOf({ joined: { ssid: "Casa" } }), null);
  assert.equal(failureOf({ forgotten: {} }), null);
  assert.equal(failureOf({ failed: { reason: "wrongPassword" } }), "wrongPassword");
  assert.equal(failureOf({ failed: { reason: "notPermitted" } }), "notPermitted");
  assert.equal(failureOf({ failed: { reason: "somethingNew" } }), "failed");
  assert.equal(failureOf({ refused: { reason: "network" } }), "notPermitted");
  assert.equal(failureOf(null), "failed");
});

test("a join's failure is said by Settings only where the session's sheet says nothing", () => {
  for (const reason of ["notFound", "unavailable"] as const) assert.equal(joinFailureNeedsNote(reason), true);
  for (const reason of ["wrongPassword", "cancelled", "notPermitted", "unsupported", "timeout", "failed", null] as const)
    assert.equal(joinFailureNeedsNote(reason), false);
});

test("a failure's words, in English and Spanish", () => {
  const en = stringsFor("en").wifi.failures;
  const es = stringsFor("es-ES").wifi.failures;
  assert.equal(failureLabel("notPermitted", en, "forget"), "Only an administrator can forget this network.");
  assert.equal(failureLabel("notPermitted", es, "forget"), "Solo un administrador puede olvidar esta red.");
  assert.equal(failureLabel("notPermitted", en, "join"), "Only an administrator can join this network.");
  assert.equal(failureLabel("notFound", en, "join"), "The network is no longer nearby.");
});

test("details: only those present, in a Mac's order", () => {
  assert.deepEqual(detailRows(null), []);
  assert.deepEqual(
    detailRows({
      interface: "wlp0s20f3",
      hardwareAddress: "a0:b1:c2:d3:e4:f5",
      ipv4: { address: "192.168.1.34", prefix: 24, subnetMask: "255.255.255.0" },
      router: "192.168.1.1",
      dns: ["192.168.1.1", "1.1.1.1"],
      ipv6: [],
    }),
    [
      { key: "address", value: "192.168.1.34" },
      { key: "subnetMask", value: "255.255.255.0" },
      { key: "router", value: "192.168.1.1" },
      { key: "dns", value: "192.168.1.1, 1.1.1.1" },
      { key: "hardwareAddress", value: "a0:b1:c2:d3:e4:f5" },
    ],
  );
  assert.deepEqual(
    detailRows({ interface: "enp0s31f6", hardwareAddress: "", ipv4: null, router: "", dns: [], ipv6: ["fe80::1"] }),
    [{ key: "ipv6", value: "fe80::1" }],
  );
});

test("Ethernet's status: no port, connected, unplugged, connecting, not connected", () => {
  const ethernet = (device: boolean, state: EthernetState["state"]): EthernetState => ({
    device,
    state,
    speed: 0,
    details: null,
  });
  assert.equal(ethernetStatus(ethernet(false, "unavailable")), "none");
  assert.equal(ethernetStatus(ethernet(true, "connected")), "connected");
  assert.equal(ethernetStatus(ethernet(true, "unplugged")), "unplugged");
  assert.equal(ethernetStatus(ethernet(true, "connecting")), "connecting");
  assert.equal(ethernetStatus(ethernet(true, "disconnected")), "notConnected");
  assert.equal(ethernetStatus(ethernet(true, "unavailable")), "notConnected");
});

test("a link's speed", () => {
  assert.equal(formatSpeed(0, "en"), "");
  assert.equal(formatSpeed(100, "en"), "100 Mb/s");
  assert.equal(formatSpeed(1000, "en"), "1000 Mb/s");
  assert.equal(formatSpeed(2500, "en"), "2.5 Gb/s");
  assert.equal(formatSpeed(2500, "es"), "2,5 Gb/s");
  assert.equal(formatSpeed(10000, "en"), "10 Gb/s");
});
