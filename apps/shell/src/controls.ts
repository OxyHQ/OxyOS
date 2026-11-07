import type {
  ShellBattery,
  ShellBluetooth,
  ShellControl,
  ShellLevel,
  ShellNetwork,
} from "./model";
import type { ShellMessages } from "./messages";

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

export function controlName(control: ShellControl, messages: ShellMessages): string {
  return messages.controls.names[control];
}

/** Whether a volume is heard: not muted, and above nothing. */
export function silent(level: ShellLevel): boolean {
  return !!level.muted || level.level <= 0;
}

/** What the bezel says to a screen reader: "Volumen, 56 %", "Micrófono silenciado". */
export function announcement(control: ShellControl, level: ShellLevel, messages: ShellMessages): string {
  const words = messages.controls;
  if (level.muted) return control === "input" ? words.microphoneMuted : words.soundMuted;
  return words.level(words.names[control], messages.common.percent(percent(level.level)));
}

/** Hours and minutes, as "3 h 1 min" or "45 min". */
function duration(minutes: number, messages: ShellMessages): string {
  return messages.controls.duration(Math.floor(minutes / 60), minutes % 60);
}

/** The battery in words, or nothing without one: "78 %, cargando (3 h 1 min para cargarse)". */
export function batteryLabel(power: ShellBattery | undefined, messages: ShellMessages): string | undefined {
  if (!power || power.battery === "absent") return undefined;
  const words = messages.controls;
  const charge = messages.common.percent(power.percent);
  switch (power.battery) {
    case "charging":
      return power.minutes > 0
        ? words.chargingFor(charge, duration(power.minutes, messages))
        : words.charging(charge);
    case "full":
      return words.charged(charge);
    case "not-charging":
      return words.notCharging(charge);
    default:
      return power.minutes > 0 ? words.remaining(charge, duration(power.minutes, messages)) : charge;
  }
}

/** The battery's menu's lines (docs/adr/0057-battery-and-energy.mdx), as a Mac's: its charge, the
    power source, and the time left or until full ("3 h 20 min remaining", "Calculating…" before
    UPower knows, "Fully Charged", "Not Charging" held at its limit). Nothing without a battery. */
export function batteryMenuLines(
  power: ShellBattery | undefined,
  messages: ShellMessages,
): { charge: string; source: string; time: string } | undefined {
  if (!power || power.battery === "absent") return undefined;
  const words = messages.battery;
  const time = (() => {
    switch (power.battery) {
      case "charging":
        return power.minutes > 0 ? words.untilFull(duration(power.minutes, messages)) : words.calculating;
      case "full":
        return words.fullyCharged;
      case "not-charging":
        return words.notCharging;
      default:
        return power.minutes > 0 ? words.remaining(duration(power.minutes, messages)) : words.calculating;
    }
  })();
  return {
    charge: messages.common.percent(power.percent),
    source: power.onAC ? words.sourceAdapter : words.sourceBattery,
    time,
  };
}

/** The applications using significant energy, in the order the session said them. */
export function significantApps<T extends { significant: boolean }>(apps: readonly T[]): T[] {
  return apps.filter((app) => app.significant);
}

/** Wi-Fi in words, or nothing without it. */
export function wifiLabel(network: ShellNetwork | undefined, messages: ShellMessages): string | undefined {
  if (!network || network.wifi === "unavailable") return undefined;
  const words = messages.controls;
  switch (network.wifi) {
    case "off":
      return words.wifiOff;
    case "disconnected":
      return words.wifiDisconnected;
    case "connecting":
      return network.ssid ? words.wifiConnectingTo(network.ssid) : words.wifiConnecting;
    default:
      return network.ssid || words.wifiConnected;
  }
}

/** Whether Wi-Fi is on (connected or not). */
export function wifiOn(network: ShellNetwork | undefined): boolean {
  return !!network && network.wifi !== "unavailable" && network.wifi !== "off";
}

/** Bluetooth in words, or nothing without it. */
export function bluetoothLabel(
  bluetooth: ShellBluetooth | undefined,
  messages: ShellMessages,
): string | undefined {
  if (!bluetooth || bluetooth.state === "unavailable") return undefined;
  if (bluetooth.state === "off") return messages.common.off;
  if (bluetooth.connected === 0) return messages.common.on;
  return messages.controls.devices(bluetooth.connected);
}

/** How many of the Wi-Fi icon's three arcs its signal fills. */
export function wifiBars(signal: number): number {
  if (signal >= 67) return 3;
  if (signal >= 34) return 2;
  return 1;
}
