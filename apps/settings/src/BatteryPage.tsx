/**
 * Settings › Battery (docs/adr/0057-battery-and-energy.mdx), as a Mac's System Settings › Battery:
 * the battery's level, what it is doing and its health; Low Power Mode; the charge and the
 * display's use over the last 24 hours and 10 days; each app's energy; and Options… (the charge
 * limit, the display's and the computer's sleep, the display dimmer on battery, the menu bar's
 * percentage). A computer without a battery has the same page as Energy:
 * Low Power Mode and the options on its power adapter.
 *
 * Everything is the session's: the state as it tells it, the history and the energy asked as the
 * page shows them, the charge limit asked of it, the rest preferences.
 */
import * as React from "react";
import { View, type LayoutChangeEvent } from "react-native";
import Svg, { Line, Rect } from "react-native-svg";
import { Button } from "@oxy.so/bloom/button";
import { Dialog } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { RiBatteryLine, RiPlugLine } from "@oxy.so/bloom/icons";
import { Loading } from "@oxy.so/bloom/loading";
import {
  SegmentedControl,
  SegmentedControlItem,
  SegmentedControlItemText,
} from "@oxy.so/bloom/segmented-control";
import { SettingsSelect } from "@oxy.so/bloom/settings-modal";
import { Switch } from "@oxy.so/bloom/switch";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  batteryStateLabel,
  chargeLimitFailureLabel,
  chartTicks,
  durationLabel,
  latestLevel,
  LOW_POWER_MODES,
  screenByDay,
  screenByHour,
  screenScale,
  sleepChoiceLabel,
  sleepChoices,
  usageLabel,
  type AppEnergy,
  type ChargeLimitFailure,
  type HistoryChart,
  type HistorySpan,
  type LowPowerMode,
  type PowerHistory,
} from "./battery";
import type { Messages } from "./messages";
import type { Preferences, SettingsActions, SettingsSnapshot } from "./model";

type PageProps = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };

const PAGE = { gap: 20 } as const;
/** A Mac's system green, for the bins in which the battery charged. */
const CHARGING_GREEN = "#34C759";
const LEVEL_HEIGHT = 112;
const SCREEN_HEIGHT = 72;
/** Room on the right for the gridlines' labels. */
const AXIS_WIDTH = 40;

function percentText(percent: number, locale: string): string {
  return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 0 }).format(percent / 100);
}

/** The width its view was laid out at, 0 until it has been. */
function useWidth(): [number, (event: LayoutChangeEvent) => void] {
  const [width, setWidth] = React.useState(0);
  const onLayout = React.useCallback((event: LayoutChangeEvent) => {
    const next = Math.floor(event.nativeEvent.layout.width);
    setWidth((current) => (current === next ? current : next));
  }, []);
  return [width, onLayout];
}

/** A chart's gridline labels down its right side, at the fractions of its height given. */
function AxisLabels({ labels, height }: { labels: { at: number; text: string }[]; height: number }) {
  const theme = useTheme();
  return (
    <View style={{ position: "absolute", right: 0, top: 0, width: AXIS_WIDTH - 4, height }} pointerEvents="none">
      {labels.map((label) => (
        <Text
          key={label.text}
          numberOfLines={1}
          style={{
            position: "absolute",
            right: 0,
            top: Math.min(height - 14, Math.max(-2, label.at * height - 7)),
            fontSize: 10,
            lineHeight: 14,
            color: theme.colors.textSecondary,
          }}
        >
          {label.text}
        </Text>
      ))}
    </View>
  );
}

/** The time labels under a chart, at the fractions of its width its ticks fall at. */
function TimeLabels({ ticks, width }: { ticks: { at: number; text: string }[]; width: number }) {
  const theme = useTheme();
  return (
    <View style={{ height: 16, width, marginTop: 4 }} pointerEvents="none">
      {ticks.map((tick) => (
        <Text
          key={`${tick.at}`}
          numberOfLines={1}
          style={{
            position: "absolute",
            left: Math.min(width - 40, Math.max(0, tick.at * width)),
            fontSize: 10,
            lineHeight: 14,
            color: theme.colors.textSecondary,
          }}
        >
          {tick.text}
        </Text>
      ))}
    </View>
  );
}

/** Bars over gridlines: each bar's height as a share of the chart's (null: no bar), its colour. */
function Bars({
  shares,
  colors,
  width,
  height,
  gridlines,
}: {
  shares: (number | null)[];
  colors: string[];
  width: number;
  height: number;
  gridlines: number[];
}) {
  const theme = useTheme();
  const slot = width / Math.max(1, shares.length);
  // A bar a whole pixel at least, with a gap where the bars are wide enough to show one.
  const gap = slot >= 6 ? 2 : slot >= 3 ? 1 : 0;
  const barWidth = Math.max(1, slot - gap);
  return (
    <Svg width={width} height={height}>
      {gridlines.map((at) => (
        <Line
          key={at}
          x1={0}
          x2={width}
          y1={Math.round(at * (height - 1)) + 0.5}
          y2={Math.round(at * (height - 1)) + 0.5}
          stroke={theme.colors.borderLight}
          strokeWidth={1}
        />
      ))}
      {shares.map((share, index) => {
        if (share === null || share <= 0) return null;
        const barHeight = Math.max(1, Math.round(Math.min(1, share) * height));
        return (
          <Rect
            key={index}
            x={index * slot + gap / 2}
            y={height - barHeight}
            width={barWidth}
            height={barHeight}
            rx={barWidth >= 4 ? 1.5 : 0}
            fill={colors[index]}
          />
        );
      })}
    </Svg>
  );
}

function spanName(span: HistorySpan, words: Messages["battery"]): string {
  return span === "day" ? words.lastDay : words.lastDays;
}

/** The time labels of a chart: hours (12 AM, 6 AM…) for the day, weekdays and dates for ten days. */
function tickTexts(chart: HistoryChart, span: HistorySpan, locale: string): { at: number; text: string }[] {
  const format =
    span === "day"
      ? new Intl.DateTimeFormat(locale, { hour: "numeric" })
      : new Intl.DateTimeFormat(locale, { weekday: "short", day: "numeric" });
  const ticks = chartTicks(chart, span);
  // Ten days have room for every other day's label.
  const shown = span === "days" ? ticks.filter((_, index) => index % 2 === ticks.length % 2) : ticks;
  return shown.map((tick) => ({ at: tick.at, text: format.format(new Date(tick.seconds * 1000)) }));
}

/** Battery Level: each bin's charge at its end, green where it charged; nothing where no session ran. */
function LevelChart({
  chart,
  span,
  strings,
  locale,
}: {
  chart: HistoryChart;
  span: HistorySpan;
  strings: Messages;
  locale: string;
}) {
  const theme = useTheme();
  const words = strings.battery;
  const [width, onLayout] = useWidth();
  const plot = Math.max(0, width - AXIS_WIDTH);
  const latest = latestLevel(chart);
  const label = words.levelChart(spanName(span, words), latest === null ? null : percentText(latest, locale));
  return (
    <View
      onLayout={onLayout}
      accessible
      role="img"
      accessibilityLabel={label}
      testID="battery-level-chart"
      style={{ gap: 6 }}
    >
      <Text style={{ fontSize: 13, fontWeight: "600" }}>{words.level}</Text>
      {plot > 0 ? (
        <View>
          <View style={{ height: LEVEL_HEIGHT }}>
            <Bars
              width={plot}
              height={LEVEL_HEIGHT}
              gridlines={[0, 0.5, 1]}
              shares={chart.levels.map((level) => (level === null ? null : level / 100))}
              colors={chart.charging.map((charging) => (charging ? CHARGING_GREEN : theme.colors.primary))}
            />
            <AxisLabels
              height={LEVEL_HEIGHT}
              labels={[
                { at: 0, text: percentText(100, locale) },
                { at: 0.5, text: percentText(50, locale) },
                { at: 1, text: percentText(0, locale) },
              ]}
            />
          </View>
          <TimeLabels width={plot} ticks={tickTexts(chart, span, locale)} />
        </View>
      ) : null}
    </View>
  );
}

/** Screen On Usage: minutes per hour over the last day, hours per day over ten. */
function ScreenChart({
  history,
  span,
  strings,
  locale,
}: {
  history: PowerHistory;
  span: HistorySpan;
  strings: Messages;
  locale: string;
}) {
  const theme = useTheme();
  const words = strings.battery;
  const [width, onLayout] = useWidth();
  const plot = Math.max(0, width - AXIS_WIDTH);
  const day = span === "day";
  const bars = day ? screenByHour(history.day) : screenByDay(history.days).map((usage) => usage.seconds);
  const scale = day ? 3600 : screenScale(bars);
  const total = bars.reduce((sum, seconds) => sum + seconds, 0);
  const label = words.screenChart(spanName(span, words), durationLabel(total / 60, words));
  // Numbers in the locale's digits; the units are the catalogue's (Hermes on OxyOS has no
  // Intl.NumberFormat unit style).
  const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
  const hours = { format: (value: number) => words.axisHours(number.format(value)) };
  const minutes = { format: (value: number) => words.axisMinutes(number.format(value)) };
  const axis = day
    ? [
        { at: 0, text: minutes.format(60) },
        { at: 0.5, text: minutes.format(30) },
        { at: 1, text: minutes.format(0) },
      ]
    : [
        { at: 0, text: hours.format(scale / 3600) },
        { at: 1, text: hours.format(0) },
      ];
  // The day chart's hours, as the level chart's quarters, are labelled by the same ticks.
  const ticks = day
    ? tickTexts(history.day, "day", locale)
    : screenByDay(history.days).map((usage, index, all) => ({
        at: index / all.length,
        text: new Intl.DateTimeFormat(locale, { weekday: "narrow" }).format(new Date(usage.start * 1000)),
      }));
  return (
    <View
      onLayout={onLayout}
      accessible
      role="img"
      accessibilityLabel={label}
      testID="battery-screen-chart"
      style={{ gap: 6 }}
    >
      <Text style={{ fontSize: 13, fontWeight: "600" }}>{words.screenOn}</Text>
      {plot > 0 ? (
        <View>
          <View style={{ height: SCREEN_HEIGHT }}>
            <Bars
              width={plot}
              height={SCREEN_HEIGHT}
              gridlines={day ? [0, 0.5, 1] : [0, 1]}
              shares={bars.map((seconds) => seconds / scale)}
              colors={bars.map(() => theme.colors.primary)}
            />
            <AxisLabels height={SCREEN_HEIGHT} labels={axis} />
          </View>
          <TimeLabels width={plot} ticks={ticks} />
        </View>
      ) : null}
    </View>
  );
}

/** "Significant energy", as a Mac's Battery settings mark an app. */
function SignificantBadge({ text }: { text: string }) {
  return (
    <View
      style={{
        paddingHorizontal: 8,
        paddingVertical: 2,
        borderRadius: 999,
        backgroundColor: "#ff9f0a26",
      }}
    >
      <Text style={{ fontSize: 11, lineHeight: 15, fontWeight: "600", color: "#c76a00" }}>{text}</Text>
    </View>
  );
}

/** A timer's choice of minutes, in a row. */
function SleepRow({
  title,
  preference,
  value,
  actions,
  strings,
  testID,
}: {
  title: string;
  preference: "displayOffBattery" | "displayOffAdapter" | "sleepBattery" | "sleepAdapter";
  value: number;
  actions: SettingsActions;
  strings: Messages;
  testID: string;
}) {
  const items = sleepChoices(value).map((minutes) => ({
    value: String(minutes),
    label: sleepChoiceLabel(minutes, strings.battery),
  }));
  return (
    <GroupedSectionItem
      title={title}
      titleNumberOfLines={2}
      testID={`${testID}-row`}
      rightElement={
        <SettingsSelect
          label={title}
          value={String(value)}
          items={items}
          onChange={(chosen) => actions.setPreference(preference, Number(chosen))}
          testID={testID}
        />
      }
    />
  );
}

/** A row whose control is a switch. */
function SwitchRow({
  title,
  description,
  value,
  onChange,
  disabled,
  testID,
}: {
  title: string;
  description?: string;
  value: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  testID: string;
}) {
  return (
    <GroupedSectionItem
      title={title}
      titleNumberOfLines={2}
      description={description}
      descriptionLines={4}
      testID={testID}
      rightElement={
        <Switch
          size="xs"
          checked={value}
          onCheckedChange={onChange}
          disabled={disabled}
          accessibilityLabel={title}
          testID={`${testID}-switch`}
        />
      }
    />
  );
}

/**
 * Options…: the charge limit (UPower's threshold, asked of the session, which says why not where
 * it could not), the timers of the battery and of the power adapter, the display dimmer on
 * battery and the menu bar's percentage.
 */
function OptionsDialog({
  open,
  onClose,
  snapshot,
  actions,
  strings,
  locale,
}: PageProps & { open: boolean; onClose: () => void }) {
  const words = strings.battery;
  const state = snapshot.power;
  const preferences = snapshot.preferences;
  const battery = !!state?.battery;
  const limit = state?.chargeLimit ?? null;
  const [pending, setPending] = React.useState<boolean | null>(null);
  const [failure, setFailure] = React.useState<ChargeLimitFailure | null>(null);
  // Shown as asked until the session tells it so.
  React.useEffect(() => {
    if (pending !== null && limit?.enabled === pending) setPending(null);
  }, [pending, limit?.enabled]);
  const setLimit = async (enabled: boolean) => {
    if (!actions.power) return;
    setFailure(null);
    setPending(enabled);
    const failed = await actions.power.setChargeLimit(enabled);
    if (failed) {
      setPending(null);
      setFailure(failed);
    }
  };
  const set = <K extends keyof Preferences>(key: K) => (value: Preferences[K]) => actions.setPreference(key, value);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={battery ? words.optionsTitle : words.energyOptionsTitle}
      label={battery ? words.optionsTitle : words.energyOptionsTitle}
      // Done stays in view below the options, which scroll, as a Mac's sheet keeps its button.
      footer={
        <View style={{ paddingHorizontal: 20, paddingVertical: 12, alignItems: "flex-end" }}>
          <Button onPress={onClose} testID="battery-options-done">
            {words.done}
          </Button>
        </View>
      }
      testID="battery-options-sheet"
    >
      <View style={{ gap: 16 }}>
        {battery && limit ? (
          <GroupedSection title={words.optimizedCharging}>
            <SwitchRow
              title={words.limitCharge(percentText(limit.end ?? 80, locale))}
              description={words.limitChargeDescription}
              value={pending ?? limit.enabled}
              onChange={(on) => {
                void setLimit(on);
              }}
              disabled={!actions.power || pending !== null}
              testID="battery-charge-limit"
            />
            {failure ? (
              <GroupedSectionItem
                title={chargeLimitFailureLabel(failure, words)}
                titleNumberOfLines={0}
                testID="battery-charge-limit-failure"
              />
            ) : null}
          </GroupedSection>
        ) : null}
        <GroupedSection>
          {battery ? (
            <SleepRow
              title={words.displayOffBattery}
              preference="displayOffBattery"
              value={preferences.displayOffBattery}
              actions={actions}
              strings={strings}
              testID="display-off-battery"
            />
          ) : null}
          <SleepRow
            title={words.displayOffAdapter}
            preference="displayOffAdapter"
            value={preferences.displayOffAdapter}
            actions={actions}
            strings={strings}
            testID="display-off-adapter"
          />
          {battery ? (
            <SleepRow
              title={words.sleepBattery}
              preference="sleepBattery"
              value={preferences.sleepBattery}
              actions={actions}
              strings={strings}
              testID="sleep-battery"
            />
          ) : null}
          <SleepRow
            title={words.sleepAdapter}
            preference="sleepAdapter"
            value={preferences.sleepAdapter}
            actions={actions}
            strings={strings}
            testID="sleep-adapter"
          />
        </GroupedSection>
        <GroupedSection>
          {battery ? (
            <SwitchRow
              title={words.dimOnBattery}
              value={preferences.dimOnBattery}
              onChange={set("dimOnBattery")}
              testID="dim-on-battery"
            />
          ) : null}
          {battery ? (
            <SwitchRow
              title={words.showPercentage}
              value={preferences.batteryPercentage}
              onChange={set("batteryPercentage")}
              testID="battery-percentage"
            />
          ) : null}
        </GroupedSection>
      </View>
    </Dialog>
  );
}

export function BatteryPage(props: PageProps) {
  const { snapshot, actions, strings, locale } = props;
  const words = strings.battery;
  const theme = useTheme();
  const state = snapshot.power;
  const power = actions.power;
  const battery = state?.battery ?? null;
  const [span, setSpan] = React.useState<HistorySpan>("day");
  const [history, setHistory] = React.useState<PowerHistory | null>(null);
  const [apps, setApps] = React.useState<AppEnergy[] | null>(null);
  const [options, setOptions] = React.useState(false);
  // The history again as the charge changes; the energy as the page opens and the period changes.
  const charge = battery ? `${battery.percent}:${battery.state}` : "";
  React.useEffect(() => {
    if (!power || !battery) return;
    let live = true;
    void power.history().then((told) => live && told && setHistory(told));
    return () => {
      live = false;
    };
  }, [power, charge]);
  React.useEffect(() => {
    if (!power) return;
    let live = true;
    void power.energy().then((told) => live && told && setApps(told));
    return () => {
      live = false;
    };
  }, [power, span]);
  if (!state) {
    return (
      <GroupedSection>
        <GroupedSectionItem
          title={words.waiting}
          testID="battery-waiting"
          rightElement={<Loading size="sm" accessibilityLabel={words.waiting} />}
        />
      </GroupedSection>
    );
  }
  const stateLabel = battery ? batteryStateLabel(battery, words) : "";
  const level = battery ? percentText(battery.percent, locale) : "";
  const health = state.health;
  const lowPowerItems = LOW_POWER_MODES.map((mode) => ({ value: mode, label: words.lowPowerModes[mode] }));
  return (
    <View style={PAGE}>
      {battery ? (
        <GroupedSection>
          <GroupedSectionItem
            title={words.level}
            icon={<RiBatteryLine />}
            value={level}
            description={stateLabel}
            testID="battery-level"
            accessibilityLabel={`${words.level}, ${level}, ${stateLabel}`}
          />
          <GroupedSectionItem
            title={words.powerSource}
            icon={<RiPlugLine />}
            value={battery.onAc ? words.onAdapter : words.onBattery}
            testID="battery-power-source"
          />
          {health ? (
            <GroupedSectionItem
              title={words.health}
              value={health.worn ? words.serviceRecommended : words.normal}
              leftInset={56}
              testID="battery-health"
            />
          ) : null}
          {health && health.maxCapacity !== null ? (
            <GroupedSectionItem
              title={words.maxCapacity}
              value={percentText(health.maxCapacity, locale)}
              leftInset={56}
              testID="battery-max-capacity"
            />
          ) : null}
          {health && health.cycles !== null ? (
            <GroupedSectionItem
              title={words.cycleCount}
              value={new Intl.NumberFormat(locale).format(health.cycles)}
              leftInset={56}
              testID="battery-cycle-count"
            />
          ) : null}
        </GroupedSection>
      ) : null}
      <GroupedSection footer={words.lowPowerModeDescription}>
        <GroupedSectionItem
          title={words.lowPowerMode}
          testID="low-power-mode-row"
          rightElement={
            <SettingsSelect<LowPowerMode>
              label={words.lowPowerMode}
              value={snapshot.preferences.lowPowerMode}
              items={lowPowerItems}
              onChange={(mode) => actions.setPreference("lowPowerMode", mode)}
              testID="low-power-mode"
            />
          }
        />
      </GroupedSection>
      {battery ? (
        <GroupedSection>
          <GroupedSectionItem
            title={words.span}
            testID="battery-history"
            content={
              <View style={{ gap: 16, paddingVertical: 4 }}>
                <View style={{ alignItems: "center" }}>
                  <SegmentedControl<HistorySpan>
                    label={words.span}
                    type="radio"
                    value={span}
                    onValueChange={setSpan}
                    testID="battery-span"
                  >
                    {(["day", "days"] as const).map((value) => (
                      <SegmentedControlItem key={value} value={value} testID={`battery-span-${value}`}>
                        <SegmentedControlItemText>{spanName(value, words)}</SegmentedControlItemText>
                      </SegmentedControlItem>
                    ))}
                  </SegmentedControl>
                </View>
                {history ? (
                  <>
                    <LevelChart
                      chart={span === "day" ? history.day : history.days}
                      span={span}
                      strings={strings}
                      locale={locale}
                    />
                    <ScreenChart history={history} span={span} strings={strings} locale={locale} />
                  </>
                ) : (
                  <Text style={{ fontSize: 13, color: theme.colors.textSecondary }} testID="battery-no-history">
                    {words.noHistory}
                  </Text>
                )}
              </View>
            }
          />
        </GroupedSection>
      ) : null}
      {apps ? (
        <GroupedSection title={words.apps} footer={words.appsDescription}>
          {apps.length === 0 ? (
            <GroupedSectionItem title={words.noApps} testID="battery-no-apps" />
          ) : (
            apps.map((app) => {
              const cpu = usageLabel(app.cpu, locale);
              const gpu = usageLabel(app.gpu, locale);
              return (
                <GroupedSectionItem
                  key={app.id}
                  title={app.name}
                  description={
                    app.running
                      ? `${words.cpu(cpu)} · ${words.gpu(gpu)}`
                      : `${words.cpu(cpu)} · ${words.gpu(gpu)} · ${words.notRunning}`
                  }
                  testID={`battery-app-${app.id}`}
                  accessibilityLabel={words.appLabel(app.name, cpu, gpu, app.significant)}
                  rightElement={app.significant ? <SignificantBadge text={words.significant} /> : undefined}
                />
              );
            })
          )}
        </GroupedSection>
      ) : null}
      <View style={{ alignItems: "flex-end" }}>
        <Button
          size="sm"
          appearance="outline"
          tone="neutral"
          testID="battery-options"
          accessibilityLabel={words.options}
          onPress={() => setOptions(true)}
        >
          {words.options}
        </Button>
      </View>
      <OptionsDialog {...props} open={options} onClose={() => setOptions(false)} />
    </View>
  );
}
