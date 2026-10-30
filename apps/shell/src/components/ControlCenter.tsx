import React, { useEffect } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Slider } from "@oxy.so/bloom/slider";
import { RiWifiLine } from "@oxy.so/bloom/icons/RiWifiLine";
import { RiBluetoothLine } from "@oxy.so/bloom/icons/RiBluetoothLine";
import { RiMoonLine } from "@oxy.so/bloom/icons/RiMoonLine";
import { RiSunLine } from "@oxy.so/bloom/icons/RiSunLine";
import { RiVolumeUpLine } from "@oxy.so/bloom/icons/RiVolumeUpLine";
import { RiVolumeMuteLine } from "@oxy.so/bloom/icons/RiVolumeMuteLine";
import { RiKeyboardLine } from "@oxy.so/bloom/icons/RiKeyboardLine";
import { RiBatteryLine } from "@oxy.so/bloom/icons/RiBatteryLine";
import { Box, PressArea } from "../primitives";
import {
  batteryLabel,
  bluetoothLabel,
  percent,
  silent,
  wifiLabel,
  wifiOn,
} from "../controls";
import type { ShellControl, ShellControls } from "../model";

export const CONTROL_CENTER_WIDTH = 320;
/** One of Bloom's icons. */
type Icon = typeof RiWifiLine;
const INK = "#1d2430";
const QUIET = "#5b6472";
const ACCENT = "#2f6fe4";
const card = {
  backgroundColor: "#ffffffb3",
  borderRadius: 16,
  padding: 12,
} as const;

/** A round button with a title and what it is, as a Mac's Control Center modules: on, it is
    the accent's. `onPress` absent, it only says (Bluetooth, which the session only looks at). */
function Module({
  icon,
  title,
  status,
  on,
  onPress,
  testID,
}: {
  icon: Icon;
  title: string;
  status: string;
  on: boolean;
  onPress?: () => void;
  testID: string;
}) {
  const Glyph = icon;
  const face = (
    <Box
      className="items-center justify-center rounded-full"
      style={{ width: 32, height: 32, backgroundColor: on ? ACCENT : "#0b183a1a" }}
    >
      <Glyph width={17} height={17} fill={on ? "#ffffff" : INK} />
    </Box>
  );
  return (
    <Box className="flex-row items-center" style={{ gap: 10, minHeight: 40 }}>
      {onPress ? (
        <PressArea
          testID={testID}
          accessibilityLabel={title}
          accessibilityRole="switch"
          accessibilityState={{ checked: on }}
          onPress={onPress}
        >
          {face}
        </PressArea>
      ) : (
        face
      )}
      <Box
        className="min-w-0 flex-1"
        accessible={!onPress}
        accessibilityLabel={onPress ? undefined : `${title}: ${status}`}
      >
        <Text numberOfLines={1} style={{ color: INK, fontSize: 13, fontWeight: "600" }}>
          {title}
        </Text>
        <Text numberOfLines={1} style={{ color: QUIET, fontSize: 11 }}>
          {status}
        </Text>
      </Box>
    </Box>
  );
}

/** A level's slider in a card, its name above it. */
function LevelCard({
  control,
  title,
  icon,
  controls,
  mute,
}: {
  control: ShellControl;
  title: string;
  icon: Icon;
  controls: ShellControls;
  /** A volume: its icon mutes it and unmutes it. */
  mute?: boolean;
}) {
  const level = controls.levels[control];
  if (!level?.available) return null;
  const Glyph = icon;
  const quiet = mute && silent(level);
  const shown = level.muted ? 0 : percent(level.level);
  return (
    <Box style={card}>
      <Text style={{ color: INK, fontSize: 13, fontWeight: "600", marginBottom: 6 }}>{title}</Text>
      <Box className="flex-row items-center" style={{ gap: 8 }}>
        {mute ? (
          <Button
            testID={`control-${control}-mute`}
            iconOnly
            icon={quiet ? RiVolumeMuteLine : Glyph}
            iconSize={16}
            accessibilityLabel={level.muted ? "Activar sonido" : "Silenciar"}
            appearance="plain"
            className="border-0 p-0"
            style={{ width: 24, height: 24 }}
            colors={{ background: "transparent", foreground: INK }}
            onPress={() => controls.setMuted?.(control, !level.muted)}
          />
        ) : (
          <Glyph width={16} height={16} fill={INK} />
        )}
        {/* One element for the assistive technology: the level as its value, a step up or down
            (a sixteenth, as the keys) as its actions. */}
        <Box
          className="flex-1"
          testID={`control-${control}`}
          accessible
          accessibilityRole="adjustable"
          accessibilityLabel={title}
          accessibilityValue={{ min: 0, max: 100, now: shown, text: `${shown} %` }}
          accessibilityActions={[{ name: "increment" }, { name: "decrement" }]}
          onAccessibilityAction={({ nativeEvent }) => {
            const step = nativeEvent.actionName === "increment" ? 1 / 16 : -1 / 16;
            controls.setLevel?.(control, Math.min(1, Math.max(0, level.level + step)));
          }}
        >
          <Slider
            value={shown}
            min={0}
            max={100}
            step={1}
            showTooltip={false}
            accessibilityLabel={title}
            minimumTrackTintColor={ACCENT}
            onValueChange={(value) => controls.setLevel?.(control, value / 100)}
          />
        </Box>
      </Box>
    </Box>
  );
}

/**
 * The Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx), from the top bar's
 * button, as a Mac's: Wi-Fi (on and off), Bluetooth (what it is), the appearance, the display's
 * and the keyboard's brightness, the sound, and the battery. What the session has not said, or
 * the machine has not, is left out. A press outside it closes it.
 */
export function ControlCenter({
  controls,
  dark,
  close,
  openSettings,
}: {
  controls: ShellControls;
  dark: boolean;
  close: () => void;
  openSettings: () => void;
}) {
  // Opened, it looks at the machine again: what another program changed shows.
  useEffect(() => {
    controls.refresh?.();
  }, []);
  const wifi = wifiLabel(controls.network);
  const bluetooth = bluetoothLabel(controls.bluetooth);
  const battery = batteryLabel(controls.power);
  return (
    <Box className="absolute inset-0">
      <PressArea
        testID="control-center-outside"
        className="absolute inset-0"
        accessible={false}
        onPress={close}
      />
      <Box
        testID="control-center"
        accessibilityRole="menu"
        accessibilityLabel="Centro de control"
        style={{
          position: "absolute",
          top: 34,
          right: 8,
          width: CONTROL_CENTER_WIDTH,
          padding: 10,
          gap: 10,
          borderRadius: 22,
          backgroundColor: "#eef1f6eb",
          boxShadow: "0 22px 70px #0b183a40, 0 0 0 .5px #0b183a1f",
        }}
      >
        <Box style={{ ...card, gap: 4 }}>
          {wifi !== undefined && (
            <Module
              testID="control-wifi"
              icon={RiWifiLine}
              title="Wi-Fi"
              status={wifi}
              on={wifiOn(controls.network)}
              onPress={() => controls.setWifi?.(!wifiOn(controls.network))}
            />
          )}
          {bluetooth !== undefined && (
            <Module
              testID="control-bluetooth"
              icon={RiBluetoothLine}
              title="Bluetooth"
              status={bluetooth}
              on={controls.bluetooth?.state === "on"}
            />
          )}
          <Module
            testID="control-appearance"
            icon={dark ? RiMoonLine : RiSunLine}
            title="Modo oscuro"
            status={dark ? "Activado" : "Desactivado"}
            on={dark}
            onPress={() => controls.setDark?.(!dark)}
          />
        </Box>
        <LevelCard control="display" title="Pantalla" icon={RiSunLine} controls={controls} />
        <LevelCard control="keyboard" title="Teclado" icon={RiKeyboardLine} controls={controls} />
        <LevelCard control="output" title="Sonido" icon={RiVolumeUpLine} controls={controls} mute />
        {battery !== undefined && (
          <Box
            style={card}
            className="flex-row items-center"
            accessible
            accessibilityLabel={`Batería: ${battery}`}
          >
            <RiBatteryLine width={16} height={16} fill={INK} />
            <Text style={{ color: INK, fontSize: 13, marginLeft: 8 }}>Batería</Text>
            <Text testID="control-battery" style={{ color: QUIET, fontSize: 12, marginLeft: "auto" }}>
              {battery}
            </Text>
          </Box>
        )}
        <Button
          appearance="plain"
          className="border-0"
          style={{ alignSelf: "flex-start", paddingHorizontal: 6, height: 26 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={() => {
            close();
            openSettings();
          }}
        >
          <Text style={{ color: INK, fontSize: 12 }}>Ajustes del sistema…</Text>
        </Button>
      </Box>
    </Box>
  );
}
