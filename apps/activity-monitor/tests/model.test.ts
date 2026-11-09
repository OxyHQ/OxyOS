import { test } from "node:test";
import assert from "node:assert/strict";
import {
  areaPath,
  canInspect,
  canQuit,
  cellTexts,
  compareRows,
  DEFAULT_SORTS,
  filterRows,
  flattenRows,
  formatBytes,
  formatCount,
  formatCpuTime,
  formatLoad,
  formatPercent,
  formatRate,
  formatShare,
  groupRows,
  History,
  HISTORY_LENGTH,
  inspectionOf,
  keepUnchanged,
  nextSort,
  pointOf,
  quitOutcome,
  rowView,
  segmentsOf,
  sortRows,
  stacked,
  type ActivitySample,
  type AppRow,
  type HistoryPoint,
  type ProcessRow,
  type SampleProcess,
  type TopRow,
} from "../src/model.ts";
import { CATALOGUES, messagesFor, messagesLanguage } from "../src/messages.ts";

function process(pid: number, name: string, extra: Partial<SampleProcess> = {}): SampleProcess {
  return {
    pid,
    ppid: 1,
    started: pid * 10,
    name,
    user: "nate",
    own: true,
    app: null,
    cpu: 0,
    cpuTime: 0,
    threads: 1,
    ...extra,
  };
}

function sample(processes: SampleProcess[], extra: Partial<ActivitySample> = {}): ActivitySample {
  return {
    view: "cpu",
    interval: 2,
    system: {
      cpu: { user: 10, system: 5, idle: 85, cores: 8 },
      load: [0.5, 0.6, 0.7],
      processes: processes.length,
      threads: 100,
      memory: {
        total: 16e9,
        used: 9e9,
        app: 6e9,
        wired: 1e9,
        compressed: 0,
        cached: 3e9,
        swapUsed: 0,
        swapTotal: 8e9,
        pressure: { some: 1, full: 0 },
        level: "normal",
      },
      disk: { read: 0, written: 0, reads: 0, writes: 0, readRate: 10, writeRate: 20, readsRate: 1, writesRate: 2 },
      network: {
        received: 0,
        sent: 0,
        packetsIn: 0,
        packetsOut: 0,
        receivedRate: 30,
        sentRate: 40,
        packetsInRate: 1,
        packetsOutRate: 1,
        interfaces: [
          {
            name: "wlan0",
            received: 5e9,
            sent: 1e9,
            packetsIn: 900,
            packetsOut: 100,
            receivedRate: 1000,
            sentRate: 10,
            packetsInRate: 1,
            packetsOutRate: 1,
          },
          {
            name: "eth0",
            received: 1e6,
            sent: 2e6,
            packetsIn: 10,
            packetsOut: 20,
            receivedRate: 0,
            sentRate: 0,
            packetsInRate: 0,
            packetsOutRate: 0,
          },
        ],
      },
    },
    apps: [
      { id: "so.oxy.settings", name: "Settings", pid: 100, icon: "/icons/settings.png" },
      { id: "so.oxy.allo", name: "Allo", pid: 200, icon: null },
    ],
    processes,
    ...extra,
  };
}

const PROCESSES = [
  process(100, "Settings", { app: "so.oxy.settings", cpu: 2, cpuTime: 5, threads: 10 }),
  process(101, "settings-helper", { app: "so.oxy.settings", cpu: 9, cpuTime: 1, threads: 3 }),
  process(200, "Allo", { app: "so.oxy.allo", cpu: 4, cpuTime: 2, threads: 4 }),
  process(1, "systemd", { user: "root", own: false, cpu: 0.5, cpuTime: 90 }),
  process(300, "oxyos-session", { cpu: 12, cpuTime: 400 }),
  process(400, "kworker", { user: "root", own: false, cpu: null }),
];

const names = (rows: readonly { name: string }[]) => rows.map((row) => row.name);

test("an app's processes are grouped under its row, its values summed; the rest are rows of their own", () => {
  const rows = groupRows(sample(PROCESSES), "cpu");
  const settings = rows.find((row) => row.key === "a:so.oxy.settings") as AppRow;
  assert.equal(settings.kind, "app");
  assert.equal(settings.name, "Settings");
  assert.equal(settings.icon, "/icons/settings.png");
  assert.equal(settings.pid, 100, "the app's own process gives its PID");
  assert.deepEqual(names(settings.children), ["Settings", "settings-helper"]);
  assert.ok(settings.children.every((child) => child.depth === 1));
  assert.equal(settings.metrics.cpu, 11);
  assert.equal(settings.metrics.threads, 13);
  assert.equal(settings.metrics.cpuTime, 6);
  assert.deepEqual(
    rows.filter((row) => row.kind === "process").map((row) => row.name).sort(),
    ["kworker", "oxyos-session", "systemd"],
  );
  // A sum of values none has is no value.
  const kworkerless = groupRows(sample([process(5, "x", { app: "so.oxy.allo", cpu: null })]), "cpu");
  assert.equal((kworkerless[0] as AppRow).metrics.cpu, null);
});

test("the Energy view lists every app the session ran, the ones not running greyed and last", () => {
  const energySample = sample(
    [process(100, "Settings", { app: "so.oxy.settings", energy: 3 }), process(300, "oxyos-session", { energy: 1 })],
    {
      view: "energy",
      energy: [
        { id: "so.oxy.settings", name: "Settings", impact: 3.5, average: 1.2, running: true },
        { id: "so.oxy.inbox", name: "Inbox", impact: null, average: 40, running: false },
      ],
    },
  );
  const rows = sortRows(groupRows(energySample, "energy"), DEFAULT_SORTS.energy);
  assert.deepEqual(names(rows), ["Settings", "oxyos-session", "Inbox"], "running apps and processes first");
  const inbox = rows[2] as AppRow;
  assert.equal(inbox.running, false);
  assert.equal(rowView(inbox, "energy", "en", "—", false).resting, true);
  assert.deepEqual(cellTexts(inbox, "energy", "en", "—"), ["Inbox", "", "40.0", "", ""]);
  // A process outside the apps has no 12-hour average.
  assert.equal(cellTexts(rows[1], "energy", "en", "—")[2], "—");
  // The app's impact is the session's measure of it.
  assert.equal(cellTexts(rows[0], "energy", "en", "—")[1], "3.5");
  // Without the battery service there is no average.
  const noAverage = groupRows(
    { ...energySample, energy: [{ id: "so.oxy.settings", name: "Settings", impact: 1, average: null, running: true }] },
    "energy",
  );
  assert.equal(cellTexts(noAverage.find((row) => row.kind === "app")!, "energy", "en", "—")[2], "—");
});

test("the Network view lists the interfaces, never processes", () => {
  const rows = groupRows(sample(PROCESSES), "network");
  assert.deepEqual(names(rows), ["wlan0", "eth0"]);
  assert.ok(rows.every((row) => row.kind === "interface"));
  assert.deepEqual(cellTexts(rows[0], "network", "en", "—"), ["wlan0", "5.00 GB", "1.00 GB", "1 KB/s", "10 bytes/s", "900", "100"]);
  assert.equal(canQuit(rows[0]), false);
  assert.equal(canInspect(rows[0]), false);
});

test("rows sort within their app and the apps among the rows by their sums", () => {
  const rows = groupRows(sample(PROCESSES), "cpu");
  const byCpu = sortRows(rows, { key: "cpu", direction: "descending" });
  assert.deepEqual(names(byCpu), ["oxyos-session", "Settings", "Allo", "systemd", "kworker"]);
  const settings = byCpu[1] as AppRow;
  assert.deepEqual(names(settings.children), ["settings-helper", "Settings"], "the helper uses more CPU");

  const ascending = sortRows(rows, { key: "cpu", direction: "ascending" });
  assert.deepEqual(names(ascending), ["systemd", "Allo", "Settings", "oxyos-session", "kworker"], "no value is last either way");
  assert.deepEqual(names((ascending[2] as AppRow).children), ["Settings", "settings-helper"]);

  const byName = sortRows(rows, { key: "name", direction: "ascending" });
  assert.deepEqual(names(byName), ["Allo", "kworker", "oxyos-session", "Settings", "systemd"]);
  const byTime = sortRows(rows, { key: "cpuTime", direction: "descending" });
  assert.deepEqual(names(byTime), ["oxyos-session", "systemd", "Settings", "Allo", "kworker"]);
  assert.deepEqual(names(sortRows(rows, { key: "user", direction: "descending" })).slice(0, 2), ["kworker", "systemd"]);
});

test("ties keep a stable order, by name and then PID", () => {
  const a = { kind: "process", key: "a", pid: 2, started: 0, name: "same", user: "", own: true, depth: 0, metrics: { cpu: 1, pid: 2 } } as ProcessRow;
  const b = { ...a, key: "b", pid: 1, metrics: { cpu: 1, pid: 1 } } as ProcessRow;
  assert.ok(compareRows(a, b, { key: "cpu", direction: "descending" }) > 0);
  assert.ok(compareRows(a, b, { key: "cpu", direction: "ascending" }) > 0);
});

test("a header's press turns its sort around, another's starts it as macOS does", () => {
  assert.deepEqual(nextSort({ key: "cpu", direction: "descending" }, "cpu"), { key: "cpu", direction: "ascending" });
  assert.deepEqual(nextSort({ key: "cpu", direction: "descending" }, "name"), { key: "name", direction: "ascending" });
  assert.deepEqual(nextSort({ key: "name", direction: "ascending" }, "memory"), { key: "memory", direction: "descending" });
});

test("the search finds names, users and PIDs, and an app by itself or by its processes", () => {
  const rows = sortRows(groupRows(sample(PROCESSES), "cpu"), DEFAULT_SORTS.cpu);
  const find = (query: string) => filterRows(rows, { query, mine: false });

  assert.deepEqual(names(find("SYSTEMD").rows), ["systemd"], "names, whatever the case");
  assert.deepEqual(names(find("root").rows), ["systemd", "kworker"], "users");
  assert.deepEqual(names(find("30").rows), ["oxyos-session"], "the start of a PID");

  const byApp = find("settings");
  assert.deepEqual(names(byApp.rows), ["Settings"]);
  assert.equal((byApp.rows[0] as AppRow).children.length, 2, "an app that matches keeps all its processes");
  assert.equal(byApp.searched.size, 0);

  const byChild = find("helper");
  assert.deepEqual(names(byChild.rows), ["Settings"], "an app shows when one of its processes matches");
  assert.deepEqual(names((byChild.rows[0] as AppRow).children), ["settings-helper"]);
  assert.ok(byChild.searched.has("a:so.oxy.settings"), "and is opened to show it");

  assert.deepEqual(find("nothing like it").rows, []);
  assert.equal(find("  ").rows.length, rows.length, "spaces alone search for nothing");
  assert.deepEqual(names(filterRows(groupRows(sample(PROCESSES), "network"), { query: "WL", mine: false }).rows), ["wlan0"]);
});

test("My Processes leaves only the person's own", () => {
  const rows = groupRows(sample(PROCESSES), "cpu");
  const mine = filterRows(rows, { query: "", mine: true }).rows;
  assert.ok(!names(mine).includes("systemd"));
  assert.ok(!names(mine).includes("kworker"));
  assert.ok(names(mine).includes("Settings"));
  // Within an app too.
  const mixed = groupRows(
    sample([process(100, "Settings", { app: "so.oxy.settings" }), process(102, "setuid-helper", { app: "so.oxy.settings", own: false })]),
    "cpu",
  );
  const kept = filterRows(mixed, { query: "", mine: true }).rows[0] as AppRow;
  assert.deepEqual(names(kept.children), ["Settings"]);
});

test("an open app's processes follow its row; a closed app's do not", () => {
  const rows = sortRows(groupRows(sample(PROCESSES), "cpu"), DEFAULT_SORTS.cpu);
  const closed = flattenRows(rows, () => false);
  assert.equal(closed.length, 5);
  const open = flattenRows(rows, (key) => key === "a:so.oxy.settings");
  assert.deepEqual(names(open).slice(0, 4), ["oxyos-session", "Settings", "settings-helper", "Settings"]);
  assert.equal(open.length, 7);
});

test("only the person's own processes and running apps may be quit", () => {
  const rows = groupRows(sample(PROCESSES), "cpu");
  const byName = (name: string) => rows.find((row) => row.name === name) as TopRow;
  assert.equal(canQuit(byName("oxyos-session")), true);
  assert.equal(canQuit(byName("systemd")), false, "root's needs an administrator");
  assert.equal(canQuit(byName("Settings")), true);
  assert.equal(canQuit(null), false);
  const resting = groupRows(
    sample([], { view: "energy", energy: [{ id: "so.oxy.inbox", name: "Inbox", impact: null, average: null, running: false }] }),
    "energy",
  );
  assert.equal(canQuit(resting[0]), false, "an app that does not run has nothing to quit");
  assert.equal(canInspect(resting[0]), false);
  assert.equal(canInspect(byName("systemd")), true, "another user's process can be inspected, restricted");
});

test("the session's answer to a quit, as the window says it", () => {
  assert.equal(quitOutcome({ activityQuit: {} }), null);
  assert.equal(quitOutcome({ activityQuitApp: {} }), null);
  assert.equal(quitOutcome({ failed: { reason: "notFound" } }), "notFound");
  assert.equal(quitOutcome({ refused: { reason: "notPermitted" } }), "notPermitted");
  assert.equal(quitOutcome({ refused: { reason: "needsInput" } }), "needsInput");
  assert.equal(quitOutcome({ refused: { reason: "invalidRequest" } }), "failed");
  assert.equal(quitOutcome(null), "failed");
  assert.equal(inspectionOf({ failed: { reason: "notFound" } }), "notFound");
  assert.equal(inspectionOf({ failed: { reason: "unavailable" } }), "failed");
  const inspection = inspectionOf({ activityInspect: { pid: 4, name: "x", restricted: true } });
  assert.equal(typeof inspection === "object" && inspection.pid, 4);
});

test("bytes are written as a Mac writes them, 1000-based", () => {
  assert.equal(formatBytes(0, "en"), "Zero KB");
  assert.equal(formatBytes(0, "es-ES"), "Cero KB");
  assert.equal(formatBytes(512, "en"), "512 bytes");
  assert.equal(formatBytes(64_000, "en"), "64 KB");
  assert.equal(formatBytes(512_000_000, "en"), "512.0 MB");
  assert.equal(formatBytes(1_234_567_890, "en"), "1.23 GB");
  assert.equal(formatBytes(1_234_567_890, "es-ES"), "1,23 GB");
  assert.equal(formatBytes(999_999_999, "en"), "1.00 GB", "a value that rounds up is written in the next unit");
  assert.equal(formatBytes(2.5e12, "en"), "2.50 TB");
  assert.equal(formatBytes(null, "en"), "");
  assert.equal(formatRate(1_200_000, "en"), "1.2 MB/s");
  assert.equal(formatRate(null, "en"), "");
});

test("percents, counts, CPU time and load averages, in English and Spanish", () => {
  assert.equal(formatPercent(12.46, "en"), "12.5");
  assert.equal(formatPercent(12.46, "es"), "12,5");
  assert.equal(formatPercent(0, "en"), "0.0");
  assert.equal(formatPercent(null, "en"), "");
  assert.equal(formatShare(3.14, "en"), "3.1%");
  assert.equal(formatShare(3.14, "es"), "3,1 %");
  assert.equal(formatCount(1093, "en"), "1,093");
  assert.equal(formatCount(12093, "es"), "12.093");
  assert.equal(formatCpuTime(3.456, "en"), "3.46");
  assert.equal(formatCpuTime(123.45, "en"), "2:03.45");
  assert.equal(formatCpuTime(3723.45, "en"), "1:02:03.45");
  assert.equal(formatCpuTime(3723.45, "es"), "1:02:03,45");
  assert.equal(formatCpuTime(60, "en"), "1:00.00");
  assert.equal(formatLoad([0.52, 0.61, 0.7], "en"), "0.52, 0.61, 0.70");
  assert.equal(formatLoad([0.52, 0.61, 0.7], "es"), "0,52; 0,61; 0,70");
});

test("a row's cells, in its view's columns", () => {
  const rows = groupRows(sample([process(300, "oxyos-session", { cpu: 12.34, cpuTime: 3723.45, threads: 1093, wakeups: 4.5, gpu: null })]), "cpu");
  assert.deepEqual(cellTexts(rows[0], "cpu", "en", "—"), ["oxyos-session", "12.3", "1:02:03.45", "1,093", "5", "", "300", "nate"]);
  const memory = groupRows(sample([process(7, "x", { memory: 512e6, swap: 0, threads: 2 })], { view: "memory" }), "memory");
  assert.deepEqual(cellTexts(memory[0], "memory", "es", "—"), ["x", "512,0 MB", "Cero KB", "2", "7", "nate"]);
  // Another user's disk counters are unreadable: empty, not zero.
  const disk = groupRows(sample([process(1, "systemd", { own: false, read: null, written: null })], { view: "disk" }), "disk");
  assert.deepEqual(cellTexts(disk[0], "disk", "en", "—"), ["systemd", "", "", "", "", "1", "nate"]);
});

test("a row whose words did not change is the same object, so it does not draw again", () => {
  const first = groupRows(sample(PROCESSES), "cpu").map((row) => rowView(row, "cpu", "en", "—", false));
  const previous = new Map(first.map((view) => [view.key, view]));
  const changed = PROCESSES.map((p) => (p.pid === 300 ? { ...p, cpu: 50 } : { ...p }));
  const next = keepUnchanged(
    previous,
    groupRows(sample(changed), "cpu").map((row) => rowView(row, "cpu", "en", "—", false)),
  );
  for (const view of next) {
    const before = previous.get(view.key);
    if (view.key === "p:300:3000") assert.notEqual(view, before);
    else assert.equal(view, before, view.key);
  }
  assert.equal(rowView(groupRows(sample(PROCESSES), "cpu")[0], "cpu", "en", "—", false).testID, "activity-row-1");
  const app = groupRows(sample(PROCESSES), "cpu").find((row) => row.kind === "app")!;
  assert.equal(rowView(app, "cpu", "en", "—", true).testID, "activity-app-so.oxy.settings");
  assert.equal(rowView(app, "cpu", "en", "—", true).expanded, true);
});

function point(at: number, interval: number, cpuUser: number | null = 10): HistoryPoint {
  return {
    at,
    interval,
    cpuUser,
    cpuSystem: 5,
    pressure: 0,
    level: "normal",
    readRate: 0,
    writeRate: 0,
    receivedRate: 0,
    sentRate: 0,
    energy: null,
  };
}

test("the history keeps the last ninety samples, oldest first", () => {
  const history = new History();
  for (let i = 0; i < HISTORY_LENGTH + 10; i += 1) history.push(point(i * 2000, 2));
  const points = history.points();
  assert.equal(points.length, HISTORY_LENGTH);
  assert.equal(points[0].at, 10 * 2000);
  assert.equal(points[points.length - 1].at, (HISTORY_LENGTH + 9) * 2000);
  assert.equal(history.version, HISTORY_LENGTH + 10);
  assert.equal(pointOf(sample(PROCESSES), 5).cpuUser, 10);
  assert.equal(pointOf(sample(PROCESSES), 5).energy, null, "energy is the Energy view's alone");
  assert.equal(
    pointOf(sample([process(1, "a", { energy: 2 }), process(2, "b", { energy: 3 })], { view: "energy" }), 5).energy,
    5,
  );
});

test("the graph's line breaks where the window was hidden, a follow began or a value is missing", () => {
  const user = (p: HistoryPoint) => p.cpuUser;
  const steady = [point(0, 0, null), point(2000, 2), point(4000, 2), point(6000, 2)];
  assert.equal(segmentsOf(steady, user).length, 1);
  assert.equal(segmentsOf(steady, user)[0].length, 3, "the first sample of a follow has no CPU");

  const hidden = [point(0, 2), point(2000, 2), point(2000 + 6000, 2), point(10000, 2)];
  assert.equal(segmentsOf(hidden, user).length, 2, "six seconds apart is more than 2.5 intervals");
  const slightlyLate = [point(0, 2), point(2000, 2), point(2000 + 4900, 2)];
  assert.equal(segmentsOf(slightlyLate, user).length, 1, "4.9 seconds is within 2.5 intervals");

  const refollowed = [point(0, 2), point(2000, 2), point(4000, 0, null), point(6000, 2)];
  assert.equal(segmentsOf(refollowed, user).length, 2);

  const old = [point(0, 2), point(200_000, 0), point(202_000, 2)];
  const segments = segmentsOf(old, user);
  assert.equal(segments.length, 1, "what is older than three minutes is not drawn");
  assert.equal(segments[0][segments[0].length - 1].x, 1, "the last sample is at the right edge");
});

test("a graph's area is one path, its runs closed shapes, stacked where asked", () => {
  const user = segmentsOf([point(0, 2), point(180_000, 2)], (p) => p.cpuUser);
  assert.deepEqual(user.map((run) => run.length), [1, 1], "180 s apart is a gap: two lone samples, each its own run");
  const system = segmentsOf([point(176_000, 2), point(178_000, 2), point(180_000, 2)], (p) => p.cpuSystem);
  assert.equal(areaPath(system, 180, 100, 100), "M176 95L178 95L180 95L180 100L178 100L176 100Z");
  const both = stacked(segmentsOf([point(176_000, 2), point(178_000, 2), point(180_000, 2)], (p) => p.cpuUser), system);
  assert.deepEqual(both[0].map((p) => p.y), [15, 15, 15]);
  assert.equal(areaPath(both, 180, 100, 100, system), "M176 85L178 85L180 85L180 95L178 95L176 95Z");
  assert.equal(areaPath([], 180, 100, 100), "");
  assert.equal(
    areaPath(system, 180, 50, 100, undefined, { downward: true, top: 50 }),
    "M176 52.5L178 52.5L180 52.5L180 50L178 50L176 50Z",
    "the lower half hangs from the middle",
  );
  assert.match(areaPath([[{ x: 1, y: 50 }]], 180, 100, 100), /^M180 50L181 50/, "one sample is a sliver");
});

test("the catalogues speak the same messages", () => {
  const shape = (value: unknown): unknown =>
    typeof value === "object" && value !== null
      ? Object.fromEntries(Object.entries(value).map(([key, inner]) => [key, shape(inner)]))
      : typeof value;
  assert.deepEqual(shape(CATALOGUES.es), shape(CATALOGUES.en));
  assert.equal(messagesLanguage("es-MX-u-hc-h12"), "es");
  assert.equal(messagesLanguage("fr-FR"), "en");
  assert.equal(messagesFor("es-ES").title, "Monitor de Actividad");
  assert.equal(messagesFor("en-US").quit.appTitle("Settings"), "Are you sure you want to quit “Settings”?");
  assert.equal(messagesFor("es").columns.wakeups, "Activaciones en reposo");
  assert.equal(messagesFor("en").inspector.title("Settings", 4242), "Settings (4242)");
});
