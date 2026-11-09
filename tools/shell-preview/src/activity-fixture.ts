/**
 * Activity Monitor's source in the browser preview (`?app=activity-monitor`): a played machine whose
 * samples come every two seconds while the app follows them, as the session's do, from a seeded
 * walk so every load plays the same. Quitting one of its own processes removes it; another user's
 * is refused, as the session refuses it.
 */
import type {
  ActivitySample,
  ActivitySource,
  ActivityView,
  SampleProcess,
  SessionAnswer,
} from "../../../apps/activity-monitor/src/model";

/** A seeded generator, so the preview plays the same machine each time. */
function seeded(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

type Played = { pid: number; name: string; user: string; app: string | null; load: number; memory: number; threads: number };

const APPS = [
  { id: "so.oxy.settings", name: "Settings", pid: 2100, icon: null },
  { id: "so.oxy.dropsea", name: "Dropsea", pid: 2200, icon: null },
  { id: "so.oxy.allo", name: "Allo", pid: 2300, icon: null },
];

function machine(): Played[] {
  const played: Played[] = [
    { pid: 1, name: "systemd", user: "root", app: null, load: 0.2, memory: 14e6, threads: 1 },
    { pid: 412, name: "systemd-journald", user: "root", app: null, load: 0.1, memory: 22e6, threads: 1 },
    { pid: 655, name: "NetworkManager", user: "root", app: null, load: 0.3, memory: 18e6, threads: 4 },
    { pid: 702, name: "bluetoothd", user: "root", app: null, load: 0.05, memory: 6e6, threads: 1 },
    { pid: 1300, name: "oxyos-session", user: "nate", app: null, load: 6, memory: 240e6, threads: 24 },
    { pid: 1320, name: "pipewire", user: "nate", app: null, load: 1.2, memory: 30e6, threads: 3 },
    { pid: 1321, name: "wireplumber", user: "nate", app: null, load: 0.3, memory: 26e6, threads: 5 },
    { pid: 1400, name: "OxyOSShell", user: "nate", app: null, load: 3, memory: 180e6, threads: 18 },
    { pid: 2100, name: "Settings", user: "nate", app: "so.oxy.settings", load: 2, memory: 150e6, threads: 21 },
    { pid: 2101, name: "Settings Web Content", user: "nate", app: "so.oxy.settings", load: 0.8, memory: 60e6, threads: 9 },
    { pid: 2200, name: "Dropsea", user: "nate", app: "so.oxy.dropsea", load: 4, memory: 310e6, threads: 30 },
    { pid: 2201, name: "dropsea-sync", user: "nate", app: "so.oxy.dropsea", load: 1.5, memory: 45e6, threads: 6 },
    { pid: 2300, name: "Allo", user: "nate", app: "so.oxy.allo", load: 1, memory: 120e6, threads: 16 },
  ];
  for (let i = 0; i < 40; i += 1) {
    played.push({ pid: 3000 + i * 7, name: `kworker/${i}:1`, user: "root", app: null, load: 0.02, memory: 0, threads: 1 });
  }
  return played;
}

export function previewActivity(): ActivitySource {
  const random = seeded(20261108);
  const played = machine();
  let view: ActivityView = "cpu";
  let listener: ((sample: ActivitySample) => void) | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;
  let first = true;
  const totals = new Map<number, { cpuTime: number; read: number; written: number }>();
  let disk = { read: 40e9, written: 22e9, reads: 900000, writes: 400000 };
  let net = { received: 6e9, sent: 1.4e9, packetsIn: 5e6, packetsOut: 2e6 };

  const tell = () => {
    const interval = first ? 0 : 2;
    const processes: SampleProcess[] = played.map((p) => {
      const cpu = Math.max(0, p.load * (0.5 + random()));
      const total = totals.get(p.pid) ?? { cpuTime: p.load * 300, read: p.memory * 3, written: p.memory };
      total.cpuTime += (cpu / 100) * interval;
      const readRate = p.user === "nate" ? p.load * 20000 * random() : null;
      const writeRate = p.user === "nate" ? p.load * 8000 * random() : null;
      total.read += (readRate ?? 0) * interval;
      total.written += (writeRate ?? 0) * interval;
      totals.set(p.pid, total);
      return {
        pid: p.pid,
        ppid: p.app ? 1300 : 1,
        started: p.pid * 13,
        name: p.name,
        user: p.user,
        own: p.user === "nate",
        app: p.app,
        cpu: first ? null : cpu,
        cpuTime: total.cpuTime,
        threads: p.threads,
        ...(view === "cpu" ? { wakeups: first ? null : cpu * 3, gpu: p.app || p.name === "oxyos-session" ? cpu / 4 : null, gpuTime: null } : {}),
        ...(view === "memory" ? { memory: p.memory * (0.98 + random() * 0.04), swap: 0, resident: p.memory, gpuMemory: null } : {}),
        ...(view === "energy" ? { energy: first ? null : cpu } : {}),
        ...(view === "disk"
          ? {
              read: p.user === "nate" ? total.read : null,
              written: p.user === "nate" ? total.written : null,
              readRate: first ? null : readRate,
              writeRate: first ? null : writeRate,
            }
          : {}),
      };
    });
    const cpuUser = 8 + random() * 14;
    const cpuSystem = 3 + random() * 6;
    const readRate = 200e3 + random() * 2e6;
    const writeRate = 100e3 + random() * 900e3;
    const receivedRate = 10e3 + random() * 400e3;
    const sentRate = 4e3 + random() * 60e3;
    disk = {
      read: disk.read + readRate * interval,
      written: disk.written + writeRate * interval,
      reads: disk.reads + Math.round(readRate / 4096) * interval,
      writes: disk.writes + Math.round(writeRate / 4096) * interval,
    };
    net = {
      received: net.received + receivedRate * interval,
      sent: net.sent + sentRate * interval,
      packetsIn: net.packetsIn + Math.round(receivedRate / 1200) * interval,
      packetsOut: net.packetsOut + Math.round(sentRate / 800) * interval,
    };
    const rates = (value: number) => (first ? null : value);
    const sample: ActivitySample = {
      view,
      interval,
      system: {
        cpu: first ? null : { user: cpuUser, system: cpuSystem, idle: 100 - cpuUser - cpuSystem, cores: 8 },
        load: [0.52 + random() * 0.2, 0.61, 0.7],
        processes: processes.length,
        threads: processes.reduce((sum, p) => sum + p.threads, 0),
        memory: {
          total: 16e9,
          used: 9.2e9,
          app: 6.1e9,
          wired: 1.4e9,
          compressed: 0.3e9,
          cached: 3.2e9,
          swapUsed: 0,
          swapTotal: 8e9,
          pressure: first ? null : { some: random() * 3, full: 0 },
          level: "normal",
        },
        disk: {
          ...disk,
          readRate: rates(readRate),
          writeRate: rates(writeRate),
          readsRate: rates(readRate / 4096),
          writesRate: rates(writeRate / 4096),
        },
        network: {
          ...net,
          receivedRate: rates(receivedRate),
          sentRate: rates(sentRate),
          packetsInRate: rates(receivedRate / 1200),
          packetsOutRate: rates(sentRate / 800),
          interfaces: [
            {
              name: "wlp0s20f3",
              ...net,
              receivedRate: rates(receivedRate),
              sentRate: rates(sentRate),
              packetsInRate: rates(receivedRate / 1200),
              packetsOutRate: rates(sentRate / 800),
            },
            { name: "enp0s31f6", received: 0, sent: 0, packetsIn: 0, packetsOut: 0, receivedRate: rates(0), sentRate: rates(0), packetsInRate: rates(0), packetsOutRate: rates(0) },
          ],
        },
      },
      apps: APPS.filter((app) => played.some((p) => p.app === app.id)),
      processes,
      ...(view === "energy"
        ? {
            energy: [
              ...APPS.filter((app) => played.some((p) => p.app === app.id)).map((app) => ({
                id: app.id,
                name: app.name,
                impact: first ? null : processes.filter((p) => p.app === app.id).reduce((sum, p) => sum + (p.cpu ?? 0), 0),
                average: 3.2,
                running: true,
              })),
              { id: "so.oxy.inbox", name: "Inbox", impact: null, average: 1.4, running: false },
            ],
          }
        : {}),
    };
    first = false;
    listener?.(sample);
  };

  return {
    follow(next, heard) {
      view = next;
      listener = heard;
      first = true;
      tell();
      timer = setInterval(tell, 2000);
      return () => {
        if (timer) clearInterval(timer);
        timer = null;
        listener = null;
      };
    },
    setView(next) {
      if (next === view) return;
      view = next;
      if (listener) tell();
    },
    async quit(pid, _started, force): Promise<SessionAnswer> {
      const index = played.findIndex((p) => p.pid === pid);
      if (index < 0) return { failed: { reason: "notFound" } };
      if (played[index].user !== "nate") return { failed: { reason: "notPermitted" } };
      console.info("[activity preview] quit", pid, force ? "(force)" : "");
      played.splice(index, 1);
      return { activityQuit: {} };
    },
    async quitApp(id, force): Promise<SessionAnswer> {
      if (!played.some((p) => p.app === id)) return { failed: { reason: "notFound" } };
      console.info("[activity preview] quit app", id, force ? "(force)" : "");
      for (let i = played.length - 1; i >= 0; i -= 1) if (played[i].app === id) played.splice(i, 1);
      return { activityQuitApp: {} };
    },
    async inspect(pid, started): Promise<SessionAnswer> {
      const p = played.find((candidate) => candidate.pid === pid);
      if (!p) return { failed: { reason: "notFound" } };
      const own = p.user === "nate";
      return {
        activityInspect: {
          pid,
          started,
          name: p.name,
          path: own ? `/usr/lib/oxyos/${p.name.toLowerCase().replace(/\s+/g, "-")}` : null,
          arguments: own ? [p.name, "--session"] : null,
          ppid: p.app ? 1300 : 1,
          parentName: p.app ? "oxyos-session" : "systemd",
          user: p.user,
          own,
          app: p.app,
          startedAt: Math.floor(Date.now() / 1000) - 3600,
          cpuTime: totals.get(pid)?.cpuTime ?? 0,
          threads: p.threads,
          memory: own ? { resident: p.memory, footprint: p.memory * 0.8, unique: p.memory * 0.7, proportional: p.memory * 0.75, swap: 0, gpu: p.memory * 0.1 } : null,
          files: own ? [{ fd: 3, path: "/home/nate/.config/oxyos/settings.json" }, { fd: 4, path: "/dev/dri/renderD128" }] : null,
          ports: own ? [{ fd: 7, kind: "tcp", local: "127.0.0.1:43122", remote: "34.120.10.2:443", state: "established" }] : null,
          cgroup: own ? `/user.slice/user-1000.slice/session.slice/${p.name}.scope` : "/system.slice",
          restricted: !own,
        },
      };
    },
  };
}
