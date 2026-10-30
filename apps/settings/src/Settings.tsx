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
import {
  SettingsCard,
  SettingsLayout,
  SettingsRow,
  SettingsSection,
  SettingsValueField,
} from "@oxy.so/bloom/settings-modal";
import type { SettingsModalPage, SettingsNavGroup } from "@oxy.so/bloom/settings-modal";
import { Switch } from "@oxy.so/bloom/switch";
import { APP_COLOR_PRESETS } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  ACCENTS,
  formatMemory,
  LOCK_AFTER_CHOICES,
  type Appearance,
  type Preferences,
  type SettingsActions,
  type SettingsSnapshot,
} from "./model";
import type { Strings } from "./strings";

type PageProps = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Strings };

function Toggle({
  label,
  description,
  value,
  onChange,
  testID,
}: {
  label: string;
  description?: string;
  value: boolean;
  onChange: (on: boolean) => void;
  testID: string;
}) {
  return (
    <SettingsRow label={label} description={description} testID={testID}>
      <Switch checked={value} onCheckedChange={onChange} accessibilityLabel={label} testID={`${testID}-switch`} />
    </SettingsRow>
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
        <SelectValue>{(v) => items.find((item) => item.value === v)?.label ?? String(v)}</SelectValue>
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

/** A value only the system changes: shown, with why it cannot be changed here yet. */
function SystemValue({ label, value, testID }: { label: string; value: string; testID: string }) {
  return (
    <SettingsRow label={label} testID={testID}>
      <SettingsValueField muted>{value}</SettingsValueField>
    </SettingsRow>
  );
}

function AppearancePage({ snapshot, actions, strings }: PageProps) {
  const { appearance, accent } = snapshot.preferences;
  const words = strings.appearance;
  return (
    <View style={{ gap: 24 }}>
      <SettingsSection label={words.mode} description={appearance === "auto" ? words.autoDescription : undefined}>
        <SettingsCard>
          <SettingsRow label={words.mode}>
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
          </SettingsRow>
          <SettingsRow label={words.accent}>
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
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
    </View>
  );
}

function WallpaperPage({ snapshot, actions, strings }: PageProps) {
  const items = [
    { value: "", label: strings.wallpaper.system },
    ...snapshot.wallpapers.map((wallpaper) => ({ value: wallpaper.id, label: wallpaper.name })),
  ];
  return (
    <SettingsSection description={strings.wallpaper.dynamic}>
      <SettingsCard>
        <SettingsRow label={strings.wallpaper.current}>
          <Choice
            label={strings.wallpaper.current}
            value={snapshot.preferences.wallpaper ?? ""}
            items={items}
            onChange={(value) => actions.setPreference("wallpaper", value === "" ? null : value)}
            testID="wallpaper"
          />
        </SettingsRow>
      </SettingsCard>
    </SettingsSection>
  );
}

function LockScreenPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.lockScreen;
  const items = LOCK_AFTER_CHOICES.map((minutes) => ({
    value: String(minutes),
    label: minutes === 0 ? words.never : words.minutes(minutes),
  }));
  return (
    <SettingsCard>
      <SettingsRow label={words.lockAfter}>
        <Choice
          label={words.lockAfter}
          value={String(snapshot.preferences.lockAfter)}
          items={items}
          onChange={(value) => actions.setPreference("lockAfter", Number(value))}
          testID="lock-after"
        />
      </SettingsRow>
    </SettingsCard>
  );
}

function KeyboardPage({ snapshot, strings }: PageProps) {
  return (
    <SettingsSection description={strings.systemNote}>
      <SettingsCard>
        <SystemValue label={strings.keyboard.layout} value={snapshot.system.keyboardLayout} testID="keyboard-layout" />
      </SettingsCard>
    </SettingsSection>
  );
}

function DateTimePage({ snapshot, actions, strings }: PageProps) {
  const words = strings.dateTime;
  return (
    <View style={{ gap: 24 }}>
      <SettingsSection description={strings.systemNote}>
        <SettingsCard>
          <SystemValue label={words.timeZone} value={snapshot.system.timeZone} testID="time-zone" />
          <SystemValue
            label={words.automatic}
            value={snapshot.system.automaticTime ? words.on : words.off}
            testID="automatic-time"
          />
        </SettingsCard>
      </SettingsSection>
      <SettingsCard>
        <Toggle
          label={words.clock24h}
          value={snapshot.preferences.clock24h}
          onChange={(on) => actions.setPreference("clock24h", on)}
          testID="clock-24h"
        />
      </SettingsCard>
    </View>
  );
}

function AccessibilityPage({ snapshot, actions, strings }: PageProps) {
  const words = strings.accessibility;
  const { preferences } = snapshot;
  const toggle = (key: keyof Preferences & ("screenReader" | "reduceMotion" | "reduceTransparency")) => (on: boolean) =>
    actions.setPreference(key, on);
  return (
    <SettingsCard>
      <Toggle
        label={words.screenReader}
        description={words.screenReaderDescription}
        value={preferences.screenReader}
        onChange={toggle("screenReader")}
        testID="screen-reader"
      />
      <Toggle
        label={words.reduceMotion}
        description={words.reduceMotionDescription}
        value={preferences.reduceMotion}
        onChange={toggle("reduceMotion")}
        testID="reduce-motion"
      />
      <Toggle
        label={words.reduceTransparency}
        description={words.reduceTransparencyDescription}
        value={preferences.reduceTransparency}
        onChange={toggle("reduceTransparency")}
        testID="reduce-transparency"
      />
    </SettingsCard>
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
    <SettingsCard>
      {rows.map(([label, value, testID]) => (
        <SettingsRow key={testID} label={label} testID={testID}>
          <Text variant="body-regular">{value}</Text>
        </SettingsRow>
      ))}
    </SettingsCard>
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
