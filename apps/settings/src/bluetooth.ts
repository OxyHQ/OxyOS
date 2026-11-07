/**
 * Bluetooth (docs/adr/0050-bluetooth.mdx): the state as the session tells it to Settings (the
 * `bluetooth` grant), and what Settings may ask. The session alone speaks to BlueZ and is the
 * machine's pairing agent: a code to confirm or type is asked in the shell's own system sheets,
 * and a connection that failed is said there too. Settings keeps no list of its own.
 */

/** What a device is, from BlueZ's icon and class: the glyph it is shown with. */
export type BluetoothKind =
  | "headphones"
  | "speaker"
  | "keyboard"
  | "mouse"
  | "trackpad"
  | "phone"
  | "gamepad"
  | "computer"
  | "other";

/** The kinds the session names; any other is shown as `other`. */
export const BLUETOOTH_KINDS: readonly BluetoothKind[] = [
  "headphones",
  "speaker",
  "keyboard",
  "mouse",
  "trackpad",
  "phone",
  "gamepad",
  "computer",
  "other",
];

/** A device the adapter knows: paired (My Devices) or seen nearby. */
export type BluetoothDevice = {
  /** Its address, "AA:BB:CC:DD:EE:FF": what every request names. */
  id: string;
  /** The same address, shown in its info sheet. */
  address: string;
  /** Its alias (renamed) or its own name; never empty. */
  name: string;
  kind: BluetoothKind;
  paired: boolean;
  connected: boolean;
  /** The session's own call under way on it (connecting includes pairing). */
  busy: null | "connecting" | "disconnecting";
  /** 0 to 100 when the device reports one (BlueZ's Battery1), else null. */
  battery: number | null;
  /** An audio device: PipeWire makes it an output once connected. */
  audio: boolean;
  /** An input device: libinput takes it as it connects. */
  input: boolean;
};

/** The machine's adapter. */
export type BluetoothAdapter = {
  /** What others see it as. */
  name: string;
  address: string;
  powered: boolean;
  /** Off by rfkill (a hardware switch, airplane mode): it cannot be turned on. */
  blocked: boolean;
  discovering: boolean;
  discoverable: boolean;
};

/** Bluetooth's state (ADR 0050). */
export type BluetoothState = {
  /** Whether BlueZ answers. */
  available: boolean;
  /** Null: no Bluetooth hardware. */
  adapter: BluetoothAdapter | null;
  /** By name (then address); devices not paired only when they have a real name. */
  devices: BluetoothDevice[];
};

/** Why a request failed. */
export type BluetoothFailure =
  | "notFound"
  | "unavailable"
  | "blocked"
  | "cancelled"
  | "rejected"
  | "timeout"
  | "busy"
  | "failed"
  | "notPermitted";

/** The session's answer to a request (or its refusal, without the grant). */
export type BluetoothAnswer =
  | { bluetooth: BluetoothState }
  | { discovering: { on: boolean } }
  | { connected: { id: string } }
  | { paired: { id: string } }
  | { disconnected: { id: string } }
  | { forgotten: { id: string } }
  | { renamed: { id: string; name: string } }
  | { failed: { reason: string } }
  | { refused: { reason: string } };

/**
 * Intents. The session answers each once it is done (a connection once paired and connected, the
 * pairing's sheets on the way); the state it then tells says what is so.
 */
export type BluetoothActions = {
  setPower: (on: boolean) => Promise<BluetoothAnswer>;
  /** Discovery held while the Bluetooth page is shown: on as it opens, off as it goes. */
  discover: (on: boolean) => Promise<BluetoothAnswer>;
  /** Pairs first when not paired, trusts it, connects. */
  connect: (id: string) => Promise<BluetoothAnswer>;
  disconnect: (id: string) => Promise<BluetoothAnswer>;
  forget: (id: string) => Promise<BluetoothAnswer>;
  /** An empty name gives it back its own. */
  rename: (id: string, name: string) => Promise<BluetoothAnswer>;
};

/** A kind the session named, as one of those Settings draws. */
export function kindOf(kind: string): BluetoothKind {
  return (BLUETOOTH_KINDS as readonly string[]).includes(kind) ? (kind as BluetoothKind) : "other";
}

/** What the page shows: nothing to ask (no BlueZ), no adapter, or the adapter. */
export function bluetoothView(
  state: BluetoothState | null | undefined,
): { kind: "unavailable" } | { kind: "noAdapter" } | { kind: "adapter"; adapter: BluetoothAdapter } {
  if (!state || !state.available) return { kind: "unavailable" };
  if (!state.adapter) return { kind: "noAdapter" };
  return { kind: "adapter", adapter: state.adapter };
}

/** Whether Bluetooth is on: powered, and not turned off by a switch. */
export function isOn(adapter: BluetoothAdapter | null | undefined): boolean {
  return !!adapter && adapter.powered && !adapter.blocked;
}

/** My Devices: the paired ones, in the session's order (by name), so a row stays where it is as it
    connects. */
export function myDevices(devices: readonly BluetoothDevice[]): BluetoothDevice[] {
  return devices.filter((device) => device.paired);
}

/** Nearby Devices: the ones seen and not paired, in the session's order. */
export function nearbyDevices(devices: readonly BluetoothDevice[]): BluetoothDevice[] {
  return devices.filter((device) => !device.paired);
}

/** What a device's row says of it: the session's call under way first, then whether it is
    connected. */
export type DeviceStatus = "connecting" | "disconnecting" | "connected" | "notConnected";

export function deviceStatus(device: BluetoothDevice): DeviceStatus {
  if (device.busy === "connecting") return "connecting";
  if (device.busy === "disconnecting") return "disconnecting";
  return device.connected ? "connected" : "notConnected";
}

/** A battery's charge as people read it in a language ("80%", "80 %"); empty when it reports none
    or is not connected (a battery last read is not shown as if current). */
export function batteryText(device: Pick<BluetoothDevice, "battery" | "connected">, language: string): string {
  if (device.battery === null || !device.connected) return "";
  const charge = Math.min(100, Math.max(0, Math.round(device.battery)));
  return new Intl.NumberFormat(language, { style: "percent" }).format(charge / 100);
}

/** The words a device is read with. */
export type DeviceWords = {
  connected: string;
  notConnected: string;
  connecting: string;
  disconnecting: string;
  battery: (percent: string) => string;
};

/** A device's status in words. */
export function statusLabel(device: BluetoothDevice, words: DeviceWords): string {
  return words[deviceStatus(device)];
}

/** What a device's row says to the screen reader: its name, its status, and its battery when
    connected and known ("AirPods Pro, Connected, Battery 80%"). */
export function deviceLabel(device: BluetoothDevice, words: DeviceWords, language: string): string {
  const battery = batteryText(device, language);
  return [device.name, statusLabel(device, words), battery ? words.battery(battery) : null]
    .filter(Boolean)
    .join(", ");
}

/** The button a paired device's row offers: connect it, disconnect it, or none while the session's
    call on it is under way. */
export function connectionAction(device: BluetoothDevice): "connect" | "disconnect" | null {
  if (device.busy) return null;
  return device.connected ? "disconnect" : "connect";
}

/** The name a rename sends, from what was typed: trimmed; empty gives the device back its own; null
    when it is the name it has, so nothing is asked. */
export function renameTo(typed: string, device: Pick<BluetoothDevice, "name">): string | null {
  const name = typed.trim();
  return name === device.name ? null : name;
}

/** A failure's reason, known or not, as one of the session's; null when the answer is no failure. */
export function failureOf(answer: BluetoothAnswer | null | undefined): BluetoothFailure | null {
  if (!answer) return "failed";
  if ("refused" in answer) return "notPermitted";
  if (!("failed" in answer)) return null;
  const reason = answer.failed.reason;
  const known: readonly BluetoothFailure[] = [
    "notFound",
    "unavailable",
    "blocked",
    "cancelled",
    "rejected",
    "timeout",
    "busy",
    "failed",
  ];
  return (known as readonly string[]).includes(reason) ? (reason as BluetoothFailure) : "failed";
}

/** Whether Settings says a failure itself: a connection's (and pairing's) the session says in its
    own sheet, and a cancelled one needs no word; a forget's and a rename's are Settings' to say. */
export function failureNeedsNote(asked: "connect" | "disconnect" | "forget" | "rename", reason: BluetoothFailure | null): boolean {
  if (reason === null || reason === "cancelled") return false;
  return asked === "forget" || asked === "rename";
}

/** The devices whose connection changed between two states, for the screen reader to hear as a
    Mac's VoiceOver does: each paired device that connected or disconnected, by name. */
export function connectionChanges(
  before: readonly BluetoothDevice[] | null | undefined,
  after: readonly BluetoothDevice[] | null | undefined,
): { name: string; connected: boolean }[] {
  if (!before || !after) return [];
  const was = new Map(before.map((device) => [device.id, device.connected]));
  return after
    .filter((device) => device.paired && was.has(device.id) && was.get(device.id) !== device.connected)
    .map((device) => ({ name: device.name, connected: device.connected }));
}
