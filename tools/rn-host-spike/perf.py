#!/usr/bin/env python3
"""Performance of the native Bloom host and the compositor, measured (docs/performance.mdx).

Runs regression scenarios on the development compositor (regress.py, which still checks every
step's expected result) and measures, from what the processes report and from /proc:

- interactions: a window dragged by its title area, live-resized from its edges, zoomed by its
  green bubble, the overview opening with the windows drawn live in it (ADR 0022) and Settings'
  sidebar scrolled. For each, the frames the compositor composed while it ran (how long apart,
  how long composing each took) and the frames the hosts drew (their loop's work per frame,
  `busy_ms`, and the part of it running JavaScript, `js_ms`, from the host's report), and the
  latency from input to the frame answering it;
- cold start: an application's process started to its first frame on the output, and the stages
  of the host's start in between (the host's report's `stage` lines);
- idle: the CPU time and the wakeups (context switches, every thread) of every process while
  nothing happens, after the session settled;
- memory: every process's resident memory (anonymous and file-backed) while idle, and its peak.

The development compositor turns every 5 ms when nothing wakes it, so its own idle numbers say
nothing of a session's (oxyos-session waits on its devices and clients); the hosts' do.

Writes perf.json into --out and prints the tables docs/performance.mdx shows. With --check, exits 1
when a budget (BUDGETS) is exceeded.

Usage: perf.py --offscreen BIN --host BIN --bundle DIR [--out DIR] [--runs N] [--gpu-only] [--check]
               [interactions] [cold-start] [idle]
"""

import argparse
import json
import os
import pathlib
import statistics
import subprocess
import sys
import tempfile
import threading
import time

HERE = pathlib.Path(__file__).resolve().parent
SCENARIOS = HERE / "scenarios"

# A display's refresh at 60 Hz: what a frame's work must fit in, and a frame that comes later than
# one and a half of them after the one before is a refresh that showed nothing new.
REFRESH_MS = 1000 / 60

# Interactions: (name, scenario, the steps measured (1-based), what happens).
INTERACTIONS = [
    ("window drag", "perf-window-drag.json", [2],
     "WindowProbe's title area dragged 250 px at a mouse's 125 Hz: the compositor moves the window, the host draws nothing"),
    ("window drag at 1.25", "perf-window-drag-scaled.json", [2],
     "the same beside the shell on a 1600x1000 output at 1.25, the ThinkPad's scale"),
    ("live resize", "window-resize.json", [1, 2, 3],
     "the Inbox's edges dragged at 125 Hz: one configure in flight, the host laying out and drawing each size"),
    ("zoom", "windows.json", [3, 4],
     "the green bubble: the window's frame animated to the work area and back (a compositor transition)"),
    ("window views", "shell-thumbnails.json", [1],
     "the overview opening, Allo's window drawn live in its card (ADR 0022)"),
    ("Settings scroll", "perf-settings-scroll.json", [1, 2],
     "Settings' sidebar scrolled 240 px down and back at a touchpad's 60 Hz"),
]

# Cold starts: (name, scenario without steps). The shell's own start is measured in the last.
SETTINGS_APP = {"module": "OxyOSSettings", "window": [960, 600], "app": {"id": "so.oxy.settings", "title": "Settings"},
                "settings": {"app": "so.oxy.settings", "preferences": {"appearance": "light", "accent": "oxy"}}}
COLD_STARTS = [
    ("Settings", SETTINGS_APP),
    ("Inbox (sample)", {"module": "OxyBloom", "window": [1024, 640]}),
    ("WindowProbe", {"module": "WindowProbe", "window": [900, 600]}),
    ("Allo beside the shell", {"module": "EditingProbe", "window": [600, 360], "shell": "OxyOSShell",
                               "app": {"id": "so.oxy.allo", "title": "Allo"}}),
]

# Idle: (name, scenario) run with one step waiting IDLE_WAIT_MS, sampled once it settled.
IDLES = [
    ("shell and Allo", {"module": "EditingProbe", "window": [600, 360], "shell": "OxyOSShell",
                        "app": {"id": "so.oxy.allo", "title": "Allo"}}),
    ("Settings", SETTINGS_APP),
]
IDLE_WAIT_MS = 9000
IDLE_SETTLE_S = 2.0
IDLE_WINDOW_S = 5.0

# What --check holds the numbers to.
BUDGETS = {
    # Every frame's work fits a 60 Hz refresh, at the 95th percentile.
    "compose_ms_p95": REFRESH_MS,
    "host_busy_ms_p95": REFRESH_MS,
    # An application or the shell doing nothing wakes at most once a second and uses no CPU to
    # speak of.
    "idle_host_wakeups_per_s": 1.0,
    "idle_host_cpu_percent": 0.1,
    # An application's first frame within half a second of its start.
    "first_frame_ms_median": 500.0,
}


def percentiles(values):
    """p50, p95, p99 and max of `values` (nearest rank), rounded; None for none."""
    if not values:
        return None
    ordered = sorted(values)

    def rank(p):
        return ordered[min(len(ordered) - 1, max(0, round(p * len(ordered) + 0.5) - 1))]

    return {"n": len(ordered), "p50": round(rank(0.50), 2), "p95": round(rank(0.95), 2),
            "p99": round(rank(0.99), 2), "max": round(ordered[-1], 2)}


class Process:
    """A process of the run as /proc shows it: who it is and what it used so far."""

    def __init__(self, pid, name):
        self.pid, self.name = pid, name
        self.cpu_ns = self.wakeups = 0
        self.memory = {}
        self.peak_kib = 0

    def sample(self):
        """Reads its CPU time (schedstat, every thread), context switches (every thread) and
        memory; false once it is gone."""
        try:
            self.cpu_ns = int(pathlib.Path(f"/proc/{self.pid}/schedstat").read_text().split()[0])
            switches = 0
            for task in pathlib.Path(f"/proc/{self.pid}/task").iterdir():
                for line in (task / "status").read_text().splitlines():
                    if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                        switches += int(line.split()[1])
            self.wakeups = switches
            status = pathlib.Path(f"/proc/{self.pid}/status").read_text().splitlines()
        except (OSError, ValueError):
            return False
        for line in status:
            key, _, value = line.partition(":")
            if key in ("VmRSS", "VmHWM", "RssAnon", "RssFile", "RssShmem"):
                self.memory[key] = int(value.split()[0])
        self.peak_kib = max(self.peak_kib, self.memory.get("VmHWM", 0))
        return True


def process_name(pid):
    """The compositor, a Bloom host by its module (the shell's is OxyOSShell) or the program."""
    try:
        argv = pathlib.Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
    except OSError:
        return None
    program = os.path.basename(argv[0].decode(errors="replace"))
    if program == "oxyos-offscreen":
        return "compositor"
    if program == "oxyos-bloom-host" and len(argv) > 2:
        return argv[2].decode(errors="replace")
    return program


class Sampler(threading.Thread):
    """Every process under `root` (the regression run's), sampled every 50 ms until stopped: each
    one's latest CPU time, context switches and memory, and its peak resident memory."""

    def __init__(self, root):
        super().__init__(daemon=True)
        self.root, self.processes, self.lock = root, {}, threading.Lock()
        self.stopping = threading.Event()

    def descendants(self):
        parents = {}
        for entry in pathlib.Path("/proc").iterdir():
            if entry.name.isdigit():
                try:
                    parents[int(entry.name)] = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[1])
                except (OSError, ValueError, IndexError):
                    pass
        found, frontier = set(), {self.root}
        while frontier:
            frontier = {pid for pid, parent in parents.items() if parent in frontier} - found
            found |= frontier
        return found

    def sample(self):
        with self.lock:
            for pid in self.descendants():
                if pid not in self.processes:
                    name = process_name(pid)
                    if name is None or name.startswith("python"):
                        continue
                    self.processes[pid] = Process(pid, name)
            for process in self.processes.values():
                process.sample()

    def snapshot(self):
        """Each process's (name, CPU ns, context switches, memory) now."""
        self.sample()
        with self.lock:
            return {pid: (p.name, p.cpu_ns, p.wakeups, dict(p.memory)) for pid, p in self.processes.items()}

    def run(self):
        while not self.stopping.wait(0.05):
            self.sample()

    def peaks(self):
        """The peak resident memory of each process by name, in MiB (the largest of a name's)."""
        peaks = {}
        for process in self.processes.values():
            peaks[process.name] = max(peaks.get(process.name, 0), round(process.peak_kib / 1024, 1))
        return peaks


def read_report(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def run_scenario(options, scenario, out, during=None):
    """Runs a scenario (a path) through regress.py into `out` while sampling its processes, with
    `during(sampler, out)` called meanwhile. Returns (summary, app report, shell report, sampler)."""
    command = [sys.executable, str(HERE / "regress.py"), str(scenario), "--offscreen", options.offscreen,
               "--host", options.host, "--bundle", options.bundle, "--out", str(out)]
    if options.gpu_only:
        command += ["--gpu-only"]
    run = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    sampler = Sampler(run.pid)
    sampler.start()
    if during:
        during(sampler, out)
    output, _ = run.communicate()
    sampler.stopping.set()
    sampler.join()
    if run.returncode != 0:
        raise SystemExit(f"{scenario} failed:\n{output[-3000:]}")
    director = (out / "director.log").read_text()
    summary = json.loads(next(line for line in director.splitlines() if line.startswith('{"mode"')))
    report = read_report(out / "host-report.jsonl")
    shell = read_report(out / "host-report.jsonl.shell")
    return summary, report, shell, sampler


def frames_between(report, start, end):
    return [line for line in report if line["kind"] == "frame" and line.get("presented", True)
            and start <= line["t"] <= end]


def interaction(options, name, scenario, steps, what, out):
    """Every run's measured steps, pooled: the compositor's frames and the hosts'."""
    intervals, compose, reads, latency = [], [], [], []
    late = composed = 0
    hosts = {"window": {"busy": [], "js": [], "render": []}, "shell": {"busy": [], "js": [], "render": []}}
    peaks = {}
    for run in range(options.runs):
        summary, report, shell, sampler = run_scenario(options, SCENARIOS / scenario, out / f"{slug(name)}-{run + 1}")
        for key, value in sampler.peaks().items():
            peaks[key] = max(peaks.get(key, 0), value)
        for index in steps:
            step = summary["steps"][index - 1]
            frames = step.get("composed", [])
            # A paced step's input lasts its events over their rate: the frames composed then are
            # the interaction's (what follows is the answer to the last of them settling).
            paced = step.get("drag_paced") or step.get("scroll_paced")
            if paced:
                span = paced[-2] / paced[-1] * 1000 + REFRESH_MS
                frames = [frame for frame in frames if frame[0] <= span]
            composed += len(frames)
            compose += [frame[1] for frame in frames]
            reads += [frame[2] for frame in frames]
            gaps = [b[0] - a[0] for a, b in zip(frames, frames[1:])]
            intervals += gaps
            late += sum(1 for gap in gaps if gap > 1.5 * REFRESH_MS)
            for source, lines in (("window", [line for line in frames_between(report, step["t_inject"], step["t_settled"])
                                              if line["surface"] == 1]),
                                  ("shell", frames_between(shell, step["t_inject"], step["t_settled"]))):
                hosts[source]["busy"] += [line["busy_ms"] for line in lines if "busy_ms" in line]
                hosts[source]["js"] += [line["js_ms"] for line in lines if "js_ms" in line]
                hosts[source]["render"] += [line["render_ms"] for line in lines]
            # A paced step's latency is its own (from each event to the frame answering it; none for
            # a window moved, which composes in the turn the motion wakes); another's, from its
            # input to the first frame that changed.
            measured = (step.get("drag") or step.get("scrolled") or {}).get("latency_ms", {}).get("median")
            if measured is None and not paced:
                measured = step.get("latency_ms")
            if measured is not None:
                latency.append(measured)
    return {
        "what": what,
        "runs": options.runs,
        "frames_composed": composed,
        "frame_interval_ms": percentiles(intervals),
        "late_frames": late,
        "compose_ms": percentiles(compose),
        "read_ms": percentiles(reads),
        "window": {key: percentiles(values) for key, values in hosts["window"].items()},
        "shell": {key: percentiles(values) for key, values in hosts["shell"].items()},
        "latency_ms": percentiles(latency),
        "peak_rss_mib": peaks,
    }


def slug(name):
    return "".join(c if c.isalnum() else "-" for c in name.lower()).strip("-")


def scenario_file(out, name, scenario, steps):
    path = out / f"{slug(name)}.json"
    path.write_text(json.dumps({"about": f"perf.py: {name}", **scenario, "steps": steps}))
    return path


def cold_start(options, name, scenario, out):
    """The application's first frame after its process started, every run, and the stages of its
    host's start (from the director's launch and the host's report)."""
    path = scenario_file(out, name, scenario, [])
    firsts, stages, shell_firsts = [], {}, []
    for run in range(options.runs):
        summary, report, shell, _ = run_scenario(options, path, out / f"{slug(name)}-{run + 1}")
        firsts.append(summary["first_frame_ms"])
        launched = summary["t_launch"]
        first = next(line["t"] for line in report if line["kind"] == "frame")
        marks = [(line["name"], line["t"]) for line in report if line["kind"] == "stage"]
        # From the process's start to the host's main, then each stage to the next, the last to
        # the first frame.
        points = [("process start", launched)] + marks + [("first frame", first)]
        for (stage, at), (_, until) in zip(points, points[1:]):
            stages.setdefault(stage, []).append((until - at) / 1e6)
        # A host that reports no stages (one built before them) has its start measured whole.
        shell_main = next((line["t"] for line in shell if line["kind"] == "stage"), None)
        if shell_main is not None:
            shell_first = next(line["t"] for line in shell if line["kind"] == "frame")
            shell_firsts.append((shell_first - shell_main) / 1e6)
    result = {"first_frame_ms": percentiles(firsts),
              "stages_ms": {stage: round(statistics.median(values), 1) for stage, values in stages.items()}}
    if shell_firsts:
        result["shell_main_to_first_frame_ms"] = percentiles(shell_firsts)
    return result


def idle(options, name, scenario, out):
    """Every process's CPU and wakeups while nothing happens, and its memory then."""
    path = scenario_file(out, name, scenario, [{"name": "nothing happens", "do": ["wait", IDLE_WAIT_MS],
                                                "allow_unchanged": True, "expect": {}}])
    measured = {}

    def during(sampler, run_out):
        # Once the application drew its first frame, the session settles, then a window is sampled.
        report = run_out / "host-report.jsonl"
        deadline = time.monotonic() + 60
        while not (report.exists() and '"kind":"frame"' in report.read_text()):
            if time.monotonic() > deadline:
                return
            time.sleep(0.05)
        time.sleep(IDLE_SETTLE_S)
        before, started = sampler.snapshot(), time.monotonic()
        time.sleep(IDLE_WINDOW_S)
        after, elapsed = sampler.snapshot(), time.monotonic() - started
        for pid, (who, cpu, wakeups, memory) in after.items():
            if pid not in before:
                continue
            measured.setdefault(who, []).append({
                "cpu_percent": (cpu - before[pid][1]) / (elapsed * 1e9) * 100,
                "wakeups_per_s": (wakeups - before[pid][2]) / elapsed,
                "rss_mib": memory.get("VmRSS", 0) / 1024,
                "anon_mib": memory.get("RssAnon", 0) / 1024,
                "file_mib": memory.get("RssFile", 0) / 1024,
            })

    peaks = {}
    for run in range(options.runs):
        *_, sampler = run_scenario(options, path, out / f"{slug(name)}-{run + 1}", during)
        for key, value in sampler.peaks().items():
            peaks[key] = max(peaks.get(key, 0), value)
    return {who: {key: round(statistics.median(sample[key] for sample in samples), 2) for key in samples[0]}
            | {"peak_rss_mib": peaks.get(who)} for who, samples in measured.items()}


def table(rows, header):
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join(lines)


def show(results):
    """The tables docs/performance.mdx shows."""
    def spread(stats):
        return "—" if not stats else f"{stats['p50']} / {stats['p95']} / {stats['max']}"

    if "interactions" in results:
        rows = []
        for name, r in results["interactions"].items():
            rows.append([name, r["frames_composed"], spread(r["frame_interval_ms"]), r["late_frames"],
                         spread(r["compose_ms"]), spread(r["read_ms"]), spread(r["window"]["busy"]),
                         spread(r["window"]["js"]), spread(r["shell"]["busy"]), spread(r["latency_ms"])])
        print(table(rows, ["interaction", "frames", "interval ms p50/p95/max", "late", "compose ms",
                           "of it reading clients ms", "app frame work ms", "app JS ms", "shell frame work ms",
                           "latency ms"]))
        print()
    if "cold_start" in results:
        rows = [[name, spread(r["first_frame_ms"]), ", ".join(f"{k} {v}" for k, v in r["stages_ms"].items())]
                for name, r in results["cold_start"].items()]
        print(table(rows, ["application", "first frame ms p50/p95/max", "stages (median ms, each to the next)"]))
        print()
    if "idle" in results:
        rows = [[name, who, f"{m['cpu_percent']:.2f}", f"{m['wakeups_per_s']:.1f}", f"{m['rss_mib']:.0f}",
                 f"{m['anon_mib']:.0f}", f"{m['file_mib']:.0f}", m["peak_rss_mib"]]
                for name, processes in results["idle"].items() for who, m in processes.items()]
        print(table(rows, ["idle", "process", "CPU %", "wakeups/s", "RSS MiB", "anon", "file", "peak"]))


def check(results):
    """The budgets results exceed, as messages."""
    over = []
    for name, r in results.get("interactions", {}).items():
        if r["compose_ms"] and r["compose_ms"]["p95"] > BUDGETS["compose_ms_p95"]:
            over.append(f"{name}: composing took {r['compose_ms']['p95']} ms at p95")
        for source in ("window", "shell"):
            busy = r[source]["busy"]
            if busy and busy["p95"] > BUDGETS["host_busy_ms_p95"]:
                over.append(f"{name}: the {source}'s host worked {busy['p95']} ms per frame at p95")
    for name, processes in results.get("idle", {}).items():
        for who, m in processes.items():
            if who == "compositor":
                continue  # the development compositor's 5 ms turns are its director's pace
            if m["wakeups_per_s"] > BUDGETS["idle_host_wakeups_per_s"]:
                over.append(f"idle {name}: {who} woke {m['wakeups_per_s']:.1f} times a second")
            if m["cpu_percent"] > BUDGETS["idle_host_cpu_percent"]:
                over.append(f"idle {name}: {who} used {m['cpu_percent']:.2f}% CPU")
    for name, r in results.get("cold_start", {}).items():
        if r["first_frame_ms"]["p50"] > BUDGETS["first_frame_ms_median"]:
            over.append(f"cold start {name}: first frame after {r['first_frame_ms']['p50']} ms")
    return over


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("parts", nargs="*", help="interactions, cold-start, idle (all by default)")
    parser.add_argument("--offscreen", required=True)
    parser.add_argument("--host", required=True)
    parser.add_argument("--bundle", required=True)
    parser.add_argument("--out", default=None)
    parser.add_argument("--runs", type=int, default=3, help="runs of each scenario, pooled")
    parser.add_argument("--gpu-only", action="store_true", help="compose on the Vulkan device alone")
    parser.add_argument("--check", action="store_true", help="exit 1 when a budget is exceeded")
    options = parser.parse_args()
    parts = options.parts or ["interactions", "cold-start", "idle"]
    if not set(parts) <= {"interactions", "cold-start", "idle"}:
        parser.error(f"unknown parts {parts}")
    out = pathlib.Path(options.out or tempfile.mkdtemp(prefix="oxyos-perf-"))
    out.mkdir(parents=True, exist_ok=True)

    results = {"machine": machine(), "gpu_only": options.gpu_only, "runs": options.runs}
    if "interactions" in parts:
        results["interactions"] = {name: interaction(options, name, scenario, steps, what, out)
                                   for name, scenario, steps, what in INTERACTIONS}
    if "cold-start" in parts:
        results["cold_start"] = {name: cold_start(options, name, scenario, out) for name, scenario in COLD_STARTS}
    if "idle" in parts:
        results["idle"] = {name: idle(options, name, scenario, out) for name, scenario in IDLES}
    (out / "perf.json").write_text(json.dumps(results, indent=1))
    show(results)
    print(f"\n{out / 'perf.json'}")
    if options.check:
        over = check(results)
        for message in over:
            print(f"OVER BUDGET: {message}")
        return 1 if over else 0
    return 0


def machine():
    """The CPU, its cores and the kernel, as the numbers' context."""
    model = next((line.split(":", 1)[1].strip() for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines()
                  if line.startswith("model name")), "?")
    return {"cpu": model, "cores": os.cpu_count(), "kernel": os.uname().release}


if __name__ == "__main__":
    sys.exit(main())
