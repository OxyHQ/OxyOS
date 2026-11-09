import { test } from "node:test";
import assert from "node:assert/strict";
import {
  currentNetwork,
  detailRows,
  diagnosticRows,
  ethernetStatus,
  failureLabel,
  failureOf,
  formatMs,
  formatSpeed,
  isJoined,
  isLocked,
  joinFailureNeedsNote,
  knownByName,
  otherNetworks,
  PRIVATE_ADDRESS_CHOICES,
  privateAddressFailure,
  privateAddressOutcome,
  privateChoice,
  resetLines,
  resetFailureOf,
  resetPlanOf,
  type Diagnostics,
  type EthernetState,
  type KnownNetwork,
  type NearbyNetwork,
  type ResetPlan,
  type WifiState,
} from "../src/network.ts";
import { messagesFor } from "../src/messages.ts";

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

test("a lock for every network that asks a password, none for open and OWE ones", () => {
  assert.equal(isLocked("open"), false);
  assert.equal(isLocked("owe"), false);
  for (const security of ["wpa", "wpa3", "wep", "enterprise"] as const) assert.equal(isLocked(security), true);
});

test("failures: the session's reasons, an unknown one as a failure, a refusal as not permitted", () => {
  // As the platform API rejects: a code, and the session's own reason when it said one.
  assert.equal(failureOf({ code: "failed", reason: "wrongPassword" }), "wrongPassword");
  assert.equal(failureOf({ code: "notPermitted", reason: "notPermitted" }), "notPermitted");
  assert.equal(failureOf({ code: "failed", reason: "somethingNew" }), "failed");
  assert.equal(failureOf({ code: "needsInput", reason: "needsInput" }), "notPermitted");
  assert.equal(failureOf({ code: "cancelled" }), "cancelled");
  assert.equal(failureOf({ code: "unavailable", reason: "notSent" }), "unavailable");
  assert.equal(failureOf(new Error("anything")), "failed");
  assert.equal(failureOf(null), "failed");
});

test("a join's failure is said by Settings only where the session's sheet says nothing", () => {
  for (const reason of ["notFound", "unavailable"] as const) assert.equal(joinFailureNeedsNote(reason), true);
  for (const reason of ["wrongPassword", "cancelled", "notPermitted", "unsupported", "timeout", "failed", null] as const)
    assert.equal(joinFailureNeedsNote(reason), false);
});

test("a failure's words, in English and Spanish", () => {
  const en = messagesFor("en").wifi.failures;
  const es = messagesFor("es-ES").wifi.failures;
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

const saved = (ssid: string, privateAddress: KnownNetwork["privateAddress"] = "fixed"): KnownNetwork => ({
  id: `id-${ssid}`,
  ssid,
  autoJoin: true,
  privateAddress,
});

test("a known network's sheet: its connection by name, whether it is the one joined, the choice shown", () => {
  const joined = nearby("Home", 73, { known: true, current: true });
  const state = { ...wifi([joined, nearby("Café", 40, { known: true })], joined), known: [saved("Home"), saved("Café", "other")] };
  assert.equal(knownByName(state, "Home")?.id, "id-Home");
  assert.equal(knownByName(state, "Nowhere"), null);
  assert.equal(isJoined(state, saved("Home")), true);
  assert.equal(isJoined(state, saved("Café")), false);
  assert.equal(isJoined({ ...state, state: "connecting" }, saved("Home")), false, "joined once connected");
  // A Mac's three, in its order; one set elsewhere shows none of them chosen.
  assert.deepEqual([...PRIVATE_ADDRESS_CHOICES], ["off", "fixed", "rotating"]);
  assert.equal(privateChoice(saved("Home", "rotating")), "rotating");
  assert.equal(privateChoice(saved("Café", "other")), null);
});

test("a private address's answer: set, its rejoin failed, or why it was not set", () => {
  assert.deepEqual(privateAddressOutcome({ privateAddress: { mode: "rotating", rejoined: true } }), {
    failure: null,
    rejoinFailed: false,
  });
  assert.deepEqual(
    privateAddressOutcome({ privateAddress: { mode: "fixed", rejoined: false, rejoinFailed: "timeout" } }),
    { failure: null, rejoinFailed: true },
  );
  assert.deepEqual(privateAddressOutcome(null), { failure: "failed", rejoinFailed: false });
  // Not set: the call rejected with the platform's error.
  assert.deepEqual(privateAddressFailure({ code: "notPermitted", reason: "notPermitted" }), {
    failure: "notPermitted",
    rejoinFailed: false,
  });
  assert.deepEqual(privateAddressFailure({ code: "needsInput", reason: "needsInput" }), {
    failure: "notPermitted",
    rejoinFailed: false,
  });
});

const plan: ResetPlan = {
  removed: [
    { id: "1", name: "LukeSkyRouter", kind: "wifi" },
    { id: "2", name: "Vecinos", kind: "wifi" },
    { id: "3", name: "Work VPN", kind: "vpn" },
  ],
  kept: [
    { id: "4", name: "Office", kind: "wifi", because: "notPermitted" },
    { id: "5", name: "Office static", kind: "ethernet", because: "ethernet" },
  ],
};

test("a reset's confirmation lists what goes and what stays, and why", () => {
  const list = (names: string[]) => names.join(", ");
  assert.deepEqual(resetLines(plan, messagesFor("en").network, list), [
    "These are removed, with their passwords:",
    "Wi-Fi: LukeSkyRouter, Vecinos",
    "VPN: Work VPN",
    "Kept, as only an administrator can remove them: Office.",
    "Kept: Office static, Ethernet's configuration, which NetworkManager would not set up again by itself.",
  ]);
  assert.deepEqual(resetLines({ removed: [], kept: [] }, messagesFor("es").network, list), [
    "No hay redes guardadas que eliminar.",
  ]);
  assert.deepEqual(resetLines({ removed: [plan.removed[2]], kept: [] }, messagesFor("es").network, list), [
    "Se eliminan, con sus contraseñas:",
    "VPN: Work VPN",
  ]);
});

test("a reset's answer: its plan before and after, or why there is none", () => {
  assert.deepEqual(resetPlanOf({ networkResetPreview: plan }), { plan });
  assert.deepEqual(resetPlanOf({ networkReset: plan }), { plan });
  assert.deepEqual(resetPlanOf(null), { failure: "failed" });
  // Its rejection, the platform's error: the session's word where the page says it.
  assert.deepEqual(resetFailureOf({ code: "busy", reason: "busy" }), { failure: "busy" });
  assert.deepEqual(resetFailureOf({ code: "failed", reason: "somethingNew" }), { failure: "failed" });
  assert.deepEqual(resetFailureOf({ code: "needsInput", reason: "needsInput" }), { failure: "notPermitted" });
  assert.deepEqual(resetFailureOf(undefined), { failure: "failed" });
});

const found: Diagnostics = {
  link: {
    ssid: "LukeSkyRouter",
    interface: "wlp0s20f3",
    bssid: "A0:B1:C2:D3:E4:F5",
    signal: 73,
    quality: "good",
    frequency: 5180,
    band: "5",
    channel: 36,
    width: 80,
    txRate: 866,
    maxRate: 1201,
    security: "wpa",
    address: "1A:2B:3C:4D:5E:6F",
    permanentAddress: "F4:A4:75:12:34:56",
    privateAddress: "fixed",
  },
  connectivity: { checked: false, state: "unknown" },
  router: { address: "192.168.8.1", verdict: "reached", sent: 3, answered: 3, ms: 1.2 },
  dns: { servers: ["192.168.8.1"], name: "oxy.so", verdict: "resolved", ms: 14, addresses: 2 },
};

test("diagnostics: a row a check, its verdict and what was measured, nothing else", () => {
  const en = messagesFor("en");
  assert.deepEqual(diagnosticRows(found, en.wifi.diagnostics, "en"), [
    {
      key: "connection",
      title: "LukeSkyRouter",
      value: "Good · 73%",
      detail: "Channel 36, 5 GHz, 80 MHz · 866 Mb/s · WPA2 Personal\nAccess point A0:B1:C2:D3:E4:F5 · Wi-Fi address 1A:2B:3C:4D:5E:6F",
    },
    { key: "router", title: "Router 192.168.8.1", value: "Answers", detail: "3 of 3 answered in 1.2 ms" },
    { key: "internet", title: "Internet", value: "Not checked", detail: "No connectivity check is set up on this computer." },
    { key: "dns", title: "DNS", value: "Resolved", detail: "oxy.so in 14 ms · Servers 192.168.8.1" },
  ]);
  // Not joined, nothing measured: no connection, no counts, no times.
  assert.deepEqual(
    diagnosticRows(
      {
        link: null,
        connectivity: { checked: true, state: "portal" },
        router: { address: null, verdict: "notChecked", sent: 0, answered: 0, ms: null },
        dns: { servers: [], name: "oxy.so", verdict: "notChecked", ms: null, addresses: 0 },
      },
      en.wifi.diagnostics,
      "en",
    ).map((row) => [row.title, row.value, row.detail]),
    [
      ["Router", "Not checked", ""],
      ["Internet", "Sign-in required", "A sign-in page blocks the internet."],
      ["DNS", "Not checked", "No servers"],
    ],
  );
  // There but silent: none answered, no time; said why.
  const silent = diagnosticRows(
    { ...found, router: { address: "192.168.8.1", verdict: "present", sent: 3, answered: 0, ms: null } },
    en.wifi.diagnostics,
    "en",
  );
  assert.equal(silent[1].detail, "0 of 3 answered · It answers for its address, but not to pings.");
  const es = messagesFor("es");
  const spanish = diagnosticRows(
    { ...found, link: { ...found.link!, band: "2.4", channel: 6, width: null, txRate: null } },
    es.wifi.diagnostics,
    "es",
  );
  assert.equal(spanish[0].detail.split("\n")[0], "Canal 6, 2,4 GHz · WPA2 Personal", "a rate the daemon did not say is not shown");
  assert.equal(spanish[1].detail, "3 de 3 respondidos en 1,2 ms");
  assert.equal(spanish[3].detail, "oxy.so en 14 ms · Servidores 192.168.8.1");
});

test("a time measured", () => {
  assert.equal(formatMs(null, "en"), "");
  assert.equal(formatMs(1.2, "en"), "1.2 ms");
  assert.equal(formatMs(1.2, "es"), "1,2 ms");
  assert.equal(formatMs(14, "en"), "14 ms");
  assert.equal(formatMs(1234, "en"), "1,234 ms");
});
