import React, { memo, useEffect } from "react";
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
import { RiNotificationOffFill } from "@oxy.so/bloom/icons/RiNotificationOffFill";
import { RiArrowRightSLine } from "@oxy.so/bloom/icons/RiArrowRightSLine";
import { Box, PressArea } from "../primitives";
import { ACCENT, FACE, INK, QUIET, card } from "./control-style";
import {
  batteryLabel,
  bluetoothLabel,
  percent,
  silent,
  wifiLabel,
  wifiOn,
} from "../controls";
import type { ShellControl, ShellControls } from "../model";
import { hasWifiView } from "../network";
import { WifiPanel } from "./WifiPanel";

/** What the Control Center shows: its modules, or Wi-Fi's view (docs/adr/0038-networks.mdx). */
export type ControlCenterPage = "modules" | "wifi";

export const CONTROL_CENTER_WIDTH = 320;
/** One of Bloom's icons. */
type Icon = typeof RiWifiLine;

/** A round button with a title and what it is, as a Mac's Control Center modules: on, it is
    the accent's. `onPress` absent, it only says (Bluetooth, which the session only looks at).
    `more`: its title and a chevron open its view (Wi-Fi's networks), a button of its own. */
function Module({
  icon,
  title,
  status,
  on,
  onPress,
  more,
  testID,
}: {
  icon: Icon;
  title: string;
  status: string;
  on: boolean;
  onPress?: () => void;
  more?: { label: string; open: () => void };
  testID: string;
}) {
  const Glyph = icon;
  const face = (
    <Box
      className="items-center justify-center rounded-full"
      style={{ width: 32, height: 32, backgroundColor: on ? ACCENT : FACE }}
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
      {more ? (
        <PressArea
          testID={`${testID}-more`}
          className="min-w-0 flex-1 flex-row items-center"
          accessibilityRole="button"
          accessibilityLabel={more.label}
          onPress={more.open}
        >
          <Box className="min-w-0 flex-1">
            <Text numberOfLines={1} style={{ color: INK, fontSize: 13, fontWeight: "600" }}>
              {title}
            </Text>
            <Text numberOfLines={1} style={{ color: QUIET, fontSize: 11 }}>
              {status}
            </Text>
          </Box>
          <RiArrowRightSLine width={16} height={16} fill={QUIET} />
        </PressArea>
      ) : (
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
      )}
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
        {/* One element for the assistive technology, Bloom's slider: the level as its value
            ("50 %"), a sixteenth up or down (as the keys) as its increment and decrement, which
            the arrows step with the keyboard on it. A drag lands on the keys' finest step, a
            quarter of a sixteenth. */}
        <Box className="flex-1">
          <Slider
            testID={`control-${control}`}
            value={level.muted ? 0 : level.level * 100}
            min={0}
            max={100}
            step={100 / 64}
            keyboardStep={100 / 16}
            showTooltip={false}
            formatValue={(value) => `${percent(value / 100)} %`}
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
 * the machine has not, is left out. A press outside it closes it. Do Not Disturb
 * (docs/adr/0026-notifications.mdx) is a module of its own below them, where the session has
 * notifications. Wi-Fi's module opens Wi-Fi's view in its place (docs/adr/0038-networks.mdx), as
 * the top bar's network item does, where the session told the networks and the machine has Wi-Fi.
 */
export const ControlCenter = memo(function ControlCenter({
  controls,
  dark,
  close,
  openSettings,
  doNotDisturb,
  page = "modules",
  showPage,
}: {
  controls: ShellControls;
  dark: boolean;
  close: () => void;
  openSettings: () => void;
  /** What it shows, and how it moves between its modules and Wi-Fi's view. */
  page?: ControlCenterPage;
  showPage?: (page: ControlCenterPage) => void;
  /** Do Not Disturb as the session says it is, and its switch (a change of the preference). */
  doNotDisturb?: { on: boolean; set: (enabled: boolean) => void };
}) {
  // Opened, it looks at the machine again: what another program changed shows.
  useEffect(() => {
    controls.refresh?.();
  }, []);
  const wifi = wifiLabel(controls.network);
  const bluetooth = bluetoothLabel(controls.bluetooth);
  const battery = batteryLabel(controls.power);
  const networks = controls.networks;
  const wifiView = hasWifiView(networks) && !!showPage;
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
        {wifiView && page === "wifi" ? (
          <WifiPanel state={networks} controls={controls} back={() => showPage?.("modules")} close={close} />
        ) : (
          <>
            <Box style={{ ...card, gap: 4 }}>
              {wifi !== undefined && (
                <Module
                  testID="control-wifi"
                  icon={RiWifiLine}
                  title="Wi-Fi"
                  status={wifi}
                  on={wifiOn(controls.network)}
                  onPress={() => controls.setWifi?.(!wifiOn(controls.network))}
                  more={wifiView ? { label: "Redes Wi-Fi", open: () => showPage?.("wifi") } : undefined}
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
            {doNotDisturb && (
              <Box style={card}>
                <Module
                  testID="control-do-not-disturb"
                  icon={RiNotificationOffFill}
                  title="No molestar"
                  status={doNotDisturb.on ? "Activado" : "Desactivado"}
                  on={doNotDisturb.on}
                  onPress={() => doNotDisturb.set(!doNotDisturb.on)}
                />
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
          </>
        )}
      </Box>
    </Box>
  );
});
