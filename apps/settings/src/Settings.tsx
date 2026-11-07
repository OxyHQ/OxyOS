/**
 * OxyOS's Settings (docs/adr/0019-settings.mdx): Bloom's settings layout as the application's
 * window, its pages drawn from the platform's snapshot, its changes sent as intents. The same on
 * OxyOS, natively and in the browser: only the model differs (docs/adr/0015-application-model.mdx).
 */
import * as React from "react";
import { AccessibilityInfo, Image, PanResponder, View } from "react-native";
import Svg, { Circle, Path } from "react-native-svg";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Dialog } from "@oxy.so/bloom/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@oxy.so/bloom/dropdown-menu";
import {
  RiAccessibilityLine,
  RiAddLine,
  RiCalendarLine,
  RiCheckLine,
  RiComputerLine,
  RiGlobalLine,
  RiImageLine,
  RiInformationLine,
  RiKeyboardLine,
  RiHand,
  RiLockLine,
  RiMoreLine,
  RiMouseLine,
  RiNotification3Line,
  RiPaletteLine,
  RiRefreshLine,
  RiShieldLine,
  RiUserLine,
  RiSubtractLine,
  RiTranslate2,
  RiVolumeUpLine,
  RiWifiLine,
} from "@oxy.so/bloom/icons";
import { resolveBloomLanguage, useBloomLocale } from "@oxy.so/bloom/locale";
import { Loading } from "@oxy.so/bloom/loading";
import {
  SegmentedControl,
  SegmentedControlItem,
  SegmentedControlItemText,
} from "@oxy.so/bloom/segmented-control";
import { Button } from "@oxy.so/bloom/button";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { useWindowKeys } from "@oxy.so/bloom/kbd";
import { SettingsLayout, SettingsSelect } from "@oxy.so/bloom/settings-modal";
import type { SettingsModalPage, SettingsNavGroup } from "@oxy.so/bloom/settings-modal";
import { RadioGroup } from "@oxy.so/bloom/radio";
import { Slider } from "@oxy.so/bloom/slider";
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
  notificationSoundsWith,
  placeOf,
  prefersSteadyCaret,
  refreshRates,
  resolutions,
  sideOfKey,
  type Appearance,
  type Display,
  type HourCycle,
  type LanguagePreference,
  type NotificationPreviews,
  type SettingsActions,
  type SettingsSnapshot,
  type Side,
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
import {
  hourCycleOf,
  languageItems,
  localeSample,
  regionItems,
  sessionLocale,
  timeFormatItems,
} from "./locale";
import {
  addable,
  INPUT_METHOD_GLYPHS,
  LAYOUT_SOURCE,
  layoutName,
  layoutShortName,
  removable,
  selectedAfterRemoving,
  withAdded,
  withRemoved,
  type InputMethod,
  type InputSourceId,
  type InputSourcesState,
} from "./input-sources";
import {
  balanceLean,
  deviceName,
  litSegments,
  METER_SEGMENTS,
  percentOf,
  typeLabel,
  VOLUME_KEY_STEP,
  type SoundActions,
  type SoundDevice,
  type SoundDirection,
  type SoundSide,
} from "./sound";
import { messagesFor, messagesLanguage, type Messages } from "./messages";
import { WallpaperPage } from "./WallpaperPage";
import { accountView, type AccountView } from "./account";
import {
  formatSize as formatBytes,
  privacyRows,
  progressOf,
  updatable,
  updateRows,
  type UpdateOutcome,
  type UpdateProgress,
  type UpdateRow,
} from "./updates";

/** A page's model and words, and the session's locale its numbers and dates are formatted in. */
type PageProps = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };

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
            // One keyboard stop, the arrows choosing the next accent (Bloom's swatch radio group), each
            // named by its colour in the session's language (Bloom's preset `label` is the colour of
            // the text on the accent, not its name).
            <RadioGroup<string>
              label={words.accent}
              variant="swatch"
              value={accent}
              onValueChange={(name) => actions.setPreference("accent", name)}
              options={ACCENTS.filter((name) => name in APP_COLOR_PRESETS).map((name) => ({
                value: name,
                label: words.accents[name] ?? name,
                color: APP_COLOR_PRESETS[name as keyof typeof APP_COLOR_PRESETS].hex,
                testID: `accent-${name}`,
              }))}
              testID="accent"
            />
          }
        />
      </GroupedSection>
    </View>
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
      {/* Each application's Play sound for notifications, as a Mac's settings for an application's
          notifications have it (docs/adr/0044-system-sounds.mdx): on unless the person turns it off. */}
      {snapshot.applications && snapshot.applications.length > 0 ? (
        <GroupedSection title={words.applications}>
          {snapshot.applications.map((app) => (
            <GroupedSectionItem
              key={app.id}
              title={app.name}
              description={words.playSound}
              testID={`notification-sound-${app.id}`}
              rightElement={
                <Switch
                  checked={!snapshot.preferences.notificationSoundsOff.includes(app.id)}
                  onCheckedChange={(on) =>
                    actions.setPreference(
                      "notificationSoundsOff",
                      notificationSoundsWith(snapshot.preferences.notificationSoundsOff, app.id, on),
                    )
                  }
                  accessibilityLabel={words.playSoundFor(app.name)}
                  testID={`notification-sound-${app.id}-switch`}
                />
              }
            />
          ))}
        </GroupedSection>
      ) : null}
    </View>
  );
}

/** Sound (docs/adr/0044-system-sounds.mdx, docs/adr/0056-sound-devices.mdx, decision 9), as a
    Mac's Sound settings: Play user interface sound effects, on unless the person turns it off;
    then Output & Input, two tabs, each its devices and its volume. Without the sound (the session
    has told none, or it cannot reach PipeWire) the page says so in their place. */
function SoundPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.sound;
  const state = snapshot.sound;
  const act = actions.sound;
  const [tab, setTab] = React.useState<SoundDirection>("output");
  const reached = Boolean(state && act && state.available);
  // The input's level is metered while the Input tab shows, and stopped when it is left or the page
  // goes, so nothing listens to the input otherwise (ADR 0056, decision 7).
  const metering = reached && tab === "input";
  const meter = React.useRef(act?.meter);
  meter.current = act?.meter;
  React.useEffect(() => {
    if (!metering) return undefined;
    meter.current?.(true);
    return () => meter.current?.(false);
  }, [metering]);
  const effects = (
    <GroupedSection footer={words.uiSoundsDescription}>
      <ToggleItem
        title={words.uiSounds}
        value={snapshot.preferences.uiSounds}
        onChange={(on) => actions.setPreference("uiSounds", on)}
        testID="ui-sounds"
      />
    </GroupedSection>
  );
  if (!state || !act || !reached) {
    return (
      <View style={PAGE}>
        {effects}
        <Notice title={words.unavailable} testID="sound-unavailable" />
      </View>
    );
  }
  const side = state[tab];
  return (
    <View style={PAGE}>
      {effects}
      <GroupedSection title={words.outputInput}>
        <GroupedSectionItem
          title={words.outputInput}
          testID="sound-tabs-row"
          content={
            <View style={{ alignItems: "center" }}>
              <SegmentedControl<SoundDirection>
                label={words.outputInput}
                type="tabs"
                value={tab}
                onValueChange={setTab}
                testID="sound-tabs"
              >
                {(["output", "input"] as const).map((direction) => (
                  <SegmentedControlItem key={direction} value={direction} testID={`sound-tab-${direction}`}>
                    <SegmentedControlItemText>{words[direction]}</SegmentedControlItemText>
                  </SegmentedControlItem>
                ))}
              </SegmentedControl>
            </View>
          }
        />
        <DeviceList direction={tab} devices={side.devices} act={act} words={words} />
      </GroupedSection>
      {tab === "output" ? (
        <SoundOutput side={side} act={act} words={words} />
      ) : (
        <SoundInput side={side} level={snapshot.soundInputLevel ?? 0} act={act} words={words} />
      )}
    </View>
  );
}

type SoundWordsOf = Messages["sound"];

/** A device's row in its list: the check of the chosen one, its name, its type at the end (a Mac's
    Type column). */
const DEVICE_OPTION = { alignItems: "stretch", borderRadius: 10, paddingHorizontal: 4 } as const;

/** A side's devices, as a Mac's table of them: each its name and its type, the chosen one checked.
    Bloom's `ChoiceList`, as Input Sources' (ADR 0039: one Tab stop, the arrows moving through it,
    Space or Return choosing; each row said once with its place, the chosen one selected), each
    named by the device and its type ("DELL U2720Q, HDMI/DisplayPort"). Choosing the one chosen
    asks nothing. */
function DeviceList({
  direction,
  devices,
  act,
  words,
}: {
  direction: SoundDirection;
  devices: readonly SoundDevice[];
  act: SoundActions;
  words: SoundWordsOf;
}) {
  const theme = useTheme();
  if (devices.length === 0) {
    return <GroupedSectionItem title={words.none[direction]} testID={`sound-${direction}-none`} />;
  }
  const chosen = devices.find((device) => device.selected)?.id ?? "";
  return (
    <GroupedSectionItem
      title={words[direction]}
      testID={`sound-${direction}-devices`}
      content={
        <ChoiceList<string>
          label={words.devices[direction]}
          value={chosen}
          onValueChange={(id) => {
            if (id !== chosen) act.select(direction, id);
          }}
          orientation="vertical"
          style={{ gap: 2 }}
          optionStyle={DEVICE_OPTION}
          options={devices.map((device) => ({
            value: device.id,
            label: words.device(deviceName(device, direction, words), typeLabel(device.transport, words)),
            testID: `sound-${direction}-${device.id}`,
          }))}
          renderContent={(option) => {
            const device = devices.find((d) => d.id === option.value);
            if (!device) return null;
            return (
              <View style={{ flexDirection: "row", alignItems: "center", gap: 10, minHeight: 32 }}>
                <View style={{ width: 20, alignItems: "center" }}>
                  {device.selected ? <RiCheckLine size="sm" fill={theme.colors.primary} /> : null}
                </View>
                <Text variant="body-regular" numberOfLines={1} style={{ flex: 1 }}>
                  {deviceName(device, direction, words)}
                </Text>
                <Text variant="body-2-regular" numberOfLines={1} style={{ color: theme.colors.textSecondary }}>
                  {typeLabel(device.transport, words)}
                </Text>
              </View>
            );
          }}
          testID={`sound-${direction}-list`}
        />
      }
    />
  );
}

/** A side's volume, as the shell's slider: the level in percent ("50 %"), the keys a sixteenth. */
function VolumeSlider({
  direction,
  volume,
  label,
  act,
  words,
  testID,
}: {
  direction: SoundDirection;
  volume: number;
  label: string;
  act: SoundActions;
  words: SoundWordsOf;
  testID: string;
}) {
  return (
    <View style={{ width: 220 }}>
      <Slider
        value={percentOf(volume)}
        min={0}
        max={100}
        step={100 / 64}
        keyboardStep={100 * VOLUME_KEY_STEP}
        showTooltip={false}
        formatValue={(value) => words.percent(Math.round(value))}
        accessibilityLabel={label}
        onValueChange={(value) => act.setVolume(direction, value / 100)}
        testID={testID}
      />
    </View>
  );
}

/** A row whose control is a slider or a meter: its title drawn and not read again, since the control is named
    by it (as Control Center's sliders, ADR 0025), so the row is one element, the slider. */
function SliderRow({ title, testID, children }: { title: string; testID: string; children: React.ReactNode }) {
  const theme = useTheme();
  return (
    <GroupedSectionItem
      title={title}
      testID={testID}
      content={
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Text
            variant="body-regular"
            numberOfLines={1}
            style={{ flex: 1, color: theme.colors.text }}
            accessibilityElementsHidden
            importantForAccessibility="no-hide-descendants"
          >
            {title}
          </Text>
          {children}
        </View>
      }
    />
  );
}

/** A slider's ends, drawn: the slider's value says them, so the reader skips them. */
function SliderEnd({ text }: { text: string }) {
  const theme = useTheme();
  return (
    <Text
      variant="caption-1-regular"
      style={{ color: theme.colors.textSecondary }}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {text}
    </Text>
  );
}

/** Output's volume with its mute, and the balance from left to right where the
    device has a left and a right. */
function SoundOutput({ side, act, words }: { side: SoundSide; act: SoundActions; words: SoundWordsOf }) {
  const theme = useTheme();
  const balance = side.balance;
  const balanceText = (value: number) => {
    const lean = balanceLean(value);
    return lean.side === "center" ? words.center : words.leaning(lean.side, words.percent(lean.percent));
  };
  if (side.volume === null && balance === null) return null;
  return (
    <GroupedSection>
      {side.volume !== null ? (
        <SliderRow title={words.outputVolume} testID="sound-output-volume-row">
          <VolumeSlider
            direction="output"
            volume={side.volume}
            label={words.outputVolume}
            act={act}
            words={words}
            testID="sound-output-volume"
          />
        </SliderRow>
      ) : null}
      {side.volume !== null ? (
        <GroupedSectionItem
          title={words.mute}
          testID="sound-mute-row"
          rightElement={
            <Switch
              checked={side.muted}
              onCheckedChange={(muted) => act.setMuted("output", muted)}
              accessibilityLabel={words.mute}
              testID="sound-mute"
            />
          }
        />
      ) : null}
      {balance !== null ? (
        <SliderRow title={words.balance} testID="sound-balance-row">
            <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
              <SliderEnd text={words.left} />
              <View style={{ width: 160 }}>
                <Slider
                  // A balance leans from the centre: no side of it is filled, as a Mac's.
                  minimumTrackTintColor={theme.colors.border}
                  value={Math.min(1, Math.max(-1, balance)) * 100}
                  min={-100}
                  max={100}
                  step={100 / 64}
                  keyboardStep={200 / 16}
                  showTooltip={false}
                  formatValue={(value) => balanceText(value / 100)}
                  accessibilityLabel={words.balance}
                  onValueChange={(value) => act.setBalance(value / 100)}
                  testID="sound-balance"
                />
              </View>
              <SliderEnd text={words.right} />
            </View>
        </SliderRow>
      ) : null}
    </GroupedSection>
  );
}

/** Input's volume, and the level as the session meters it while the tab is shown. */
function SoundInput({
  side,
  level,
  act,
  words,
}: {
  side: SoundSide;
  level: number;
  act: SoundActions;
  words: SoundWordsOf;
}) {
  return (
    <GroupedSection>
      {side.volume !== null ? (
        <SliderRow title={words.inputVolume} testID="sound-input-volume-row">
          <VolumeSlider
            direction="input"
            volume={side.volume}
            label={words.inputVolume}
            act={act}
            words={words}
            testID="sound-input-volume"
          />
        </SliderRow>
      ) : null}
      <SliderRow title={words.inputLevel} testID="sound-input-level-row">
        <InputLevel level={level} words={words} />
      </SliderRow>
    </GroupedSection>
  );
}

/** The input level, as a Mac's: a row of small rounded segments, those the level reaches lit in
    the accent. One element, a progress bar named by what it measures, its value in percent. */
function InputLevel({ level, words }: { level: number; words: SoundWordsOf }) {
  const theme = useTheme();
  const lit = litSegments(level);
  const percent = percentOf(level);
  return (
    <View
      accessible
      accessibilityRole="progressbar"
      accessibilityLabel={words.inputLevel}
      accessibilityValue={{ min: 0, max: 100, now: percent, text: words.percent(percent) }}
      testID="sound-input-level"
      style={{ width: 220, flexDirection: "row", gap: 3 }}
    >
      {Array.from({ length: METER_SEGMENTS }, (_, index) => (
        <View
          key={index}
          style={{
            flex: 1,
            height: 8,
            borderRadius: 2,
            backgroundColor: index < lit ? theme.colors.primary : theme.colors.border,
          }}
        />
      ))}
    </View>
  );
}

/** Mouse and Trackpad (docs/adr/0042-scrolling.mdx, decision 6): Natural scrolling, as a Mac's pages
    have it, each its own: on for a trackpad and off for a mouse unless the person turns them. */
function ScrollingPage({
  snapshot,
  actions,
  strings,
  preference,
}: PageProps & { preference: "trackpadNaturalScrolling" | "mouseNaturalScrolling" }) {
  const words = strings.scrolling;
  return (
    <GroupedSection>
      <ToggleItem
        title={words.naturalScrolling}
        description={words.naturalScrollingDescription}
        value={snapshot.preferences[preference]}
        onChange={(on) => actions.setPreference(preference, on)}
        testID={preference === "trackpadNaturalScrolling" ? "trackpad-scrolling" : "mouse-scrolling"}
      />
    </GroupedSection>
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
          <SettingsSelect
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
 * main display shows a bar along its top, as a Mac's arrangement shows the menu bar. For those who
 * do not drag (docs/adr/0039-keyboard-access.mdx): it is a keyboard stop, and while it has the
 * keyboard the arrows move it to that side of the others, as its custom accessibility actions do
 * for the screen reader; its value is where it stands ("right of Built-in Display"), so a move is
 * said once, by its new place, as a focused control's new value is.
 */
function ArrangedDisplay({
  display,
  displays,
  label,
  nameOf,
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
  nameOf: (display: Display) => string;
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
  const move = (side: Side) => {
    const at = besideOthers(displays, display.output, side);
    // Already there, it goes nowhere and nothing is said.
    if (at.x !== display.x || at.y !== display.y) actions.setDisplayOrigin(display.output, at.x, at.y);
  };
  const [focused, setFocused] = React.useState(false);
  // The focused display's keys, as a Mac's key view has them: the arrows move it; Space and Return
  // choose it, as a press does (the host presses only what answers a click, and this view is a
  // drag's).
  useWindowKeys(
    (event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      if (event.key === " " || event.key === "Enter") {
        onChoose();
        return true;
      }
      const side = sideOfKey(event.key);
      if (!side) return false;
      move(side);
      return true;
    },
    { enabled: focused },
  );
  const where = placeOf(displays, display.output);
  return (
    <View
      {...responder.panHandlers}
      testID={`arranged-${display.output}`}
      accessible
      focusable
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityValue={where ? { text: words.place(where.side, nameOf(where.of)) } : undefined}
      accessibilityHint={displays.length > 1 ? words.moveHint : undefined}
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
        if (side) move(side);
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
              nameOf={nameOf}
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

/** The seconds left of a mode on trial, counted down from when the session said it: a trial's
    own from its first render, never the one before's. */
function useSecondsLeft(trial: SettingsSnapshot["displays"]["trial"]): number {
  const output = trial?.output;
  const seconds = trial?.seconds ?? 0;
  const [counted, setCounted] = React.useState({ output, seconds, left: seconds });
  React.useEffect(() => {
    setCounted({ output, seconds, left: seconds });
    if (seconds === 0) return;
    const start = Date.now();
    const timer = setInterval(() => {
      const left = Math.max(0, seconds - Math.floor((Date.now() - start) / 1000));
      setCounted((was) => (was.left === left ? was : { output, seconds, left }));
    }, 250);
    return () => clearInterval(timer);
  }, [output, seconds]);
  return counted.output === output && counted.seconds === seconds ? counted.left : seconds;
}

/** The seconds at which a mode on trial says how long it has left, beyond its start (said with its
    dialog, the countdown its description): as the lock screen says its wait at sensible intervals
    rather than each second (docs/adr/0039-keyboard-access.mdx, decision 14). */
const TRIAL_SAID_AT = [10, 5];

/**
 * A mode on trial, said as it runs: how long it has left at {@link TRIAL_SAID_AT}, and that the
 * display went back when it ran out unanswered, which the person may not see (the trial is there
 * because the picture may be wrong). An answer the person gave (Keep, Revert, Escape) says nothing:
 * the keyboard's return says where they are.
 */
function useTrialSaid(
  trial: SettingsSnapshot["displays"]["trial"],
  secondsLeft: number,
  words: Messages["displays"],
): React.MutableRefObject<boolean> {
  const answered = React.useRef(false);
  const onTrial = trial !== null;
  React.useEffect(() => {
    if (onTrial && TRIAL_SAID_AT.includes(secondsLeft)) {
      AccessibilityInfo.announceForAccessibility(words.keepDescription(secondsLeft));
    }
  }, [onTrial, secondsLeft, words]);
  const wasOnTrial = React.useRef(onTrial);
  React.useEffect(() => {
    if (wasOnTrial.current && !onTrial && !answered.current) {
      AccessibilityInfo.announceForAccessibility(words.reverted);
    }
    if (onTrial && !wasOnTrial.current) answered.current = false;
    wasOnTrial.current = onTrial;
  }, [onTrial, words]);
  return answered;
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
  const answered = useTrialSaid(trial, secondsLeft, words);
  const answer = (keep: boolean) => {
    answered.current = true;
    actions.confirmDisplays(keep);
  };
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
    // The mode shown chosen again changes nothing, and puts nothing on trial.
    if (next && next !== mode) actions.setDisplayMode(display.output, next);
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
            <SettingsSelect
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
              <SettingsSelect
                label={words.refresh}
                value={String(mode?.refreshMhz ?? "")}
                items={rates.map((rate) => ({ value: String(rate.refreshMhz), label: formatRefresh(rate.refreshMhz) }))}
                onChange={(value) => {
                  const next = rates.find((rate) => String(rate.refreshMhz) === value);
                  if (next && next !== mode) actions.setDisplayMode(display.output, next);
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
      {/* Revert is the default button, which the keyboard starts on and Return presses: the trial
          is there because the picture may be unreadable, and a person who cannot see it is safe
          pressing Return (or Escape, or doing nothing). Keep is a deliberate Tab away. */}
      <AlertDialog
        open={trial !== null}
        onOpenChange={() => {}}
        title={words.keepTitle}
        description={words.keepDescription(secondsLeft)}
        confirmLabel={words.keep}
        cancelLabel={words.revert}
        onConfirm={() => answer(true)}
        onCancel={() => answer(false)}
        defaultAction="cancel"
        testID="keep-displays"
      />
    </View>
  );
}

/** Keyboard: the system's layout, and the person's input sources. */
function KeyboardPage(props: PageProps) {
  const { snapshot, strings } = props;
  return (
    <View style={PAGE}>
      <GroupedSection footer={strings.systemNote}>
        <GroupedSectionItem
          title={strings.keyboard.layout}
          value={layoutName(snapshot.system.keyboardLayout, strings.keyboard.layoutNames)}
          testID="keyboard-layout"
        />
      </GroupedSection>
      {snapshot.inputSources ? <InputSources {...props} state={snapshot.inputSources} /> : null}
    </View>
  );
}

/** A source's name: its layout's as people read it, an input method's as macOS names it. */
function sourceName(id: InputSourceId, state: InputSourcesState, words: Messages["keyboard"]): string {
  return id === LAYOUT_SOURCE ? layoutName(state.layout, words.layoutNames) : words.methods[id];
}

/** A source's option in a list: as wide as the list, its highlight inset from the list's edges.
    The inset is the option's own width too, which the keyboard needs to start on it in a sheet
    first laid out narrower than its content. */
const SOURCE_OPTION = { alignItems: "stretch", borderRadius: 10, paddingHorizontal: 4 } as const;

/** One source's row, drawn: its glyph as the top bar shows it, its name, and a check on the one
    typing now. Decorative: the list's option is the control, named by its label. */
function SourceRow({
  id,
  state,
  words,
  selected,
  current,
}: {
  id: InputSourceId;
  state: InputSourcesState;
  words: Messages["keyboard"];
  selected: boolean;
  current: boolean;
}) {
  const theme = useTheme();
  const ink = selected ? theme.colors.primaryForeground : theme.colors.text;
  const layout = id === LAYOUT_SOURCE;
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 12,
        paddingHorizontal: 10,
        paddingVertical: 7,
        borderRadius: 10,
        backgroundColor: selected ? theme.colors.primary : "transparent",
      }}
    >
      <View
        style={{
          width: 28,
          height: 22,
          borderRadius: 6,
          alignItems: "center",
          justifyContent: "center",
          borderWidth: 1,
          borderColor: selected ? theme.colors.primaryForeground : theme.colors.textSecondary,
        }}
      >
        <Text style={{ color: ink, fontSize: layout ? 11 : 13, fontWeight: "600" }}>
          {layout ? layoutShortName(state.layout) : INPUT_METHOD_GLYPHS[id]}
        </Text>
      </View>
      <Text numberOfLines={1} style={{ flex: 1, color: ink, fontSize: 14 }}>
        {sourceName(id, state, words)}
      </Text>
      {current ? <RiCheckLine size="sm" fill={ink} /> : null}
    </View>
  );
}

/**
 * Input Sources (docs/adr/0041-input-methods.mdx, decision 6), as a Mac's Text Input: the sources
 * in effect, the layout first, a list one row of which is selected (Bloom's `ChoiceList`: one Tab
 * stop, the arrows moving through it, Space or Return selecting; each row said with its place and,
 * for the one typing now, "in use"). **+** opens a sheet of the input methods the session's engine
 * has and the person has not added, the first chosen, added with Add; **−** removes the selected
 * one, never the layout. A change is the `input_sources` preference, the list without the layout:
 * the session keeps it and tells the sources as they are then. An addition or a removal is said
 * once it is in effect, as nothing else under the keyboard says it.
 */
function InputSources({ snapshot, actions, strings, state }: PageProps & { state: InputSourcesState }) {
  const words = strings.keyboard;
  const kept = snapshot.preferences.inputSources;
  const [selected, setSelected] = React.useState<InputSourceId>(state.current);
  const shown = state.sources.includes(selected) ? selected : (state.sources[0] ?? LAYOUT_SOURCE);
  const canAdd = addable(state);
  const [adding, setAdding] = React.useState(false);
  const [choice, setChoice] = React.useState<InputMethod | null>(null);
  const plus = React.useRef<React.ComponentRef<typeof View>>(null);
  const minusFocused = React.useRef(false);
  // A change asked for, said once the session's sources show it; a source added is selected
  // then, as a Mac selects the one it adds (until then the selection stays where it was).
  const asked = React.useRef<{ kind: "added" | "removed"; id: InputMethod } | null>(null);
  React.useEffect(() => {
    const change = asked.current;
    if (!change) return;
    const there = state.sources.includes(change.id);
    if (change.kind === "added" ? !there : there) return;
    asked.current = null;
    if (change.kind === "added") setSelected(change.id);
    AccessibilityInfo.announceForAccessibility(words[change.kind](words.methods[change.id]));
  }, [state.sources, words]);

  const add = () => {
    if (!choice || !canAdd.includes(choice)) return;
    asked.current = { kind: "added", id: choice };
    actions.setPreference("inputSources", withAdded(kept, choice));
  };
  const remove = () => {
    if (!removable(shown)) return;
    const next = selectedAfterRemoving(state.sources, shown);
    asked.current = { kind: "removed", id: shown };
    setSelected(next);
    actions.setPreference("inputSources", withRemoved(kept, shown));
    // The − button goes disabled under the keyboard: the keyboard goes to +, as it would otherwise
    // be left on nothing.
    if (!removable(next) && minusFocused.current && plus.current) {
      AccessibilityInfo.sendAccessibilityEvent(plus.current as never, "focus");
    }
  };
  const notInstalled = state.available.length === 0;
  return (
    <View>
    <GroupedSection title={words.inputSources} footer={notInstalled ? words.notInstalled : words.switchHint}>
      <GroupedSectionItem
        title={words.inputSources}
        testID="input-sources"
        content={
          <View style={{ gap: 8 }}>
            <ChoiceList<InputSourceId>
              label={words.sourcesList}
              value={shown}
              onValueChange={setSelected}
              orientation="vertical"
              style={{ gap: 2 }}
              optionStyle={SOURCE_OPTION}
              options={state.sources.map((id) => {
                const name = sourceName(id, state, words);
                return {
                  value: id,
                  label: id === state.current ? `${name}, ${words.inUse}` : name,
                  testID: `input-source-${id}`,
                };
              })}
              renderContent={(option, { selected: isSelected }) => (
                <SourceRow
                  id={option.value}
                  state={state}
                  words={words}
                  selected={isSelected}
                  current={option.value === state.current}
                />
              )}
              testID="input-sources-list"
            />
            <View style={{ flexDirection: "row", gap: 4 }}>
              <Button
                ref={plus}
                size="sm"
                appearance="plain"
                tone="neutral"
                iconOnly
                icon={RiAddLine}
                accessibilityLabel={words.add}
                disabled={canAdd.length === 0}
                onPress={() => {
                  setChoice(canAdd[0] ?? null);
                  setAdding(true);
                }}
                testID="input-sources-add"
              />
              <Button
                size="sm"
                appearance="plain"
                tone="neutral"
                iconOnly
                icon={RiSubtractLine}
                accessibilityLabel={removable(shown) ? words.removeSource(sourceName(shown, state, words)) : words.remove}
                disabled={!removable(shown)}
                onFocus={() => {
                  minusFocused.current = true;
                }}
                onBlur={() => {
                  minusFocused.current = false;
                }}
                onPress={remove}
                testID="input-sources-remove"
              />
            </View>
          </View>
        }
      />
    </GroupedSection>
      <Dialog
        open={adding}
        onClose={() => setAdding(false)}
        title={words.addTitle}
        actions={[
          { label: words.cancel, color: "cancel", testID: "input-sources-add-cancel" },
          {
            label: words.confirmAdd,
            color: "default",
            disabled: choice === null,
            onPress: add,
            testID: "input-sources-add-confirm",
          },
        ]}
        testID="input-sources-add-sheet"
      >
        {choice !== null ? (
          <ChoiceList<InputMethod>
            label={words.addList}
            value={choice}
            onValueChange={setChoice}
            orientation="vertical"
            style={{ gap: 2 }}
            optionStyle={SOURCE_OPTION}
            options={canAdd.map((id) => ({ value: id, label: words.methods[id], testID: `input-source-add-${id}` }))}
            renderContent={(option, { selected: isSelected }) => (
              <SourceRow id={option.value} state={state} words={words} selected={isSelected} current={false} />
            )}
            testID="input-sources-add-list"
          />
        ) : null}
      </Dialog>
    </View>
  );
}

/** Date & Time. Its 24-hour switch is the hour cycle in effect (the region's, or the person's
    choice) and sets the person's (docs/adr/0040-language-and-region.mdx, decision 7). */
function DateTimePage({ snapshot, actions, strings }: PageProps) {
  const words = strings.dateTime;
  const cycle = hourCycleOf(sessionLocale(snapshot.preferences, snapshot.system.locale));
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
          value={cycle === "h23"}
          onChange={(on) => actions.setPreference("hourCycle", on ? "h23" : "h12")}
          testID="clock-24h"
        />
      </GroupedSection>
    </View>
  );
}

/**
 * Language & Region (docs/adr/0040-language-and-region.mdx, decision 7), as a Mac's: the language,
 * the region and the time format, each the system's (Automatic, named) or the person's, and a
 * sample of a date, a time and a number as the choice formats them. A change is a preference: the
 * session keeps it and tells every application the new locale, Settings among them.
 */
function LanguageRegionPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.languageRegion;
  const { preferences, system } = snapshot;
  const sample = localeSample(sessionLocale(preferences, system.locale));
  return (
    <View style={PAGE}>
      <GroupedSection footer={words.languageDescription}>
        <GroupedSectionItem
          title={words.language}
          testID="language-row"
          rightElement={
            <SettingsSelect<LanguagePreference>
              label={words.language}
              value={preferences.language}
              items={languageItems(words, system.locale)}
              onChange={(value) => actions.setPreference("language", value)}
              testID="language-select"
            />
          }
        />
        <GroupedSectionItem
          title={words.region}
          testID="region-row"
          rightElement={
            <SettingsSelect
              label={words.region}
              value={preferences.region}
              items={regionItems(words, system.locale, preferences.region)}
              onChange={(value) => actions.setPreference("region", value)}
              testID="region-select"
            />
          }
        />
        <GroupedSectionItem
          title={words.timeFormat}
          testID="time-format-row"
          rightElement={
            <SettingsSelect<HourCycle>
              label={words.timeFormat}
              value={preferences.hourCycle}
              items={timeFormatItems(words, preferences, system.locale)}
              onChange={(value) => actions.setPreference("hourCycle", value)}
              testID="time-format"
            />
          }
        />
      </GroupedSection>
      <View testID="language-sample">
        <GroupedSection>
          <GroupedSectionItem title={words.date} value={sample.date} testID="language-sample-date" />
          <GroupedSectionItem title={words.time} value={sample.time} testID="language-sample-time" />
          <GroupedSectionItem title={words.number} value={sample.number} testID="language-sample-number" />
        </GroupedSection>
      </View>
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
      <ToggleItem
        title={words.steadyCaret}
        description={words.steadyCaretDescription}
        value={prefersSteadyCaret(preferences)}
        onChange={(on) => actions.setPreference("caretBlink", !on)}
        testID="steady-caret"
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
    which Atlas's Updates shows too. Settings never runs apt and never shows a fake OS update.

    An update asked for is one control from Update to done, as Atlas's is (ADR 0039): *Updating…*
    while the machine works, *Updated* (or when it updates, the app running) once it ended; it keeps
    the keyboard's and the screen reader's focus throughout, a press while it works or once it is
    done asks nothing, and the row stays where it was after the service's list drops it, as the App
    Store keeps Updated Recently. The start and the end are said once each, as the person may have
    moved on: a control renamed in place is not said by a screen reader. */
function SoftwareUpdatePage({ snapshot, actions, strings, locale }: PageProps) {
  const words = strings.softwareUpdate;
  const language = messagesLanguage(locale);
  const updates = snapshot.updates;
  const [progress, setProgress] = React.useState<Record<string, { state: UpdateProgress; row: UpdateRow }>>({});
  // The rows as last shown, in order: a row the service's list dropped keeps its place.
  const shownOrder = React.useRef<string[]>([]);
  // Under way, as the requests are: a second press never asks again.
  const underWay = React.useRef(new Set<string>());
  if (!updates || !actions.updates) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailable} testID="software-update-unavailable" />
      </GroupedSection>
    );
  }
  const act = actions.updates;
  const state = updates.state;
  const listed = updateRows(state, language);
  const kept = Object.values(progress)
    .map(({ row }) => row)
    .filter((row) => !listed.some((one) => one.id === row.id));
  const byId = new Map([...listed, ...kept].map((row) => [row.id, row]));
  const order = [
    ...shownOrder.current.filter((id) => byId.has(id)),
    ...[...byId.keys()].filter((id) => !shownOrder.current.includes(id)),
  ];
  shownOrder.current = order;
  const rows = order.map((id) => byId.get(id) as UpdateRow);
  const now = updatable(listed).filter((row) => progress[row.id] === undefined);
  const start = (targets: UpdateRow[], ask: () => Promise<UpdateOutcome[]>) => {
    const starting = targets.filter((row) => !underWay.current.has(row.id));
    if (starting.length === 0) return;
    for (const row of starting) underWay.current.add(row.id);
    setProgress((current) => ({
      ...current,
      ...Object.fromEntries(starting.map((row) => [row.id, { state: "updating" as const, row }])),
    }));
    AccessibilityInfo.announceForAccessibility(
      starting.length === 1 ? words.updatingLabel(starting[0].name) : words.updatingCount(starting.length),
    );
    void ask().then((outcomes) => {
      const said: string[] = [];
      const ended: Record<string, { state: UpdateProgress; row: UpdateRow } | null> = {};
      for (const row of starting) {
        underWay.current.delete(row.id);
        const how = progressOf(outcomes.find((outcome) => outcome.id === row.id)?.outcome);
        if (how === "updated") said.push(words.updatedLabel(row.name));
        else if (how === "atNextLaunch") said.push(words.atNextLaunchLabel(row.name));
        else if (how === "failed") said.push(words.failedLabel(row.name));
        // Declined in the consent sheet (it was answered) or failed: the row is the service's again.
        ended[row.id] = how === "updated" || how === "atNextLaunch" ? { state: how, row } : null;
      }
      setProgress((current) => {
        const next = { ...current };
        for (const [id, how] of Object.entries(ended)) {
          if (how) next[id] = how;
          else delete next[id];
        }
        return next;
      });
      if (said.length > 0) AccessibilityInfo.announceForAccessibility(said.join(". "));
    });
  };
  const checked = state?.checking
    ? words.checking
    : state?.checkFailed
      ? words.checkFailed
      : state?.checkedAt
        ? words.lastChecked(new Date(state.checkedAt).toLocaleString(locale))
        : words.neverChecked;
  const updating = Object.values(progress).some(({ state: how }) => how === "updating");
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
          title={listed.length === 0 && state?.checkedAt && !state.checking ? words.upToDate : words.applications}
          description={checked}
          testID="software-update-status"
        />
        {rows.map((row) => {
          const how = progress[row.id]?.state;
          // One control from Update to done; a row waiting for its next launch from the start has none.
          const [label, accessibilityLabel] =
            how === "updating"
              ? [words.updating, words.updatingLabel(row.name)]
              : how === "updated"
                ? [words.updated, words.updatedLabel(row.name)]
                : how === "atNextLaunch"
                  ? [words.atNextLaunchShort, words.atNextLaunchLabel(row.name)]
                  : [words.update, words.updateLabel(row.name)];
          return (
            <GroupedSectionItem
              key={row.id}
              title={row.name}
              description={[
                words.version(row.version, formatBytes(row.size, language)),
                how ? "" : row.needsConsent ? words.needsConsent : row.atNextLaunch ? words.atNextLaunch : "",
                row.notes,
              ]
                .filter(Boolean)
                .join("\n")}
              testID={`software-update-${row.id}`}
              rightElement={
                row.atNextLaunch && how === undefined ? undefined : (
                  <Button
                    size="sm"
                    appearance={how === undefined || how === "updating" ? "outline" : "plain"}
                    tone="neutral"
                    testID={`software-update-${row.id}-update`}
                    // Each row's button names its application: several say "Update" on screen.
                    accessibilityLabel={accessibilityLabel}
                    // Pressed while it works or once done, it asks nothing (and keeps the focus).
                    onPress={how === undefined ? () => start([row], () => act.update(row.id)) : undefined}
                  >
                    {label}
                  </Button>
                )
              }
            />
          );
        })}
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
        {now.length > 0 || updating ? (
          <Button
            appearance="solid"
            tone="accent"
            testID="software-update-all"
            onPress={now.length > 0 ? () => start(now, act.updateAll) : undefined}
          >
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
            {rows.map((row) => {
              const capability = words.capabilities[row.capability] ?? row.capability;
              return (
                <GroupedSectionItem
                  key={row.capability}
                  title={capability}
                  description={row.decided ? undefined : words.notAsked}
                  testID={`privacy-${app.id}-${row.capability}`}
                  rightElement={
                    // Named with its application, as each switch is reached alone (Tab, the reader's
                    // focus): the section's heading is not said with it ("Camera for Allo").
                    <Switch
                      checked={row.granted}
                      onCheckedChange={(granted: boolean) => act.decide(app.id, row.capability, granted)}
                      accessibilityLabel={words.capabilityOf(capability, app.name)}
                      testID={`privacy-${app.id}-${row.capability}-switch`}
                    />
                  }
                />
              );
            })}
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
  words: Messages["wifi"],
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
  strings: Messages,
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
  // A network joined is said once, as it connects, as VoiceOver hears a Mac's Wi-Fi join: the
  // row it was joined from goes, so nothing focused would say it.
  const joinedName =
    state?.available && state.wifi.device && state.wifi.state === "connected" ? currentNetwork(state.wifi)?.ssid ?? "" : "";
  const saidJoined = React.useRef(joinedName);
  React.useEffect(() => {
    if (joinedName === saidJoined.current) return;
    saidJoined.current = joinedName;
    if (joinedName) AccessibilityInfo.announceForAccessibility(words.joined(joinedName));
  }, [joinedName]);
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
    if (joinFailureNeedsNote(reason)) {
      const note = { title: words.couldNotJoin(ssid), description: failureLabel(reason, words.failures, "join") };
      showNote(note);
      // Said as it shows: the note's dialog does not take the screen reader's focus.
      AccessibilityInfo.announceForAccessibility(`${note.title}. ${note.description}`);
    }
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
function NetworkPage({ snapshot, strings, locale }: PageProps) {
  const words = strings.network;
  const language = messagesLanguage(locale);
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

/** The account's picture, or its initials in a circle when it has none: decorative, its name is
    beside it. Drawn as the sidebar's icon too, at the icon's size. */
function AccountPicture({ view, size }: { view: AccountView & { kind: "signed_in" }; size: number }) {
  const theme = useTheme();
  return (
    <View
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        overflow: "hidden",
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: theme.colors.primary,
      }}
    >
      {view.picture ? (
        <Image source={{ uri: view.picture }} style={{ width: size, height: size }} />
      ) : (
        <Text style={{ color: theme.colors.primaryForeground, fontSize: size * 0.4, fontWeight: "600" }}>
          {view.initials}
        </Text>
      )}
    </View>
  );
}

/**
 * The Oxy account (docs/adr/0020-oxy-account-login.mdx, decision 5), as a Mac's Apple Account page:
 * the picture, the name and the handle; managing the account; signing out, which ends the session
 * and returns to the login screen; and removing the account from this computer, which deletes its
 * files here, each confirmed first. What came of it is the platform's: a refusal is said.
 */
function OxyAccountPage({ actions, strings, view }: PageProps & { view: AccountView }) {
  const words = strings.account;
  const account = actions.account;
  const [confirming, setConfirming] = React.useState<"sign-out" | "remove" | null>(null);
  const [failure, setFailure] = React.useState<string | null>(null);
  const name = view.kind === "signed_in" ? view.name : words.group;
  const run = async (ask: (() => Promise<{ done: true } | { failed: string }>) | undefined, failed: string) => {
    setConfirming(null);
    const outcome = ask ? await ask() : { failed: "unavailable" };
    if ("failed" in outcome) setFailure(failed);
  };
  return (
    <View style={PAGE}>
      {view.kind === "signed_in" ? (
        <View style={{ alignItems: "center", gap: 4 }} testID="oxy-account-card">
          <AccountPicture view={view} size={72} />
          <Text variant="title-3-semibold" style={{ marginTop: 8 }} testID="oxy-account-name">
            {view.name}
          </Text>
          <Text variant="body-regular" style={{ opacity: 0.7 }} testID="oxy-account-handle">
            {words.handle(view.handle)}
          </Text>
        </View>
      ) : null}
      {view.kind === "signed_in" && !view.online ? <Notice title={words.offline} testID="oxy-account-offline" /> : null}
      {view.kind === "signed_out" ? <Notice title={words.signedOut} testID="oxy-account-signed-out" /> : null}
      <GroupedSection>
        {view.kind === "signed_in" && account?.manage ? (
          <GroupedSectionItem
            title={words.manage}
            onPress={account.manage}
            showChevron
            accessibilityRole="button"
            testID="oxy-account-manage"
          />
        ) : null}
        <GroupedSectionItem
          title={words.signOut}
          onPress={() => setConfirming("sign-out")}
          disabled={!account}
          showChevron={false}
          accessibilityRole="button"
          testID="oxy-account-sign-out"
        />
      </GroupedSection>
      <GroupedSection>
        <GroupedSectionItem
          title={words.remove}
          onPress={() => setConfirming("remove")}
          destructive
          disabled={!account}
          showChevron={false}
          accessibilityRole="button"
          testID="oxy-account-remove"
        />
      </GroupedSection>
      <AlertDialog
        open={confirming === "sign-out"}
        onOpenChange={(open) => {
          if (!open) setConfirming(null);
        }}
        title={words.signOutTitle}
        description={words.signOutDescription}
        confirmLabel={words.signOutConfirm}
        cancelLabel={words.cancel}
        onConfirm={() => void run(account?.signOut, words.signOutFailed)}
        testID="oxy-account-sign-out-confirm"
      />
      <AlertDialog
        open={confirming === "remove"}
        onOpenChange={(open) => {
          if (!open) setConfirming(null);
        }}
        title={words.removeTitle(name)}
        description={words.removeDescription}
        confirmLabel={words.removeConfirm}
        cancelLabel={words.cancel}
        destructive
        defaultAction="cancel"
        onConfirm={() => void run(account?.remove, words.removeFailed)}
        testID="oxy-account-remove-confirm"
      />
      <AlertDialog
        open={failure !== null}
        onOpenChange={(open) => {
          if (!open) setFailure(null);
        }}
        title={failure ?? ""}
        confirmLabel={words.ok}
        hideCancel
        testID="oxy-account-failed"
      />
    </View>
  );
}

/**
 * Settings, in the session's language (docs/adr/0040-language-and-region.mdx, decision 5): its
 * catalogue is chosen from the locale its root gives `BloomProvider`, so it switches in the same
 * render as Bloom's own words when the session tells a new one.
 */
export function Settings({
  snapshot,
  actions,
  initialPage,
}: {
  snapshot: SettingsSnapshot;
  actions: SettingsActions;
  /** The page to open at (its launch URL's, `oxyos-settings://PAGE`), when it is one of its pages. */
  initialPage?: string;
}) {
  const told = useBloomLocale();
  const strings = messagesFor(resolveBloomLanguage(told));
  const locale = told ?? sessionLocale(snapshot.preferences, snapshot.system.locale);
  const props = { snapshot, actions, strings, locale };
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
  // The Oxy account first, as a Mac's sidebar begins with its Apple Account: its picture and name,
  // where the platform has an account to show.
  const account = accountView(snapshot.account);
  const AccountIcon = React.useMemo(() => {
    if (account?.kind !== "signed_in") return RiUserLine;
    const shown = account;
    return function AccountIcon({ width }: { width?: number | string }) {
      return <AccountPicture view={shown} size={typeof width === "number" ? width : 20} />;
    };
    // The picture changes only with what it shows.
  }, [account?.kind, account?.kind === "signed_in" ? `${account.picture}|${account.initials}` : ""]);
  const accountGroup: SettingsNavGroup[] = account
    ? [
        {
          key: "account",
          label: strings.account.group,
          items: [
            {
              key: "oxy-account",
              label: account.kind === "signed_in" ? account.name : strings.account.group,
              icon: AccountIcon,
              page: "oxy-account",
            },
          ],
        },
      ]
    : [];
  const groups: SettingsNavGroup[] = [
    ...accountGroup,
    ...connections,
    {
      key: "personal",
      label: strings.groups.personal,
      items: [
        { key: "appearance", label: words.appearance, icon: RiPaletteLine, page: "appearance" },
        { key: "wallpaper", label: words.wallpaper, icon: RiImageLine, page: "wallpaper" },
        { key: "notifications", label: words.notifications, icon: RiNotification3Line, page: "notifications" },
        { key: "sound", label: words.sound, icon: RiVolumeUpLine, page: "sound" },
        { key: "lock-screen", label: words.lockScreen, icon: RiLockLine, page: "lock-screen" },
        { key: "displays", label: words.displays, icon: RiComputerLine, page: "displays" },
        { key: "accessibility", label: words.accessibility, icon: RiAccessibilityLine, page: "accessibility" },
      ],
    },
    {
      key: "general",
      label: strings.groups.general,
      items: [
        { key: "language-region", label: words.languageRegion, icon: RiTranslate2, page: "language-region" },
        { key: "keyboard", label: words.keyboard, icon: RiKeyboardLine, page: "keyboard" },
        // After Keyboard, as a Mac's System Settings lists them.
        { key: "mouse", label: words.mouse, icon: RiMouseLine, page: "mouse" },
        { key: "trackpad", label: words.trackpad, icon: RiHand, page: "trackpad" },
        { key: "date-time", label: words.dateTime, icon: RiCalendarLine, page: "date-time" },
        { key: "software-update", label: words.softwareUpdate, icon: RiRefreshLine, page: "software-update" },
        { key: "privacy", label: words.privacy, icon: RiShieldLine, page: "privacy" },
        { key: "about", label: words.about, icon: RiInformationLine, page: "about" },
      ],
    },
  ];
  const pages: Record<string, SettingsModalPage> = {
    ...(account
      ? {
          "oxy-account": {
            title: strings.account.group,
            content: <OxyAccountPage {...props} view={account} />,
          },
        }
      : {}),
    appearance: { title: words.appearance, content: <AppearancePage {...props} /> },
    wallpaper: { title: words.wallpaper, content: <WallpaperPage {...props} /> },
    notifications: { title: words.notifications, content: <NotificationsPage {...props} /> },
    sound: { title: words.sound, content: <SoundPage {...props} /> },
    "lock-screen": { title: words.lockScreen, content: <LockScreenPage {...props} /> },
    displays: { title: words.displays, content: <DisplaysPage {...props} /> },
    accessibility: { title: words.accessibility, content: <AccessibilityPage {...props} /> },
    keyboard: { title: words.keyboard, content: <KeyboardPage {...props} /> },
    mouse: { title: words.mouse, content: <ScrollingPage {...props} preference="mouseNaturalScrolling" /> },
    trackpad: { title: words.trackpad, content: <ScrollingPage {...props} preference="trackpadNaturalScrolling" /> },
    "language-region": { title: words.languageRegion, content: <LanguageRegionPage {...props} /> },
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
  // Opened at a page (`oxyos-settings://PAGE`): that page, shown at once even in a narrow window.
  const opened = initialPage !== undefined && Object.prototype.hasOwnProperty.call(pages, initialPage);
  return (
    <SettingsLayout
      groups={groups}
      pages={pages}
      defaultPage={opened ? initialPage : "appearance"}
      initialView={opened ? "page" : "navigation"}
      labels={{ nav: strings.title }}
      testID="settings"
    />
  );
}
