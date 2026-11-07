/**
 * The sound's outputs and inputs as the session says they are (docs/adr/0051-sound-devices.mdx):
 * the state it tells the shell (`{"sound": State}`), read as it is, and what the Control Center's
 * Sound view makes of it. Nothing here asks anything: the session alone speaks to PipeWire.
 */

import type { ShellMessages } from "./messages";

/** What a device is, as the session says it. */
export type SoundKind = "speakers" | "headphones" | "headset" | "hdmi" | "line" | "microphone" | "other";
/** How it is attached. */
export type SoundTransport = "builtIn" | "hdmi" | "usb" | "bluetooth" | "virtual" | "other";

/** An output or an input: its id (what choosing it sends back), the name the system gave it, what
    it is, how it is attached, and whether it is the one chosen. */
export type SoundDevice = {
  id: string;
  name: string;
  kind: SoundKind;
  transport: SoundTransport;
  selected: boolean;
};

/** The outputs or the inputs, in the session's order (built-in, HDMI, USB, Bluetooth, virtual),
    and the chosen one's volume (0 to 1), mute and balance (-1 to 1); null where it has none. */
export type SoundSide = {
  devices: readonly SoundDevice[];
  volume: number | null;
  muted: boolean;
  balance: number | null;
};

export type SoundState = {
  /** Whether the sound server answers. */
  available: boolean;
  output: SoundSide;
  input: SoundSide;
};

/** A device's name as a Mac's Sound shows it: a built-in one by what it is ("Altavoces
    integrados", "Auriculares externos" for the jack, "Salida de línea"); any other as the system
    names it (a display's, "AirPods Pro", a USB headset's). */
export function soundDeviceName(device: Pick<SoundDevice, "name" | "kind" | "transport">, messages: ShellMessages): string {
  if (device.transport !== "builtIn") return device.name;
  const words = messages.sound;
  switch (device.kind) {
    case "speakers":
      return words.builtInSpeakers;
    case "headphones":
    case "headset":
      return words.externalHeadphones;
    case "line":
      return words.lineOut;
    default:
      return device.name;
  }
}

/** The output chosen, if the session says one is. */
export function selectedOutput(state: SoundState | null | undefined): SoundDevice | undefined {
  return state?.output.devices.find((device) => device.selected);
}

/** Whether the Control Center has a Sound view to show: the session told the sound, its server
    answers, and there is an output to choose. */
export function hasSoundView(state: SoundState | null | undefined): state is SoundState {
  return !!state && state.available && state.output.devices.length > 0;
}
