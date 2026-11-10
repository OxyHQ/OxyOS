import React, { memo, useState } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Switch } from "@oxy.so/bloom/switch";
import { SpinnerIcon } from "@oxy.so/bloom/loading";
import { SubtleHover } from "@oxy.so/bloom/subtle-hover";
import { RiArrowLeftSLine } from "@oxy.so/bloom/icons/RiArrowLeftSLine";
import { Box, PressArea, Scroll } from "../primitives";
import type { ShellControls } from "../model";
import {
  bluetoothEmptyText,
  bluetoothSwitchable,
  deviceBattery,
  deviceLabel,
  pairedDevices,
  type BluetoothDevice,
  type BluetoothState,
} from "../bluetooth";
import { FACE, INK, QUIET, card } from "./control-style";
import { useTheme } from "@oxy.so/bloom/theme";
import { BluetoothGlyph, DeviceBattery } from "./BluetoothGlyph";
import { WIFI_LIST_MAX_HEIGHT, WIFI_ROW_HEIGHT } from "./WifiPanel";
import { useShellMessages } from "../useShellMessages";

/** A paired device: its kind's glyph on a round face (the accent's while connected), its name, its
    charge while connected, and progress while the session connects or disconnects it. Pressed, it
    is connected, or disconnected when it is; while busy, pressing it does nothing. */
function DeviceRow({
  device,
  connect,
  disconnect,
}: {
  device: BluetoothDevice;
  connect: (id: string) => void;
  disconnect: (id: string) => void;
}) {
  const messages = useShellMessages();
  const [hovered, setHovered] = useState(false);
  const accent = useTheme().colors.primary;
  const busy = device.busy !== null;
  const battery = deviceBattery(device, messages);
  return (
    <PressArea
      testID={`bluetooth-device-${device.id}`}
      accessibilityRole="button"
      accessibilityLabel={deviceLabel(device, messages)}
      accessibilityState={{ selected: device.connected, busy }}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      onPress={() => {
        if (busy) return;
        if (device.connected) disconnect(device.id);
        else connect(device.id);
      }}
      style={{
        height: WIFI_ROW_HEIGHT,
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
        paddingHorizontal: 6,
        borderRadius: 8,
      }}
    >
      <SubtleHover native active={hovered} style={{ borderRadius: 8 }} />
      <Box
        className="items-center justify-center rounded-full"
        style={{ width: 26, height: 26, backgroundColor: device.connected ? accent : FACE }}
      >
        <BluetoothGlyph kind={device.kind} size={15} color={device.connected ? "#ffffff" : INK} />
      </Box>
      <Text numberOfLines={1} style={{ flex: 1, color: INK, fontSize: 13 }}>
        {device.name}
      </Text>
      {busy && <SpinnerIcon size={14} color={QUIET} />}
      {!busy && battery !== undefined && device.battery !== null && (
        <Box className="flex-row items-center" style={{ gap: 5 }}>
          <Text style={{ color: QUIET, fontSize: 11, fontVariant: ["tabular-nums"] }}>{battery}</Text>
          <DeviceBattery level={device.battery} color={device.battery <= 10 ? "#d93025" : INK} faint={QUIET} />
        </Box>
      )}
    </PressArea>
  );
}

/**
 * The Control Center's Bluetooth view (docs/adr/0055-bluetooth.mdx), as a Mac's: in place of the
 * modules, Bluetooth's switch; with Bluetooth on, the paired devices, each with its glyph (on the
 * accent's face while connected) and its charge, connected or disconnected as it is pressed;
 * "Bluetooth Settings…" at its foot, where devices are found and paired. It never discovers: the
 * session pairs and connects, asking what a pairing needs and saying what failed in its system
 * sheets.
 */
export const BluetoothPanel = memo(function BluetoothPanel({
  state,
  controls,
  back,
  close,
}: {
  state: BluetoothState;
  controls: ShellControls;
  /** Back to the modules. */
  back: () => void;
  /** Closes the Control Center (Settings opened). */
  close: () => void;
}) {
  const messages = useShellMessages();
  const words = messages.bluetooth;
  const connect = (id: string) => controls.connectBluetooth?.(id);
  const disconnect = (id: string) => controls.disconnectBluetooth?.(id);
  const empty = bluetoothEmptyText(state, messages);
  return (
    <Box testID="control-bluetooth-detail" style={{ gap: 10 }}>
      <Box className="flex-row items-center" style={{ height: 32, gap: 6, paddingRight: 4 }}>
        <Button
          testID="bluetooth-back"
          iconOnly
          icon={RiArrowLeftSLine}
          iconSize={18}
          accessibilityLabel={words.back}
          appearance="plain"
          className="border-0 p-0"
          style={{ width: 28, height: 28 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={back}
        />
        <Text accessibilityRole="header" style={{ flex: 1, color: INK, fontSize: 15, fontWeight: "600" }}>
          Bluetooth
        </Text>
        <Switch
          testID="bluetooth-switch"
          size="sm"
          accessibilityLabel="Bluetooth"
          checked={!!state.adapter?.powered}
          disabled={!bluetoothSwitchable(state) || !controls.setBluetooth}
          onCheckedChange={(on) => controls.setBluetooth?.(on)}
        />
      </Box>
      {empty !== undefined ? (
        <Box style={{ ...card, alignItems: "center", paddingVertical: 18 }}>
          <Text testID="bluetooth-empty" style={{ color: QUIET, fontSize: 13, textAlign: "center" }}>
            {empty}
          </Text>
        </Box>
      ) : (
        <Scroll style={{ maxHeight: WIFI_LIST_MAX_HEIGHT }} contentContainerStyle={{ gap: 10 }}>
          <Box style={{ ...card, paddingHorizontal: 6, paddingVertical: 8 }}>
            <Text
              accessibilityRole="header"
              style={{ color: QUIET, fontSize: 11, fontWeight: "600", paddingHorizontal: 6, marginBottom: 4 }}
            >
              {words.devices}
            </Text>
            {pairedDevices(state).map((device) => (
              <DeviceRow key={device.id} device={device} connect={connect} disconnect={disconnect} />
            ))}
          </Box>
        </Scroll>
      )}
      {controls.openBluetoothSettings && (
        <Button
          testID="bluetooth-settings"
          appearance="plain"
          className="border-0"
          style={{ alignSelf: "flex-start", paddingHorizontal: 6, height: 26 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={() => {
            close();
            controls.openBluetoothSettings?.();
          }}
        >
          <Text style={{ color: INK, fontSize: 12 }}>{words.settings}</Text>
        </Button>
      )}
    </Box>
  );
});
