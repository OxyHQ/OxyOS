import React, { memo, useCallback, useEffect, useMemo } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { RiWifiLine } from "@oxy.so/bloom/icons/RiWifiLine";
import { RiBluetoothLine } from "@oxy.so/bloom/icons/RiBluetoothLine";
import { RiMoonLine } from "@oxy.so/bloom/icons/RiMoonLine";
import { RiSunLine } from "@oxy.so/bloom/icons/RiSunLine";
import { RiVolumeUpLine } from "@oxy.so/bloom/icons/RiVolumeUpLine";
import { RiKeyboardLine } from "@oxy.so/bloom/icons/RiKeyboardLine";
import { RiBatteryLine } from "@oxy.so/bloom/icons/RiBatteryLine";
import { RiNotificationOffFill } from "@oxy.so/bloom/icons/RiNotificationOffFill";
import { RiArrowRightSLine } from "@oxy.so/bloom/icons/RiArrowRightSLine";
import { Box, PressArea } from "../primitives";
import { FACE, INK, QUIET, card } from "./control-style";
import { batteryLabel, bluetoothLabel, wifiLabel, wifiOn } from "../controls";
import type { ShellControl, ShellControls, ShellLevel } from "../model";
import { hasWifiView } from "../network";
import { hasSoundView, selectedOutput, soundDeviceName } from "../sound";
import { bluetoothSummary, bluetoothSwitchable, hasBluetoothView } from "../bluetooth";
import { WifiPanel } from "./WifiPanel";
import { SoundPanel } from "./SoundPanel";
import { BluetoothPanel } from "./BluetoothPanel";
import { LevelSlider, type Icon } from "./LevelSlider";
import { useShellMessages } from "../useShellMessages";
import { BloomColorScope, useReducedTransparency, useTheme } from "@oxy.so/bloom/theme";
import { useAccent } from "./accent";
import { glass } from "./glass";

/** What the Control Center shows: its modules, or Wi-Fi's (docs/adr/0038-networks.mdx), Sound's
    (docs/adr/0056-sound-devices.mdx) or Bluetooth's (docs/adr/0055-bluetooth.mdx) view. */
export type ControlCenterPage = "modules" | "wifi" | "sound" | "bluetooth";

export const CONTROL_CENTER_WIDTH = 320;

/** A round button with a title and what it is, as a Mac's Control Center modules: on, it is
    the accent's. `onPress` absent, it only says (Bluetooth, where the session told only its
    state). `more`: its title and a chevron open its view (Wi-Fi's networks, Bluetooth's
    devices), a button of its own, whose value is the status shown. Each module is said once,
    as VoiceOver says a Mac's: its switch by its title, its state and `value` (what the status
    says beyond on or off: the network joined), and its status text, which says no more than
    that, is not read again after it. Memoized: a module renders when what it shows changes. */
const Module = memo(function Module({
  icon,
  title,
  status,
  value,
  on,
  onPress,
  more,
  testID,
}: {
  icon: Icon;
  title: string;
  status: string;
  /** The switch's value, said after its title. */
  value?: string;
  on: boolean;
  onPress?: () => void;
  more?: { label: string; open: () => void };
  testID: string;
}) {
  const Glyph = icon;
  const accent = useTheme().colors.primary;
  const face = (
    <Box
      className="items-center justify-center rounded-full"
      style={{ width: 32, height: 32, backgroundColor: on ? accent : FACE }}
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
          accessibilityValue={value ? { text: value } : undefined}
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
          accessibilityValue={{ text: status }}
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
          // Its switch says it; without one, it is one element, its title and its status.
          accessible={!onPress}
          accessibilityLabel={onPress ? undefined : title}
          accessibilityValue={onPress ? undefined : { text: status }}
          importantForAccessibility={onPress ? "no-hide-descendants" : undefined}
          accessibilityElementsHidden={!!onPress}
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
});

/** A level's slider in a card, its name above it. `more`: a chevron after the slider opens its
    view (the Sound card's outputs), a button of its own whose value is what it chose (the output
    played to), as a Mac's Sound module. Given its own level, it renders when that changes and no
    other card does. */
const LevelCard = memo(function LevelCard({
  control,
  title,
  icon,
  level,
  setLevel,
  setMuted,
  mute,
  more,
}: {
  control: ShellControl;
  title: string;
  icon: Icon;
  level: ShellLevel | undefined;
  setLevel: ShellControls["setLevel"];
  setMuted: ShellControls["setMuted"];
  /** A volume: its icon mutes it and unmutes it. */
  mute?: boolean;
  more?: { label: string; value?: string; open: () => void };
}) {
  if (!level?.available) return null;
  return (
    <Box style={card}>
      {/* The slider is named by it: the title is not read again as text. */}
      <Text
        importantForAccessibility="no-hide-descendants"
        accessibilityElementsHidden
        style={{ color: INK, fontSize: 13, fontWeight: "600", marginBottom: 6 }}
      >
        {title}
      </Text>
      <LevelSlider
        control={control}
        title={title}
        icon={icon}
        level={level}
        setLevel={setLevel}
        setMuted={setMuted}
        mute={mute}
      >
        {more && (
          <PressArea
            testID={`control-${control}-more`}
            className="items-center justify-center"
            style={{ width: 24, height: 24 }}
            accessibilityRole="button"
            accessibilityLabel={more.label}
            accessibilityValue={more.value ? { text: more.value } : undefined}
            onPress={more.open}
          >
            <RiArrowRightSLine width={16} height={16} fill={QUIET} />
          </PressArea>
        )}
      </LevelSlider>
    </Box>
  );
});

/**
 * The Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx), from the top bar's
 * button, as a Mac's: Wi-Fi (on and off), Bluetooth (what it is), the appearance, the display's
 * and the keyboard's brightness, the sound, and the battery. What the session has not said, or
 * the machine has not, is left out. A press outside it closes it. Do Not Disturb
 * (docs/adr/0026-notifications.mdx) is a module of its own below them, where the session has
 * notifications. Wi-Fi's module opens Wi-Fi's view in its place (docs/adr/0038-networks.mdx), as
 * the top bar's network item does, where the session told the networks and the machine has Wi-Fi;
 * the Sound card's chevron opens Sound's view, its outputs to choose from
 * (docs/adr/0056-sound-devices.mdx), where the session told the sound and it has an output.
 * Bluetooth's module turns Bluetooth on and off and opens Bluetooth's view in its place
 * (docs/adr/0055-bluetooth.mdx), where the session told Bluetooth and the machine has an adapter;
 * elsewhere it only says Bluetooth's state.
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
  /** What it shows, and how it moves between its modules and Wi-Fi's, Sound's or Bluetooth's
      view. */
  page?: ControlCenterPage;
  showPage?: (page: ControlCenterPage) => void;
  /** Do Not Disturb as the session says it is, and its switch (a change of the preference). */
  doNotDisturb?: { on: boolean; set: (enabled: boolean) => void };
}) {
  const reduced = useReducedTransparency();
  const accentPreset = useAccent();
  // Opened, it looks at the machine again: what another program changed shows.
  useEffect(() => {
    controls.refresh?.();
  }, []);
  const messages = useShellMessages();
  const words = messages.controlCenter;
  const wifi = wifiLabel(controls.network, messages);
  // Bluetooth's state as the session said it to the Control Center, else from Bluetooth's own.
  const devices = controls.bluetoothDevices;
  const toldBluetooth = controls.bluetooth ?? bluetoothSummary(devices);
  const bluetooth = bluetoothLabel(toldBluetooth, messages);
  const bluetoothOn = toldBluetooth?.state === "on";
  const bluetoothView = hasBluetoothView(devices) && !!showPage;
  const battery = batteryLabel(controls.power, messages);
  const networks = controls.networks;
  const wifiView = hasWifiView(networks) && !!showPage;
  const sound = controls.sound;
  const soundView = hasSoundView(sound) && !!showPage;
  const output = selectedOutput(sound);
  const outputName = output && soundDeviceName(output, messages);
  // What the modules and cards are given stays the same object while what it says does, so that
  // each renders when what it shows changes: a volume dragged renders its slider alone.
  const { setWifi, setBluetooth, setDark, setLevel, setMuted } = controls;
  const wifiIsOn = wifiOn(controls.network);
  const toggleWifi = useCallback(() => setWifi?.(!wifiIsOn), [setWifi, wifiIsOn]);
  const bluetoothSwitches = bluetoothView && bluetoothSwitchable(devices) && !!setBluetooth;
  const toggleBluetooth = useCallback(() => setBluetooth?.(!bluetoothOn), [setBluetooth, bluetoothOn]);
  const toggleDark = useCallback(() => setDark?.(!dark), [setDark, dark]);
  const setDoNotDisturb = doNotDisturb?.set;
  const doNotDisturbOn = doNotDisturb?.on ?? false;
  const toggleDoNotDisturb = useCallback(() => setDoNotDisturb?.(!doNotDisturbOn), [setDoNotDisturb, doNotDisturbOn]);
  const wifiMore = useMemo(
    () => (wifiView ? { label: words.wifiNetworks, open: () => showPage?.("wifi") } : undefined),
    [wifiView, words, showPage],
  );
  const bluetoothMore = useMemo(
    () => (bluetoothView ? { label: words.bluetoothDevices, open: () => showPage?.("bluetooth") } : undefined),
    [bluetoothView, words, showPage],
  );
  const soundMore = useMemo(
    () =>
      soundView ? { label: words.outputDevices, value: outputName, open: () => showPage?.("sound") } : undefined,
    [soundView, words, outputName, showPage],
  );
  // In the person's accent, as a Mac's Control Center draws a control that is on.
  return (
    <BloomColorScope colorPreset={accentPreset} asChild>
    <Box className="absolute inset-0">
      <PressArea
        testID="control-center-outside"
        className="absolute inset-0"
        accessible={false}
        onPress={close}
      />
      <Box
        testID="control-center"
        // A panel of controls, as a Mac's Control Center window, not a menu: it holds no items.
        role="dialog"
        accessibilityLabel={messages.common.controlCenter}
        style={{
          position: "absolute",
          top: 34,
          right: 8,
          width: CONTROL_CENTER_WIDTH,
          padding: 10,
          gap: 10,
          borderRadius: 22,
          backgroundColor: glass("#eef1f6eb", reduced),
          boxShadow: "0 22px 70px #0b183a40, 0 0 0 .5px #0b183a1f",
        }}
      >
        {wifiView && page === "wifi" ? (
          <WifiPanel state={networks} controls={controls} back={() => showPage?.("modules")} close={close} />
        ) : soundView && page === "sound" ? (
          <SoundPanel state={sound} controls={controls} back={() => showPage?.("modules")} close={close} />
        ) : bluetoothView && page === "bluetooth" ? (
          <BluetoothPanel state={devices} controls={controls} back={() => showPage?.("modules")} close={close} />
        ) : (
          <>
            <Box style={{ ...card, gap: 4 }}>
              {wifi !== undefined && (
                <Module
                  testID="control-wifi"
                  icon={RiWifiLine}
                  title="Wi-Fi"
                  status={wifi}
                  on={wifiIsOn}
                  // The network joined is said by "Redes Wi-Fi" where it shows; else by the
                  // switch, while Wi-Fi is on (off, its state says it).
                  value={!wifiView && wifiIsOn ? wifi : undefined}
                  onPress={toggleWifi}
                  more={wifiMore}
                />
              )}
              {bluetooth !== undefined && (
                <Module
                  testID="control-bluetooth"
                  icon={RiBluetoothLine}
                  title="Bluetooth"
                  status={bluetooth}
                  on={bluetoothOn}
                  // A switch where the session told Bluetooth and it can move (not blocked by the
                  // machine's switch); its devices said by "Bluetooth Devices".
                  onPress={bluetoothSwitches ? toggleBluetooth : undefined}
                  more={bluetoothMore}
                />
              )}
              <Module
                testID="control-appearance"
                icon={dark ? RiMoonLine : RiSunLine}
                title={words.darkMode}
                status={dark ? messages.common.on : messages.common.off}
                on={dark}
                onPress={toggleDark}
              />
            </Box>
            <LevelCard
              control="display"
              title={words.display}
              icon={RiSunLine}
              level={controls.levels.display}
              setLevel={setLevel}
              setMuted={setMuted}
            />
            <LevelCard
              control="keyboard"
              title={words.keyboard}
              icon={RiKeyboardLine}
              level={controls.levels.keyboard}
              setLevel={setLevel}
              setMuted={setMuted}
            />
            <LevelCard
              control="output"
              title={words.sound}
              icon={RiVolumeUpLine}
              level={controls.levels.output}
              setLevel={setLevel}
              setMuted={setMuted}
              mute
              more={soundMore}
            />
            {battery !== undefined && (
              <Box
                style={card}
                className="flex-row items-center"
                accessible
                // A level indicator, as the top bar's battery.
                role="meter"
                accessibilityLabel={messages.topBar.battery}
                accessibilityValue={{ min: 0, max: 100, now: controls.power?.percent ?? 0, text: battery }}
              >
                <RiBatteryLine width={16} height={16} fill={INK} />
                <Text style={{ color: INK, fontSize: 13, marginLeft: 8 }}>{words.battery}</Text>
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
                  title={messages.common.doNotDisturb}
                  status={doNotDisturbOn ? messages.common.on : messages.common.off}
                  on={doNotDisturbOn}
                  onPress={toggleDoNotDisturb}
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
              <Text style={{ color: INK, fontSize: 12 }}>{words.systemSettings}</Text>
            </Button>
          </>
        )}
      </Box>
    </Box>
    </BloomColorScope>
  );
});
