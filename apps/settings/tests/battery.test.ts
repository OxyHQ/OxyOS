import { test } from "node:test";
import assert from "node:assert/strict";
import {
  batteryStateLabel,
  chargeLimitFailureLabel,
  chargeLimitFailureOf,
  chartTicks,
  durationLabel,
  energyOf,
  historyOf,
  latestLevel,
  localDay,
  LOW_POWER_MODES,
  screenByDay,
  screenByHour,
  screenScale,
  significantApps,
  sleepChoiceLabel,
  sleepChoices,
  usageLabel,
  type AppEnergy,
  type HistoryChart,
  type PowerBattery,
} from "../src/battery.ts";
import { messagesFor } from "../src/messages.ts";
import { ENERGY_DEFAULTS, PREFERENCE_KEYS } from "../src/model.ts";

const en = messagesFor("en").battery;
const es = messagesFor("es").battery;

const battery = (extra: Partial<PowerBattery>): PowerBattery => ({
  percent: 64,
  state: "discharging",
  onAc: false,
  minutes: 200,
  low: false,
  ...extra,
});

/** Noon of 7 November 2026, UTC. */
const NOON = 1_794_052_800;
const UTC = () => 0;
const MADRID = () => 60;

test("the battery's state is said with its time, as a Mac's Battery settings say it", () => {
  assert.equal(batteryStateLabel(battery({}), en), "Discharging · 3 h 20 min remaining");
  assert.equal(batteryStateLabel(battery({}), es), "Descargando · quedan 3 h 20 min");
  assert.equal(
    batteryStateLabel(battery({ state: "charging", onAc: true, minutes: 45 }), en),
    "Charging · 45 min until full",
  );
  assert.equal(
    batteryStateLabel(battery({ state: "charging", onAc: true, minutes: 120 }), es),
    "Cargando · 2 h para cargarse",
  );
  // A time the session does not know is not said.
  assert.equal(batteryStateLabel(battery({ minutes: 0 }), en), "Discharging");
  assert.equal(batteryStateLabel(battery({ state: "full", onAc: true }), en), "Fully charged");
  // Plugged in, held at its charge limit.
  assert.equal(batteryStateLabel(battery({ state: "notCharging", onAc: true }), en), "Not charging");
  assert.equal(batteryStateLabel(battery({ state: "notCharging", onAc: true }), es), "No se está cargando");
  assert.equal(durationLabel(59.6, en), "1 h");
});

test("the sleep timers offer a Mac's choices, three hours at most, and the session's own", () => {
  assert.deepEqual(sleepChoices(2), [1, 2, 5, 10, 15, 30, 60, 120, 180, 0]);
  // A value set elsewhere (the preferences file) is offered where it falls, Never last.
  assert.deepEqual(sleepChoices(45), [1, 2, 5, 10, 15, 30, 45, 60, 120, 180, 0]);
  assert.deepEqual(
    [1, 2, 60, 120, 0].map((minutes) => sleepChoiceLabel(minutes, en)),
    ["1 minute", "2 minutes", "1 hour", "2 hours", "Never"],
  );
  assert.deepEqual(
    [1, 10, 180, 0].map((minutes) => sleepChoiceLabel(minutes, es)),
    ["1 minuto", "10 minutos", "3 horas", "Nunca"],
  );
});

test("the energy preferences are the session's, with its defaults", () => {
  assert.equal(PREFERENCE_KEYS.lowPowerMode, "low_power_mode");
  assert.equal(PREFERENCE_KEYS.batteryPercentage, "battery_percentage");
  assert.deepEqual(ENERGY_DEFAULTS, {
    batteryPercentage: false,
    lowPowerMode: "never",
    displayOffBattery: 2,
    displayOffAdapter: 10,
    sleepBattery: 10,
    sleepAdapter: 0,
    dimOnBattery: true,
  });
  assert.deepEqual(
    LOW_POWER_MODES.map((mode) => en.lowPowerModes[mode]),
    ["Never", "Always", "Only on Battery", "Only on Power Adapter"],
  );
});

test("an app's time is said in the largest units that matter", () => {
  assert.equal(usageLabel(0.4, "en"), "< 1 s");
  assert.equal(usageLabel(12, "en"), "12 s");
  assert.equal(usageLabel(245, "en"), "4 min 5 s");
  assert.equal(usageLabel(300, "en"), "5 min");
  assert.equal(usageLabel(7380, "en"), "2 h 3 min");
  assert.equal(usageLabel(3600, "es"), "1 h");
});

test("the session's answers are read, and a charge limit refused says why", () => {
  assert.equal(chargeLimitFailureOf({ chargeLimit: {} }), null);
  assert.equal(chargeLimitFailureOf({ failed: { reason: "notPermitted" } }), "notPermitted");
  assert.equal(chargeLimitFailureOf({ refused: { reason: "needsInput" } }), "failed");
  assert.equal(chargeLimitFailureOf(null), "failed");
  assert.equal(chargeLimitFailureLabel("notPermitted", en), "Only an administrator can change this.");
  assert.equal(chargeLimitFailureLabel("notPermitted", es), "Solo un administrador puede cambiar esto.");
  const chart: HistoryChart = { end: NOON, bin: 900, levels: [null, 64], charging: [false, false], screen: [0, 900] };
  assert.deepEqual(historyOf({ powerHistory: { day: chart, days: chart } }), { day: chart, days: chart });
  assert.equal(historyOf({ powerHistory: { day: chart, days: { ...chart, screen: [] } } }), null);
  assert.equal(historyOf({ refused: { reason: "notPermitted" } }), null);
  assert.deepEqual(energyOf({ energy: { apps: [] } }), []);
  assert.equal(energyOf({ failed: {} }), null);
});

test("the ten days' screen use is summed by the local day it falls in", () => {
  // 48 hours ending at noon UTC: 12 h of 7 November, all of the 6th and 12 h of the 5th.
  const screen = Array.from({ length: 48 }, () => 1800);
  const chart: HistoryChart = {
    end: NOON,
    bin: 3600,
    levels: screen.map(() => 50),
    charging: screen.map(() => false),
    screen,
  };
  assert.deepEqual(
    screenByDay(chart, UTC).map(({ day, seconds }) => [day, seconds]),
    [
      ["2026-11-05", 12 * 1800],
      ["2026-11-06", 24 * 1800],
      ["2026-11-07", 12 * 1800],
    ],
  );
  // In Madrid (UTC+1) the days begin an hour sooner.
  assert.deepEqual(
    screenByDay(chart, MADRID).map(({ day, seconds }) => [day, seconds]),
    [
      ["2026-11-05", 11 * 1800],
      ["2026-11-06", 24 * 1800],
      ["2026-11-07", 13 * 1800],
    ],
  );
  // At most ten days, the newest kept.
  const long = { ...chart, levels: Array(240).fill(null), charging: Array(240).fill(false), screen: Array(240).fill(60) };
  assert.equal(screenByDay(long, UTC).length, 10);
  assert.equal(localDay(NOON - 1, UTC), "2026-11-07");
  assert.equal(screenScale([0, 3 * 3600 + 1]), 4 * 3600);
  assert.equal(screenScale([0]), 3600);
});

test("the day's quarters are summed by the hour, and labelled every six hours", () => {
  const screen = Array.from({ length: 96 }, (_, index) => (index % 4 === 0 ? 900 : 0));
  const day: HistoryChart = {
    end: NOON,
    bin: 900,
    levels: screen.map(() => 64),
    charging: screen.map(() => false),
    screen,
  };
  const hours = screenByHour(day);
  assert.equal(hours.length, 24);
  assert.ok(hours.every((seconds) => seconds === 900));
  // From noon yesterday: 18:00, 00:00, 06:00, the chart's start at noon too.
  assert.deepEqual(
    chartTicks(day, "day", UTC).map((tick) => tick.at),
    [0, 0.25, 0.5, 0.75],
  );
  const days: HistoryChart = { ...day, bin: 3600, levels: Array(240).fill(50), charging: Array(240).fill(false), screen: Array(240).fill(0) };
  // Each midnight of the ten days: nine whole and the start of today's.
  assert.equal(chartTicks(days, "days", UTC).length, 10);
  assert.equal(latestLevel({ ...day, levels: [...day.levels.slice(0, 95), null] }), 64);
  assert.equal(latestLevel({ ...day, levels: day.levels.map(() => null) }), null);
});

test("significant energy is a running app's alone", () => {
  const app = (id: string, significant: boolean, running: boolean): AppEnergy => ({
    id,
    name: id,
    cpu: 10,
    gpu: 1,
    recent: significant ? 0.5 : 0.01,
    significant,
    running,
  });
  assert.deepEqual(
    significantApps([app("a", true, true), app("b", false, true), app("c", true, false)]).map((a) => a.id),
    ["a"],
  );
});
