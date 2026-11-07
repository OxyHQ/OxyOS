import React from "react";
import Svg, { Path, Rect } from "react-native-svg";
import { RiBluetoothLine } from "@oxy.so/bloom/icons/RiBluetoothLine";
import { RiComputerLine } from "@oxy.so/bloom/icons/RiComputerLine";
import { RiGamepadLine } from "@oxy.so/bloom/icons/RiGamepadLine";
import { RiHeadphoneLine } from "@oxy.so/bloom/icons/RiHeadphoneLine";
import { RiKeyboardLine } from "@oxy.so/bloom/icons/RiKeyboardLine";
import { RiMouseLine } from "@oxy.so/bloom/icons/RiMouseLine";
import { RiSmartphoneLine } from "@oxy.so/bloom/icons/RiSmartphoneLine";
import { RiSpeakerLine } from "@oxy.so/bloom/icons/RiSpeakerLine";
import type { BluetoothKind } from "../bluetooth";

/** A trackpad, in the line of Bloom's Remix icons: a wide rounded slab. Bloom has none. */
function TrackpadIcon({ width, height, fill }: { width: number; height: number; fill: string }) {
  return (
    <Svg width={width} height={height} viewBox="0 0 24 24">
      <Path
        fillRule="evenodd"
        fill={fill}
        d="M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zm0 2v12h16V6H4z"
      />
    </Svg>
  );
}

const ICONS: Record<Exclude<BluetoothKind, "trackpad">, typeof RiBluetoothLine> = {
  headphones: RiHeadphoneLine,
  speaker: RiSpeakerLine,
  keyboard: RiKeyboardLine,
  mouse: RiMouseLine,
  phone: RiSmartphoneLine,
  gamepad: RiGamepadLine,
  computer: RiComputerLine,
  other: RiBluetoothLine,
};

/**
 * What a Bluetooth device is, as a Mac shows it (docs/adr/0055-bluetooth.mdx): headphones, a
 * speaker, a keyboard, a mouse, a trackpad, a phone, a gamepad, a computer, and Bluetooth's own
 * glyph for any other. Bloom's icons, but the trackpad, which it has not.
 */
export function BluetoothGlyph({ kind, size, color }: { kind: BluetoothKind; size: number; color: string }) {
  if (kind === "trackpad") return <TrackpadIcon width={size} height={size} fill={color} />;
  const Glyph = ICONS[kind] ?? RiBluetoothLine;
  return <Glyph width={size} height={size} fill={color} />;
}

/** A device's charge, as a Mac's Bluetooth menu draws it: a small battery filled to its level. */
export function DeviceBattery({ level, color, faint }: { level: number; color: string; faint: string }) {
  const filled = Math.max(1, Math.round((Math.min(100, Math.max(0, level)) / 100) * 16));
  return (
    <Svg width={22} height={11} viewBox="0 0 22 11">
      <Rect x={0.5} y={0.5} width={19} height={10} rx={2.5} stroke={faint} fill="none" />
      <Rect x={20.5} y={3.5} width={1.5} height={4} rx={0.75} fill={faint} />
      <Rect x={2} y={2} width={filled} height={7} rx={1.25} fill={color} />
    </Svg>
  );
}
