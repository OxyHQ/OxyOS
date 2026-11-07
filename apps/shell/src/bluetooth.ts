/**
 * Bluetooth as the session says it is (docs/adr/0055-bluetooth.mdx): the state it tells the shell
 * (`{"bluetooth": State}`), read as it is, and what the Control Center's Bluetooth view makes of
 * it. Nothing here asks anything: the session alone speaks to BlueZ, and is the machine's pairing
 * agent, asking what a pairing needs in its own system sheets.
 */

import type { ShellMessages } from "./messages";
import type { ShellBluetooth } from "./model";

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

/** A device the adapter knows: paired, or nearby while it discovers. */
export type BluetoothDevice = {
  /** Its address, which every request names. */
  id: string;
  /** The same address, shown. */
  address: string;
  /** Its alias (renamed) or its own name; never empty. */
  name: string;
  kind: BluetoothKind;
  paired: boolean;
  connected: boolean;
  /** The session's own call under way on it (connecting includes pairing). */
  busy: null | "connecting" | "disconnecting";
  /** 0 to 100 where the device reports one, else null. */
  battery: number | null;
  /** An audio device: an output once connected. */
  audio: boolean;
  /** An input device: taken by the input stack as it connects. */
  input: boolean;
};

export type BluetoothAdapter = {
  /** What other devices see it as. */
  name: string;
  address: string;
  powered: boolean;
  /** Off by rfkill (a hardware switch, airplane mode): it cannot be turned on. */
  blocked: boolean;
  discovering: boolean;
  discoverable: boolean;
};

export type BluetoothState = {
  /** Whether BlueZ answers. */
  available: boolean;
  /** Null: the machine has no Bluetooth. */
  adapter: BluetoothAdapter | null;
  /** By name, then address; unpaired ones only with a real name. */
  devices: readonly BluetoothDevice[];
};

/** The Bluetooth view's list, as a Mac's Control Center's: the paired devices, in the session's
    order (by name), so a row does not move as it connects. */
export function pairedDevices(state: BluetoothState): BluetoothDevice[] {
  return state.devices.filter((device) => device.paired);
}

/** A device's state, shown: "Connected", "Not Connected", "Connecting…". */
export function deviceStatus(device: BluetoothDevice, messages: ShellMessages): string {
  const words = messages.bluetooth;
  if (device.busy === "connecting") return words.connectingShown;
  if (device.busy === "disconnecting") return words.disconnectingShown;
  return device.connected ? words.connected : words.notConnected;
}

/** A device's charge, shown while it is connected and says one ("80%"); else nothing. */
export function deviceBattery(device: BluetoothDevice, messages: ShellMessages): string | undefined {
  if (!device.connected || device.battery === null) return undefined;
  return messages.common.percent(Math.round(Math.min(100, Math.max(0, device.battery))));
}

/** A device's row for a screen reader, as VoiceOver says a Mac's: "Magic Keyboard, Connected,
    Battery 80%", "WH-1000XM4, Not Connected", "AirPods, Connecting". */
export function deviceLabel(device: BluetoothDevice, messages: ShellMessages): string {
  const words = messages.bluetooth;
  const parts = [device.name];
  if (device.busy === "connecting") parts.push(words.connecting);
  else if (device.busy === "disconnecting") parts.push(words.disconnecting);
  else parts.push(device.connected ? words.connected : words.notConnected);
  const battery = deviceBattery(device, messages);
  if (battery !== undefined) parts.push(words.battery(battery));
  return parts.join(", ");
}

/** What the Bluetooth view says in place of its list, or nothing when it has one. */
export function bluetoothEmptyText(state: BluetoothState, messages: ShellMessages): string | undefined {
  const words = messages.bluetooth;
  if (!state.available || !state.adapter) return words.unavailable;
  if (state.adapter.blocked) return words.blocked;
  if (!state.adapter.powered) return words.off;
  if (!state.devices.some((device) => device.paired)) return words.none;
  return undefined;
}

/** Whether Bluetooth's switch can move: BlueZ answers and the adapter is not blocked. */
export function bluetoothSwitchable(state: BluetoothState): boolean {
  return state.available && !!state.adapter && !state.adapter.blocked;
}

/** Whether the Control Center has a Bluetooth view to show: the session told Bluetooth's state and
    the machine has an adapter. */
export function hasBluetoothView(
  state: BluetoothState | null | undefined,
): state is BluetoothState & { adapter: BluetoothAdapter } {
  return !!state && !!state.adapter;
}

/** Bluetooth's module's state from the state told, where the session told no other: unavailable
    without BlueZ or an adapter, off, or on with how many devices are connected. */
export function bluetoothSummary(state: BluetoothState | null | undefined): ShellBluetooth | undefined {
  if (!state) return undefined;
  if (!state.available || !state.adapter) return { state: "unavailable", connected: 0 };
  if (!state.adapter.powered) return { state: "off", connected: 0 };
  return { state: "on", connected: state.devices.filter((device) => device.connected).length };
}
