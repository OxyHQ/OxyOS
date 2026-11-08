/**
 * Wi-Fi and networks (docs/adr/0038-networks.mdx): the network's state as the session tells it to
 * Settings (the `network` grant), and what Settings may ask. The session alone speaks to
 * NetworkManager; Settings never sees or asks for a password (the session asks it in its own
 * system sheet) and keeps no list of its own.
 */

/** How a network is secured, as the session names it. */
export type Security = "open" | "owe" | "wpa" | "wpa3" | "wep" | "enterprise";

/** A network nearby: one per name, its strongest access point. */
export type NearbyNetwork = {
  ssid: string;
  /** 0 to 100. */
  signal: number;
  security: Security;
  /** A saved connection has its name. */
  known: boolean;
  /** It is the network joined. */
  current: boolean;
  /** The session can join it (enterprise and WEP networks it cannot, yet). */
  supported: boolean;
};

/**
 * The address the computer joins a known network with (docs/adr/0038-networks.mdx, 8 November
 * 2026), as a Mac's Private Wi-Fi Address: fixed (one of NetworkManager's making for this network,
 * what a new network gets), rotating (another after every start of the computer), off (the
 * hardware's own), or a hardware address someone set elsewhere.
 */
export type PrivateAddress = "fixed" | "rotating" | "off" | "other";

/** The choices a person makes, in a Mac's order. */
export const PRIVATE_ADDRESS_CHOICES = ["off", "fixed", "rotating"] as const;
export type PrivateAddressChoice = (typeof PRIVATE_ADDRESS_CHOICES)[number];

/** A saved Wi-Fi connection: `id` is the connection's UUID. */
export type KnownNetwork = { id: string; ssid: string; autoJoin: boolean; privateAddress: PrivateAddress };

/** A connection's addresses, as the daemon configured them. */
export type NetworkDetails = {
  interface: string;
  hardwareAddress: string;
  ipv4: { address: string; prefix: number; subnetMask: string } | null;
  router: string;
  dns: string[];
  ipv6: string[];
};

export type WifiState = {
  /** A Wi-Fi device exists. */
  device: boolean;
  enabled: boolean;
  /** Its hardware switch turns it off. */
  hardwareBlocked: boolean;
  state: "unavailable" | "off" | "disconnected" | "connecting" | "connected";
  /** The network joined (or being joined by the daemon), or null. */
  current: NearbyNetwork | null;
  /** Nearby, strongest first. */
  networks: NearbyNetwork[];
  known: KnownNetwork[];
  /** Of the network joined. */
  details: NetworkDetails | null;
};

export type EthernetState = {
  device: boolean;
  state: "unavailable" | "unplugged" | "disconnected" | "connecting" | "connected";
  /** Mb/s; 0 unknown. */
  speed: number;
  details: NetworkDetails | null;
};

/** The network's state (ADR 0038, decision 3). */
export type NetworkState = {
  /** Whether the daemon answers. */
  available: boolean;
  wifi: WifiState;
  ethernet: EthernetState;
  /** The name being joined, or null. */
  joining: string | null;
};

/** Why a request failed. */
export type FailureReason =
  | "wrongPassword"
  | "cancelled"
  | "unsupported"
  | "notFound"
  | "notPermitted"
  | "unavailable"
  | "timeout"
  | "failed";

/** The session's answer to a join, a forget or a private address (or its refusal, without the grant). */
export type NetworkAnswer =
  | { joined: { ssid: string } }
  | { forgotten: Record<string, never> }
  | { privateAddress: { mode: PrivateAddress; rejoined: boolean; rejoinFailed?: string } }
  | { failed: { reason: string } }
  | { refused: { reason: string } };

/** A saved connection a reset counts. */
export type SavedConnection = { id: string; name: string; kind: "wifi" | "vpn" | "ethernet" };

/** Why a reset keeps one: an administrator's, Ethernet's, or the daemon refused. */
export type KeptConnection = SavedConnection & { because: "notPermitted" | "ethernet" | "failed" };

/** What resetting the network settings removes and keeps: before, as the confirmation lists it,
    and after, as it went. */
export type ResetPlan = { removed: SavedConnection[]; kept: KeptConnection[] };

export type ResetAnswer =
  | { networkResetPreview: ResetPlan }
  | { networkReset: ResetPlan }
  | { failed: { reason: string } }
  | { refused: { reason: string } };

export type RouterVerdict = "reached" | "present" | "unreachable" | "notAllowed" | "notChecked";
export type DnsVerdict = "resolved" | "failed" | "timedOut" | "notChecked";
export type ConnectivityState = "full" | "limited" | "portal" | "none" | "unknown";

/** Wi-Fi Diagnostics as the session measured them, once, when asked: nothing estimated. */
export type Diagnostics = {
  /** Null when Wi-Fi is not joined. */
  link: {
    ssid: string;
    interface: string;
    bssid: string;
    signal: number;
    quality: "good" | "fair" | "poor";
    frequency: number;
    band: "2.4" | "5" | "6" | null;
    channel: number | null;
    width: number | null;
    txRate: number | null;
    maxRate: number | null;
    security: Security;
    address: string;
    permanentAddress: string;
    privateAddress: PrivateAddress;
  } | null;
  connectivity: { checked: boolean; state: ConnectivityState };
  router: { address: string | null; verdict: RouterVerdict; sent: number; answered: number; ms: number | null };
  dns: { servers: string[]; name: string; verdict: DnsVerdict; ms: number | null; addresses: number };
};

export type DiagnosticsAnswer = { wifiDiagnostics: Diagnostics } | { failed: { reason: string } } | { refused: { reason: string } };

/**
 * Intents. The session answers a join once it is done (a password asked on the way, in its own
 * sheet) and a forget once the connection is gone; the state it then tells says what is so.
 */
export type NetworkActions = {
  setWifi: (enabled: boolean) => void;
  join: (ssid: string) => Promise<NetworkAnswer>;
  forget: (id: string) => Promise<NetworkAnswer>;
  /** NetworkManager refuses scans more often than every ten seconds or so. */
  scan: () => void;
  /** A known network's private address; the network joined is joined again with it. */
  setPrivateAddress: (id: string, mode: PrivateAddressChoice) => Promise<NetworkAnswer>;
  /** What resetting the network settings would remove and keep; nothing changes. */
  resetPreview: () => Promise<ResetAnswer>;
  resetNetwork: () => Promise<ResetAnswer>;
  /** Wi-Fi Diagnostics, run once as asked. */
  diagnose: () => Promise<DiagnosticsAnswer>;
};

/** How often a page showing the networks nearby asks for a scan (ADR 0038, decision 6). */
export const SCAN_INTERVAL_MS = 10_000;

/** The networks nearby other than the one joined, as a Mac lists them: the known ones first, then
    the others, each by its signal, strongest first (the name breaking a tie). */
export function otherNetworks(networks: readonly NearbyNetwork[]): NearbyNetwork[] {
  return networks
    .filter((network) => !network.current && !network.known)
    .sort(
      (a, b) =>
        Number(b.known) - Number(a.known) || b.signal - a.signal || a.ssid.localeCompare(b.ssid),
    );
}

/** The network joined, among those nearby. */
export function currentNetwork(state: WifiState): NearbyNetwork | null {
  return (
    state.networks.find((network) => network.current) ?? state.current
  );
}


/** Whether joining asks for a password: a lock is shown. Open and OWE (encrypted, but with no
    password) networks have none. */
export function isLocked(security: Security): boolean {
  return security !== "open" && security !== "owe";
}

/** A failure's reason, known or not, as one of the session's. */
export function failureOf(answer: NetworkAnswer | null | undefined): FailureReason | null {
  if (!answer) return "failed";
  if ("refused" in answer) return "notPermitted";
  if (!("failed" in answer)) return null;
  const reason = answer.failed.reason;
  const known: readonly FailureReason[] = [
    "wrongPassword",
    "cancelled",
    "unsupported",
    "notFound",
    "notPermitted",
    "unavailable",
    "timeout",
    "failed",
  ];
  return (known as readonly string[]).includes(reason) ? (reason as FailureReason) : "failed";
}

/** Whether Settings must say a join's failure itself: the session's own sheet says a wrong password,
    an unsupported network, one polkit refused, a timeout and any other failure of the join
    (decision 4), and a cancelled join needs no word. */
export function joinFailureNeedsNote(reason: FailureReason | null): reason is "notFound" | "unavailable" {
  return reason === "notFound" || reason === "unavailable";
}

/** A failure as the person reads it, from the words for each; a join refused for want of an
    administrator says so of joining, the rest as the words say them. */
export function failureLabel(
  reason: FailureReason,
  words: Record<FailureReason, string> & { notPermittedJoin: string },
  asked: "join" | "forget",
): string {
  return asked === "join" && reason === "notPermitted" ? words.notPermittedJoin : words[reason];
}

/** A connection's details, in the order a Mac lists them, only those present. */
export type DetailKey = "address" | "subnetMask" | "router" | "dns" | "ipv6" | "hardwareAddress";

export function detailRows(details: NetworkDetails | null): { key: DetailKey; value: string }[] {
  if (!details) return [];
  const rows: { key: DetailKey; value: string }[] = [
    { key: "address", value: details.ipv4?.address ?? "" },
    { key: "subnetMask", value: details.ipv4?.subnetMask ?? "" },
    { key: "router", value: details.router },
    { key: "dns", value: details.dns.join(", ") },
    { key: "ipv6", value: details.ipv6.join(", ") },
    { key: "hardwareAddress", value: details.hardwareAddress },
  ];
  return rows.filter((row) => row.value !== "");
}

/** What Network's Ethernet row says: no port, connected, the cable unplugged, connecting, or not
    connected (a device the daemon cannot use yet, or one it is not using). */
export type EthernetStatus = "none" | "connected" | "unplugged" | "connecting" | "notConnected";

export function ethernetStatus(ethernet: EthernetState): EthernetStatus {
  if (!ethernet.device) return "none";
  switch (ethernet.state) {
    case "connected":
      return "connected";
    case "unplugged":
      return "unplugged";
    case "connecting":
      return "connecting";
    default:
      return "notConnected";
  }
}

/** A link's speed as people read it: "1000 Mb/s", "2.5 Gb/s"; empty when unknown. */
export function formatSpeed(megabits: number, language: string): string {
  if (megabits <= 0) return "";
  if (megabits >= 10_000 || (megabits > 1000 && megabits % 1000 !== 0)) {
    const gigabits = String(Math.round(megabits / 100) / 10);
    return `${language === "es" ? gigabits.replace(".", ",") : gigabits} Gb/s`;
  }
  return `${megabits} Mb/s`;
}

/** The known network a network's sheet is about: the saved connection of that name. */
export function knownByName(state: WifiState, ssid: string): KnownNetwork | null {
  return state.known.find((known) => known.ssid === ssid) ?? null;
}

/** Whether `known` is the network joined: its sheet then shows the address in use, and changing
    its private address joins it again. */
export function isJoined(state: WifiState, known: KnownNetwork): boolean {
  return state.state === "connected" && currentNetwork(state)?.ssid === known.ssid;
}

/** The private address a sheet shows as chosen: none of the three for one set elsewhere. */
export function privateChoice(known: KnownNetwork): PrivateAddressChoice | null {
  return known.privateAddress === "other" ? null : known.privateAddress;
}

/** A private address's answer: the reason it failed, if it did, and whether the network joined
    could not be joined again (the address is changed all the same). */
export function privateAddressOutcome(answer: NetworkAnswer | null | undefined): {
  failure: FailureReason | null;
  rejoinFailed: boolean;
} {
  if (answer && "privateAddress" in answer)
    return { failure: null, rejoinFailed: answer.privateAddress.rejoinFailed !== undefined };
  return { failure: failureOf(answer), rejoinFailed: false };
}

/** A reset's answer, before or after: its plan, or why there is none. */
export function resetPlanOf(answer: ResetAnswer | null | undefined): { plan: ResetPlan } | { failure: string } {
  if (answer && "networkResetPreview" in answer) return { plan: answer.networkResetPreview };
  if (answer && "networkReset" in answer) return { plan: answer.networkReset };
  if (answer && "refused" in answer) return { failure: "notPermitted" };
  const reason = answer && "failed" in answer ? answer.failed.reason : "failed";
  return { failure: ["notPermitted", "unavailable", "busy"].includes(reason) ? reason : "failed" };
}

/** What a reset's confirmation says goes and stays, in lines: the Wi-Fi networks and the VPNs
    removed, then what is kept and why. */
export function resetLines(
  plan: ResetPlan,
  words: {
    resetGoes: string;
    resetNothing: string;
    resetKinds: Record<"wifi" | "vpn", string>;
    resetLine: (kind: string, names: string) => string;
    resetKeptAdmin: (names: string) => string;
    resetKeptEthernet: (names: string) => string;
  },
  list: (names: string[]) => string,
): string[] {
  const lines: string[] = [];
  const named = (kind: "wifi" | "vpn") => plan.removed.filter((saved) => saved.kind === kind).map((saved) => saved.name);
  const removed = (["wifi", "vpn"] as const).filter((kind) => named(kind).length > 0);
  if (removed.length === 0) lines.push(words.resetNothing);
  else {
    lines.push(words.resetGoes);
    for (const kind of removed) lines.push(words.resetLine(words.resetKinds[kind], list(named(kind))));
  }
  const kept = (because: KeptConnection["because"]) =>
    plan.kept.filter((saved) => saved.because === because).map((saved) => saved.name);
  if (kept("notPermitted").length > 0) lines.push(words.resetKeptAdmin(list(kept("notPermitted"))));
  if (kept("ethernet").length > 0) lines.push(words.resetKeptEthernet(list(kept("ethernet"))));
  return lines;
}

/** A check of diagnostics: what it is, its verdict, and what was measured. */
export type DiagnosticRow = { key: "connection" | "router" | "internet" | "dns"; title: string; value: string; detail: string };

/** A time measured, as people read it: tenths under 10 ms ("1.2 ms", "1,2 ms"), whole above. */
export function formatMs(ms: number | null, language: string): string {
  if (ms === null) return "";
  const digits = ms < 10 ? 1 : 0;
  return `${new Intl.NumberFormat(language, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(ms)} ms`;
}

/** The words diagnostics are said in. */
export type DiagnosticWords = {
  quality: Record<"good" | "fair" | "poor", string>;
  channel: (channel: string, band: string, width: string) => string;
  securities: Record<Security, string>;
  accessPoint: (bssid: string) => string;
  address: (address: string) => string;
  router: (address: string | null) => string;
  routerVerdicts: Record<RouterVerdict, string>;
  routerDetails: Partial<Record<RouterVerdict, string>>;
  answered: (answered: number, sent: number, time: string) => string;
  internet: string;
  connectivity: Record<ConnectivityState, string>;
  connectivityDetails: Partial<Record<ConnectivityState, string>>;
  dns: string;
  dnsVerdicts: Record<DnsVerdict, string>;
  dnsDetails: Record<DnsVerdict, (name: string, time: string) => string>;
  servers: (servers: string) => string;
  noServers: string;
};

/** Diagnostics as rows, one a check, each with its plain verdict and, under it, what was measured:
    the connection (when Wi-Fi is joined), the router, the internet, DNS. Nothing not measured is
    shown. */
export function diagnosticRows(found: Diagnostics, words: DiagnosticWords, language: string): DiagnosticRow[] {
  const rows: DiagnosticRow[] = [];
  const line = (...parts: string[]) => parts.filter(Boolean).join(" · ");
  const link = found.link;
  if (link) {
    const percent = new Intl.NumberFormat(language, { style: "percent" }).format(link.signal / 100);
    const band = link.band ? `${link.band.replace(".", language === "es" ? "," : ".")} GHz` : "";
    const width = link.width ? `${link.width} MHz` : "";
    const channel = link.channel === null && !band ? "" : words.channel(link.channel === null ? "" : String(link.channel), band, width);
    rows.push({
      key: "connection",
      title: link.ssid,
      value: `${words.quality[link.quality]} · ${percent}`,
      detail: [
        line(channel, link.txRate ? `${link.txRate} Mb/s` : "", words.securities[link.security]),
        line(words.accessPoint(link.bssid), words.address(link.address)),
      ].join("\n"),
    });
  }
  const router = found.router;
  rows.push({
    key: "router",
    title: words.router(router.address),
    value: words.routerVerdicts[router.verdict],
    detail: line(
      router.sent > 0 ? words.answered(router.answered, router.sent, formatMs(router.ms, language)) : "",
      words.routerDetails[router.verdict] ?? "",
    ),
  });
  rows.push({
    key: "internet",
    title: words.internet,
    value: words.connectivity[found.connectivity.state],
    detail: words.connectivityDetails[found.connectivity.state] ?? "",
  });
  const dns = found.dns;
  rows.push({
    key: "dns",
    title: words.dns,
    value: words.dnsVerdicts[dns.verdict],
    detail: line(
      words.dnsDetails[dns.verdict](dns.name, formatMs(dns.ms, language)),
      dns.servers.length ? words.servers(dns.servers.join(", ")) : words.noServers,
    ),
  });
  return rows;
}
