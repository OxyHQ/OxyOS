/**
 * OxyOS's Settings (docs/adr/0019-settings.mdx): Bloom's settings layout as the application's
 * window, its pages drawn from the platform's snapshot, its changes sent as intents. The same on
 * OxyOS, natively and in the browser: only the model differs (docs/adr/0015-application-model.mdx).
 */
import * as React from "react";
import { Pressable, View } from "react-native";
import {
  RiAccessibilityLine,
  RiCalendarLine,
  RiImageLine,
  RiInformationLine,
  RiKeyboardLine,
  RiLockLine,
  RiNotification3Line,
  RiPaletteLine,
} from "@oxy.so/bloom/icons";
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
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { SettingsLayout } from "@oxy.so/bloom/settings-modal";
import type { SettingsModalPage, SettingsNavGroup } from "@oxy.so/bloom/settings-modal";
import { Switch } from "@oxy.so/bloom/switch";
import { APP_COLOR_PRESETS } from "@oxy.so/bloom/theme";
import {
  ACCENTS,
  formatMemory,
  LOCK_AFTER_CHOICES,
  type Appearance,
  type NotificationPreviews,
  type SettingsActions,
  type SettingsSnapshot,
} from "./model";
import type { Strings } from "./strings";

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
  const rows: [string, string, string][] = [
    [words.name, system.computerName, "about-name"],
    [words.version, system.version, "about-version"],
    [words.processor, system.processor, "about-processor"],
    [words.memory, formatMemory(system.memoryBytes), "about-memory"],
    [words.display, system.display, "about-display"],
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
  const groups: SettingsNavGroup[] = [
    {
      key: "personal",
      label: strings.groups.personal,
      items: [
        { key: "appearance", label: words.appearance, icon: RiPaletteLine, page: "appearance" },
        { key: "wallpaper", label: words.wallpaper, icon: RiImageLine, page: "wallpaper" },
        { key: "notifications", label: words.notifications, icon: RiNotification3Line, page: "notifications" },
        { key: "lock-screen", label: words.lockScreen, icon: RiLockLine, page: "lock-screen" },
        { key: "accessibility", label: words.accessibility, icon: RiAccessibilityLine, page: "accessibility" },
      ],
    },
    {
      key: "system",
      label: strings.groups.system,
      items: [
        { key: "keyboard", label: words.keyboard, icon: RiKeyboardLine, page: "keyboard" },
        { key: "date-time", label: words.dateTime, icon: RiCalendarLine, page: "date-time" },
        { key: "about", label: words.about, icon: RiInformationLine, page: "about" },
      ],
    },
  ];
  const pages: Record<string, SettingsModalPage> = {
    appearance: { title: words.appearance, content: <AppearancePage {...props} /> },
    wallpaper: { title: words.wallpaper, content: <WallpaperPage {...props} /> },
    notifications: { title: words.notifications, content: <NotificationsPage {...props} /> },
    "lock-screen": { title: words.lockScreen, content: <LockScreenPage {...props} /> },
    accessibility: { title: words.accessibility, content: <AccessibilityPage {...props} /> },
    keyboard: { title: words.keyboard, content: <KeyboardPage {...props} /> },
    "date-time": { title: words.dateTime, content: <DateTimePage {...props} /> },
    about: { title: words.about, content: <AboutPage {...props} /> },
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
