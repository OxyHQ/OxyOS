#!/usr/bin/env python3
"""Performance of the native Bloom host and the compositor, measured (docs/performance.mdx).

Runs regression scenarios on the development compositor (regress.py, which still checks every
step's expected result) and measures, from what the processes report and from /proc:

- interactions: a window dragged by its title area, live-resized from its edges, zoomed by its
  green bubble, the overview opening with the windows drawn live in it (ADR 0022) and Settings'
  sidebar scrolled. For each, the frames the compositor composed while it ran (how long apart,
  how long composing each took) and the frames the hosts drew (their loop's work per frame,
  `busy_ms`, and the part of it running JavaScript, `js_ms`, from the host's report, both by the
  wall clock, and that JavaScript's time on the host's CPU, `js_cpu_ms`), and the latency from input
  to the frame answering it;
- the shell: what the session's shell pays for an event (shell-events.json): another window zoomed
  and restored, which changes nothing the shell shows, and the pointer over the dock; for each, the
  most JavaScript any one piece of the shell's work ran on the shell's CPU, against the scenario's
  budget, and the same piece's JavaScript by the wall clock and its time blocked;
- responses: Dropsea used as a person uses it (perf-dropsea.json at 1, perf-dropsea-scaled.json at 1.25,
  perf-dropsea-home.json on a home of 360,000 files, zoomed at 1.25):
  for each input, the latency from it to the first frame that changed, against the scenario's
  budget, and the work the application's host did for it (`busy_ms` summed over the frames until
  the step settled, and its JavaScript);
- cold start: an application's process started to its first frame on the output, and the stages
  of the host's start in between (the host's report's `stage` lines);
- idle: the CPU time and the wakeups (context switches, every thread) of every process while
  nothing happens, after the session settled;
- memory: every process's resident memory (anonymous and file-backed) while idle, and its peak;
- the display's rate: windows dragged and zoomed, and animations, on a 120 Hz output and a 60 Hz one
  (refresh-*.json): how many frames each step's motion composed and how far apart they came, against
  its claims, which the gate checks only as functional (regress.py, --speed-budgets);
- the genie: a window minimized into the dock and restored out of it through the Dock's mesh
  (genie*.json, docs/adr/0043-the-genie.mdx) on a 120 Hz output at 1 and a 60 Hz one at 1.25: the
  frames the compositor composed while it ran, how far apart and how long composing each took, and
  how many each transition drew against its claim (one a refresh, which the gate holds only as at
  most);
- the session idle (`session`, not among the parts run by default): a running `oxyos-session` of this
  user and the Bloom hosts it started (the shell, the applications), read-only from /proc over
  SESSION_WINDOW_S while nothing happens on the machine: each process's CPU and wakeups, and each
  thread's, held to the idle budget (at most one wakeup a second and 0.1 % of a core, the session's
  own process as an application). It needs no --offscreen, --runtimes or --packages, and touches
  nothing of the session;
- applications side by side: the shell, Settings and three more applications on one compositor, and
  each host's memory from /proc/PID/smaps_rollup once all have drawn: RSS, PSS (a shared page's
  size divided among the processes mapping it) and USS (its private pages), and what of each
  bytecode file (the runtime's framework, an application's index.hbc) is resident and shared
  (docs/adr/0030-installing-applications.mdx, decision 17).

The development compositor turns every 5 ms when nothing wakes it, so its own idle numbers say
nothing of a session's (oxyos-session waits on its devices and clients); the hosts' do.

Writes perf.json into --out and prints the tables docs/performance.mdx shows. Each regression run
checks what its steps do and enforces no time (regress.py --collect): the scenarios' own budgets on
time (a step's `latency_ms`, `js_work`'s, `resize_work`'s) are measured over every run and judged
here, at the median of the runs, or at the percentile a budget states (`js_ms_p95_max`: the 95th
of every run's frames pooled), with the runs over it named. With --check, exits 1 when one of
them, or a budget of BUDGETS, is exceeded. A frame's budget is a refresh of the display the interaction
runs on (regress.py's `display_hz`: the scenario's primary output's, 60 Hz without one), so a
scenario on a 120 Hz output holds every frame to 8.3 ms.

Usage: perf.py --offscreen BIN --runtimes DIR --packages DIR [--out DIR] [--runs N] [--gpu-only]
               [--renderer cpu|gpu] [--check]
               [interactions] [responses] [shell] [cold-start] [idle] [applications] [refresh] [genie]
       perf.py [--out DIR] [--check] session

`responses` needs packages built with DROPSEA_DIR (docs/react-native-android-adaptation.mdx).
"""

import argparse
import json
import os
import re
import pathlib
import statistics
import subprocess
import sys
import tempfile
import threading
import time

HERE = pathlib.Path(__file__).resolve().parent
SCENARIOS = HERE / "scenarios"
sys.path.insert(0, str(HERE))
from regress import animated_values, display_hz  # noqa: E402  (beside this file)


def refresh_ms(scenario):
    """A refresh of the display `scenario` (a file) runs on, in ms: what a frame's work must fit
    in; a frame that comes later than one and a half of them after the one before is a refresh that
    showed nothing new."""
    return 1000 / display_hz(json.loads(pathlib.Path(scenario).read_text()))

# Interactions: (name, scenario, the steps measured (1-based), what happens).
INTERACTIONS = [
    ("window drag", "perf-window-drag.json", [2],
     "WindowProbe's title area dragged 250 px at a mouse's 125 Hz: the compositor moves the window, the host draws nothing"),
    ("window drag at 1.25", "perf-window-drag-scaled.json", [2],
     "the same beside the shell on a 1600x1000 output at 1.25, the ThinkPad's scale"),
    ("live resize", "window-resize.json", [1, 2, 3],
     "the Inbox's edges dragged at 125 Hz: one configure in flight, the host laying out and drawing each size"),
    ("Dropsea live resize", "perf-dropsea-resize.json", [3, 4],
     "Dropsea's right edge dragged 300 px in and back at 125 Hz: Bloom's shell, its panels and Dropsea's items are flex"),
    ("zoom", "windows.json", [3, 4],
     "the green bubble: the window's frame animated to the work area and back (a compositor transition)"),
    ("window views", "shell-thumbnails.json", [1],
     "the overview opening, Allo's window drawn live in its card (ADR 0022)"),
    ("Settings scroll", "perf-settings-scroll.json", [1, 2],
     "Settings' sidebar scrolled 240 px down and back at a touchpad's 60 Hz"),
]

# The display's rate (docs/adr/0018-several-displays.mdx): windows and animations at 120 and at 60 Hz.
# What the gate's regress.py holds only as functional (a frame in more than one, never faster than a
# refresh, each frame answered once and in order) is measured here over every run and judged at the
# median of the runs: how many frames fit, and how far apart they came.
REFRESH = [
    ("windows at 120 Hz", "refresh-120-windows.json"),
    ("windows at 60 Hz", "refresh-60-windows.json"),
    ("animations at 120 Hz", "refresh-120-animations.json"),
]

# The genie (docs/adr/0043-the-genie.mdx): (name, scenario, the steps measured (1-based), what
# happens): the steps whose window goes into the dock or comes out of it on the compositor's own
# clock, never held.
GENIE = [
    ("genie at 120 Hz", "genie.json", [9, 10],
     "GenieProbe's 800x500 window restored out of Allo's icon and minimized into it, 500 ms each, beside the shell"),
    ("genie at 1.25", "genie-scaled.json", [9, 10],
     "the same on a 1600x1000 output at 1.25 and 60 Hz, the ThinkPad's"),
]

# Responses: (name, scenario). Each step with an input is measured; its budget is the scenario's own
# `latency_ms`, which --check holds the median of the runs to (step_budgets).
RESPONSES = [
    ("Dropsea", "perf-dropsea.json"),
    ("Dropsea at 1.25", "perf-dropsea-scaled.json"),
    ("Dropsea on a big home, zoomed at 1.25", "perf-dropsea-home.json"),
]

# The shell's responses: (name, scenario). Each step that holds the shell to a `js_work` budget
# (`in_shell`) is measured: the most JavaScript any one piece of the shell's work ran in it (a
# frame's, or work that drew none: an event the shell is told), against that budget, which --check
# holds the median to, and the frames the shell presented.
SHELL_RESPONSES = [
    ("the shell", "shell-events.json"),
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

# Applications side by side: the shell and Settings run by the compositor, and these joining on its
# socket as further applications, each in a window of its own.
SIDE_BY_SIDE = {**SETTINGS_APP, "shell": "OxyOSShell"}
SIDE_BY_SIDE_WAIT_MS = 40000
JOINING = [("Inbox", "OxyBloom"), ("Allo", "EditingProbe"), ("WindowProbe", "WindowProbe")]
IDLE_SETTLE_S = 2.0
IDLE_WINDOW_S = 5.0

# The session idle: how long a running session is sampled while nothing happens.
SESSION_WINDOW_S = 30.0

# What --check holds the numbers to.
BUDGETS = {
    # Every frame's work fits a refresh of the display it runs on (1.0: one; `refresh_ms`), at the
    # 95th percentile.
    "compose_refreshes_p95": 1.0,
    "host_busy_refreshes_p95": 1.0,
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
               "--runtimes", options.runtimes, "--packages", options.packages, "--out", str(out),
               # A run measuring speed never counts what React re-renders (regress.py): the count's
               # allocations move where Hermes' collections fall, into the times measured.
               "--no-render-counts",
               # Each run checks what its steps do and enforces no time: the steps' budgets on time
               # are measured here over every run and judged at the median (step_budgets).
               "--collect"]
    if options.gpu_only:
        command += ["--gpu-only"]
    if options.renderer:
        command += ["--renderer", options.renderer]
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
    if isinstance(scenario, pathlib.Path) and scenario.parent == SCENARIOS:
        STEP_RUNS.setdefault(scenario.name, []).append(
            measure_steps(json.loads(scenario.read_text()), summary, report, shell))
    return summary, report, shell, sampler


# The steps' budgets on time, as each run measured them: {scenario's file name: [one per run:
# {(step index, host, budget): a value, or for a budget at the 95th percentile the values it is of}]}.
STEP_RUNS = {}
# Budgets at a percentile of the values of every run pooled; the rest are judged at the median of
# each run's value.
POOLED = {"js_ms_p95_max": 95, "busy_refreshes_p95_max": 95}
ZOOM_LINE = re.compile(r"zoom \(.*\): its configure .* its frame presented ([\d.]+) ms after")


def measure_steps(scenario, summary, report, shell):
    """What one run measured of each step's budgets on time, as regress.py would check them (the same
    windows: from the step before's end to the step's own): `js_work`'s (the most JavaScript one
    piece of work ran on the CPU, blocked and by the clock), `resize_work`'s (the frames' JavaScript
    on the CPU and their work in refreshes, the zoom's frame), and the step's `latency_ms`."""
    measured = {}
    hz = display_hz(scenario)
    start = summary["t_launch"]
    for index, (step, result) in enumerate(zip(scenario["steps"], summary["steps"])):
        end = result["t_settled"]
        expect = step.get("expect", {})
        for host, wanted, lines in (("app", expect, report), ("shell", expect.get("in_shell", {}), shell)):
            work = [line for line in lines if line["kind"] == "timing" and start <= line["t"] <= end]
            for budget, measure in (("js_ms_max", "cpu"), ("js_blocked_ms_max", "blocked"), ("js_wall_ms_max", "parts")):
                if budget in wanted.get("js_work", {}):
                    measured[(index, host, budget)] = max(
                        (line.get(measure, line["parts"]).get("javascript", 0) for line in work), default=0.0)
            resize = wanted.get("resize_work", {})
            if resize:
                begun = result.get("t_inject") or start
                frames = [line for line in lines if line["kind"] == "frame" and line.get("surface") == 1
                          and begun <= line["t"] <= end and line.get("presented", True)]
                if "js_ms_p95_max" in resize:
                    measured[(index, host, "js_ms_p95_max")] = [line.get("js_cpu_ms", 0) for line in frames]
                if "busy_refreshes_p95_max" in resize:
                    measured[(index, host, "busy_refreshes_p95_max")] = [line.get("busy_ms", 0) * hz / 1000
                                                                         for line in frames]
                if "zoom_presented_ms_max" in resize:
                    zooms = [float(m.group(1)) for m in (ZOOM_LINE.search(line["text"]) for line in lines
                                                          if line["kind"] == "log" and begun <= line["t"] <= end) if m]
                    if zooms:
                        measured[(index, host, "zoom_presented_ms_max")] = max(zooms)
        if "latency_ms" in expect and result.get("latency_ms") is not None:
            measured[(index, "app", "latency_ms")] = result["latency_ms"]
        start = end
    return measured


def nearest(values, p):
    """The `p`th percentile of `values` as regress.py takes it."""
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(p / 100 * (len(ordered) - 1)))] if ordered else 0.0


def step_budgets():
    """Each step's budget on time over every run of its scenario: the median of the runs' values,
    or the percentile it states of all the runs' values pooled, and the runs that were over it."""
    entries = []
    for name, runs in STEP_RUNS.items():
        steps = json.loads((SCENARIOS / name).read_text())["steps"]
        for key in sorted({key for run in runs for key in run}):
            index, host, budget = key
            expect = steps[index].get("expect", {})
            wanted = expect if host == "app" else expect.get("in_shell", {})
            limit = (wanted.get(budget) if budget == "latency_ms"
                     else {**wanted.get("js_work", {}), **wanted.get("resize_work", {})}.get(budget))
            values = [run[key] for run in runs if key in run]
            if budget in POOLED:
                measured = nearest([v for run in values for v in run], POOLED[budget])
                each = [nearest(run, POOLED[budget]) for run in values]
                how = f"p{POOLED[budget]} of every run"
            else:
                measured = statistics.median(values)
                each = values
                how = "median of the runs"
            entries.append({"scenario": name, "step": index + 1, "name": steps[index].get("name", ""), "host": host,
                            "budget": budget, "limit": limit, "measured": round(measured, 2), "how": how,
                            "runs": [round(v, 2) for v in each],
                            "slow_runs": [run + 1 for run, v in enumerate(each) if v > limit]})
    return entries


def rate_claims(scenario, summary, report):
    """One run's measure of each step's claims on the display's rate (regress.py checks them only
    under --speed-budgets): {(step index, claim): (value, [least, most])}, `most` None for none."""
    claims = {}
    start = summary["t_launch"]
    for index, (step, result) in enumerate(zip(scenario["steps"], summary["steps"])):
        end = result["t_settled"]
        expect = step.get("expect", {})
        drag, wanted = result.get("drag") or {}, expect.get("drag", {})
        if "composed_min" in wanted:
            claims[(index, "frames composed in the drag")] = (drag.get("composed", 0), [wanted["composed_min"], None])
        if "composed_gap_ms" in wanted:
            claims[(index, "ms between them, median")] = ((drag.get("composed_gap_ms") or {}).get("median"),
                                                          wanted["composed_gap_ms"])
        # Each transition's frames against its claim (`refresh_frames`, the genie's): one a refresh.
        seen = list((result.get("transitions") or {}).get("seen", []))
        for wanted in expect.get("transitions", {}).get("seen", []):
            if "refresh_frames" in wanted:
                match = next((t for t in seen if t["kind"] == wanted.get("kind", t["kind"])
                              and t["change"] == wanted.get("change", t["change"])), None)
                if match:
                    seen.remove(match)
                claims[(index, f"frames its {wanted.get('change', 'transition')} drew")] = (
                    None if match is None else match["frames"], wanted["refresh_frames"])
        if "frames_min" in expect.get("transitions", {}):
            claims[(index, "frames composed in its transitions")] = (
                (result.get("transitions") or {}).get("frames", 0), [expect["transitions"]["frames_min"], None])
        if "window_frames" in expect:
            count = sum(1 for line in report if line["kind"] == "frame" and line.get("surface") == 1
                        and line.get("presented", True) and start <= line["t"] <= end)
            claims[(index, "frames the window presented")] = (count, expect["window_frames"])
        wanted = expect.get("animation_frames", {})
        stamps = [line["timestamp"] for line in report if line["kind"] == "animation_frame" and start <= line["t"] <= end]
        if "count" in wanted:
            claims[(index, "requestAnimationFrame's frames")] = (len(stamps), wanted["count"])
        if "median_interval_ms" in wanted:
            gaps = [b - a for a, b in zip(stamps, stamps[1:])]
            claims[(index, "ms between them, median")] = (statistics.median(gaps) if gaps else None,
                                                          wanted["median_interval_ms"])
        for kind in ("animated", "committed"):
            if kind in expect:
                bounds = expect.get(f"{kind}_frames", 8)
                got = animated_values(report, start, end, kind)
                counts = [len(got.get(view, {}).get(prop, [])) for view, props in expect[kind].items() for prop in props]
                claims[(index, f"frames {kind}, fewest of its values")] = (
                    min(counts, default=0), bounds if isinstance(bounds, list) else [bounds, None])
        start = end
    return claims


def refresh(options, name, scenario, out):
    """Every run's rate claims, each judged at the median of the runs."""
    runs = []
    for run in range(options.runs):
        summary, report, _, _ = run_scenario(options, SCENARIOS / scenario, out / f"{slug(name)}-{run + 1}")
        runs.append(rate_claims(json.loads((SCENARIOS / scenario).read_text()), summary, report))
    return judged(scenario, runs)


def genie(options, name, scenario, steps, what, out):
    """The genie's steps as an interaction (the compositor's frames), and its transitions' claims
    judged at the median of the same runs."""
    runs = []
    measured = interaction(options, name, scenario, steps, what, out, claims=runs)
    return {**measured, "claims": judged(scenario, runs)}


def judged(scenario, runs):
    """Each rate claim of `runs` (one `rate_claims` each), judged at the median of the runs."""
    steps = json.loads((SCENARIOS / scenario).read_text())["steps"]
    entries = []
    for key in sorted({key for run in runs for key in run}):
        index, claim = key
        values = [run[key][0] for run in runs if key in run and run[key][0] is not None]
        least, most = next(run[key][1] for run in runs if key in run)
        measured = statistics.median(values) if values else None
        held = measured is not None and measured >= least and (most is None or measured <= most)
        entries.append({"scenario": scenario, "step": index + 1, "name": steps[index].get("name", ""), "claim": claim,
                        "least": least, "most": most, "measured": None if measured is None else round(measured, 2),
                        "runs": [round(v, 2) for v in values], "held": held})
    return entries


def frames_between(report, start, end):
    return [line for line in report if line["kind"] == "frame" and line.get("presented", True)
            and start <= line["t"] <= end]


def interaction(options, name, scenario, steps, what, out, claims=None):
    """Every run's measured steps, pooled: the compositor's frames and the hosts'; with `claims` a
    list, each run's rate claims (`rate_claims`) appended to it."""
    intervals, compose, reads, latency = [], [], [], []
    late = composed = 0
    refresh = refresh_ms(SCENARIOS / scenario)
    hosts = {source: {"busy": [], "js": [], "js_cpu": [], "render": []} for source in ("window", "shell")}
    peaks = {}
    for run in range(options.runs):
        summary, report, shell, sampler = run_scenario(options, SCENARIOS / scenario, out / f"{slug(name)}-{run + 1}")
        if claims is not None:
            claims.append(rate_claims(json.loads((SCENARIOS / scenario).read_text()), summary, report))
        for key, value in sampler.peaks().items():
            peaks[key] = max(peaks.get(key, 0), value)
        for index in steps:
            step = summary["steps"][index - 1]
            frames = step.get("composed", [])
            # A paced step's input lasts its events over their rate: the frames composed then are
            # the interaction's (what follows is the answer to the last of them settling).
            paced = step.get("drag_paced") or step.get("scroll_paced") or step.get("move_paced")
            if paced:
                span = paced[-2] / paced[-1] * 1000 + refresh
                frames = [frame for frame in frames if frame[0] <= span]
            composed += len(frames)
            compose += [frame[1] for frame in frames]
            reads += [frame[2] for frame in frames]
            gaps = [b[0] - a[0] for a, b in zip(frames, frames[1:])]
            intervals += gaps
            late += sum(1 for gap in gaps if gap > 1.5 * refresh)
            for source, lines in (("window", [line for line in frames_between(report, step["t_inject"], step["t_settled"])
                                              if line["surface"] == 1]),
                                  ("shell", frames_between(shell, step["t_inject"], step["t_settled"]))):
                hosts[source]["busy"] += [line["busy_ms"] for line in lines if "busy_ms" in line]
                hosts[source]["js"] += [line["js_ms"] for line in lines if "js_ms" in line]
                hosts[source]["js_cpu"] += [line["js_cpu_ms"] for line in lines if "js_cpu_ms" in line]
                hosts[source]["render"] += [line["render_ms"] for line in lines]
            # A paced step's latency is its own (from each event to the frame answering it; none for
            # a window moved, which composes in the turn the motion wakes); another's, from its
            # input to the first frame that changed.
            measured = (step.get("drag") or step.get("scrolled") or step.get("moved") or {}).get("latency_ms", {}).get("median")
            if measured is None and not paced:
                measured = step.get("latency_ms")
            if measured is not None:
                latency.append(measured)
    return {
        "what": what,
        "runs": options.runs,
        "refresh_ms": round(refresh, 3),
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


def responses(options, name, scenario, out):
    """Each input step of `scenario` over every run: its latency, the host's work for it (every
    frame's `busy_ms` from the input until the step settled, summed, and the JavaScript in it), the
    frames that took, and for a paced motion what it showed."""
    path = SCENARIOS / scenario
    steps = json.loads(path.read_text())["steps"]
    measured = {}
    for run in range(options.runs):
        summary, report, _, _ = run_scenario(options, path, out / f"{slug(name)}-{run + 1}")
        for index, (step, result) in enumerate(zip(steps, summary["steps"])):
            if step["do"][0] == "wait":
                continue
            frames = [line for line in frames_between(report, result["t_inject"], result["t_settled"])
                      if line["surface"] == 1]
            entry = measured.setdefault(index, {"name": step["name"], "budget_ms": step["expect"].get("latency_ms"),
                                                "latency": [], "work": [], "js": [], "js_cpu": [], "frames": []})
            if result.get("latency_ms") is not None:
                entry["latency"].append(result["latency_ms"])
            entry["work"].append(sum(line.get("busy_ms", 0) for line in frames))
            entry["js"].append(sum(line.get("js_ms", 0) for line in frames))
            entry["js_cpu"].append(sum(line.get("js_cpu_ms", 0) for line in frames))
            entry["frames"].append(len(frames))
    return [{"step": entry["name"], "budget_ms": entry["budget_ms"], "latency_ms": percentiles(entry["latency"]),
             "work_ms": percentiles(entry["work"]), "js_ms": percentiles(entry["js"]),
             "js_cpu_ms": percentiles(entry["js_cpu"]),
             "frames": percentiles(entry["frames"])} for _, entry in sorted(measured.items())]


def shell_responses(options, name, scenario, out):
    """Each step of `scenario` that holds the shell to a `js_work` budget, over every run: the most
    JavaScript one piece of the shell's work ran in it on the shell's CPU (its report's `timing`
    lines, frames and work that drew none), as regress.py holds it to the budget, that piece's
    JavaScript by the wall clock and blocked, and the frames the shell presented."""
    path = SCENARIOS / scenario
    steps = json.loads(path.read_text())["steps"]
    measured = {}
    for run in range(options.runs):
        summary, _, shell, _ = run_scenario(options, path, out / f"{slug(name)}-{run + 1}")
        start = summary["t_launch"]
        for index, (step, result) in enumerate(zip(steps, summary["steps"])):
            # From the step before's end, as regress.py checks it: the input and what followed.
            work = [line for line in shell if line["kind"] == "timing" and start <= line["t"] <= result["t_settled"]]
            start = result["t_settled"]
            budget = step["expect"].get("in_shell", {}).get("js_work", {}).get("js_ms_max")
            if budget is None:
                continue
            entry = measured.setdefault(index, {"name": step["name"], "budget_ms": budget, "js": [], "wall": [],
                                                "blocked": [], "frames": []})
            costly = max(work, key=lambda line: line["cpu"].get("javascript", 0), default=None)
            for key, measure in (("js", "cpu"), ("wall", "parts"), ("blocked", "blocked")):
                entry[key].append(costly[measure].get("javascript", 0) if costly else 0.0)
            entry["frames"].append(sum(1 for line in work if line["presented"]))
    return [{"step": entry["name"], "budget_ms": entry["budget_ms"], "js_ms": percentiles(entry["js"]),
             "js_wall_ms": percentiles(entry["wall"]), "js_blocked_ms": percentiles(entry["blocked"]),
             "frames": percentiles(entry["frames"])} for _, entry in sorted(measured.items())]


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



def session_processes():
    """This user's running `oxyos-session` and the Bloom hosts it started, by pid: what each is (the
    session, or a host by its module: OxyOSShell, an application's)."""
    found = {}
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            argv = (entry / "cmdline").read_bytes().split(b"\0")
            parent = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            continue
        found[int(entry.name)] = (argv, parent)
    sessions = {pid for pid, (argv, _) in found.items() if pathlib.Path(argv[0].decode(errors="replace")).name == "oxyos-session"}
    processes = {pid: "oxyos-session" for pid in sessions}
    for pid, (argv, parent) in found.items():
        if parent in sessions and pathlib.Path(argv[0].decode(errors="replace")).name == "oxyos-bloom-host" and len(argv) > 2:
            processes[pid] = argv[2].decode(errors="replace")
    return processes


def threads_of(pid):
    """A process's CPU time (ns, every thread's) and each thread's context switches, by thread id
    and name."""
    cpu = int(pathlib.Path(f"/proc/{pid}/schedstat").read_text().split()[0])
    threads = {}
    for task in pathlib.Path(f"/proc/{pid}/task").iterdir():
        try:
            status = (task / "status").read_text()
        except OSError:
            continue
        name = next((line.split(":", 1)[1].strip() for line in status.splitlines() if line.startswith("Name:")), "?")
        switches = sum(int(line.split()[1]) for line in status.splitlines() if "ctxt_switches" in line)
        threads[task.name] = (name, switches)
    return cpu, threads


def session_idle(window):
    """The running session idle (the module's doc): each process's CPU % and wakeups a second over
    `window` seconds, and its threads that woke; None without a session."""
    processes = session_processes()
    if not processes:
        return None
    before, started = {pid: threads_of(pid) for pid in processes}, time.monotonic()
    time.sleep(window)
    elapsed = time.monotonic() - started
    measured = {}
    for pid, who in processes.items():
        try:
            cpu, threads = threads_of(pid)
        except OSError:
            continue  # it ended meanwhile
        woke = {}
        for tid, (name, switches) in threads.items():
            if tid in before[pid][1]:
                woke[name] = woke.get(name, 0) + switches - before[pid][1][tid][1]
        measured[f"{who} ({pid})"] = {
            "cpu_percent": round((cpu - before[pid][0]) / (elapsed * 1e9) * 100, 3),
            "wakeups_per_s": round(sum(woke.values()) / elapsed, 2),
            "threads": {name: round(count / elapsed, 2) for name, count in sorted(woke.items()) if count},
        }
    return {"window_s": round(elapsed, 1), "processes": measured}


def smaps(pid):
    """A process's memory from smaps_rollup (kB), and each bytecode file it maps: resident and
    proportional (kB), and whether its resident pages are shared."""
    rollup = {}
    for line in pathlib.Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()[1:]:
        key, _, value = line.partition(":")
        if value.strip().endswith("kB"):
            rollup[key] = int(value.split()[0])
    files, current = {}, None
    for line in pathlib.Path(f"/proc/{pid}/smaps").read_text().splitlines():
        fields = line.split()
        if len(fields) >= 5 and "-" in fields[0] and not fields[0].endswith(":"):
            current = fields[5] if len(fields) > 5 and fields[5].endswith(".hbc") else None
            if current:
                files.setdefault(current, {"Rss": 0, "Pss": 0, "Shared_Clean": 0, "Private_Clean": 0, "Private_Dirty": 0})
        elif current and fields and fields[0].rstrip(":") in files[current]:
            files[current][fields[0].rstrip(":")] += int(fields[1])
    return rollup, files


def applications(options, out):
    """Each host's RSS, PSS and USS with the shell, Settings and three more applications running."""
    sys.path.insert(0, str(HERE))
    from regress import Machine
    path = scenario_file(out, "applications side by side", SIDE_BY_SIDE,
                         [{"name": "applications run side by side", "do": ["wait", SIDE_BY_SIDE_WAIT_MS],
                           "allow_unchanged": True, "expect": {}}])
    measured = []

    def during(sampler, run_out):
        report = run_out / "host-report.jsonl"
        deadline = time.monotonic() + 60
        while not (report.exists() and '"kind":"frame"' in report.read_text()):
            if time.monotonic() > deadline:
                return
            time.sleep(0.05)
        settings = next(pid for pid, process in sampler.snapshot().items() if process[0] == "OxyOSSettings")
        environ = dict(item.split("=", 1) for item in
                       pathlib.Path(f"/proc/{settings}/environ").read_text().split("\0") if "=" in item)
        machine = Machine(pathlib.Path(options.runtimes), pathlib.Path(options.packages))
        joined = []
        for name, module in JOINING:
            host, bundle, script = machine.run(module)
            joined.append((name, subprocess.Popen(
                [str(host), str(bundle), module, "--window", "800", "500"],
                env=dict(os.environ, OXYOS_COMPOSITOR_SOCKET=environ["OXYOS_COMPOSITOR_SOCKET"],
                         OXYOS_SCRIPT_URL=f"file://{script}",
                         OXYOS_HOST_REPORT=str(run_out / f"joined-{module}.jsonl")),
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)))
        try:
            deadline = time.monotonic() + 60
            for name, process in joined:
                report = run_out / f"joined-{process.args[2]}.jsonl"
                while not (report.exists() and '"kind":"frame"' in report.read_text()):
                    if time.monotonic() > deadline or process.poll() is not None:
                        raise SystemExit(f"{name} drew no frame")
                    time.sleep(0.05)
            time.sleep(IDLE_SETTLE_S)
            hosts = [(process[0], pid) for pid, process in sampler.snapshot().items()
                     if process[0] in ("OxyOSShell", "OxyOSSettings")]
            hosts += [(name, process.pid) for name, process in joined]
            for who, pid in hosts:
                rollup, files = smaps(pid)
                measured.append({"who": who, "rss_mib": rollup.get("Rss", 0) / 1024,
                                 "pss_mib": rollup.get("Pss", 0) / 1024,
                                 "uss_mib": (rollup.get("Private_Clean", 0) + rollup.get("Private_Dirty", 0)) / 1024,
                                 "bytecode": {os.path.basename(os.path.dirname(f)) + "/" + os.path.basename(f):
                                              {key: round(value / 1024, 2) for key, value in kib.items()}
                                              for f, kib in files.items()}})
        finally:
            for _, process in joined:
                process.terminate()
            for _, process in joined:
                process.wait()

    runs = []
    for run in range(options.runs):
        measured.clear()
        run_scenario(options, path, out / f"applications-{run + 1}", during)
        runs.append(list(measured))
    by_who = {}
    for run in runs:
        for sample in run:
            by_who.setdefault(sample["who"], []).append(sample)
    result = {who: {key: round(statistics.median(sample[key] for sample in samples), 2)
                    for key in ("rss_mib", "pss_mib", "uss_mib")} | {"bytecode": samples[-1]["bytecode"]}
              for who, samples in by_who.items()}
    result["total"] = {key: round(sum(r[key] for r in result.values()), 2) for key in ("rss_mib", "pss_mib", "uss_mib")}
    return result

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
                         spread(r["window"]["js"]), spread(r["window"].get("js_cpu")), spread(r["shell"]["busy"]),
                         spread(r["latency_ms"])])
        print(table(rows, ["interaction", "frames", "interval ms p50/p95/max", "late", "compose ms",
                           "of it reading clients ms", "app frame work ms", "app JS ms", "app JS CPU ms",
                           "shell frame work ms", "latency ms"]))
        print()
    if "responses" in results:
        def middle(stats):
            return "—" if not stats else f"{stats['p50']} / {stats['max']}"

        for name, steps in results["responses"].items():
            rows = [[step["step"], middle(step["latency_ms"]), step["budget_ms"] or "—", middle(step["work_ms"]),
                     middle(step["js_ms"]), middle(step.get("js_cpu_ms"))] for step in steps]
            print(f"{name}:")
            print(table(rows, ["input", "latency ms p50 / max", "budget", "app work ms p50 / max", "app JS ms p50 / max",
                               "app JS CPU ms p50 / max"]))
            print()
    if "shell" in results:
        for name, steps in results["shell"].items():
            rows = [[step["step"], f"{step['js_ms']['p50']} / {step['js_ms']['max']}", step["budget_ms"],
                     f"{step['js_wall_ms']['p50']} / {step['js_wall_ms']['max']}",
                     f"{step['js_blocked_ms']['p50']} / {step['js_blocked_ms']['max']}",
                     f"{step['frames']['p50']} / {step['frames']['max']}"] for step in steps]
            print(f"{name}:")
            print(table(rows, ["event", "shell JS CPU ms p50 / max (the most of one piece of work)", "budget",
                               "its JS by the clock p50 / max", "blocked p50 / max", "shell frames p50 / max"]))
            print()
    if results.get("budgets"):
        rows = [[f"{e['scenario']} {e['step']}", e["host"], e["budget"], e["limit"], f"{e['measured']} ({e['how']})",
                 " ".join(map(str, e["runs"])),
                 ", ".join(f"run {run}" for run in e["slow_runs"]) or "none"] for e in results["budgets"]]
        print("The scenarios' budgets on time, over every run:")
        print(table(rows, ["step", "host", "budget", "at most", "measured", "each run", "runs over it"]))
        print()
    if "cold_start" in results:
        rows = [[name, spread(r["first_frame_ms"]), ", ".join(f"{k} {v}" for k, v in r["stages_ms"].items())]
                for name, r in results["cold_start"].items()]
        print(table(rows, ["application", "first frame ms p50/p95/max", "stages (median ms, each to the next)"]))
        print()
    if "applications" in results:
        rows = [[who, m["rss_mib"], m["pss_mib"], m["uss_mib"],
                 ", ".join(f"{name} {kib['Rss']} resident, {kib['Pss']} PSS" for name, kib in m.get("bytecode", {}).items())]
                for who, m in results["applications"].items()]
        print(table(rows, ["side by side", "RSS MiB", "PSS MiB", "USS MiB", "bytecode mapped (MiB)"]))
    if "genie" in results:
        rows = [[name, r["frames_composed"], spread(r["frame_interval_ms"]), r["late_frames"],
                 spread(r["compose_ms"]), spread(r["read_ms"])] for name, r in results["genie"].items()]
        print(table(rows, ["genie", "frames", "interval ms p50/p95/max", "late", "compose ms p50/p95/max",
                           "of it reading clients ms"]))
        rows = [[name, e["step"], e["claim"], e["measured"], f"{e['least']} to {e['most']}",
                 ", ".join(str(v) for v in e["runs"]), "yes" if e["held"] else "NO"]
                for name, r in results["genie"].items() for e in r["claims"]]
        print(table(rows, ["genie", "step", "claim", "median", "bound", "runs", "held"]))
        print()
    if "refresh" in results:
        rows = [[name, e["step"], e["claim"], e["measured"],
                 f"{e['least']} to {e['most']}" if e["most"] is not None else f"at least {e['least']}",
                 ", ".join(str(v) for v in e["runs"]), "yes" if e["held"] else "NO"]
                for name, entries in results["refresh"].items() for e in entries]
        print(table(rows, ["display rate", "step", "claim", "median", "bound", "runs", "held"]))
    if "idle" in results:
        rows = [[name, who, f"{m['cpu_percent']:.2f}", f"{m['wakeups_per_s']:.1f}", f"{m['rss_mib']:.0f}",
                 f"{m['anon_mib']:.0f}", f"{m['file_mib']:.0f}", m["peak_rss_mib"]]
                for name, processes in results["idle"].items() for who, m in processes.items()]
        print(table(rows, ["idle", "process", "CPU %", "wakeups/s", "RSS MiB", "anon", "file", "peak"]))
    if results.get("session"):
        rows = [[who, f"{m['cpu_percent']:.3f}", f"{m['wakeups_per_s']:.2f}",
                 ", ".join(f"{name} {rate}" for name, rate in m["threads"].items()) or "none"]
                for who, m in results["session"]["processes"].items()]
        print(table(rows, [f"session idle, {results['session']['window_s']} s", "CPU %", "wakeups/s",
                           "threads that woke (a second)"]))
    elif "session" in results:
        print("session idle: no oxyos-session of this user is running")


def check(results):
    """The budgets results exceed, as messages."""
    over = []
    for name, r in results.get("interactions", {}).items():
        refresh = r["refresh_ms"]
        if r["compose_ms"] and r["compose_ms"]["p95"] > BUDGETS["compose_refreshes_p95"] * refresh:
            over.append(f"{name}: composing took {r['compose_ms']['p95']} ms at p95 (a refresh is {refresh} ms)")
        for source in ("window", "shell"):
            busy = r[source]["busy"]
            if busy and busy["p95"] > BUDGETS["host_busy_refreshes_p95"] * refresh:
                over.append(f"{name}: the {source}'s host worked {busy['p95']} ms per frame at p95 "
                            f"(a refresh is {refresh} ms)")
    # The scenarios' own budgets on time, judged over every run (step_budgets): a response's latency,
    # a step's JavaScript, a resize's frames.
    for entry in results.get("budgets", []):
        if entry["measured"] > entry["limit"]:
            slow = ", ".join(f"run {run} ({entry['runs'][run - 1]})" for run in entry["slow_runs"])
            over.append(f"{entry['scenario']} step {entry['step']} ({entry['host']}): {entry['budget']} "
                        f"{entry['measured']}, the {entry['how']}, over {entry['limit']} (slow: {slow})")
    for name, processes in results.get("idle", {}).items():
        for who, m in processes.items():
            if who == "compositor":
                continue  # the development compositor's 5 ms turns are its director's pace
            if m["wakeups_per_s"] > BUDGETS["idle_host_wakeups_per_s"]:
                over.append(f"idle {name}: {who} woke {m['wakeups_per_s']:.1f} times a second")
            if m["cpu_percent"] > BUDGETS["idle_host_cpu_percent"]:
                over.append(f"idle {name}: {who} used {m['cpu_percent']:.2f}% CPU")
    for who, m in ((results.get("session") or {}).get("processes") or {}).items():
        if m["wakeups_per_s"] > BUDGETS["idle_host_wakeups_per_s"]:
            over.append(f"session idle: {who} woke {m['wakeups_per_s']:.2f} times a second ({m['threads']})")
        if m["cpu_percent"] > BUDGETS["idle_host_cpu_percent"]:
            over.append(f"session idle: {who} used {m['cpu_percent']:.3f}% CPU")
    for name, r in results.get("genie", {}).items():
        refresh = r["refresh_ms"]
        if r["compose_ms"] and r["compose_ms"]["p95"] > BUDGETS["compose_refreshes_p95"] * refresh:
            over.append(f"{name}: composing took {r['compose_ms']['p95']} ms at p95 (a refresh is {refresh} ms)")
    claims = {name: r["claims"] for name, r in results.get("genie", {}).items()}
    for name, entries in [*results.get("refresh", {}).items(), *claims.items()]:
        for e in entries:
            if not e["held"]:
                over.append(f"{name} step {e['step']}: {e['claim']} {e['measured']}, the median of the runs "
                            f"{e['runs']}, outside {e['least']} to {e['most']}")
    for name, r in results.get("cold_start", {}).items():
        if r["first_frame_ms"]["p50"] > BUDGETS["first_frame_ms_median"]:
            over.append(f"cold start {name}: first frame after {r['first_frame_ms']['p50']} ms")
    return over


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("parts", nargs="*",
                        help="interactions, responses, shell, cold-start, idle, applications, refresh, genie (all "
                             "by default), and session (the running session idle, only when named)")
    parser.add_argument("--offscreen")
    parser.add_argument("--runtimes", help="the runtimes (regress.py)")
    parser.add_argument("--packages", help="the applications' packages and the probes (regress.py)")
    parser.add_argument("--out", default=None)
    parser.add_argument("--runs", type=int, default=3, help="runs of each scenario, pooled")
    parser.add_argument("--gpu-only", action="store_true", help="compose on the Vulkan device alone")
    parser.add_argument("--renderer", choices=["cpu", "gpu"], default=None,
                        help="the hosts' painter (docs/adr/0031-gpu-buffers.mdx); by default the GPU's where offered")
    parser.add_argument("--check", action="store_true", help="exit 1 when a budget is exceeded")
    options = parser.parse_args()
    every = ["interactions", "responses", "shell", "cold-start", "idle", "applications", "refresh", "genie"]
    parts = options.parts or every
    if not set(parts) <= set(every) | {"session"}:
        parser.error(f"unknown parts {parts}")
    if set(parts) - {"session"} and not (options.offscreen and options.runtimes and options.packages):
        parser.error("--offscreen, --runtimes and --packages are needed for every part but session")
    out = pathlib.Path(options.out or tempfile.mkdtemp(prefix="oxyos-perf-"))
    out.mkdir(parents=True, exist_ok=True)

    results = {"machine": machine(), "gpu_only": options.gpu_only, "runs": options.runs}
    if "interactions" in parts:
        results["interactions"] = {name: interaction(options, name, scenario, steps, what, out)
                                   for name, scenario, steps, what in INTERACTIONS}
    if "responses" in parts:
        results["responses"] = {name: responses(options, name, scenario, out) for name, scenario in RESPONSES}
    if "shell" in parts:
        results["shell"] = {name: shell_responses(options, name, scenario, out) for name, scenario in SHELL_RESPONSES}
    if "cold-start" in parts:
        results["cold_start"] = {name: cold_start(options, name, scenario, out) for name, scenario in COLD_STARTS}
    if "idle" in parts:
        results["idle"] = {name: idle(options, name, scenario, out) for name, scenario in IDLES}
    if "applications" in parts:
        results["applications"] = applications(options, out)
    if "refresh" in parts:
        results["refresh"] = {name: refresh(options, name, scenario, out) for name, scenario in REFRESH}
    if "genie" in parts:
        results["genie"] = {name: genie(options, name, scenario, steps, what, out)
                            for name, scenario, steps, what in GENIE}
    if "session" in parts:
        results["session"] = session_idle(SESSION_WINDOW_S)
    results["budgets"] = step_budgets()
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
