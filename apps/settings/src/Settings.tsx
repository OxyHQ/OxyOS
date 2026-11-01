/**
 * OxyOS's Settings (docs/adr/0019-settings.mdx): Bloom's settings layout as the application's
 * window, its pages drawn from the platform's snapshot, its changes sent as intents. The same on
 * OxyOS, natively and in the browser: only the model differs (docs/adr/0015-application-model.mdx).
 */
import * as React from "react";
import { PanResponder, Pressable, View } from "react-native";
import Svg, { Circle, Path } from "react-native-svg";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { Dialog } from "@oxy.so/bloom/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@oxy.so/bloom/dropdown-menu";
import {
  RiAccessibilityLine,
  RiCalendarLine,
  RiComputerLine,
  RiGlobalLine,
  RiImageLine,
  RiInformationLine,
  RiKeyboardLine,
  RiLockLine,
  RiMoreLine,
  RiNotification3Line,
  RiPaletteLine,
  RiRefreshLine,
  RiShieldLine,
  RiWifiLine,
} from "@oxy.so/bloom/icons";
import { Loading } from "@oxy.so/bloom/loading";
import {
  SegmentedControl,
  SegmentedControlItem,
  SegmentedControlItemText,
} from "@oxy.so/bloom/segmented-control";
import {
  Select,
  SelectContent,
  SelectIcon,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectTrigger,
  SelectValue,
} from "@oxy.so/bloom/select";
import { Button } from "@oxy.so/bloom/button";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { SettingsLayout } from "@oxy.so/bloom/settings-modal";
import type { SettingsModalPage, SettingsNavGroup } from "@oxy.so/bloom/settings-modal";
import { Switch } from "@oxy.so/bloom/switch";
import { APP_COLOR_PRESETS, useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  ACCENTS,
  besideOthers,
  diagonalInches,
  formatMemory,
  formatRefresh,
  formatSize,
  looksLike,
  LOCK_AFTER_CHOICES,
  refreshRates,
  resolutions,
  type Appearance,
  type Display,
  type NotificationPreviews,
  type SettingsActions,
  type SettingsSnapshot,
} from "./model";
import {
  currentNetwork,
  detailRows,
  ethernetStatus,
  failureLabel,
  failureOf,
  formatSpeed,
  isLocked,
  joinFailureNeedsNote,
  otherNetworks,
  SCAN_INTERVAL_MS,
  signalBars,
  type KnownNetwork,
  type NearbyNetwork,
  type NetworkDetails,
} from "./network";
import type { Strings } from "./strings";
import { formatSize as formatBytes, privacyRows, updatable, updateRows } from "./updates";

type PageProps = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Strings };

/** A row whose control is a switch, as a Mac's settings lists show one. */
function ToggleItem({
  title,
  description,
  value,
  onChange,
  testID,
}: {
  title: string;
  description?: string;
  value: boolean;
  onChange: (on: boolean) => void;
  testID: string;
}) {
  return (
    <GroupedSectionItem
      title={title}
      description={description}
      testID={testID}
      rightElement={<Switch checked={value} onCheckedChange={onChange} accessibilityLabel={title} testID={`${testID}-switch`} />}
    />
  );
}

function Choice<T extends string>({
  label,
  value,
  items,
  onChange,
  testID,
}: {
  label: string;
  value: T;
  items: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
  testID: string;
}) {
  return (
    <Select value={value} onValueChange={(next) => onChange(next as T)}>
      <SelectTrigger label={label} testID={testID}>
        <SelectValue>
          {(chosen) => {
            // The Select gives back the chosen item, or its value before one is chosen.
            const key =
              typeof chosen === "object" && chosen !== null && "value" in chosen
                ? (chosen as { value: string }).value
                : chosen;
            return items.find((item) => item.value === key)?.label ?? String(key);
          }}
        </SelectValue>
        <SelectIcon />
      </SelectTrigger>
      <SelectContent
        label={label}
        items={items}
        valueExtractor={(item) => item.value}
        renderItem={(item) => (
          <SelectItem value={item.value} label={item.label}>
            <SelectItemIndicator />
            <SelectItemText>{item.label}</SelectItemText>
          </SelectItem>
        )}
      />
    </Select>
  );
}

const PAGE = { gap: 24 } as const;

function AppearancePage({ snapshot, actions, strings }: PageProps) {
  const { appearance, accent } = snapshot.preferences;
  const words = strings.appearance;
  return (
    <View style={PAGE}>
      <GroupedSection footer={appearance === "auto" ? words.autoDescription : undefined}>
        <GroupedSectionItem
          title={words.mode}
          testID="appearance-row"
          rightElement={
            <SegmentedControl<Appearance>
              label={words.mode}
              type="radio"
              value={appearance}
              onValueChange={(value) => actions.setPreference("appearance", value)}
              testID="appearance"
            >
              {(["light", "dark", "auto"] as const).map((mode) => (
                <SegmentedControlItem key={mode} value={mode} testID={`appearance-${mode}`}>
                  <SegmentedControlItemText>{words[mode]}</SegmentedControlItemText>
                </SegmentedControlItem>
              ))}
            </SegmentedControl>
          }
        />
        <GroupedSectionItem
          title={words.accent}
          testID="accent-row"
          rightElement={
            <View style={{ flexDirection: "row", gap: 10 }} accessibilityRole="radiogroup" accessibilityLabel={words.accent}>
              {ACCENTS.filter((name) => name in APP_COLOR_PRESETS).map((name) => {
                const chosen = name === accent;
                const preset = APP_COLOR_PRESETS[name as keyof typeof APP_COLOR_PRESETS];
                return (
                  <Pressable
                    key={name}
                    testID={`accent-${name}`}
                    accessibilityRole="radio"
                    accessibilityLabel={preset.label ?? name}
                    accessibilityState={{ checked: chosen }}
                    onPress={() => actions.setPreference("accent", name)}
                    style={{
                      width: 22,
                      height: 22,
                      borderRadius: 11,
                      backgroundColor: preset.hex,
                      borderWidth: chosen ? 3 : 0,
                      borderColor: "rgba(255,255,255,0.9)",
                    }}
                  />
                );
              })}
            </View>
          }
        />
      </GroupedSection>
    </View>
  );
}

function WallpaperPage({ snapshot, actions, strings }: PageProps) {
  const items = [
    { value: "", label: strings.wallpaper.system },
    ...snapshot.wallpapers.map((wallpaper) => ({ value: wallpaper.id, label: wallpaper.name })),
  ];
  return (
    <GroupedSection footer={strings.wallpaper.dynamic}>
      <GroupedSectionItem
        title={strings.wallpaper.current}
        rightElement={
          <Choice
            label={strings.wallpaper.current}
            value={snapshot.preferences.wallpaper ?? ""}
            items={items}
            onChange={(value) => actions.setPreference("wallpaper", value === "" ? null : value)}
            testID="wallpaper"
          />
        }
      />
    </GroupedSection>
  );
}

/** Notifications (docs/adr/0026-notifications.mdx): Do Not Disturb, and when the lock screen may
    show what notifications say, as a Mac's Notifications settings. */
function NotificationsPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.notifications;
  const { doNotDisturb, notificationPreviews } = snapshot.preferences;
  return (
    <View style={PAGE}>
      <GroupedSection>
        <ToggleItem
          title={words.doNotDisturb}
          description={words.doNotDisturbDescription}
          value={doNotDisturb}
          onChange={(on) => actions.setPreference("doNotDisturb", on)}
          testID="do-not-disturb"
        />
      </GroupedSection>
      <GroupedSection footer={words.previewsDescription}>
        <GroupedSectionItem
          title={words.previews}
          testID="notification-previews-row"
          rightElement={
            <SegmentedControl<NotificationPreviews>
              label={words.previews}
              type="radio"
              value={notificationPreviews}
              onValueChange={(value) => actions.setPreference("notificationPreviews", value)}
              testID="notification-previews"
            >
              {(["always", "unlocked"] as const).map((when) => (
                <SegmentedControlItem key={when} value={when} testID={`notification-previews-${when}`}>
                  <SegmentedControlItemText>{words[when]}</SegmentedControlItemText>
                </SegmentedControlItem>
              ))}
            </SegmentedControl>
          }
        />
      </GroupedSection>
    </View>
  );
}

function LockScreenPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.lockScreen;
  const items = LOCK_AFTER_CHOICES.map((minutes) => ({
    value: String(minutes),
    label: minutes === 0 ? words.never : words.minutes(minutes),
  }));
  return (
    <GroupedSection>
      <GroupedSectionItem
        title={words.lockAfter}
        rightElement={
          <Choice
            label={words.lockAfter}
            value={String(snapshot.preferences.lockAfter)}
            items={items}
            onChange={(value) => actions.setPreference("lockAfter", Number(value))}
            testID="lock-after"
          />
        }
      />
    </GroupedSection>
  );
}

/** How tall the arrangement is drawn, and the room around it. */
const ARRANGEMENT_HEIGHT = 200;
const ARRANGEMENT_PADDING = 20;

/**
 * One display in the arrangement, drawn to scale where it is. Pressed, it is the display whose
 * settings are shown, as a Mac's Displays settings choose one; dragged, it is dropped where the
 * person leaves it, and the session places it beside the others (touching one, over none). The
 * main display shows a bar along its top, as a Mac's arrangement shows the menu bar. Its custom
 * accessibility actions move it to one side of the others, for those who do not drag.
 */
function ArrangedDisplay({
  display,
  displays,
  label,
  chosen,
  onChoose,
  place,
  factor,
  actions,
  strings,
}: {
  display: Display;
  displays: readonly Display[];
  label: string;
  chosen: boolean;
  onChoose: () => void;
  place: { left: number; top: number; width: number; height: number };
  factor: number;
  actions: SettingsActions;
  strings: PageProps["strings"];
}) {
  const theme = useTheme();
  const words = strings.displays;
  const [drag, setDrag] = React.useState({ dx: 0, dy: 0 });
  const responder = React.useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onMoveShouldSetPanResponder: () => true,
        onPanResponderMove: (_, gesture) => setDrag({ dx: gesture.dx, dy: gesture.dy }),
        onPanResponderRelease: (_, gesture) => {
          setDrag({ dx: 0, dy: 0 });
          if (Math.abs(gesture.dx) + Math.abs(gesture.dy) < 3) {
            onChoose();
            return;
          }
          actions.setDisplayOrigin(
            display.output,
            Math.round(display.x + gesture.dx / factor),
            Math.round(display.y + gesture.dy / factor),
          );
        },
        onPanResponderTerminate: () => setDrag({ dx: 0, dy: 0 }),
      }),
    [actions, display.output, display.x, display.y, factor, onChoose],
  );
  const sides = [
    ["moveLeft", "left"],
    ["moveRight", "right"],
    ["moveAbove", "above"],
    ["moveBelow", "below"],
  ] as const;
  return (
    <View
      {...responder.panHandlers}
      testID={`arranged-${display.output}`}
      accessible
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected: chosen }}
      accessibilityActions={[
        { name: "activate" },
        ...sides.map(([name]) => ({ name, label: words[name] })),
      ]}
      onAccessibilityAction={(event) => {
        if (event.nativeEvent.actionName === "activate") {
          onChoose();
          return;
        }
        const side = sides.find(([name]) => name === event.nativeEvent.actionName)?.[1];
        if (!side) return;
        const at = besideOthers(displays, display.output, side);
        actions.setDisplayOrigin(display.output, at.x, at.y);
      }}
      style={{
        position: "absolute",
        left: place.left + drag.dx,
        top: place.top + drag.dy,
        width: place.width,
        height: place.height,
        borderRadius: 6,
        borderWidth: chosen ? 3 : 1,
        borderColor: chosen ? theme.colors.text : theme.colors.background,
        backgroundColor: theme.colors.primary,
        alignItems: "center",
        justifyContent: "center",
        overflow: "hidden",
      }}
    >
      {display.primary ? (
        <View
          testID={`main-bar-${display.output}`}
          style={{
            position: "absolute",
            left: 0,
            right: 0,
            top: 0,
            height: 6,
            backgroundColor: theme.colors.primaryForeground,
            opacity: 0.85,
          }}
        />
      ) : null}
      <Text numberOfLines={1} style={{ color: theme.colors.primaryForeground, fontSize: 12, paddingHorizontal: 4 }}>
        {label}
      </Text>
    </View>
  );
}

/** The displays where they are in the global space, drawn to scale and centred; the one whose
    settings are shown is outlined. */
function Arrangement({
  displays,
  chosen,
  onChoose,
  nameOf,
  actions,
  strings,
}: {
  displays: readonly Display[];
  chosen: number;
  onChoose: (output: number) => void;
  nameOf: (display: Display) => string;
  actions: SettingsActions;
  strings: PageProps["strings"];
}) {
  const [width, setWidth] = React.useState(0);
  const left = Math.min(...displays.map((d) => d.x));
  const top = Math.min(...displays.map((d) => d.y));
  const right = Math.max(...displays.map((d) => d.x + d.width));
  const bottom = Math.max(...displays.map((d) => d.y + d.height));
  const factor = Math.min(
    (width - 2 * ARRANGEMENT_PADDING) / Math.max(1, right - left),
    (ARRANGEMENT_HEIGHT - 2 * ARRANGEMENT_PADDING) / Math.max(1, bottom - top),
  );
  const originX = (width - (right - left) * factor) / 2;
  const originY = (ARRANGEMENT_HEIGHT - (bottom - top) * factor) / 2;
  return (
    <View
      testID="arrangement"
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
      style={{ height: ARRANGEMENT_HEIGHT }}
    >
      {width > 0
        ? displays.map((display) => (
            <ArrangedDisplay
              key={display.output}
              display={display}
              displays={displays}
              label={nameOf(display)}
              chosen={display.output === chosen}
              onChoose={() => onChoose(display.output)}
              place={{
                left: originX + (display.x - left) * factor,
                top: originY + (display.y - top) * factor,
                width: display.width * factor,
                height: display.height * factor,
              }}
              factor={factor}
              actions={actions}
              strings={strings}
            />
          ))
        : null}
    </View>
  );
}

/** The seconds left of a mode on trial, counted down from when the session said it. */
function useSecondsLeft(trial: SettingsSnapshot["displays"]["trial"]): number {
  const [left, setLeft] = React.useState(trial?.seconds ?? 0);
  const output = trial?.output;
  const seconds = trial?.seconds ?? 0;
  React.useEffect(() => {
    setLeft(seconds);
    if (seconds === 0) return;
    const start = Date.now();
    const timer = setInterval(() => {
      setLeft(Math.max(0, seconds - Math.floor((Date.now() - start) / 1000)));
    }, 250);
    return () => clearInterval(timer);
  }, [output, seconds]);
  return left;
}

/**
 * Displays (docs/adr/0018-several-displays.mdx), as a Mac's: with several, their arrangement and
 * which is the main one; for the one chosen, its scale ("Larger text" to "More space", what it
 * looks like at each), its resolution and its refresh rate. A resolution or refresh rate is shown
 * on trial: kept only if the person keeps it before it goes back.
 */
function DisplaysPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.displays;
  const { displays, trial } = snapshot.displays;
  const [chosen, setChosen] = React.useState<number | null>(null);
  const secondsLeft = useSecondsLeft(trial);
  const display =
    displays.find((d) => d.output === chosen) ?? displays.find((d) => d.primary) ?? displays[0];
  const nameOf = (d: Display) => (d.builtin ? words.builtin : d.name);
  if (!display) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.none} testID="no-display" />
      </GroupedSection>
    );
  }
  const several = displays.length > 1;
  const mode = display.modes[display.mode];
  const sizes = resolutions(display);
  const rates = mode ? refreshRates(display, mode.width, mode.height) : [];
  const looks = (scale: number) => {
    const size = looksLike(display, scale);
    return formatSize(size.width, size.height);
  };
  // A resolution keeps the refresh rate shown where it has it, else takes its fastest.
  const chooseResolution = (value: string) => {
    const [width, height] = value.split("x").map(Number);
    const same = refreshRates(display, width, height);
    const next = same.find((m) => m.refreshMhz === mode?.refreshMhz) ?? same[0];
    if (next) actions.setDisplayMode(display.output, next);
  };
  return (
    <View style={PAGE}>
      {several ? (
        <GroupedSection title={words.arrangement} footer={words.arrangementDescription}>
          <Arrangement
            displays={displays}
            chosen={display.output}
            onChoose={setChosen}
            nameOf={nameOf}
            actions={actions}
            strings={strings}
          />
        </GroupedSection>
      ) : null}
      <GroupedSection>
        <GroupedSectionItem
          title={nameOf(display)}
          description={words.size(
            diagonalInches(display.widthMm, display.heightMm),
            mode ? formatSize(mode.width, mode.height) : formatSize(display.width, display.height),
          )}
          icon={<RiComputerLine />}
          testID="display-name"
        />
        {several ? (
          <GroupedSectionItem
            title={words.useAs}
            testID="use-as-row"
            rightElement={
              <SegmentedControl<string>
                label={words.useAs}
                type="radio"
                value={display.primary ? "main" : "extended"}
                onValueChange={(value) => {
                  if (value === "main") actions.setPrimaryDisplay(display.output);
                }}
                testID="use-as"
              >
                <SegmentedControlItem value="main" testID="use-as-main">
                  <SegmentedControlItemText>{words.main}</SegmentedControlItemText>
                </SegmentedControlItem>
                <SegmentedControlItem value="extended" testID="use-as-extended">
                  <SegmentedControlItemText>{words.extended}</SegmentedControlItemText>
                </SegmentedControlItem>
              </SegmentedControl>
            }
          />
        ) : null}
      </GroupedSection>
      <GroupedSection footer={words.scaleDescription(looks(display.recommended))}>
        {display.scales.length > 1 ? (
          <GroupedSectionItem
            title={words.scale}
            testID="scale-row"
            rightElement={
              <SegmentedControl<string>
                label={words.scale}
                type="radio"
                value={String(display.scale)}
                onValueChange={(value) => actions.setDisplayScale(display.output, Number(value))}
                testID="scale"
              >
                {/* Larger text on the left, more space on the right, as a Mac lays them out. */}
                {[...display.scales].reverse().map((scale) => (
                  <SegmentedControlItem
                    key={scale}
                    value={String(scale)}
                    accessibilityLabel={words.looksLike(looks(scale))}
                    testID={`scale-${scale}`}
                  >
                    <SegmentedControlItemText>{looks(scale)}</SegmentedControlItemText>
                  </SegmentedControlItem>
                ))}
              </SegmentedControl>
            }
          />
        ) : null}
        <GroupedSectionItem
          title={words.resolution}
          rightElement={
            <Choice
              label={words.resolution}
              value={mode ? `${mode.width}x${mode.height}` : ""}
              items={sizes.map((size) => {
                const text = formatSize(size.width, size.height);
                return { value: `${size.width}x${size.height}`, label: size.native ? words.native(text) : text };
              })}
              onChange={chooseResolution}
              testID="resolution"
            />
          }
        />
        {rates.length > 1 ? (
          <GroupedSectionItem
            title={words.refresh}
            rightElement={
              <Choice
                label={words.refresh}
                value={String(mode?.refreshMhz ?? "")}
                items={rates.map((rate) => ({ value: String(rate.refreshMhz), label: formatRefresh(rate.refreshMhz) }))}
                onChange={(value) => {
                  const next = rates.find((rate) => String(rate.refreshMhz) === value);
                  if (next) actions.setDisplayMode(display.output, next);
                }}
                testID="refresh-rate"
              />
            }
          />
        ) : (
          <GroupedSectionItem
            title={words.refresh}
            value={mode ? formatRefresh(mode.refreshMhz) : ""}
            testID="refresh-rate"
          />
        )}
      </GroupedSection>
      <AlertDialog
        open={trial !== null}
        onOpenChange={() => {}}
        title={words.keepTitle}
        description={words.keepDescription(secondsLeft)}
        confirmLabel={words.keep}
        cancelLabel={words.revert}
        onConfirm={() => actions.confirmDisplays(true)}
        onCancel={() => actions.confirmDisplays(false)}
        dismissible={false}
        testID="keep-displays"
      />
    </View>
  );
}

function KeyboardPage({ snapshot, strings }: PageProps) {
  return (
    <GroupedSection footer={strings.systemNote}>
      <GroupedSectionItem title={strings.keyboard.layout} value={snapshot.system.keyboardLayout} testID="keyboard-layout" />
    </GroupedSection>
  );
}

function DateTimePage({ snapshot, actions, strings }: PageProps) {
  const words = strings.dateTime;
  return (
    <View style={PAGE}>
      <GroupedSection footer={strings.systemNote}>
        <GroupedSectionItem title={words.timeZone} value={snapshot.system.timeZone} testID="time-zone" />
        <GroupedSectionItem
          title={words.automatic}
          value={snapshot.system.automaticTime ? words.on : words.off}
          testID="automatic-time"
        />
      </GroupedSection>
      <GroupedSection>
        <ToggleItem
          title={words.clock24h}
          value={snapshot.preferences.clock24h}
          onChange={(on) => actions.setPreference("clock24h", on)}
          testID="clock-24h"
        />
      </GroupedSection>
    </View>
  );
}

function AccessibilityPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.accessibility;
  const { preferences } = snapshot;
  const toggle = (key: "screenReader" | "reduceMotion" | "reduceTransparency") => (on: boolean) =>
    actions.setPreference(key, on);
  return (
    <GroupedSection>
      <ToggleItem
        title={words.screenReader}
        description={words.screenReaderDescription}
        value={preferences.screenReader}
        onChange={toggle("screenReader")}
        testID="screen-reader"
      />
      <ToggleItem
        title={words.reduceMotion}
        description={words.reduceMotionDescription}
        value={preferences.reduceMotion}
        onChange={toggle("reduceMotion")}
        testID="reduce-motion"
      />
      <ToggleItem
        title={words.reduceTransparency}
        description={words.reduceTransparencyDescription}
        value={preferences.reduceTransparency}
        onChange={toggle("reduceTransparency")}
        testID="reduce-transparency"
      />
    </GroupedSection>
  );
}

function AboutPage({ snapshot, strings }: PageProps) {
  const words = strings.about;
  const { system } = snapshot;
  // The main display, as the session shows it, when the host says none of its own.
  const main = snapshot.displays.displays.find((d) => d.primary);
  const mainMode = main?.modes[main.mode];
  const display =
    system.display ||
    (main && mainMode
      ? `${main.builtin ? strings.displays.builtin : main.name}, ${formatSize(mainMode.width, mainMode.height)}`
      : "");
  const rows: [string, string, string][] = [
    [words.name, system.computerName, "about-name"],
    [words.version, system.version, "about-version"],
    [words.processor, system.processor, "about-processor"],
    [words.memory, formatMemory(system.memoryBytes), "about-memory"],
    [words.display, display, "about-display"],
  ];
  return (
    <GroupedSection>
      {rows
        .filter(([, value]) => value !== "")
        .map(([title, value, testID]) => (
          <GroupedSectionItem key={testID} title={title} value={value} testID={testID} />
        ))}
    </GroupedSection>
  );
}

/** Software Update (docs/adr/0030-installing-applications.mdx, decision 16): the OS's update
    first, then every application update the account has, from the install service's one state,
    which Atlas's Updates shows too. Settings never runs apt and never shows a fake OS update. */
function SoftwareUpdatePage({ snapshot, actions, strings }: PageProps) {
  const words = strings.softwareUpdate;
  const language = strings.title === "Ajustes" ? "es" : "en";
  const updates = snapshot.updates;
  if (!updates || !actions.updates) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailable} testID="software-update-unavailable" />
      </GroupedSection>
    );
  }
  const act = actions.updates;
  const state = updates.state;
  const rows = updateRows(state, language);
  const now = updatable(rows);
  const checked = state?.checking
    ? words.checking
    : state?.checkFailed
      ? words.checkFailed
      : state?.checkedAt
        ? words.lastChecked(new Date(state.checkedAt).toLocaleString(language))
        : words.neverChecked;
  return (
    <View style={{ gap: 16 }}>
      <GroupedSection>
        <GroupedSectionItem
          title={words.system}
          description={updates.system.kind === "apt" ? words.systemApt : words.systemUnknown}
          testID="software-update-system"
        />
      </GroupedSection>
      <GroupedSection>
        <GroupedSectionItem
          title={rows.length === 0 && state?.checkedAt && !state.checking ? words.upToDate : words.applications}
          description={checked}
          testID="software-update-status"
        />
        {rows.map((row) => (
          <GroupedSectionItem
            key={row.id}
            title={row.name}
            description={[
              words.version(row.version, formatBytes(row.size, language)),
              row.needsConsent ? words.needsConsent : row.atNextLaunch ? words.atNextLaunch : "",
              row.notes,
            ]
              .filter(Boolean)
              .join("\n")}
            testID={`software-update-${row.id}`}
            rightElement={
              row.atNextLaunch ? undefined : (
                <Button
                  size="sm"
                  appearance="outline"
                  tone="neutral"
                  testID={`software-update-${row.id}-update`}
                  onPress={() => act.update(row.id)}
                >
                  {words.update}
                </Button>
              )
            }
          />
        ))}
      </GroupedSection>
      <View style={{ flexDirection: "row", justifyContent: "flex-end", gap: 8 }}>
        <Button
          appearance="outline"
          tone="neutral"
          testID="software-update-check"
          disabled={state?.checking}
          onPress={act.checkForUpdates}
        >
          {words.check}
        </Button>
        {now.length > 0 ? (
          <Button appearance="solid" tone="accent" testID="software-update-all" onPress={act.updateAll}>
            {words.updateAll}
          </Button>
        ) : null}
      </View>
    </View>
  );
}

/** Privacy (decision 8): each installed application's capabilities asked at first use, as its
    account decided them, each a switch the install service keeps; it applies at the app's next
    launch. */
function PrivacyPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.privacy;
  const apps = snapshot.privacy;
  if (apps === undefined || !actions.privacy) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailable} testID="privacy-unavailable" />
      </GroupedSection>
    );
  }
  const act = actions.privacy;
  const listed = (apps ?? []).map((app) => ({ app, rows: privacyRows(app) })).filter(({ rows }) => rows.length > 0);
  return (
    <View style={{ gap: 16 }}>
      <Text style={{ fontSize: 13 }}>{words.description}</Text>
      {listed.length === 0 ? (
        <GroupedSection>
          <GroupedSectionItem title={words.none} testID="privacy-none" />
        </GroupedSection>
      ) : (
        listed.map(({ app, rows }) => (
          <GroupedSection key={app.id} title={app.name}>
            {rows.map((row) => (
              <GroupedSectionItem
                key={row.capability}
                title={words.capabilities[row.capability] ?? row.capability}
                description={row.decided ? undefined : words.notAsked}
                testID={`privacy-${app.id}-${row.capability}`}
                rightElement={
                  <Switch
                    checked={row.granted}
                    onCheckedChange={(granted: boolean) => act.decide(app.id, row.capability, granted)}
                    accessibilityLabel={words.capabilities[row.capability] ?? row.capability}
                    testID={`privacy-${app.id}-${row.capability}-switch`}
                  />
                }
              />
            ))}
          </GroupedSection>
        ))
      )}
    </View>
  );
}

/** A network's signal in three arcs, as the shell's top bar draws it; the arcs it does not reach
    faint. Its words are its row's. */
function SignalGlyph({ signal }: { signal: number }) {
  const theme = useTheme();
  const bars = signalBars(signal);
  const arc = (on: boolean) => (on ? theme.colors.text : theme.colors.textTertiary);
  return (
    <Svg width={18} height={14} viewBox="0 0 20 16">
      <Path d="M1 5.6a13 13 0 0 1 18 0" stroke={arc(bars >= 3)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M4.2 8.9a8.4 8.4 0 0 1 11.6 0" stroke={arc(bars >= 2)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M7.4 12.1a3.9 3.9 0 0 1 5.2 0" stroke={arc(bars >= 1)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Circle cx="10" cy="14.6" r="1.3" fill={arc(bars >= 1)} />
    </Svg>
  );
}

/** At a network's row's end, as a Mac lists networks: a lock when joining asks for a password,
    then its signal; neither for a known network not nearby. */
function NetworkGlyphs({ network }: { network: NearbyNetwork | undefined }) {
  const theme = useTheme();
  if (!network) return null;
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
      {isLocked(network.security) ? <RiLockLine size="sm" fill={theme.colors.textSecondary} /> : null}
      <SignalGlyph signal={network.signal} />
    </View>
  );
}

/** What a network's row says to the screen reader: its name, its state, its lock and signal. */
function networkLabel(
  ssid: string,
  network: NearbyNetwork | undefined,
  words: Strings["wifi"],
  ...states: (string | null | undefined)[]
): string {
  const glyphs = network
    ? [isLocked(network.security) ? words.secured : words.open, words.signal(signalBars(network.signal))]
    : [];
  return [ssid, ...states, ...glyphs].filter(Boolean).join(", ");
}

/** A connection's details (ADR 0038, decision 7), one row each, only those it has. */
function detailItems(
  details: NetworkDetails | null,
  strings: Strings,
  hardwareLabel: string,
  prefix: string,
): React.ReactNode[] {
  const labels = { ...strings.details, hardwareAddress: hardwareLabel };
  return detailRows(details).map((row) => (
    <GroupedSectionItem key={row.key} title={labels[row.key]} value={row.value} testID={`${prefix}-${row.key}`} />
  ));
}

/** A dialog's content kept while it closes, so its words do not vanish under its animation. */
function useHeld<T>(): [T | null, boolean, (value: T) => void, () => void] {
  const [held, setHeld] = React.useState<{ value: T | null; open: boolean }>({ value: null, open: false });
  const show = React.useCallback((value: T) => setHeld({ value, open: true }), []);
  const hide = React.useCallback(() => setHeld((current) => ({ ...current, open: false })), []);
  return [held.value, held.open, show, hide];
}

/** A row that only says something: a page with nothing else to show. */
function Notice({ title, testID }: { title: string; testID: string }) {
  return (
    <GroupedSection>
      <GroupedSectionItem title={title} titleNumberOfLines={0} testID={testID} />
    </GroupedSection>
  );
}

/**
 * Wi-Fi (docs/adr/0038-networks.mdx, decision 7), as a Mac's: the switch; the network joined, its
 * signal and lock, and its details; the known networks, each forgotten from its menu once
 * confirmed; the other networks nearby, each joined when chosen. Settings never asks a password:
 * the session asks it in its own sheet, and says there why a join failed. The page asks for a
 * scan when it opens and every ten seconds while it is shown, as Control Center's view does.
 */
function WifiPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.wifi;
  const state = snapshot.network;
  const act = actions.network;
  const scanning = Boolean(act && state?.available && state.wifi.device && state.wifi.enabled);
  const [details, detailsOpen, showDetails, hideDetails] = useHeld<string>();
  const [forgetting, forgetOpen, askForget, hideForget] = useHeld<KnownNetwork>();
  const [note, noteOpen, showNote, hideNote] = useHeld<{ title: string; description: string }>();
  React.useEffect(() => {
    if (!scanning || !act) return;
    act.scan();
    const timer = setInterval(act.scan, SCAN_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [scanning, act]);
  if (!state || !act || !state.available) return <Notice title={words.unavailable} testID="wifi-unavailable" />;
  const wifi = state.wifi;
  if (!wifi.device) return <Notice title={words.noDevice} testID="wifi-no-device" />;
  const current = currentNetwork(wifi);
  const currentName = current?.ssid ?? null;
  const currentState = wifi.state === "connecting" ? words.connecting : words.connected;
  const nearby = (ssid: string) => wifi.networks.find((network) => network.ssid === ssid);
  const others = otherNetworks(wifi.networks);
  const join = async (ssid: string) => {
    const reason = failureOf(await act.join(ssid));
    if (joinFailureNeedsNote(reason))
      showNote({ title: words.couldNotJoin(ssid), description: failureLabel(reason, words.failures, "join") });
  };
  const forget = async (known: KnownNetwork) => {
    const reason = failureOf(await act.forget(known.id));
    if (reason) showNote({ title: words.couldNotForget, description: failureLabel(reason, words.failures, "forget") });
  };
  return (
    <View style={PAGE}>
      <GroupedSection footer={wifi.enabled ? undefined : wifi.hardwareBlocked ? words.hardwareBlocked : words.off}>
        <GroupedSectionItem
          title={words.wifi}
          icon={<RiWifiLine />}
          testID="wifi"
          rightElement={
            <Switch
              checked={wifi.enabled}
              onCheckedChange={act.setWifi}
              accessibilityLabel={words.wifi}
              testID="wifi-switch"
            />
          }
        />
        {wifi.enabled && currentName ? (
          <GroupedSectionItem
            title={currentName}
            description={currentState}
            leftInset={56}
            testID="wifi-current"
            accessibilityLabel={networkLabel(currentName, current ?? undefined, words, currentState)}
            rightElement={
              <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
                <NetworkGlyphs network={current ?? undefined} />
                <Button
                  size="sm"
                  appearance="outline"
                  tone="neutral"
                  accessibilityLabel={words.detailsLabel(currentName)}
                  testID="wifi-details"
                  onPress={() => showDetails(currentName)}
                >
                  {words.details}
                </Button>
              </View>
            }
          />
        ) : null}
      </GroupedSection>
      {wifi.enabled && wifi.known.length > 0 ? (
        <GroupedSection title={words.known}>
          {wifi.known.map((known) => {
            // One nearby and not joined is joined by choosing it, as a Mac's list joins it; the
            // session has its password.
            const there = nearby(known.ssid);
            const joinable = !!there && !there.current;
            const joining = state.joining === known.ssid;
            return (
              <GroupedSectionItem
                key={known.id}
                title={known.ssid}
                testID={`wifi-known-${known.ssid}`}
                accessibilityLabel={networkLabel(known.ssid, there, words, joining ? words.connecting : null)}
                accessibilityState={{ busy: joining }}
                showChevron={false}
                onPress={
                  joinable
                    ? () => {
                        void join(known.ssid);
                      }
                    : undefined
                }
                rightElement={
                  <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
                    <NetworkGlyphs network={nearby(known.ssid)} />
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          size="sm"
                          appearance="plain"
                          tone="neutral"
                          iconOnly
                          icon={RiMoreLine}
                          accessibilityLabel={words.more(known.ssid)}
                          testID={`wifi-known-${known.ssid}-more`}
                        />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent label={words.more(known.ssid)} align="end" size="sm">
                        <DropdownMenuItem testID={`wifi-forget-${known.ssid}`} onPress={() => askForget(known)}>
                          {words.forgetItem}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </View>
                }
              />
            );
          })}
        </GroupedSection>
      ) : null}
      {wifi.enabled ? (
        <GroupedSection title={words.other}>
          {others.length === 0 ? (
            <GroupedSectionItem title={words.none} testID="wifi-none" />
          ) : (
            others.map((network) => {
              const joining = state.joining === network.ssid;
              const unsupported = network.supported ? undefined : words.notSupported;
              return (
                <GroupedSectionItem
                  key={network.ssid}
                  title={network.ssid}
                  description={unsupported}
                  showChevron={false}
                  testID={`wifi-network-${network.ssid}`}
                  accessibilityLabel={networkLabel(
                    network.ssid,
                    network,
                    words,
                    joining ? words.connecting : null,
                    unsupported,
                  )}
                  accessibilityState={{ busy: joining }}
                  onPress={() => {
                    void join(network.ssid);
                  }}
                  rightElement={
                    <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
                      {joining ? (
                        <Loading
                          size="sm"
                          accessibilityLabel={words.joining(network.ssid)}
                          testID={`wifi-joining-${network.ssid}`}
                        />
                      ) : null}
                      <NetworkGlyphs network={network} />
                    </View>
                  }
                />
              );
            })
          )}
        </GroupedSection>
      ) : null}
      <Dialog
        open={detailsOpen}
        onClose={hideDetails}
        title={details ?? ""}
        label={words.detailsLabel(details ?? "")}
        actions={[{ label: words.ok, color: "default", onPress: hideDetails, testID: "wifi-details-ok" }]}
        testID="wifi-details-sheet"
      >
        <GroupedSection>{detailItems(wifi.details, strings, strings.details.wifiAddress, "wifi-details")}</GroupedSection>
      </Dialog>
      <AlertDialog
        open={forgetOpen}
        onOpenChange={(open) => {
          if (!open) hideForget();
        }}
        title={words.forgetTitle(forgetting?.ssid ?? "")}
        description={words.forgetDescription}
        confirmLabel={words.forget}
        cancelLabel={words.cancel}
        destructive
        onConfirm={() => {
          if (forgetting) void forget(forgetting);
        }}
        testID="wifi-forget-confirm"
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
        testID="wifi-failure"
      />
    </View>
  );
}

/** Network (decision 7): Ethernet's state, speed and details; VPN only says it is not there yet,
    since nothing in OxyOS configures one. */
function NetworkPage({ snapshot, strings }: PageProps) {
  const words = strings.network;
  const language = strings.title === "Ajustes" ? "es" : "en";
  const state = snapshot.network;
  if (!state || !state.available) return <Notice title={words.unavailable} testID="network-unavailable" />;
  const ethernet = state.ethernet;
  const status = ethernetStatus(ethernet);
  const said = {
    none: words.noPort,
    connected: words.connected,
    unplugged: words.unplugged,
    connecting: words.connecting,
    notConnected: words.notConnected,
  }[status];
  const speed = status === "connected" ? formatSpeed(ethernet.speed, language) : "";
  return (
    <View style={PAGE}>
      <GroupedSection title={words.ethernet}>
        <GroupedSectionItem
          title={words.ethernet}
          value={said}
          icon={<RiGlobalLine />}
          testID="ethernet-status"
          accessibilityLabel={`${words.ethernet}, ${said}`}
        />
        {speed ? <GroupedSectionItem title={words.speed} value={speed} testID="ethernet-speed" /> : null}
        {status === "connected"
          ? detailItems(ethernet.details, strings, strings.details.hardwareAddress, "ethernet")
          : null}
      </GroupedSection>
      <GroupedSection title={words.vpn}>
        <GroupedSectionItem title={words.vpnPlaceholder} titleNumberOfLines={0} testID="vpn-placeholder" />
      </GroupedSection>
    </View>
  );
}

export function Settings({
  snapshot,
  actions,
  strings,
}: {
  snapshot: SettingsSnapshot;
  actions: SettingsActions;
  strings: Strings;
}) {
  const props = { snapshot, actions, strings };
  const words = strings.pages;
  // Wi-Fi and Network first, as a Mac lists them, only with the platform's `network` grant.
  const connections: SettingsNavGroup[] =
    snapshot.network === undefined
      ? []
      : [
          {
            key: "connections",
            label: strings.groups.connections,
            items: [
              { key: "wifi", label: words.wifi, icon: RiWifiLine, page: "wifi" },
              { key: "network", label: words.network, icon: RiGlobalLine, page: "network" },
            ],
          },
        ];
  const groups: SettingsNavGroup[] = [
    ...connections,
    {
      key: "personal",
      label: strings.groups.personal,
      items: [
        { key: "appearance", label: words.appearance, icon: RiPaletteLine, page: "appearance" },
        { key: "wallpaper", label: words.wallpaper, icon: RiImageLine, page: "wallpaper" },
        { key: "notifications", label: words.notifications, icon: RiNotification3Line, page: "notifications" },
        { key: "lock-screen", label: words.lockScreen, icon: RiLockLine, page: "lock-screen" },
        { key: "displays", label: words.displays, icon: RiComputerLine, page: "displays" },
        { key: "accessibility", label: words.accessibility, icon: RiAccessibilityLine, page: "accessibility" },
      ],
    },
    {
      key: "system",
      label: strings.groups.system,
      items: [
        { key: "keyboard", label: words.keyboard, icon: RiKeyboardLine, page: "keyboard" },
        { key: "date-time", label: words.dateTime, icon: RiCalendarLine, page: "date-time" },
        { key: "software-update", label: words.softwareUpdate, icon: RiRefreshLine, page: "software-update" },
        { key: "privacy", label: words.privacy, icon: RiShieldLine, page: "privacy" },
        { key: "about", label: words.about, icon: RiInformationLine, page: "about" },
      ],
    },
  ];
  const pages: Record<string, SettingsModalPage> = {
    appearance: { title: words.appearance, content: <AppearancePage {...props} /> },
    wallpaper: { title: words.wallpaper, content: <WallpaperPage {...props} /> },
    notifications: { title: words.notifications, content: <NotificationsPage {...props} /> },
    "lock-screen": { title: words.lockScreen, content: <LockScreenPage {...props} /> },
    displays: { title: words.displays, content: <DisplaysPage {...props} /> },
    accessibility: { title: words.accessibility, content: <AccessibilityPage {...props} /> },
    keyboard: { title: words.keyboard, content: <KeyboardPage {...props} /> },
    "date-time": { title: words.dateTime, content: <DateTimePage {...props} /> },
    about: { title: words.about, content: <AboutPage {...props} /> },
    "software-update": { title: words.softwareUpdate, content: <SoftwareUpdatePage {...props} /> },
    privacy: { title: words.privacy, content: <PrivacyPage {...props} /> },
    ...(snapshot.network === undefined
      ? {}
      : {
          wifi: { title: words.wifi, content: <WifiPage {...props} /> },
          network: { title: words.network, content: <NetworkPage {...props} /> },
        }),
  };
  return (
    <SettingsLayout
      groups={groups}
      pages={pages}
      defaultPage="appearance"
      labels={{ nav: strings.title }}
      testID="settings"
    />
  );
}
