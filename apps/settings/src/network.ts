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

/** A saved Wi-Fi connection: `id` is the connection's UUID. */
export type KnownNetwork = { id: string; ssid: string; autoJoin: boolean };

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

/** The session's answer to a join or a forget (or its refusal, without the grant). */
export type NetworkAnswer =
  | { joined: { ssid: string } }
  | { forgotten: Record<string, never> }
  | { failed: { reason: string } }
  | { refused: { reason: string } };

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
};

/** How often a page showing the networks nearby asks for a scan (ADR 0038, decision 6). */
export const SCAN_INTERVAL_MS = 10_000;

/** The networks nearby other than the one joined, as a Mac lists them: the known ones first, then
    the others, each by its signal, strongest first (the name breaking a tie). */
export function otherNetworks(networks: readonly NearbyNetwork[]): NearbyNetwork[] {
  return networks
    .filter((network) => !network.current)
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

/** How many of the Wi-Fi glyph's three arcs a signal fills, as the shell's top bar draws it: 0 for
    none (no signal at all), else 1 to 3. */
export function signalBars(signal: number): 0 | 1 | 2 | 3 {
  if (signal <= 0) return 0;
  if (signal >= 67) return 3;
  if (signal >= 34) return 2;
  return 1;
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
