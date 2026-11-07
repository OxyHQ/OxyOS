/**
 * Sound's outputs and inputs (docs/adr/0056-sound-devices.mdx): the state as the session tells it
 * to Settings (its `settings` grant), and what Settings may ask. The session alone speaks to
 * PipeWire; Settings keeps no list of its own.
 */

/** What a device is, as the session names it. */
export type SoundKind = "speakers" | "headphones" | "headset" | "hdmi" | "line" | "microphone" | "other";
/** How it is connected. */
export type SoundTransport = "builtIn" | "hdmi" | "usb" | "bluetooth" | "virtual" | "other";

export type SoundDevice = { id: string; name: string; kind: SoundKind; transport: SoundTransport; selected: boolean };

/** One side, its devices in the session's order and the chosen one's volume (0 to 1), mute and
    balance (-1 left to 1 right; null where it has no left and right). */
export type SoundSide = { devices: SoundDevice[]; volume: number | null; muted: boolean; balance: number | null };

export type SoundState = {
  /** Whether the session reaches PipeWire. */
  available: boolean;
  output: SoundSide;
  input: SoundSide;
};

export type SoundDirection = "output" | "input";

/**
 * Intents. The session answers each with the state as it is then, which it also tells; a change
 * it refuses leaves the state as it was.
 */
export type SoundActions = {
  select: (direction: SoundDirection, id: string) => void;
  setVolume: (direction: SoundDirection, volume: number) => void;
  setMuted: (direction: SoundDirection, muted: boolean) => void;
  setBalance: (balance: number) => void;
  /**
   * Follows the chosen input's level (0 to 1), told as it changes; returns the stop. The input is
   * metered only while a view follows it (ADR 0056, decision 7): the Input tab's meter, while shown.
   */
  followLevel: (listener: (level: number) => void) => () => void;
};

/** The input level's segments, as a Mac's: sixteen. */
export const METER_SEGMENTS = 16;

/** The keys' step of a volume, as the shell's slider and a Mac's keys: a sixteenth. */
export const VOLUME_KEY_STEP = 1 / 16;

/** The words a device's name and type are said in (Settings' `sound` messages). */
export type SoundWords = {
  builtIn: {
    speakers: string;
    headphones: string;
    lineOut: string;
    microphone: string;
    externalMicrophone: string;
    lineIn: string;
  };
  types: Record<SoundTransport, string>;
};

/** A device's name: the machine's own by its kind, in the person's language, as a Mac names them
    ("Built-in Speakers", "External Headphones"); any other by the name the session gives it. */
export function deviceName(device: SoundDevice, direction: SoundDirection, words: SoundWords): string {
  if (device.transport !== "builtIn") return device.name;
  const own = words.builtIn;
  if (direction === "output") {
    switch (device.kind) {
      case "speakers":
        return own.speakers;
      case "headphones":
      case "headset":
        return own.headphones;
      case "line":
        return own.lineOut;
      default:
        return device.name;
    }
  }
  switch (device.kind) {
    case "microphone":
      return own.microphone;
    case "headset":
      return own.externalMicrophone;
    case "line":
      return own.lineIn;
    default:
      return device.name;
  }
}

/** A device's type, as a Mac's *Type* column says it. */
export function typeLabel(transport: SoundTransport, words: SoundWords): string {
  return words.types[transport] ?? words.types.other;
}

/** A level (0 to 1) in whole percent. */
export function percentOf(level: number): number {
  if (!Number.isFinite(level)) return 0;
  return Math.round(Math.min(1, Math.max(0, level)) * 100);
}

/** How many of the input level's segments a level lights. */
export function litSegments(level: number): number {
  if (!Number.isFinite(level)) return 0;
  return Math.min(METER_SEGMENTS, Math.max(0, Math.round(level * METER_SEGMENTS)));
}

/** Which side a balance leans to and by how much (whole percent), or centred. */
export function balanceLean(balance: number): { side: "left" | "right" | "center"; percent: number } {
  const percent = percentOf(Math.abs(balance));
  if (percent === 0) return { side: "center", percent: 0 };
  return { side: balance < 0 ? "left" : "right", percent };
}
