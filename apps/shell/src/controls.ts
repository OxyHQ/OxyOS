import type {
  ShellBattery,
  ShellBluetooth,
  ShellControl,
  ShellLevel,
  ShellNetwork,
} from "./model";

/** The bezel's segments, as a Mac's: a level moves in sixteen steps. */
export const SEGMENTS = 16;
/** How long the bezel stays after its key, as a Mac's (then it fades). */
export const BEZEL_MS = 1500;

/** A level in whole percent. */
export function percent(level: number): number {
  return Math.round(Math.min(1, Math.max(0, level)) * 100);
}

/** How many of the bezel's segments a level fills: none when muted. */
export function segments(level: ShellLevel): number {
  if (level.muted) return 0;
  return Math.round(Math.min(1, Math.max(0, level.level)) * SEGMENTS);
}

const NAMES: Record<ShellControl, string> = {
  output: "Volumen",
  input: "Micrófono",
  display: "Brillo",
  keyboard: "Retroiluminación del teclado",
};

export function controlName(control: ShellControl): string {
  return NAMES[control];
}

/** Whether a volume is heard: not muted, and above nothing. */
export function silent(level: ShellLevel): boolean {
  return !!level.muted || level.level <= 0;
}

/** What the bezel says to a screen reader: "Volumen, 56 %", "Micrófono silenciado". */
export function announcement(control: ShellControl, level: ShellLevel): string {
  if (level.muted) return control === "input" ? "Micrófono silenciado" : "Sonido silenciado";
  return `${NAMES[control]}, ${percent(level.level)} %`;
}

/** Hours and minutes, as "3 h 1 min" or "45 min". */
function duration(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest} min`;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** The battery in words, or nothing without one: "78 %, cargando (3 h 1 min para cargarse)". */
export function batteryLabel(power: ShellBattery | undefined): string | undefined {
  if (!power || power.battery === "absent") return undefined;
  const charge = `${power.percent} %`;
  switch (power.battery) {
    case "charging":
      return power.minutes > 0
        ? `${charge}, cargando (${duration(power.minutes)} para cargarse)`
        : `${charge}, cargando`;
    case "full":
      return `${charge}, cargada`;
    case "not-charging":
      return `${charge}, conectada, sin cargar`;
    default:
      return power.minutes > 0 ? `${charge}, ${duration(power.minutes)} restantes` : charge;
  }
}

/** Wi-Fi in words, or nothing without it. */
export function wifiLabel(network: ShellNetwork | undefined): string | undefined {
  if (!network || network.wifi === "unavailable") return undefined;
  switch (network.wifi) {
    case "off":
      return "Desactivada";
    case "disconnected":
      return "Sin conexión";
    case "connecting":
      return network.ssid ? `Conectando a ${network.ssid}` : "Conectando";
    default:
      return network.ssid || "Conectada";
  }
}

/** Whether Wi-Fi is on (connected or not). */
export function wifiOn(network: ShellNetwork | undefined): boolean {
  return !!network && network.wifi !== "unavailable" && network.wifi !== "off";
}

/** Bluetooth in words, or nothing without it. */
export function bluetoothLabel(bluetooth: ShellBluetooth | undefined): string | undefined {
  if (!bluetooth || bluetooth.state === "unavailable") return undefined;
  if (bluetooth.state === "off") return "Desactivado";
  if (bluetooth.connected === 0) return "Activado";
  return bluetooth.connected === 1 ? "1 dispositivo" : `${bluetooth.connected} dispositivos`;
}

/** How many of the Wi-Fi icon's three arcs its signal fills. */
export function wifiBars(signal: number): number {
  if (signal >= 67) return 3;
  if (signal >= 34) return 2;
  return 1;
}
