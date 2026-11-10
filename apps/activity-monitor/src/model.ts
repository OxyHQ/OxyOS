/**
 * Activity Monitor's model: what the session tells it of the machine's activity (the `activity`
 * grant: `activityFollow`, `activityQuit`, `activityQuitApp`, `activityInspect`), and what the
 * window makes of it, pure so the tests can hold it to macOS's Activity Monitor: the processes
 * grouped under the apps they belong to, sorted within and across the groups, filtered by the
 * search and the View menu's choice, the system's last three minutes for the graphs, and every
 * number written as a Mac writes it.
 *
 * The session samples only while the window shows and something follows (`follow`), every two
 * seconds; the app keeps no timer of its own, so every change on screen is a sample's.
 */

/** The table's views, in the toolbar's order. */
export type ActivityView = "cpu" | "memory" | "energy" | "disk" | "network";
export const VIEWS: readonly ActivityView[] = ["cpu", "memory", "energy", "disk", "network"];

/** A disk's counters since boot and their rates over the sample's interval (null on the first). */
export type DiskActivity = {
  read: number;
  written: number;
  reads: number;
  writes: number;
  readRate: number | null;
  writeRate: number | null;
  readsRate: number | null;
  writesRate: number | null;
};

/** The network's counters since boot and their rates (null on the first sample). */
export type Traffic = {
  received: number;
  sent: number;
  packetsIn: number;
  packetsOut: number;
  receivedRate: number | null;
  sentRate: number | null;
  packetsInRate: number | null;
  packetsOutRate: number | null;
};

export type NetworkInterface = Traffic & { name: string };

export type MemoryLevel = "normal" | "warning" | "critical";

export type SystemActivity = {
  /** Percent of the whole machine; null on the first sample of a follow. */
  cpu: { user: number; system: number; idle: number; cores: number } | null;
  load: [number, number, number];
  processes: number;
  threads: number;
  memory: {
    total: number;
    used: number;
    app: number;
    wired: number;
    compressed: number;
    cached: number;
    swapUsed: number;
    swapTotal: number;
    /** Percent of the interval some or all tasks stalled on memory (PSI); null without it. */
    pressure: { some: number; full: number } | null;
    level: MemoryLevel;
  };
  disk: DiskActivity;
  network: Traffic & { interfaces: NetworkInterface[] };
};

/** An application the session launched, and its process. */
export type SampleApp = { id: string; name: string; pid: number; icon: string | null };

/** A process; the fields past `threads` are the sampled view's only. */
export type SampleProcess = {
  pid: number;
  ppid: number;
  /** Clock ticks since boot: with the pid, names the process exactly. */
  started: number;
  name: string;
  user: string;
  /** The person's own (real, effective and saved uid). */
  own: boolean;
  /** The launched application whose process tree it is in. */
  app: string | null;
  /** Percent of one core over the interval; null on the first sample. */
  cpu: number | null;
  cpuTime: number;
  threads: number;
  wakeups?: number | null;
  gpu?: number | null;
  gpuTime?: number | null;
  memory?: number;
  swap?: number;
  resident?: number;
  gpuMemory?: number | null;
  energy?: number | null;
  read?: number | null;
  written?: number | null;
  readRate?: number | null;
  writeRate?: number | null;
};

/** An application this session ran, running or not, and its energy (the Energy view). */
export type EnergyApp = {
  id: string;
  name: string;
  impact: number | null;
  /** The 12-hour average, in the same unit; null without the battery service. */
  average: number | null;
  running: boolean;
};

/** One sample, as the session tells it (`{activity: …}`, unasked). */
export type ActivitySample = {
  view: ActivityView;
  /** Seconds the rates cover; 0 on the first sample of a follow. */
  interval: number;
  system: SystemActivity;
  apps: SampleApp[];
  processes: SampleProcess[];
  energy?: EnergyApp[];
};

/** A process inspected (`{activityInspect: …}`). */
export type Inspection = {
  pid: number;
  started: number;
  name: string;
  path: string | null;
  arguments: string[] | null;
  ppid: number;
  parentName: string | null;
  user: string;
  own: boolean;
  app: string | null;
  /** Unix seconds. */
  startedAt: number;
  cpuTime: number;
  threads: number;
  memory: {
    resident: number;
    footprint: number;
    unique: number;
    proportional: number;
    swap: number;
    gpu: number;
  } | null;
  files: { fd: number; path: string }[] | null;
  ports: { fd: number; kind: string; local: string; remote: string | null; state: string | null }[] | null;
  cgroup: string | null;
  /** Another user's process: its details need an administrator (the fields above are null). */
  restricted: boolean;
};

/** A session answer as it arrives: `{activityQuit: {}}`, `{refused: {reason}}`, `{failed: {reason}}`… */
export type SessionAnswer = Record<string, unknown> | null | undefined;

/**
 * What Activity Monitor asks of the platform: on OxyOS the session's `activity` vocabulary
 * (`@oxyos/platform`'s `activity`, docs/adr/0058-the-platform-api.mdx), in the browser preview a
 * fixture.
 */
export type ActivitySource = {
  /**
   * Follows the samples of `view`: the session samples from the first follower until the last
   * stops (the function returned), and only while the window shows.
   */
  follow(view: ActivityView, listener: (sample: ActivitySample) => void): () => void;
  /** Changes the view followed; a sample of it follows at once. Nothing while none follows. */
  setView(view: ActivityView): void;
  /** SIGTERM, or SIGKILL with `force`; cites the press or key that asked. */
  quit(pid: number, started: number, force: boolean): Promise<SessionAnswer>;
  /** The app's own close request to each window, or with `force` the session's Force Quit. */
  quitApp(id: string, force: boolean): Promise<SessionAnswer>;
  inspect(pid: number, started: number): Promise<SessionAnswer>;
};

// ---------------------------------------------------------------------------------------------
//  Rows
// ---------------------------------------------------------------------------------------------

/** What a column sorts by and shows. */
export type ColumnKey =
  | "name"
  | "cpu"
  | "cpuTime"
  | "threads"
  | "wakeups"
  | "gpu"
  | "pid"
  | "user"
  | "memory"
  | "swap"
  | "energy"
  | "average"
  | "written"
  | "read"
  | "writeRate"
  | "readRate"
  | "received"
  | "sent"
  | "receivedRate"
  | "sentRate"
  | "packetsIn"
  | "packetsOut";

/** Each view's columns, in macOS's order. */
export const COLUMNS: Record<ActivityView, readonly ColumnKey[]> = {
  cpu: ["name", "cpu", "cpuTime", "threads", "wakeups", "gpu", "pid", "user"],
  memory: ["name", "memory", "swap", "threads", "pid", "user"],
  energy: ["name", "energy", "average", "pid", "user"],
  disk: ["name", "written", "read", "writeRate", "readRate", "pid", "user"],
  network: ["name", "received", "sent", "receivedRate", "sentRate", "packetsIn", "packetsOut"],
};

/** The numbers a row sums or shows, by column. */
type Metrics = Partial<Record<ColumnKey, number | null>>;

export type ProcessRow = {
  kind: "process";
  key: string;
  pid: number;
  started: number;
  name: string;
  user: string;
  own: boolean;
  /** 1 under an app's row. */
  depth: 0 | 1;
  metrics: Metrics;
};

export type AppRow = {
  kind: "app";
  key: string;
  id: string;
  name: string;
  icon: string | null;
  /** The app's own process, when it runs. */
  pid: number | null;
  started: number | null;
  user: string;
  /** Every process of it is the person's (an app the session launched always is). */
  own: boolean;
  running: boolean;
  children: ProcessRow[];
  metrics: Metrics;
};

export type InterfaceRow = { kind: "interface"; key: string; name: string; metrics: Metrics };

export type TopRow = AppRow | ProcessRow | InterfaceRow;
export type Row = TopRow;

export const processKey = (pid: number, started: number) => `p:${pid}:${started}`;
export const appKey = (id: string) => `a:${id}`;
export const interfaceKey = (name: string) => `i:${name}`;

function processMetrics(process: SampleProcess): Metrics {
  return {
    cpu: process.cpu,
    cpuTime: process.cpuTime,
    threads: process.threads,
    wakeups: process.wakeups ?? null,
    gpu: process.gpu ?? null,
    pid: process.pid,
    memory: process.memory ?? null,
    swap: process.swap ?? null,
    energy: process.energy ?? null,
    written: process.written ?? null,
    read: process.read ?? null,
    writeRate: process.writeRate ?? null,
    readRate: process.readRate ?? null,
  };
}

/** The columns an app's row sums over its processes. */
const SUMMED: readonly ColumnKey[] = [
  "cpu",
  "cpuTime",
  "threads",
  "wakeups",
  "gpu",
  "memory",
  "swap",
  "energy",
  "written",
  "read",
  "writeRate",
  "readRate",
];

/** The sum of the processes' values of each column: null where none has one. */
export function summed(children: readonly ProcessRow[]): Metrics {
  const totals: Metrics = {};
  for (const key of SUMMED) {
    let total: number | null = null;
    for (const child of children) {
      const value = child.metrics[key];
      if (value != null) total = (total ?? 0) + value;
    }
    totals[key] = total;
  }
  return totals;
}

function processRow(process: SampleProcess, depth: 0 | 1): ProcessRow {
  return {
    kind: "process",
    key: processKey(process.pid, process.started),
    pid: process.pid,
    started: process.started,
    name: process.name,
    user: process.user,
    own: process.own,
    depth,
    metrics: processMetrics(process),
  };
}

/**
 * The table's rows for `view`: each launched app's processes under the app's row (its values
 * summed), every other process a row of its own; in the Energy view every app this session ran,
 * running or not; in the Network view the interfaces, as the session gives no process's traffic.
 */
export function groupRows(sample: ActivitySample, view: ActivityView): TopRow[] {
  if (view === "network") {
    return sample.system.network.interfaces.map((network) => ({
      kind: "interface",
      key: interfaceKey(network.name),
      name: network.name,
      metrics: {
        received: network.received,
        sent: network.sent,
        receivedRate: network.receivedRate,
        sentRate: network.sentRate,
        packetsIn: network.packetsIn,
        packetsOut: network.packetsOut,
      },
    }));
  }
  const apps = new Map(sample.apps.map((app) => [app.id, app]));
  const energy = view === "energy" && sample.energy ? new Map(sample.energy.map((app) => [app.id, app])) : null;
  const children = new Map<string, ProcessRow[]>();
  const rows: TopRow[] = [];
  for (const process of sample.processes) {
    if (process.app === null) {
      rows.push(processRow(process, 0));
      continue;
    }
    const group = children.get(process.app);
    if (group) group.push(processRow(process, 1));
    else children.set(process.app, [processRow(process, 1)]);
  }
  const ids = new Set<string>(children.keys());
  if (energy) for (const id of energy.keys()) ids.add(id);
  for (const id of ids) {
    const app = apps.get(id);
    const ran = energy?.get(id);
    const own = children.get(id) ?? [];
    const main = own.find((child) => child.pid === app?.pid) ?? own[0];
    const metrics = summed(own);
    if (ran) {
      // The session's own measure of the app, which counts what its tree's processes no longer do.
      metrics.energy = ran.impact;
      metrics.average = ran.average;
    }
    rows.push({
      kind: "app",
      key: appKey(id),
      id,
      name: app?.name ?? ran?.name ?? id,
      icon: app?.icon ?? null,
      pid: main?.pid ?? null,
      started: main?.started ?? null,
      user: main?.user ?? "",
      own: own.every((child) => child.own),
      running: own.length > 0 || (ran?.running ?? false),
      children: own,
      metrics: { ...metrics, pid: main?.pid ?? null },
    });
  }
  return rows;
}

// ---------------------------------------------------------------------------------------------
//  Sorting
// ---------------------------------------------------------------------------------------------

export type SortDirection = "ascending" | "descending";
export type Sort = { key: ColumnKey; direction: SortDirection };

/** A column's first direction: names and users A to Z, numbers the largest first, as macOS. */
export function firstDirection(key: ColumnKey): SortDirection {
  return key === "name" || key === "user" || key === "pid" ? "ascending" : "descending";
}

/** The sort after pressing `key`'s header: the other direction of the same column, or its first. */
export function nextSort(sort: Sort, key: ColumnKey): Sort {
  if (sort.key === key) return { key, direction: sort.direction === "ascending" ? "descending" : "ascending" };
  return { key, direction: firstDirection(key) };
}

/** Each view's sort as it opens, as macOS's. */
export const DEFAULT_SORTS: Record<ActivityView, Sort> = {
  cpu: { key: "cpu", direction: "descending" },
  memory: { key: "memory", direction: "descending" },
  energy: { key: "energy", direction: "descending" },
  disk: { key: "written", direction: "descending" },
  network: { key: "received", direction: "descending" },
};

/** A row's value for `key`: its name and user as text, every other column as a number or null. */
export function columnValue(row: Row, key: ColumnKey): number | string | null {
  if (key === "name") return row.name;
  if (key === "user") return row.kind === "interface" ? null : row.user;
  if (key === "pid" && row.kind === "process") return row.pid;
  return row.metrics[key] ?? null;
}

/** Names compared as a person reads them (case aside, digits as numbers): `localeCompare`, since the
    Bloom host's Hermes has no `Intl.Collator`. */
const collator = {
  compare: (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true }),
};

/**
 * Each name's place among `names` in a person's order (`collator`), so a sort compares numbers:
 * `localeCompare` for every pair of hundreds of rows on every sample would be thousands of calls.
 * The order of the last set of names is kept, since a sample's names are mostly the last one's.
 */
let lastRanks: { key: string; ranks: Map<string, number> } | null = null;
export function rankNames(names: Iterable<string>): Map<string, number> {
  const unique = [...new Set(names)].sort();
  const key = unique.join("\u0000");
  if (lastRanks?.key === key) return lastRanks.ranks;
  const ranks = new Map(unique.sort(collator.compare).map((name, index) => [name, index]));
  lastRanks = { key, ranks };
  return ranks;
}

/** Two texts in a person's order, by their ranks when given. */
function compareText(a: string, b: string, ranks?: Map<string, number>): number {
  const left = ranks?.get(a);
  const right = ranks?.get(b);
  if (left !== undefined && right !== undefined) return left - right;
  return collator.compare(a, b);
}

/**
 * The order of two rows by `sort`: a row without a value always last, whichever the direction, so
 * the rows that have one stay together; ties by name, then by PID, so a sample never shuffles them.
 * `ranks` (rankNames of their names and users) spares comparing texts.
 */
export function compareRows(a: Row, b: Row, sort: Sort, ranks?: Map<string, number>): number {
  // Apps that no longer run (the Energy view's) go after everything that does.
  const resting = Number(a.kind === "app" && !a.running) - Number(b.kind === "app" && !b.running);
  if (resting !== 0) return resting;
  const left = columnValue(a, sort.key);
  const right = columnValue(b, sort.key);
  if (left === null || right === null || left === "" || right === "") {
    const missing = Number(left === null || left === "") - Number(right === null || right === "");
    if (missing !== 0) return missing;
  } else {
    const order =
      typeof left === "string" || typeof right === "string"
        ? compareText(String(left), String(right), ranks)
        : left - right;
    if (order !== 0) return sort.direction === "ascending" ? order : -order;
  }
  const byName = compareText(a.name, b.name, ranks);
  if (byName !== 0) return byName;
  return (Number(columnValue(a, "pid")) || 0) - (Number(columnValue(b, "pid")) || 0);
}

/** The rows sorted by `sort`, the apps among the rows by their sums and each app's processes among themselves. */
export function sortRows(rows: readonly TopRow[], sort: Sort): TopRow[] {
  const texts: string[] = [];
  for (const row of rows) {
    texts.push(row.name);
    if (row.kind !== "interface") texts.push(row.user);
    if (row.kind === "app") for (const child of row.children) texts.push(child.name, child.user);
  }
  const ranks = rankNames(texts);
  const compare = (a: Row, b: Row) => compareRows(a, b, sort, ranks);
  return rows
    .map((row) =>
      row.kind === "app" && row.children.length > 1 ? { ...row, children: [...row.children].sort(compare) } : row,
    )
    .sort(compare);
}

// ---------------------------------------------------------------------------------------------
//  Filtering
// ---------------------------------------------------------------------------------------------

export type Filter = {
  /** The search field's text: a name, a user or the start of a PID. */
  query: string;
  /** The View menu's My Processes: only the person's own. */
  mine: boolean;
};

function matches(text: string, pid: number | null, user: string | null, query: string): boolean {
  if (text.toLowerCase().includes(query)) return true;
  if (user && user.toLowerCase().includes(query)) return true;
  return pid !== null && String(pid).startsWith(query);
}

/**
 * The rows the search and the View menu leave: an app's row shows when it matches (with all its
 * processes) or when any of its processes does (with those alone). `searched` names the app rows
 * kept for their processes alone, which the table opens so what matched shows.
 */
export function filterRows(rows: readonly TopRow[], filter: Filter): { rows: TopRow[]; searched: Set<string> } {
  const query = filter.query.trim().toLowerCase();
  const searched = new Set<string>();
  const kept: TopRow[] = [];
  for (const row of rows) {
    if (row.kind === "interface") {
      if (!query || row.name.toLowerCase().includes(query)) kept.push(row);
      continue;
    }
    if (row.kind === "process") {
      if (filter.mine && !row.own) continue;
      if (!query || matches(row.name, row.pid, row.user, query)) kept.push(row);
      continue;
    }
    const mine = filter.mine ? row.children.filter((child) => child.own) : row.children;
    if (filter.mine && row.running && mine.length === 0) continue;
    if (!query || matches(`${row.name} ${row.id}`, row.pid, row.user, query)) {
      kept.push(mine === row.children ? row : { ...row, children: mine });
      continue;
    }
    const found = mine.filter((child) => matches(child.name, child.pid, child.user, query));
    if (found.length === 0) continue;
    searched.add(row.key);
    kept.push({ ...row, children: found });
  }
  return { rows: kept, searched };
}

/** The rows the table lists, in order: each open app's processes right after it. */
export function flattenRows(rows: readonly TopRow[], open: (key: string) => boolean): Row[] {
  const out: Row[] = [];
  for (const row of rows) {
    out.push(row);
    if (row.kind === "app" && row.children.length > 0 && open(row.key)) out.push(...row.children);
  }
  return out;
}

/** Whether the person may ask to quit the row: their own process, or their own running app. */
export function canQuit(row: Row | null | undefined): boolean {
  if (!row || row.kind === "interface") return false;
  if (row.kind === "app") return row.running && row.own && row.children.length > 0;
  return row.own;
}

/** Whether the row has a process to inspect: a process, or a running app's own. */
export function canInspect(row: Row | null | undefined): boolean {
  if (!row || row.kind === "interface") return false;
  return row.kind === "process" || (row.pid !== null && row.started !== null);
}

/** What the session's answer to a quit says, for the moment under the table: null when it quit. */
export type QuitOutcome = null | "failed" | "notFound" | "notPermitted" | "needsInput";
export function quitOutcome(answer: SessionAnswer): QuitOutcome {
  if (answer && ("activityQuit" in answer || "activityQuitApp" in answer)) return null;
  const reason = (
    ((answer?.refused ?? answer?.failed) as { reason?: string } | undefined) ?? {}
  ).reason;
  if (reason === "notFound" || reason === "notPermitted" || reason === "needsInput") return reason;
  return "failed";
}

/** The inspection an answer carries, or why there is none. */
export function inspectionOf(answer: SessionAnswer): Inspection | "notFound" | "failed" {
  if (answer && typeof answer.activityInspect === "object" && answer.activityInspect) {
    return answer.activityInspect as Inspection;
  }
  const reason = ((answer?.failed as { reason?: string } | undefined) ?? {}).reason;
  return reason === "notFound" ? "notFound" : "failed";
}

// ---------------------------------------------------------------------------------------------
//  History
// ---------------------------------------------------------------------------------------------

/** The system at one sample, as the graphs draw it. */
export type HistoryPoint = {
  /** When it arrived, in milliseconds. */
  at: number;
  /** The seconds its rates cover; 0 begins a follow. */
  interval: number;
  cpuUser: number | null;
  cpuSystem: number | null;
  pressure: number | null;
  level: MemoryLevel;
  readRate: number | null;
  writeRate: number | null;
  receivedRate: number | null;
  sentRate: number | null;
  /** The Energy view's sum of every process's energy impact; null in the other views. */
  energy: number | null;
};

/** Ninety samples: three minutes at a sample every two seconds. */
export const HISTORY_LENGTH = 90;
/** The seconds the graphs span. */
export const HISTORY_SPAN = 180;
/** How many intervals between two samples break the graph's line: the window was hidden. */
export const GAP_INTERVALS = 2.5;
/** The interval the session samples at. */
export const SAMPLE_INTERVAL = 2;

/** The point a sample adds to the history. */
export function pointOf(sample: ActivitySample, at: number): HistoryPoint {
  const { cpu, memory, disk, network } = sample.system;
  let energy: number | null = null;
  if (sample.view === "energy") {
    for (const process of sample.processes) {
      if (process.energy != null) energy = (energy ?? 0) + process.energy;
    }
  }
  return {
    at,
    interval: sample.interval,
    cpuUser: cpu?.user ?? null,
    cpuSystem: cpu?.system ?? null,
    pressure: memory.pressure?.some ?? null,
    level: memory.level,
    readRate: disk.readRate,
    writeRate: disk.writeRate,
    receivedRate: network.receivedRate,
    sentRate: network.sentRate,
    energy,
  };
}

/** The last `HISTORY_LENGTH` points, kept in a ring so a sample allocates nothing but its point. */
export class History {
  private ring: (HistoryPoint | undefined)[] = new Array(HISTORY_LENGTH);
  private next = 0;
  private count = 0;
  /** Grows by one with each point: what a memoized graph depends on. */
  version = 0;

  push(point: HistoryPoint): void {
    this.ring[this.next] = point;
    this.next = (this.next + 1) % HISTORY_LENGTH;
    this.count = Math.min(HISTORY_LENGTH, this.count + 1);
    this.version += 1;
  }

  /** The points, oldest first. */
  points(): HistoryPoint[] {
    const out: HistoryPoint[] = [];
    const start = (this.next - this.count + HISTORY_LENGTH) % HISTORY_LENGTH;
    for (let i = 0; i < this.count; i += 1) out.push(this.ring[(start + i) % HISTORY_LENGTH] as HistoryPoint);
    return out;
  }
}

/** A run of a graph's line: x from 0 (three minutes ago) to 1 (the last sample), y its value. */
export type Segment = { x: number; y: number }[];

/**
 * A series' unbroken runs over the last `HISTORY_SPAN` seconds before the last point: the line
 * breaks where a value is missing, where a follow began again (interval 0) and where two samples
 * are further apart than `GAP_INTERVALS` intervals (the window was hidden, so nothing was read).
 */
export function segmentsOf(points: readonly HistoryPoint[], value: (point: HistoryPoint) => number | null): Segment[] {
  if (points.length === 0) return [];
  const end = points[points.length - 1].at;
  const start = end - HISTORY_SPAN * 1000;
  const segments: Segment[] = [];
  let run: Segment | null = null;
  let previous: HistoryPoint | null = null;
  for (const point of points) {
    const y = value(point);
    const interval = point.interval || previous?.interval || SAMPLE_INTERVAL;
    const broken =
      previous === null || point.interval === 0 || point.at - previous.at > GAP_INTERVALS * interval * 1000;
    previous = point;
    if (point.at < start || y === null || !Number.isFinite(y)) {
      run = null;
      continue;
    }
    if (broken || run === null) {
      run = [];
      segments.push(run);
    }
    run.push({ x: (point.at - start) / (HISTORY_SPAN * 1000), y });
  }
  return segments;
}

/**
 * One SVG path of filled areas: each run from `base(x)` up to its values, scaled so `max` reaches
 * the top of a `width` by `height` box; `base` stacks an area on another (CPU's user on system).
 * `downward` hangs the areas from the box's top instead, and `top` moves the box down: the lower
 * half of a mirrored graph (Disk's writes under its reads, Network's sent under its received).
 * A run of one sample is a sliver a pixel wide, so it shows.
 */
export function areaPath(
  segments: readonly Segment[],
  width: number,
  height: number,
  max: number,
  base?: readonly Segment[],
  { downward = false, top: origin = 0 }: { downward?: boolean; top?: number } = {},
): string {
  const scale = max > 0 ? height / max : 0;
  const y = (value: number) => {
    const offset = Math.max(0, Math.min(height, value * scale));
    return origin + (downward ? offset : height - offset);
  };
  const round = (n: number) => Math.round(n * 10) / 10;
  let path = "";
  segments.forEach((segment, index) => {
    if (segment.length === 0) return;
    const under = base?.[index];
    const top = segment.map((p) => ({ x: p.x * width, y: y(p.y) }));
    const bottom = segment.map((p, i) => ({ x: p.x * width, y: y(under?.[i]?.y ?? 0) }));
    if (top.length === 1) {
      top.push({ x: top[0].x + 1, y: top[0].y });
      bottom.push({ x: bottom[0].x + 1, y: bottom[0].y });
    }
    path += `M${round(top[0].x)} ${round(top[0].y)}`;
    for (let i = 1; i < top.length; i += 1) path += `L${round(top[i].x)} ${round(top[i].y)}`;
    for (let i = bottom.length - 1; i >= 0; i -= 1) path += `L${round(bottom[i].x)} ${round(bottom[i].y)}`;
    path += "Z";
  });
  return path;
}

/** Runs stacked: each value the sum of the two series at the same sample (CPU's user over system). */
export function stacked(over: readonly Segment[], under: readonly Segment[]): Segment[] {
  return over.map((segment, index) =>
    segment.map((point, i) => ({ x: point.x, y: point.y + (under[index]?.[i]?.y ?? 0) })),
  );
}

/** The largest value of the runs, at least `floor`: a rate graph's top. */
export function peakOf(segments: readonly Segment[], floor: number): number {
  let peak = floor;
  for (const segment of segments) for (const point of segment) if (point.y > peak) peak = point.y;
  return peak;
}

// ---------------------------------------------------------------------------------------------
//  Formatting, as a Mac writes it (decimal units, as ByteCountFormatter's)
// ---------------------------------------------------------------------------------------------

/** A number format whose answers are kept: a sample's values are mostly last sample's (a PID, a
    thread count, an idle process's 0.0), so each is formatted once rather than on every sample,
    which would be thousands of calls into the host's `Intl` every two seconds. */
type CachedFormat = { format(value: number): string };
/** The answers kept per format, at most: a few samples' worth of distinct values. */
const KEPT_ANSWERS = 8192;
const formats = new Map<string, CachedFormat>();
function numberFormat(locale: string, digits: number, grouping = true): CachedFormat {
  const id = `${locale}|${digits}|${grouping}`;
  let format = formats.get(id);
  if (!format) {
    const intl = new Intl.NumberFormat(locale, {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
      useGrouping: grouping,
    });
    const answers = new Map<number, string>();
    format = {
      format(value: number) {
        let text = answers.get(value);
        if (text === undefined) {
          if (answers.size >= KEPT_ANSWERS) answers.clear();
          text = intl.format(value);
          answers.set(value, text);
        }
        return text;
      },
    };
    formats.set(id, format);
  }
  return format;
}

const spanish = (locale: string) => locale.toLowerCase().startsWith("es");

/**
 * Bytes as macOS's Activity Monitor writes them, 1000-based: "Zero KB", "512 bytes", "64 KB",
 * "512.0 MB", "1.23 GB", "2.05 TB".
 */
export function formatBytes(bytes: number | null | undefined, locale: string): string {
  if (bytes == null || !Number.isFinite(bytes)) return "";
  const value = Math.max(0, bytes);
  if (value === 0) return spanish(locale) ? "Cero KB" : "Zero KB";
  if (value < 1000) return `${numberFormat(locale, 0).format(value)} bytes`;
  let index = 0;
  while (index < BYTE_UNITS.length - 1 && value >= BYTE_UNITS[index + 1][0]) index += 1;
  // A value that rounds up to the next unit is written in it ("1.00 GB", never "1,000.0 MB").
  const [size, , digits] = BYTE_UNITS[index];
  const rounded = Math.round((value / size) * 10 ** digits) / 10 ** digits;
  if (rounded >= 1000 && index < BYTE_UNITS.length - 1) index += 1;
  const [unitSize, unit, unitDigits] = BYTE_UNITS[index];
  return `${numberFormat(locale, unitDigits).format(value / unitSize)} ${unit}`;
}

/** The units bytes are written in, with the decimals macOS gives each. */
const BYTE_UNITS: readonly [number, string, number][] = [
  [1e3, "KB", 0],
  [1e6, "MB", 1],
  [1e9, "GB", 2],
  [1e12, "TB", 2],
];

/** A rate of bytes: "1.2 MB/s"; empty when the session has none yet. */
export function formatRate(bytes: number | null | undefined, locale: string): string {
  const text = formatBytes(bytes, locale);
  return text ? `${text}/s` : "";
}

/** A percent in a column, one decimal and no sign: "12.5", "12,5". */
export function formatPercent(value: number | null | undefined, locale: string): string {
  if (value == null || !Number.isFinite(value)) return "";
  return numberFormat(locale, 1).format(value);
}

/** A percent in the summary: "12.5%", "12,5 %" (Spanish spaces it). */
export function formatShare(value: number | null | undefined, locale: string): string {
  if (value == null || !Number.isFinite(value)) return "";
  return `${numberFormat(locale, 1).format(value)}${spanish(locale) ? " %" : "%"}`;
}

/** A count: "1,093", "1093", "10.093". */
export function formatCount(value: number | null | undefined, locale: string): string {
  if (value == null || !Number.isFinite(value)) return "";
  return numberFormat(locale, 0).format(Math.round(value));
}

/** A rate of things per second (wake ups, packets, operations), whole. */
export function formatPerSecond(value: number | null | undefined, locale: string): string {
  return formatCount(value, locale);
}

/** An energy impact, one decimal as macOS's. */
export function formatImpact(value: number | null | undefined, locale: string): string {
  return formatPercent(value, locale);
}

/**
 * CPU time as macOS's column writes it: seconds with hundredths ("3.45"), then minutes
 * ("2:03.45"), then hours ("1:02:03.45").
 */
export function formatCpuTime(seconds: number | null | undefined, locale: string): string {
  if (seconds == null || !Number.isFinite(seconds)) return "";
  const hundredths = Math.round(Math.max(0, seconds) * 100);
  const hours = Math.floor(hundredths / 360000);
  const minutes = Math.floor((hundredths % 360000) / 6000);
  const rest = (hundredths % 6000) / 100;
  const decimal = numberFormat(locale, 2, false).format(rest);
  const padded = rest < 10 && (hours > 0 || minutes > 0) ? `0${decimal}` : decimal;
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${padded}`;
  if (minutes > 0) return `${minutes}:${padded}`;
  return decimal;
}

/** The load averages: "0.52, 0.61, 0.70" ("0,52; 0,61; 0,70" where the comma is the decimal). */
export function formatLoad(load: readonly number[], locale: string): string {
  const format = numberFormat(locale, 2);
  return load.map((value) => format.format(value)).join(spanish(locale) ? "; " : ", ");
}

/** The words a row's cells show for `view`'s columns. */
export function cellTexts(row: Row, view: ActivityView, locale: string, none: string): string[] {
  return COLUMNS[view].map((key) => {
    const value = columnValue(row, key);
    switch (key) {
      case "name":
      case "user":
        return value === null ? "" : String(value);
      case "pid":
        return value === null ? "" : String(value);
      case "cpu":
      case "gpu":
        return formatPercent(value as number | null, locale);
      case "cpuTime":
        return formatCpuTime(value as number | null, locale);
      case "threads":
      case "packetsIn":
      case "packetsOut":
        return formatCount(value as number | null, locale);
      case "wakeups":
        return formatPerSecond(value as number | null, locale);
      case "energy":
        return formatImpact(value as number | null, locale);
      case "average":
        return row.kind === "app" ? (value === null ? none : formatImpact(value as number, locale)) : none;
      case "memory":
      case "swap":
      case "written":
      case "read":
      case "received":
      case "sent":
        return formatBytes(value as number | null, locale);
      case "writeRate":
      case "readRate":
      case "receivedRate":
      case "sentRate":
        return formatRate(value as number | null, locale);
      default:
        return "";
    }
  });
}

/** What the table draws of a row: its words and how it shows; equal views draw the same. */
export type RowView = {
  key: string;
  kind: Row["kind"];
  testID: string;
  cells: string[];
  icon: string | null;
  depth: 0 | 1;
  /** An app's row with processes: it has a disclosure triangle. */
  expandable: boolean;
  expanded: boolean;
  /** An app that no longer runs (the Energy view's): greyed. */
  resting: boolean;
};

export function rowView(row: Row, view: ActivityView, locale: string, none: string, expanded: boolean): RowView {
  return {
    key: row.key,
    kind: row.kind,
    testID:
      row.kind === "app"
        ? `activity-app-${row.id}`
        : row.kind === "process"
          ? `activity-row-${row.pid}`
          : `activity-interface-${row.name}`,
    cells: cellTexts(row, view, locale, none),
    icon: row.kind === "app" ? row.icon : null,
    depth: row.kind === "process" ? row.depth : 0,
    expandable: row.kind === "app" && row.children.length > 0,
    expanded: row.kind === "app" && row.children.length > 0 && expanded,
    resting: row.kind === "app" && !row.running,
  };
}

function sameView(a: RowView, b: RowView): boolean {
  if (
    a.testID !== b.testID ||
    a.icon !== b.icon ||
    a.depth !== b.depth ||
    a.expandable !== b.expandable ||
    a.expanded !== b.expanded ||
    a.resting !== b.resting ||
    a.cells.length !== b.cells.length
  ) {
    return false;
  }
  for (let i = 0; i < a.cells.length; i += 1) if (a.cells[i] !== b.cells[i]) return false;
  return true;
}

/**
 * The rows' views, each the previous one where nothing it draws changed: a memoized row is then
 * the same object as before and does not draw again, so a sample redraws only the rows it moved.
 */
export function keepUnchanged(previous: ReadonlyMap<string, RowView>, next: readonly RowView[]): RowView[] {
  return next.map((view) => {
    const before = previous.get(view.key);
    return before && sameView(before, view) ? before : view;
  });
}
