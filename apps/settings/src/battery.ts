/**
 * Battery and energy (docs/adr/0057-battery-and-energy.mdx): what the session tells Settings of the
 * battery (the `settings` grant), what Settings may ask of it, and the words and charts Settings ›
 * Battery makes of them. The session alone speaks to UPower, power-profiles-daemon and logind; Low
 * Power Mode, the display's and the machine's sleep and the menu bar's percentage are preferences.
 */

/** What the battery is doing, as the session names it. */
export type BatteryActivity = "charging" | "discharging" | "full" | "notCharging";

export type PowerBattery = {
  /** 0 to 100. */
  percent: number;
  state: BatteryActivity;
  onAc: boolean;
  /** Until empty (discharging) or full (charging); 0 unknown. */
  minutes: number;
  /** Discharging at 10 % or below, or UPower warns. */
  low: boolean;
};

/** The battery's health, as UPower reads it from the battery itself. */
export type PowerHealth = {
  /** Its full charge as a share of its design, in percent; null where the firmware says none. */
  maxCapacity: number | null;
  /** Charge cycles, where the firmware reports them. */
  cycles: number | null;
  /** Below 80 % of its design: a Mac's *Service Recommended*. */
  worn: boolean;
};

/** UPower's charge threshold: where charging stops while it is on. */
export type ChargeLimit = { enabled: boolean; end: number | null; start: number | null };

/** When Low Power Mode is on, as the session's preference keeps it. */
export type LowPowerMode = "never" | "always" | "battery" | "adapter";

export type PowerState = {
  /** None: the machine has no battery (a desktop), or UPower is not there. */
  battery: PowerBattery | null;
  health: PowerHealth | null;
  /** None: UPower cannot limit this battery's charge. */
  chargeLimit: ChargeLimit | null;
  lowPower: {
    mode: LowPowerMode;
    /** In effect now (the mode and the power source). */
    on: boolean;
    /** power-profiles-daemon is there. */
    profiles: boolean;
    /** The session holds its power-saver profile. */
    held: boolean;
  };
  showPercentage: boolean;
  /** What UPower does when the battery runs out (`HybridSleep`, `Hibernate`, `PowerOff`…). */
  criticalAction: string | null;
};

/** One chart of the history: `levels.length` bins of `bin` seconds, the last ending at `end` (seconds
    since the epoch); each bin's charge at its end (none where no session ran), whether it charged
    in it and the seconds the display was on in it. */
export type HistoryChart = {
  end: number;
  bin: number;
  levels: (number | null)[];
  charging: boolean[];
  screen: number[];
};

/** The last 24 hours by the quarter of an hour, and the last ten days by the hour. */
export type PowerHistory = { day: HistoryChart; days: HistoryChart };

/** An application's energy this session, the most first. */
export type AppEnergy = {
  id: string;
  name: string;
  /** Seconds of the processor and of the GPU. */
  cpu: number;
  gpu: number;
  /** Its recent use, in shares of one core. */
  recent: number;
  significant: boolean;
  running: boolean;
};

/** Why a charge limit was not changed. */
export type ChargeLimitFailure = "unsupported" | "notPermitted" | "unavailable" | "failed";

/** What Settings asks of the session. Each answers once it is done. */
export type PowerActions = {
  /** The history, or null when the session could not say it. */
  history: () => Promise<PowerHistory | null>;
  /** Each application's energy, sampled now; null when the session could not say it. */
  energy: () => Promise<AppEnergy[] | null>;
  /** The battery's charge limited or not; why not, or null once done. */
  setChargeLimit: (enabled: boolean) => Promise<ChargeLimitFailure | null>;
};

/** The choices of Low Power Mode, in a Mac's order. */
export const LOW_POWER_MODES: readonly LowPowerMode[] = ["never", "always", "battery", "adapter"];

/** The choices of how long before the display turns off or the machine sleeps, in minutes (0:
    never); at most three hours, as the session takes them. */
export const SLEEP_CHOICES = [1, 2, 5, 10, 15, 30, 60, 120, 180, 0] as const;

/** The two charts' spans. */
export type HistorySpan = "day" | "days";

/** The words the logic below needs (Settings' catalogue's `battery`). */
export type BatteryWords = {
  hoursMinutes: (hours: number, minutes: number) => string;
  remaining: (duration: string) => string;
  untilFull: (duration: string) => string;
  discharging: string;
  charging: string;
  full: string;
  notCharging: string;
  never: string;
  minutes: (n: number) => string;
  hours: (n: number) => string;
  failures: Record<ChargeLimitFailure, string>;
};

/** A duration in minutes as "3 h 20 min", "45 min", "2 h". */
export function durationLabel(minutes: number, words: Pick<BatteryWords, "hoursMinutes">): string {
  const whole = Math.max(0, Math.round(minutes));
  return words.hoursMinutes(Math.floor(whole / 60), whole % 60);
}

/** What the battery is doing in words, with its time where the session knows it: "Discharging · 3 h
    20 min remaining", "Charging · 45 min until full", "Fully charged", "Not charging" (plugged in,
    held at its limit). */
export function batteryStateLabel(battery: PowerBattery, words: BatteryWords): string {
  switch (battery.state) {
    case "charging":
      return battery.minutes > 0
        ? `${words.charging} · ${words.untilFull(durationLabel(battery.minutes, words))}`
        : words.charging;
    case "discharging":
      return battery.minutes > 0
        ? `${words.discharging} · ${words.remaining(durationLabel(battery.minutes, words))}`
        : words.discharging;
    case "full":
      return words.full;
    case "notCharging":
      return words.notCharging;
  }
}

/** A sleep timer's choice in words: "2 minutes", "1 hour", "Never". */
export function sleepChoiceLabel(minutes: number, words: Pick<BatteryWords, "never" | "minutes" | "hours">): string {
  if (minutes === 0) return words.never;
  return minutes % 60 === 0 ? words.hours(minutes / 60) : words.minutes(minutes);
}

/** The choices offered for a timer: the list, and the session's value where it is not one of them. */
export function sleepChoices(current: number): number[] {
  const timed: number[] = SLEEP_CHOICES.filter((minutes) => minutes !== 0);
  if (current !== 0 && !timed.includes(current)) timed.push(current);
  return [...timed.sort((a, b) => a - b), 0];
}

/** A time used, for an application's row: "2 h 3 min", "4 min 5 s", "12 s", "< 1 s". */
export function usageLabel(seconds: number, language: string): string {
  const number = (n: number) => new Intl.NumberFormat(language).format(n);
  if (seconds < 1) return `< ${number(1)} s`;
  const whole = Math.round(seconds);
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const rest = whole % 60;
  if (hours > 0) return minutes > 0 ? `${number(hours)} h ${number(minutes)} min` : `${number(hours)} h`;
  if (minutes > 0) return rest > 0 ? `${number(minutes)} min ${number(rest)} s` : `${number(minutes)} min`;
  return `${number(rest)} s`;
}

/** Why a charge limit was not changed, in words. */
export function chargeLimitFailureLabel(reason: ChargeLimitFailure, words: Pick<BatteryWords, "failures">): string {
  return words.failures[reason] ?? words.failures.failed;
}

/** The session's answer to a charge limit: why it was not changed, or null once done. */
export function chargeLimitFailureOf(answer: unknown): ChargeLimitFailure | null {
  const told = answer as { chargeLimit?: unknown; failed?: { reason?: string }; refused?: { reason?: string } } | null;
  if (told && told.chargeLimit) return null;
  const reason = told?.failed?.reason ?? told?.refused?.reason;
  return reason === "unsupported" || reason === "notPermitted" || reason === "unavailable" ? reason : "failed";
}

/** The session's history answer, read; null when it is not one. */
export function historyOf(answer: unknown): PowerHistory | null {
  const told = (answer as { powerHistory?: PowerHistory } | null)?.powerHistory;
  return told && isChart(told.day) && isChart(told.days) ? told : null;
}

function isChart(chart: unknown): chart is HistoryChart {
  const c = chart as HistoryChart | null;
  return (
    !!c &&
    typeof c.end === "number" &&
    typeof c.bin === "number" &&
    Array.isArray(c.levels) &&
    Array.isArray(c.charging) &&
    Array.isArray(c.screen) &&
    c.levels.length === c.charging.length &&
    c.levels.length === c.screen.length
  );
}

/** The session's energy answer, read; null when it is not one. */
export function energyOf(answer: unknown): AppEnergy[] | null {
  const apps = (answer as { energy?: { apps?: unknown } } | null)?.energy?.apps;
  return Array.isArray(apps) ? (apps as AppEnergy[]) : null;
}

/** When bin `index` of `chart` begins, in seconds since the epoch. */
export function binStart(chart: HistoryChart, index: number): number {
  return chart.end - (chart.levels.length - index) * chart.bin;
}

/** The local calendar day a moment is in, as "YYYY-MM-DD", by the time zone's offset in minutes
    at that moment (east of UTC positive; the runtime's own by default). */
export function localDay(seconds: number, offsetMinutes: (seconds: number) => number = runtimeOffset): string {
  const shifted = new Date((seconds + offsetMinutes(seconds) * 60) * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${shifted.getUTCFullYear()}-${pad(shifted.getUTCMonth() + 1)}-${pad(shifted.getUTCDate())}`;
}

function runtimeOffset(seconds: number): number {
  return -new Date(seconds * 1000).getTimezoneOffset();
}

/** One day of the ten-day chart: when it begins (the first of its bins), and the display's seconds on. */
export type DayUsage = { day: string; start: number; seconds: number };

/** The display's use per local day, the ten-day chart's hours summed by the day they fall in, the
    last ten days (today last); the oldest, cut by the chart's start, only as much as the chart has. */
export function screenByDay(
  chart: HistoryChart,
  offsetMinutes: (seconds: number) => number = runtimeOffset,
  days = 10,
): DayUsage[] {
  const byDay: DayUsage[] = [];
  chart.screen.forEach((seconds, index) => {
    const start = binStart(chart, index);
    const day = localDay(start, offsetMinutes);
    const last = byDay[byDay.length - 1];
    if (last && last.day === day) last.seconds += seconds;
    else byDay.push({ day, start, seconds });
  });
  return byDay.slice(-days);
}

/** Where a chart's labels go: each local hour a multiple of `everyHours` (the day chart: 12 AM, 6, 12
    PM, 6), or each local day's start (the ten-day chart), as fractions of the chart's width. */
export function chartTicks(
  chart: HistoryChart,
  span: HistorySpan,
  offsetMinutes: (seconds: number) => number = runtimeOffset,
  everyHours = 6,
): { at: number; seconds: number }[] {
  const ticks: { at: number; seconds: number }[] = [];
  const n = chart.levels.length;
  const width = n * chart.bin;
  const start = chart.end - width;
  for (let index = 0; index < n; index++) {
    const seconds = binStart(chart, index);
    const local = seconds + offsetMinutes(seconds) * 60;
    const hour = Math.floor(local / 3600);
    const minuteOfHour = Math.floor(local / 60) % 60;
    if (minuteOfHour !== 0) continue;
    const startsDay = ((hour % 24) + 24) % 24 === 0;
    if (span === "days" ? startsDay : hour % everyHours === 0) {
      ticks.push({ at: (seconds - start) / width, seconds });
    }
  }
  return ticks;
}

/** The charge now: the newest bin's, or the newest known. */
export function latestLevel(chart: HistoryChart): number | null {
  for (let index = chart.levels.length - 1; index >= 0; index--) {
    const level = chart.levels[index];
    if (level !== null && level !== undefined) return level;
  }
  return null;
}

/** The applications using significant energy now, as the battery's menu lists them. */
export function significantApps(apps: readonly AppEnergy[]): AppEnergy[] {
  return apps.filter((app) => app.running && app.significant);
}

/** The highest bar of a screen chart, rounded up to whole hours (at least one). */
export function screenScale(secondsPerBar: readonly number[]): number {
  const most = Math.max(0, ...secondsPerBar);
  return Math.max(3600, Math.ceil(most / 3600) * 3600);
}

/** The display's use per hour of the day chart (its quarters summed by four), the newest last. */
export function screenByHour(chart: HistoryChart): number[] {
  const per = Math.max(1, Math.round(3600 / chart.bin));
  const hours: number[] = [];
  for (let index = 0; index < chart.screen.length; index += per) {
    hours.push(chart.screen.slice(index, index + per).reduce((sum, seconds) => sum + seconds, 0));
  }
  return hours;
}
