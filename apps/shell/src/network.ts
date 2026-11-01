/**
 * Wi-Fi and networks as the session says they are (docs/adr/0038-networks.mdx, decision 3): the
 * state it tells the shell (`{"network": State}`), read as it is, and what the Control Center's
 * Wi-Fi view makes of it. Nothing here asks anything: the session alone speaks to the daemon.
 */

/** How a network is protected, as the session says it. */
export type WifiSecurity = "open" | "owe" | "wpa" | "wpa3" | "wep" | "enterprise";

/** A connection's addresses (the network joined, or the cable). */
export type NetworkDetails = {
  interface: string;
  hardwareAddress: string;
  ipv4: { address: string; prefix: number; subnetMask: string } | null;
  router: string | null;
  dns: readonly string[];
  ipv6: readonly string[];
};

/** A network nearby: one per name, its strongest access point. */
export type WifiNetwork = {
  ssid: string;
  /** 0 to 100. */
  signal: number;
  security: WifiSecurity;
  /** A saved connection has its name. */
  known: boolean;
  /** The network joined (or being joined by the daemon: Wi-Fi's state says `connecting`). */
  current: boolean;
  /** False for one OxyOS cannot join (enterprise, WEP): listed, and its sheet says so. */
  supported: boolean;
};

/** A saved Wi-Fi connection: its UUID, its name, whether it is joined by itself. */
export type KnownNetwork = { id: string; ssid: string; autoJoin: boolean };

export type NetworkState = {
  /** Whether the network daemon answers. */
  available: boolean;
  wifi: {
    /** A Wi-Fi device exists. */
    device: boolean;
    enabled: boolean;
    /** Its hardware switch turned it off. */
    hardwareBlocked: boolean;
    state: "unavailable" | "off" | "disconnected" | "connecting" | "connected";
    /** The network joined (or being joined by the daemon), or null. */
    current: WifiNetwork | null;
    /** Nearby, strongest first. */
    networks: readonly WifiNetwork[];
    known: readonly KnownNetwork[];
    details: NetworkDetails | null;
  };
  ethernet: {
    device: boolean;
    state: "unavailable" | "unplugged" | "disconnected" | "connecting" | "connected";
    /** Mb/s; 0 unknown. */
    speed: number;
    details: NetworkDetails | null;
  };
  /** The name being joined, or null. */
  joining: string | null;
};

/** How often the Wi-Fi view asks the session to scan while it is open: NetworkManager refuses
    scans more often (ADR 0038, decision 6). */
export const SCAN_INTERVAL_MS = 10_000;

const bySignal = (a: WifiNetwork, b: WifiNetwork) => b.signal - a.signal || a.ssid.localeCompare(b.ssid);

/** The Wi-Fi view's two lists, as a Mac's: the known networks nearby (the joined one first, then by
    signal), then the others, strongest first. */
export function wifiSections(networks: readonly WifiNetwork[]): {
  known: WifiNetwork[];
  other: WifiNetwork[];
} {
  const known = networks
    .filter((network) => network.known || network.current)
    .sort((a, b) => Number(b.current) - Number(a.current) || bySignal(a, b));
  const other = networks.filter((network) => !network.known && !network.current).sort(bySignal);
  return { known, other };
}

/** How many of the Wi-Fi icon's three arcs a signal fills: none for one barely heard. */
export function signalArcs(signal: number): number {
  if (signal >= 67) return 3;
  if (signal >= 34) return 2;
  if (signal >= 10) return 1;
  return 0;
}

/** Whether joining asks for something (a password, or what OxyOS does not support): a lock. */
export function isProtected(network: Pick<WifiNetwork, "security">): boolean {
  return network.security !== "open" && network.security !== "owe";
}

/** Whether a network is being joined: the session joining it, or the daemon connecting to it. */
export function networkJoining(network: WifiNetwork, state: Pick<NetworkState, "joining" | "wifi">): boolean {
  if (network.current) return state.wifi.state === "connecting";
  return state.joining === network.ssid;
}

/** A network's row for a screen reader: "Casa, conectada, protegida, señal 3 de 3",
    "Café, señal 1 de 3", "Oficina, conectando, protegida, no compatible, señal 2 de 3". */
export function networkLabel(network: WifiNetwork, state: Pick<NetworkState, "joining" | "wifi">): string {
  const parts = [network.ssid];
  if (networkJoining(network, state)) parts.push("conectando");
  else if (network.current) parts.push("conectada");
  if (isProtected(network)) parts.push("protegida");
  if (!network.supported) parts.push("no compatible");
  parts.push(`señal ${signalArcs(network.signal)} de 3`);
  return parts.join(", ");
}

/** What the Wi-Fi view says in place of its lists, or nothing when it has them. */
export function wifiEmptyText(state: NetworkState): string | undefined {
  if (!state.available) return "El servicio de red no responde.";
  if (state.wifi.hardwareBlocked) return "Wi-Fi está desactivada con el interruptor del ordenador.";
  if (!state.wifi.enabled) return "Wi-Fi desactivada";
  if (state.wifi.networks.length === 0) return "Buscando redes…";
  return undefined;
}

/** Whether the Control Center has a Wi-Fi view to show: the session told the networks and the
    machine has a Wi-Fi device. */
export function hasWifiView(state: NetworkState | null | undefined): state is NetworkState {
  return !!state && state.wifi.device;
}
