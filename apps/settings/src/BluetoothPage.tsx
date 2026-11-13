/**
 * Settings › Bluetooth (docs/adr/0055-bluetooth.mdx), as a Mac's: the switch, and the name others
 * see this computer as while the page is open; My Devices, each with its glyph, its state, its
 * battery when connected, a Connect or Disconnect button and its ⓘ sheet (its name, address, type
 * and battery; forgetting it, once confirmed); and Nearby Devices, each paired and connected by its
 * Connect button. Settings never asks or shows a pairing code: the session asks it in the shell's
 * own sheets, and says there why a connection failed. Settings says only a forget's or a rename's
 * failure. The page holds discovery while it is shown and Bluetooth is on (`discover(true)` as it
 * opens, `discover(false)` as it goes); the session keeps it running meanwhile, so nothing here
 * polls.
 */
import * as React from "react";
import { AccessibilityInfo, View } from "react-native";
import Svg, { Path, Rect } from "react-native-svg";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { Button } from "@oxy.so/bloom/button";
import { Dialog } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import {
  RiBluetoothLine,
  RiComputerLine,
  RiGamepadLine,
  RiHeadphoneLine,
  RiInformationLine,
  RiKeyboardLine,
  RiMouseLine,
  RiSmartphoneLine,
  RiSpeakerLine,
} from "@oxy.so/bloom/icons";
import { Loading } from "@oxy.so/bloom/loading";
import { Switch } from "@oxy.so/bloom/switch";
import { TextField, TextFieldInput } from "@oxy.so/bloom/text-field";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  batteryText,
  bluetoothView,
  connectionAction,
  connectionChanges,
  deviceLabel,
  failureNeedsNote,
  failureOf,
  isOn,
  kindOf,
  myDevices,
  nearbyDevices,
  renameTo,
  statusLabel,
  type BluetoothActions,
  type BluetoothDevice,
  type BluetoothKind,
} from "./bluetooth";
import type { SettingsActions, SettingsSnapshot } from "./model";
import type { Messages } from "./messages";

type Props = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };
type Words = Messages["bluetooth"];

const PAGE = { gap: 20 } as const;
const TRAILING = { flexDirection: "row", alignItems: "center", gap: 10 } as const;
/** A device's round face, as a Mac's Bluetooth list draws one. */
const FACE = 28;

/** A glyph drawn as Bloom's icons are: its box and its fill. */
type Glyph = React.ComponentType<{ width?: number; height?: number; fill?: string }>;

/** A trackpad, which Bloom's icons have none of: a rounded pad and the line of its click. */
function TrackpadGlyph({ width = 16, height = 16, fill = "currentColor" }: { width?: number; height?: number; fill?: string }) {
  return (
    <Svg width={width} height={height} viewBox="0 0 24 24">
      <Rect x="3" y="4" width="18" height="16" rx="3" stroke={fill} strokeWidth="2" fill="none" />
      <Path d="M3 15.5h18" stroke={fill} strokeWidth="2" />
    </Svg>
  );
}

const GLYPHS: Record<BluetoothKind, Glyph> = {
  headphones: RiHeadphoneLine,
  speaker: RiSpeakerLine,
  keyboard: RiKeyboardLine,
  mouse: RiMouseLine,
  trackpad: TrackpadGlyph,
  phone: RiSmartphoneLine,
  gamepad: RiGamepadLine,
  computer: RiComputerLine,
  other: RiBluetoothLine,
};

/** A device's glyph by its kind in a round face: the accent's while connected, as the shell's
    Control Center draws it. Decorative: its row says what it is. */
function DeviceFace({ kind, connected }: { kind: BluetoothKind; connected: boolean }) {
  const theme = useTheme();
  const Icon = GLYPHS[kindOf(kind)];
  return (
    <View
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
      style={{
        width: FACE,
        height: FACE,
        borderRadius: FACE / 2,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: connected ? theme.colors.primary : theme.colors.backgroundTertiary,
      }}
    >
      <Icon width={16} height={16} fill={connected ? theme.colors.primaryForeground : theme.colors.textSecondary} />
    </View>
  );
}

/** A battery's charge as a Mac's list shows it: a small battery filled to its level, red when low,
    and its percentage, read as one ("Battery 80%"). */
function Battery({ device, words, locale }: { device: BluetoothDevice; words: Words; locale: string }) {
  const theme = useTheme();
  const text = batteryText(device, locale);
  if (!text || device.battery === null) return null;
  const level = Math.min(100, Math.max(0, device.battery));
  const color = level <= 20 ? theme.colors.error : theme.colors.text;
  return (
    <View accessible accessibilityLabel={words.battery(text)} style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
      <Svg width={22} height={12} viewBox="0 0 22 12">
        <Rect x="0.75" y="0.75" width="18" height="10.5" rx="2.5" stroke={theme.colors.textSecondary} strokeWidth="1.5" fill="none" />
        <Rect x="2.5" y="2.5" width={Math.max(1, (14.5 * level) / 100)} height="7" rx="1.25" fill={color} />
        <Path d="M20.5 4.25v3.5" stroke={theme.colors.textSecondary} strokeWidth="1.5" strokeLinecap="round" />
      </Svg>
      <Text variant="body-2-regular" style={{ color: theme.colors.textSecondary }}>
        {text}
      </Text>
    </View>
  );
}

/** A section's footer, as Bloom's compact sections draw theirs (aligned with their rows' text, in
    the secondary label's colour), with a name scenarios can find it by. */
function SectionFooter({ text, testID }: { text: string; testID: string }) {
  const theme = useTheme();
  return (
    <Text
      variant="caption-1-regular"
      style={{ paddingHorizontal: 10, paddingTop: 6, color: theme.colors.textSecondary }}
      testID={testID}
    >
      {text}
    </Text>
  );
}

/** The Connect or Disconnect button at a device's row's end, or a spinner while the session's call
    on it is under way. */
function ConnectionControl({
  device,
  words,
  onConnect,
  onDisconnect,
}: {
  device: BluetoothDevice;
  words: Words;
  onConnect: (device: BluetoothDevice) => void;
  onDisconnect: (device: BluetoothDevice) => void;
}) {
  const action = connectionAction(device);
  if (action === null) {
    return (
      <Loading
        size="sm"
        accessibilityLabel={device.busy === "connecting" ? words.connectingLabel(device.name) : words.disconnecting}
        testID={`bluetooth-busy-${device.id}`}
      />
    );
  }
  const connect = action === "connect";
  return (
    <Button
      size="sm"
      appearance="outline"
      tone="neutral"
      accessibilityLabel={connect ? words.connectLabel(device.name) : words.disconnectLabel(device.name)}
      testID={`bluetooth-${action}-${device.id}`}
      onPress={() => (connect ? onConnect(device) : onDisconnect(device))}
    >
      {connect ? words.connect : words.disconnect}
    </Button>
  );
}

/**
 * A paired device's ⓘ sheet, as a Mac's: its name, which renames it when submitted (an empty name
 * gives it back its own) and when the sheet is done; its address, type, battery and state; connect
 * or disconnect; forget, confirmed first.
 */
function DeviceSheet({
  device,
  open,
  words,
  locale,
  act,
  onClose,
  onForget,
  onConnect,
  onDisconnect,
  onFailed,
}: {
  device: BluetoothDevice | null;
  open: boolean;
  words: Words;
  locale: string;
  act: BluetoothActions;
  onClose: () => void;
  onForget: (device: BluetoothDevice) => void;
  onConnect: (device: BluetoothDevice) => void;
  onDisconnect: (device: BluetoothDevice) => void;
  onFailed: (title: string, reason: ReturnType<typeof failureOf>) => void;
}) {
  const [typed, setTyped] = React.useState(device?.name ?? "");
  const rename = async () => {
    if (!device) return;
    const name = renameTo(typed, device);
    if (name === null) return;
    const reason = await act.rename(device.id, name).then((renamed) => {
      setTyped(renamed);
      return null;
    }, failureOf);
    if (failureNeedsNote("rename", reason)) {
      setTyped(device.name);
      onFailed(words.couldNotRename(device.name), reason);
    }
  };
  const name = device?.name ?? "";
  const battery = device ? batteryText(device, locale) : "";
  const action = device ? connectionAction(device) : null;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={name}
      label={words.info(name)}
      actions={[
        {
          label: words.done,
          color: "default",
          onPress: () => {
            void rename();
            onClose();
          },
          testID: "bluetooth-info-done",
        },
      ]}
      testID="bluetooth-info-sheet"
    >
      {device ? (
        <View style={{ gap: 16 }}>
          <GroupedSection>
            <GroupedSectionItem
              title={words.name}
              rightElement={
                <View style={{ width: 220 }}>
                  <TextField size="sm">
                    <TextFieldInput
                      label={words.name}
                      value={typed}
                      onChangeText={setTyped}
                      placeholder={device.name}
                      autoCorrect={false}
                      returnKeyType="done"
                      onSubmitEditing={() => {
                        void rename();
                      }}
                      testID="bluetooth-info-name"
                    />
                  </TextField>
                </View>
              }
            />
            <GroupedSectionItem title={words.address} value={device.address} testID="bluetooth-info-address" />
            <GroupedSectionItem title={words.type} value={words.kinds[kindOf(device.kind)]} testID="bluetooth-info-type" />
            {battery ? (
              <GroupedSectionItem title={words.batteryLevel} value={battery} testID="bluetooth-info-battery" />
            ) : null}
            <GroupedSectionItem title={words.status} value={statusLabel(device, words)} testID="bluetooth-info-status" />
          </GroupedSection>
          <GroupedSection>
            <GroupedSectionItem
              title={action === "disconnect" ? words.disconnect : action === "connect" ? words.connect : statusLabel(device, words)}
              onPress={
                action === "connect"
                  ? () => onConnect(device)
                  : action === "disconnect"
                    ? () => onDisconnect(device)
                    : undefined
              }
              disabled={action === null}
              accessibilityState={{ busy: action === null }}
              showChevron={false}
              accessibilityRole="button"
              testID={action === "disconnect" ? "bluetooth-info-disconnect" : "bluetooth-info-connect"}
            />
            <GroupedSectionItem
              title={words.forgetItem}
              destructive
              showChevron={false}
              accessibilityRole="button"
              onPress={() => onForget(device)}
              testID="bluetooth-info-forget"
            />
          </GroupedSection>
        </View>
      ) : null}
    </Dialog>
  );
}

/** A dialog's content kept while it closes, so its words do not vanish under its animation. */
function useHeld<T>(): [T | null, boolean, (value: T) => void, () => void] {
  const [held, setHeld] = React.useState<{ value: T | null; open: boolean }>({ value: null, open: false });
  const show = React.useCallback((value: T) => setHeld({ value, open: true }), []);
  const hide = React.useCallback(() => setHeld((current) => ({ ...current, open: false })), []);
  return [held.value, held.open, show, hide];
}

export function BluetoothPage({ snapshot, actions, strings, locale }: Props) {
  const words = strings.bluetooth;
  const state = snapshot.bluetooth;
  const act = actions.bluetooth;
  const view = bluetoothView(state);
  const on = view.kind === "adapter" && isOn(view.adapter);
  // The sheet opened, by device; a serial so each opening starts from the device's name.
  const [info, infoOpen, showInfo, hideInfo] = useHeld<{ id: string; serial: number }>();
  const [forgetting, forgetOpen, askForget, hideForget] = useHeld<BluetoothDevice>();
  const [note, noteOpen, showNote, hideNote] = useHeld<{ title: string; description: string }>();
  const opened = React.useRef(0);
  // Discovery is held while the page is shown and Bluetooth is on, and let go as it goes or turns
  // off; the session discovers (and is discoverable) meanwhile, and ends it if Settings quits.
  React.useEffect(() => {
    if (!on || !act) return;
    act.discover(true).catch(() => {});
    return () => {
      act.discover(false).catch(() => {});
    };
  }, [on, act]);
  // A device connected or disconnected is said once, as VoiceOver hears a Mac's: the button that
  // asked it gives way to a spinner, then the other button, so nothing focused would say it.
  const devices = state?.devices;
  const before = React.useRef(devices);
  React.useEffect(() => {
    const changes = connectionChanges(before.current, devices);
    before.current = devices;
    for (const change of changes) {
      AccessibilityInfo.announceForAccessibility(
        change.connected ? words.connectedTo(change.name) : words.disconnectedFrom(change.name),
      );
    }
  }, [devices]);
  const shown = info ? devices?.find((device) => device.id === info.id && device.paired) ?? null : null;
  // A device forgotten (here or by the shell) takes its sheet with it.
  React.useEffect(() => {
    if (infoOpen && !shown) hideInfo();
  }, [infoOpen, shown, hideInfo]);
  if (view.kind === "unavailable" || !state || !act) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailable} titleNumberOfLines={0} testID="bluetooth-unavailable" />
      </GroupedSection>
    );
  }
  if (view.kind === "noAdapter") {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.noAdapter} titleNumberOfLines={0} testID="bluetooth-no-adapter" />
      </GroupedSection>
    );
  }
  const adapter = view.adapter;
  const mine = myDevices(state.devices);
  const nearby = nearbyDevices(state.devices);
  const say = (title: string, reason: ReturnType<typeof failureOf>) => {
    const shownNote = { title, description: words.failures[reason ?? "failed"] };
    showNote(shownNote);
    // Said as it shows: the note's dialog does not take the screen reader's focus.
    AccessibilityInfo.announceForAccessibility(`${shownNote.title}. ${shownNote.description}`);
  };
  // A connection's failure is the session's to say, in its own sheet.
  const connect = (device: BluetoothDevice) => {
    act.connect(device.id).catch(() => {});
  };
  const disconnect = (device: BluetoothDevice) => {
    act.disconnect(device.id).catch(() => {});
  };
  const forget = async (device: BluetoothDevice) => {
    const reason = await act.forget(device.id).then(() => null, failureOf);
    if (failureNeedsNote("forget", reason)) say(words.couldNotForget(device.name), reason);
  };
  const footer = adapter.blocked
    ? { text: words.blocked, testID: "bluetooth-blocked" }
    : adapter.powered
      ? { text: words.discoverable(adapter.name), testID: "bluetooth-discoverable" }
      : { text: words.off, testID: "bluetooth-off" };
  return (
    <View style={PAGE}>
      <View>
        <GroupedSection>
          <GroupedSectionItem
            title={words.bluetooth}
            icon={<RiBluetoothLine />}
            testID="bluetooth"
            rightElement={
              <Switch
                size="xs"
                checked={on}
                disabled={adapter.blocked}
                onCheckedChange={(checked) => {
                  act.setPower(checked).catch(() => {});
                }}
                accessibilityLabel={words.bluetooth}
                testID="bluetooth-switch"
              />
            }
          />
        </GroupedSection>
        <SectionFooter text={footer.text} testID={footer.testID} />
      </View>
      {on && mine.length > 0 ? (
        <GroupedSection title={words.myDevices}>
          {mine.map((device) => (
            <GroupedSectionItem
              key={device.id}
              title={device.name}
              description={statusLabel(device, words)}
              icon={<DeviceFace kind={device.kind} connected={device.connected} />}
              testID={`bluetooth-device-${device.id}`}
              accessibilityLabel={deviceLabel(device, words, locale)}
              accessibilityState={{ busy: device.busy !== null }}
              rightElement={
                <View style={TRAILING}>
                  <Battery device={device} words={words} locale={locale} />
                  <ConnectionControl device={device} words={words} onConnect={connect} onDisconnect={disconnect} />
                  <Button
                    size="sm"
                    appearance="plain"
                    tone="neutral"
                    iconOnly
                    icon={RiInformationLine}
                    accessibilityLabel={words.info(device.name)}
                    testID={`bluetooth-info-${device.id}`}
                    onPress={() => {
                      opened.current += 1;
                      showInfo({ id: device.id, serial: opened.current });
                    }}
                  />
                </View>
              }
            />
          ))}
        </GroupedSection>
      ) : null}
      {on ? (
        <View>
          {/* Discovery runs as long as the page is open, so nothing beside the title spins: an
              animation for as long as discovery would draw a frame every refresh while the page
              sits open (docs/performance.mdx, *Idle*); "Searching…" says it while nothing is found. */}
          <GroupedSection title={words.nearby}>
            {nearby.length === 0 ? (
              <GroupedSectionItem title={words.searching} testID="bluetooth-searching" />
            ) : (
              nearby.map((device) => (
                <GroupedSectionItem
                  key={device.id}
                  title={device.name}
                  icon={<DeviceFace kind={device.kind} connected={false} />}
                  testID={`bluetooth-nearby-${device.id}`}
                  accessibilityLabel={
                    device.busy === "connecting" ? `${device.name}, ${words.connecting}` : device.name
                  }
                  accessibilityState={{ busy: device.busy !== null }}
                  rightElement={
                    <ConnectionControl device={device} words={words} onConnect={connect} onDisconnect={disconnect} />
                  }
                />
              ))
            )}
          </GroupedSection>
        </View>
      ) : null}
      <DeviceSheet
        key={info ? `${info.id}-${info.serial}` : "none"}
        device={shown}
        open={infoOpen && shown !== null}
        words={words}
        locale={locale}
        act={act}
        onClose={hideInfo}
        onForget={(device) => {
          hideInfo();
          askForget(device);
        }}
        onConnect={connect}
        onDisconnect={disconnect}
        onFailed={say}
      />
      <AlertDialog
        open={forgetOpen}
        onOpenChange={(open) => {
          if (!open) hideForget();
        }}
        title={words.forgetTitle(forgetting?.name ?? "")}
        description={words.forgetDescription}
        confirmLabel={words.forget}
        cancelLabel={words.cancel}
        destructive
        onConfirm={() => {
          if (forgetting) void forget(forgetting);
        }}
        // Cancelled, the device's sheet comes back, as a Mac's alert returns to its sheet.
        onCancel={() => {
          if (!forgetting) return;
          opened.current += 1;
          showInfo({ id: forgetting.id, serial: opened.current });
        }}
        testID="bluetooth-forget-confirm"
      />
      <AlertDialog
        open={noteOpen}
        onOpenChange={(open) => {
          if (!open) hideNote();
        }}
        title={note?.title ?? ""}
        description={note?.description}
        confirmLabel={words.ok}
        hideCancel
        testID="bluetooth-failure"
      />
    </View>
  );
}
