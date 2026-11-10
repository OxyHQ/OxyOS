"""The saturation benchmark (`perf.py saturation`, docs/performance.mdx "Saturation";
docs/adr/0066-quality-of-service.mdx, decision 1): input-to-present latency while the machine is
busy, as a person feels it.

Each interaction (a list scrolled, the pointer moved over a window that follows it, a window dragged,
keys typed) runs in the development compositor under each load S0 to S7, a run at a time, and every
input's latency is measured from its device's time (the time the input carries, as libinput's would,
whenever the compositor got to it) to the first frame that shows its answer (`device_latency_ms` of
the director's paced steps). Under load the compositor's own wait to run is in it, as a person sees
it.

The run is the machine: it needs a control group delegated to it (Delegate=yes), in which it makes
the tree a session's machine has, and puts each process where the machine would:

    ROOT/harness                                   this program
    ROOT/machine                                   S6's memory limit: a machine of so much RAM
      system.slice/oxyos-backupd.service           S5's backup, with the unit file's own settings
      user.slice/user-seat.slice/session.scope     the person's logind session: regress.py and all
                                                   the development compositor starts (the shell,
                                                   the applications, as a session does today)
      user.slice/user-seat.slice/user@.service/    the person's user manager: app.slice,
                                                   background.slice, session.slice (empty until the
                                                   session puts something there: OXYOS_CGROUP_FIXTURE)
      user.slice/user-other.slice                  S7: another user's work

The loads an application makes (S1 to S4, S6) are applications the session launches (development
entries the compositor starts through its launcher, `also_launch`), so they are wherever the session
puts what it launches; the others (S5's backup and index, S7) are placed here as the machine places
them. Each load starts `LEAD_MS` before the input, and its progress is recorded: a load yields, it
is not starved.
"""

import itertools
import json
import os
import pathlib
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time

HERE = pathlib.Path(__file__).resolve().parent
TREE = HERE.parents[1]
SCENARIOS = HERE / "scenarios"

# How long a load runs before the input measured under it.
LEAD_MS = 5000

# S6's machine: the memory the whole tree may use, the hog a tenth more.
MACHINE_MIB = 3072
HOG_FACTOR = 1.1

# The loads: (id, what it is).
LOADS = [
    ("S0", "none"),
    ("S1", "a process computing on every CPU, all of one application (a terminal's `make -jN`)"),
    ("S2", "one application's process with two threads computing for every CPU"),
    ("S3", "a cold `cargo build` of oxyos-search, -j every CPU, started over as it ends (an application's)"),
    ("S4", "five hidden applications animating at the display's rate and one hidden one running JavaScript without rest"),
    ("S5", "a full first backup (restic, the backup service's unit settings) and the search index built, each started over"),
    ("S6", f"an application writing over 110 % of the machine's memory again and again, its pages compressing "
           f"about 2:1 (a machine of {MACHINE_MIB} MiB)"),
    ("S7", "another user's processes computing on every CPU"),
]


# S5: the backup's files (random, so nothing is deduplicated or compressed away) and the index's.
BACKUP_FILES = 6
BACKUP_FILE_MIB = 256
INDEX_FILES = 20000

# The interactions: (name, scenario file or dict, the steps measured (1-based), the step record's
# key with `device_latency_ms`, what happens).
POINTER_SCENARIO = {
    "about": "The saturation benchmark's pointer (perf.py saturation): the mouse moved across PointerTrackProbe, "
             "which draws a dot where the pointer is on every pointermove, at a mouse's 125 Hz.",
    "module": "PointerTrackProbe", "window": [640, 400],
    "steps": [{"name": "The pointer across the window, 120 motions at 125 Hz, each answered by a frame",
               "do": ["move_paced", [40, 40, 600, 360, 120, 125]], "expect": {"moved": {"presented_min": 30}}}],
}
TYPING_SCENARIO = {
    "about": "The saturation benchmark's typing (perf.py saturation): EditingProbe's line focused, then twenty keys "
             "at a person's ten a second, each carrying its time.",
    "module": "EditingProbe", "window": [800, 480],
    "steps": [{"name": "focus the line", "do": ["press", [400, 42]], "expect": {"focus_input": "line"}},
              {"name": "twenty keys at ten a second", "do": ["type_paced", ["the quick brown fox.", 10]],
               "expect": {"texts": ['Line: "the quick brown fox." [20,20]'], "typed": {"presented_min": 10}}}],
}
INTERACTIONS = [
    ("scroll", "perf-settings-scroll.json", [1, 2], "scrolled",
     "Settings' sidebar scrolled 240 px down and back by a touchpad at 60 Hz"),
    ("pointer", POINTER_SCENARIO, [1], "moved", "a window following the pointer moved across it at 125 Hz"),
    ("drag", "perf-window-drag.json", [2], "drag", "WindowProbe dragged by its title area at a mouse's 125 Hz"),
    ("typing", TYPING_SCENARIO, [2], "typed", "twenty keys typed into a field at ten a second"),
]

# A run that lasts longer than this is ended (everything in the session's scope): it has hung.
RUN_DEADLINE_S = 900

# The pass rule (`--check`): under every load, p95 at most the unloaded p95 plus a refresh, and no
# late frame in a drag.
REFRESH_MS = 1000 / 60


class Fixture:
    """The control-group tree a session's machine has (the module's comment), made in the control
    group delegated to this run, and how to start a process in any of it."""

    CONTROLLERS = ("cpuset", "cpu", "io", "memory", "pids")

    def __init__(self):
        line = next(line for line in pathlib.Path("/proc/self/cgroup").read_text().splitlines()
                    if line.startswith("0::"))
        self.root = pathlib.Path("/sys/fs/cgroup") / line[3:].lstrip("/")
        if not (os.access(self.root / "cgroup.procs", os.W_OK)
                and os.access(self.root / "cgroup.subtree_control", os.W_OK)):
            raise SystemExit(
                f"perf.py saturation needs a control group delegated to it, and {self.root} is not: run it "
                "through tools/rn-host-spike/perf-saturation.sh (a development machine) or "
                "tools/rn-host-spike/perf-device.sh (a device beside a person's session)")
        harness = self.make(self.root / "harness")
        # Every process of the run's group (this one, and what started it: perf-device.sh's strace),
        # so the root holds only groups and may give its controllers to them.
        for pid in [os.getpid(), *(self.root / "cgroup.procs").read_text().split()]:
            try:
                (harness / "cgroup.procs").write_text(str(pid))
            except ProcessLookupError:
                pass
        self.controllers = [c for c in self.CONTROLLERS
                            if c in (self.root / "cgroup.controllers").read_text().split()]
        self.enable(self.root)
        self.machine = self.make(self.root / "machine")
        self.system = self.make(self.machine / "system.slice")
        user = self.make(self.machine / "user.slice")
        seat = self.make(user / "user-seat.slice")
        self.session = self.make(seat / "session.scope")
        self.manager = self.make(seat / "user@.service")
        for slice_ in ("app.slice", "background.slice", "session.slice"):
            self.make(self.manager / slice_)
        self.other = self.make(user / "user-other.slice")
        for parent in (self.machine, self.system, user, seat, self.manager, *(self.manager / s for s in
                                                                          ("app.slice", "background.slice",
                                                                           "session.slice"))):
            self.enable(parent)
        self.cpus = sorted(os.sched_getaffinity(0))

    @staticmethod
    def make(path):
        path.mkdir(exist_ok=True)
        return path

    def enable(self, group):
        """The controllers this run has, for `group`'s children (each alone: one the kernel refuses,
        io without a cost model, leaves the others)."""
        for controller in self.controllers:
            try:
                (group / "cgroup.subtree_control").write_text(f"+{controller}")
            except OSError as error:
                print(f"saturation: {controller} not enabled in {group.name}: {error}", file=sys.stderr)

    def unit(self, name):
        """The control group of a system unit of the machine (`system.slice/NAME`)."""
        return self.make(self.system / name)

    def limit_memory(self, mib):
        """The machine's memory: `mib` for the whole tree, or none."""
        (self.machine / "memory.max").write_text("max" if mib is None else str(mib << 20))

    def clear(self):
        """What the session made in the user manager's tree during a run, gone (a scope a launch made
        whose processes ended): every empty group below its slices."""
        for slice_ in ("app.slice", "background.slice", "session.slice"):
            for group in sorted((self.manager / slice_).glob("**/"), key=lambda p: -len(p.parts)):
                if group != self.manager / slice_:
                    try:
                        group.rmdir()
                    except OSError:
                        pass


# Started in a control group, with a unit's settings for the process: what systemd does before it
# runs a unit's program (Nice=, CPUSchedulingPolicy=, IOSchedulingClass=, IOSchedulingPriority=).
PLACE = r"""
import ctypes, os, sys
group, nice, policy, ioclass, ioprio, *command = sys.argv[1:]
open(os.path.join(group, "cgroup.procs"), "w").write(str(os.getpid()))
if nice != "-":
    os.setpriority(os.PRIO_PROCESS, 0, int(nice))
if policy != "-":
    os.sched_setscheduler(0, {"idle": os.SCHED_IDLE, "batch": os.SCHED_BATCH, "other": os.SCHED_OTHER}[policy],
                          os.sched_param(0))
if ioclass != "-":
    number = {"x86_64": 251, "aarch64": 30}[os.uname().machine]
    value = ({"realtime": 1, "best-effort": 2, "idle": 3}[ioclass] << 13) | int(ioprio)
    if ctypes.CDLL(None, use_errno=True).syscall(number, 1, 0, value) != 0:
        raise OSError(ctypes.get_errno(), "ioprio_set")
os.execvp(command[0], command)
"""


def placed(group, command, unit=None):
    """`command` started in `group`, with the process settings of `unit` (`unit_settings`)."""
    unit = unit or {}
    nice = unit.get("Nice", "-")
    policy = unit.get("CPUSchedulingPolicy", "-")
    ioclass = unit.get("IOSchedulingClass", "-")
    ioprio = unit.get("IOSchedulingPriority", "4" if ioclass == "best-effort" else "0")
    return [sys.executable, "-I", "-c", PLACE, str(group), nice, policy, ioclass, ioprio, *map(str, command)]


def unit_settings(path):
    """A unit file's or a drop-in's settings that say where and how its processes run (its
    `[Service]`, `[Slice]` or `[Scope]` section's), as systemd reads them (the last of a key wins)."""
    settings, section = {}, None
    for line in pathlib.Path(path).read_text().splitlines():
        line = line.strip()
        if line.startswith("["):
            section = line
        elif section in ("[Service]", "[Slice]", "[Scope]") and "=" in line and not line.startswith("#"):
            key, value = line.split("=", 1)
            settings[key.strip()] = value.strip()
    return settings


def generated(unit, scope="system"):
    """What the image's systemd generators (config/'s usr/lib/systemd/<scope>-generators) write for
    `unit` on this machine, read as systemd reads a drop-in over its unit (its settings, the last of
    a key winning): the efficiency cores' AllowedCPUs (ADR 0066, decision 6) on a hybrid processor,
    nothing elsewhere, and nothing from a tree without generators."""
    generators = TREE / "config/includes.chroot_after_packages/usr/lib/systemd" / f"{scope}-generators"
    settings = {}
    if not generators.is_dir():
        return settings
    with tempfile.TemporaryDirectory() as out:
        for program in sorted(generators.iterdir()):
            subprocess.run(["sh", str(program), out, out, out], check=True,
                           env=dict(os.environ, SYSTEMD_SCOPE=scope))
        for drop_in in sorted(pathlib.Path(out, f"{unit}.d").glob("*.conf")):
            settings.update(unit_settings(drop_in))
    return settings


def size_bytes(value, total):
    """A systemd size (`512M`, `25%` of `total`, `infinity`) in bytes, or "max"."""
    value = value.strip()
    if value in ("infinity", ""):
        return "max"
    if value.endswith("%"):
        return str(int(total * float(value[:-1]) / 100))
    units = {"K": 1 << 10, "M": 1 << 20, "G": 1 << 30, "T": 1 << 40}
    if value[-1].upper() in units:
        return str(int(float(value[:-1]) * units[value[-1].upper()]))
    return str(int(value))


def apply_unit(group, settings, machine_bytes):
    """A unit's resource settings written to its control group, as systemd writes them:
    CPUWeight= (`idle` is cpu.idle), IOWeight=, Memory{Min,Low,High,Max}= and AllowedCPUs=. Returns what
    was written, for the record."""
    written = {}

    def write(name, value):
        try:
            (group / name).write_text(value)
            written[name] = value
        except OSError as error:
            written[name] = f"refused: {error}"

    weight = settings.get("CPUWeight")
    if weight == "idle":
        write("cpu.idle", "1")
    elif weight:
        write("cpu.weight", weight)
    if settings.get("IOWeight"):
        write("io.weight", f"default {settings['IOWeight']}")
    for key, name in (("MemoryMin", "memory.min"), ("MemoryLow", "memory.low"),
                      ("MemoryHigh", "memory.high"), ("MemoryMax", "memory.max")):
        if settings.get(key):
            write(name, size_bytes(settings[key], machine_bytes))
    if settings.get("AllowedCPUs"):
        write("cpuset.cpus", settings["AllowedCPUs"])
    return written


# The image's drop-ins for the machine's units and the user manager's (config/; docs/adr/
# 0066-quality-of-service.mdx, decision 7) and the fixture's control groups each stands for: their
# memory protections and limits written to the fixture as systemd writes them, so a load measures
# them. A tree without them (before step 6) writes nothing.
IMAGE_UNITS = TREE / "config/includes.chroot_after_packages/usr/lib/systemd"
DROP_INS = [
    ("system/user.slice.d", lambda f: [f.machine / "user.slice"]),
    ("system/user-.slice.d", lambda f: [f.session.parent, f.other]),
    ("system/session-.scope.d", lambda f: [f.session]),
    ("system/user@.service.d", lambda f: [f.manager]),
    ("user/session.slice.d", lambda f: [f.manager / "session.slice"]),
    ("user/background.slice.d", lambda f: [f.manager / "background.slice"]),
    ("user/app.slice.d", lambda f: [f.manager / "app.slice"]),
]


def apply_image_units(fixture, machine_bytes):
    """Each of the image's drop-ins applied to the groups it stands for in the fixture: what was
    written, by group."""
    written = {}
    for folder, groups in DROP_INS:
        for drop_in in sorted((IMAGE_UNITS / folder).glob("*.conf")):
            settings = unit_settings(drop_in)
            for group in groups(fixture):
                done = apply_unit(group, settings, machine_bytes)
                if done:
                    written.setdefault(str(group.relative_to(fixture.root)), {}).update(done)
    return written


def zswap_state():
    """The machine's zswap and MGLRU settings, as the run found them (the device's, never changed by
    the benchmark)."""
    def read(path):
        try:
            return pathlib.Path(path).read_text().strip()
        except OSError:
            return None
    zswap = "/sys/module/zswap/parameters/"
    return {"zswap": read(zswap + "enabled"), "compressor": read(zswap + "compressor"),
            "max_pool_percent": read(zswap + "max_pool_percent"), "shrinker": read(zswap + "shrinker_enabled"),
            "mglru_min_ttl_ms": read("/sys/kernel/mm/lru_gen/min_ttl_ms")}


def oom_kills(group):
    """The processes the kernel's OOM killer ended in a control group and below (memory.events'
    `oom_kill`)."""
    try:
        for line in (group / "memory.events").read_text().splitlines():
            key, _, value = line.partition(" ")
            if key == "oom_kill":
                return int(value)
    except OSError:
        pass
    return 0


def told_pressure(director_log):
    """The levels the development compositor said the machine's memory pressure moved through in a
    run (`[session] memory pressure: …`)."""
    try:
        text = pathlib.Path(director_log).read_text(errors="replace")
    except OSError:
        return []
    return [line.rsplit(": ", 1)[1].strip().lower() for line in text.splitlines()
            if line.startswith("[session] memory pressure: ") and line.rsplit(": ", 1)[1].strip() in
            ("Normal", "Warning", "Critical")]


def pressure(group):
    """A control group's pressure stall totals (µs): {cpu, memory, io} of `some`."""
    totals = {}
    for kind in ("cpu", "memory", "io"):
        try:
            line = (group / f"{kind}.pressure").read_text().splitlines()[0]
            totals[kind] = int(line.rsplit("total=", 1)[1])
        except (OSError, IndexError, ValueError):
            pass
    return totals


def thread_schedstat(pid):
    """The main thread of `pid`: (ns on the CPU, ns waiting to run, times it ran)."""
    try:
        run, wait, slices = pathlib.Path(f"/proc/{pid}/task/{pid}/schedstat").read_text().split()
        return int(run), int(wait), int(slices)
    except (OSError, ValueError):
        return None


def turn_thread(pid):
    """The development compositor's turn (its main thread): its nice and the EEVDF slice it runs
    with (`se.slice`, ns), what docs/adr/0066's step 2 asks for it."""
    try:
        stat = pathlib.Path(f"/proc/{pid}/task/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        sched = pathlib.Path(f"/proc/{pid}/task/{pid}/sched").read_text()
    except OSError:
        return None
    slice_ns = next((int(line.split(":")[1]) for line in sched.splitlines() if line.startswith("se.slice")), None)
    # After the name: state is field 3, nice field 19.
    return {"nice": int(stat[16]), "policy": int(stat[38]), "slice_ns": slice_ns}


def read_bytes(path):
    try:
        return int(pathlib.Path(path).read_text())
    except (OSError, ValueError):
        return None


def read_progress(path):
    try:
        return json.loads(pathlib.Path(path).read_text())
    except (OSError, ValueError):
        return None


class Watch:
    """While one run lasts, every 100 ms: the development compositor's main thread's schedstat, the
    session's and the applications' pressure, and each load's progress, each stamped on the
    monotonic clock the director's steps are stamped on."""

    def __init__(self, fixture, sampler, progress_files):
        self.fixture, self.sampler, self.progress_files = fixture, sampler, progress_files
        self.samples = []
        self.started = time.monotonic()
        # The compositor's pid once seen, and the samples since it went.
        self.compositor_pid, self.gone_for, self.swept = None, 0, False
        self.stopping = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def compositor(self):
        with self.sampler.lock:
            return next((pid for pid, process in self.sampler.processes.items() if process.name == "compositor"), None)

    def sweep(self, everything=False):
        """What the run left running once its compositor went (an application it launched, a
        load): every process of the session's scope that perf.py did not start itself (regress.py,
        S5's index), and everything in the user manager's slices, so regress.py's output closes and
        it ends. With `everything`, regress.py too: a run that hung."""
        me = os.getpid()
        for pid in (self.fixture.session / "cgroup.procs").read_text().split():
            try:
                parent = int(pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[1])
                if everything or parent != me:
                    os.kill(int(pid), 9)
            except (OSError, ValueError, IndexError):
                pass
        for slice_ in ("app.slice", "background.slice", "session.slice"):
            try:
                (self.fixture.manager / slice_ / "cgroup.kill").write_text("1")
            except OSError:
                pass

    def run(self):
        while not self.stopping.wait(0.1):
            pid = self.compositor()
            if pid and self.compositor_pid is None:
                self.compositor_pid = pid
            if self.compositor_pid and not self.swept:
                self.gone_for = 0 if pathlib.Path(f"/proc/{self.compositor_pid}").exists() else self.gone_for + 1
                if self.gone_for >= 30:
                    # Gone three seconds: what it launched should have ended with it.
                    self.sweep()
                    self.swept = True
            if time.monotonic() - self.started > RUN_DEADLINE_S:
                print(f"saturation: a run hung {RUN_DEADLINE_S} s; ended", file=sys.stderr, flush=True)
                self.sweep(everything=True)
                self.started = float("inf")
            self.samples.append({
                "t": time.monotonic_ns(),
                "compositor": thread_schedstat(pid) if pid else None,
                "turn": turn_thread(pid) if pid else None,
                "session": pressure(self.fixture.session),
                "apps": pressure(self.fixture.manager / "app.slice"),
                "loads": {name: read_progress(path) for name, path in self.progress_files.items()},
                # The machine's memory compressed in zswap's pool and written to swap, in bytes.
                "zswap": read_bytes(self.fixture.machine / "memory.zswap.current"),
                "swap": read_bytes(self.fixture.machine / "memory.swap.current"),
            })

    def stop(self):
        self.stopping.set()
        self.thread.join()

    def between(self, start_ns, end_ns):
        """What changed from the last sample before `start_ns` to the first after `end_ns`."""
        before = [s for s in self.samples if s["t"] <= start_ns] or self.samples[:1]
        after = [s for s in self.samples if s["t"] >= end_ns] or self.samples[-1:]
        if not before or not after:
            return {}
        a, b = before[-1], after[0]
        seconds = max((b["t"] - a["t"]) / 1e9, 1e-9)
        result = {"seconds": round(seconds, 3), "turn": b.get("turn")}
        if a["compositor"] and b["compositor"]:
            ran = b["compositor"][2] - a["compositor"][2]
            waited = b["compositor"][1] - a["compositor"][1]
            result["compositor_wait_us"] = round(waited / max(ran, 1) / 1000, 1)
            result["compositor_waited_ms"] = round(waited / 1e6, 1)
        for where in ("session", "apps"):
            result[f"{where}_pressure"] = {kind: round((b[where][kind] - a[where][kind]) / (seconds * 1e6) * 100, 2)
                                           for kind in b[where] if kind in a[where]}
        loads = {}
        for name in b["loads"]:
            first = next((s["loads"].get(name) for s in reversed(before) if s["loads"].get(name)), None)
            last = b["loads"].get(name)
            if first and last:
                loads[name] = {"work_per_s": round((last["work"] - first["work"]) / seconds, 3),
                               "cores": round((last["cpu_ms"] - first["cpu_ms"]) / (seconds * 1000), 2)}
        result["loads"] = loads
        within = [s for s in self.samples if a["t"] <= s["t"] <= b["t"]]
        for kind in ("zswap", "swap"):
            seen = [s[kind] for s in within if s.get(kind) is not None]
            if seen:
                result[f"{kind}_mib"] = round(max(seen) / (1 << 20), 1)
        return result


def percentiles(values):
    if not values:
        return None
    ordered = sorted(values)

    def rank(p):
        return ordered[min(len(ordered) - 1, max(0, round(p * len(ordered) + 0.5) - 1))]

    return {"n": len(ordered), "p50": round(rank(0.50), 1), "p95": round(rank(0.95), 1),
            "p99": round(rank(0.99), 1), "max": round(ordered[-1], 1)}


class Loads:
    """The loads of one run of the benchmark: what each needs made once, what the session launches
    for it (`also_launch`) and what is started here and kept running across its interactions."""

    def __init__(self, options, fixture, out, search):
        self.options, self.fixture, self.out = options, fixture, out
        # perf.py's search service and its generated home (`search`).
        self.search_service, self.search_home = search
        self.offscreen = str(pathlib.Path(options.offscreen).resolve())
        self.n = len(fixture.cpus)
        self.progress = out / "loads"
        self.progress.mkdir(parents=True, exist_ok=True)
        self.running = []
        self.search = None

    def progress_file(self, name):
        return self.progress / f"{name}.progress"

    def app(self, id, name, *load):
        """An application of the session running `oxyos-offscreen --load ...`."""
        return {"id": f"so.oxy.perf.{id}", "name": name,
                "exec": ["{offscreen}", "--load", *map(str, load), "--progress", str(self.progress_file(id))]}

    def applications(self, load):
        """What the session launches for `load`: its `also_launch`."""
        if load == "S1":
            return [self.app("processes", "Processes", "processes", self.n)]
        if load == "S2":
            return [self.app("threads", "Threads", "spin", 2 * self.n)]
        if load == "S3":
            return [self.build_app()]
        if load == "S4":
            return ([{"id": f"so.oxy.perf.animation{n}", "name": f"Animation {n}", "module": "LoadAnimationProbe",
                      "minimized": True} for n in range(1, 6)]
                    + [{"id": "so.oxy.perf.script", "name": "Script", "module": "LoadScriptProbe", "minimized": True}])
        if load == "S6":
            return [self.app("memory", "Memory", "memory", int(MACHINE_MIB * HOG_FACTOR))]
        return []

    def build_app(self):
        """S3: oxyos-search built cold, -j every CPU, again each time it ends, with the machine's
        toolchain and crates (offline), into a target of the run's own."""
        target = self.out / "s3-target"
        home = pathlib.Path(os.environ.get("SATURATION_TOOLCHAIN_HOME") or pathlib.Path.home())
        script = (f'rm -rf "{target}" && cargo build --offline --quiet -j {self.n} -p oxyos-search '
                  f'--manifest-path "{TREE / "Cargo.toml"}" --target-dir "{target}"')
        environment = [f"RUSTUP_HOME={home / '.rustup'}", f"CARGO_HOME={home / '.cargo'}",
                       f"PATH={home / '.cargo/bin'}:/usr/local/bin:/usr/bin:/bin"]
        return {"id": "so.oxy.perf.build", "name": "Build",
                "exec": ["/usr/bin/env", *environment, "{offscreen}", "--load", "repeat", "--progress",
                         str(self.progress_file("build")), "--", "/bin/sh", "-c", script]}

    def start(self, load):
        """What `load` runs outside the session, started; and S6's machine limited."""
        self.fixture.limit_memory(MACHINE_MIB if load == "S6" else None)
        if load == "S5":
            self.start_backup()
            self.start_index()
        if load == "S7":
            command = [self.offscreen, "--load", "processes", self.n, "--progress", self.progress_file("other")]
            self.running.append(subprocess.Popen(placed(self.fixture.other, command)))
        if self.running or self.search:
            time.sleep(LEAD_MS / 1000)

    def stop(self):
        for process in self.running:
            process.terminate()
        for process in self.running:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        self.running = []
        if self.search:
            self.search["stop"].set()
            self.search["thread"].join()
            self.search = None
        self.fixture.limit_memory(None)

    def backup_home(self):
        home = self.out / "s5-backup-home"
        marker = home / ".made"
        if not marker.exists():
            shutil.rmtree(home, ignore_errors=True)
            home.mkdir(parents=True)
            for n in range(BACKUP_FILES):
                with open(home / f"blob-{n}.bin", "wb") as blob:
                    for _ in range(BACKUP_FILE_MIB):
                        blob.write(os.urandom(1 << 20))
            marker.write_text("")
        return home

    def start_backup(self):
        """S5's backup: restic's first backup of a home into a new repository, again each time it
        ends, in the backup service's unit with the unit file's settings (tools/oxyos-backupd)."""
        restic = shutil.which("restic")
        if restic is None:
            raise SystemExit("S5 needs restic (the backup service's engine)")
        unit = TREE / "tools/oxyos-backupd/oxyos-backupd.service"
        settings = {**unit_settings(unit), **generated("oxyos-backupd.service")}
        group = self.fixture.unit("oxyos-backupd.service")
        written = apply_unit(group, settings, MACHINE_MIB << 20)
        (self.out / "s5-unit.json").write_text(json.dumps({"settings": settings, "written": written}, indent=1))
        repository = self.out / "s5-repository"
        script = (f'rm -rf "{repository}" && "{restic}" init --quiet --repo "{repository}" && '
                  f'"{restic}" backup --quiet --no-cache --repo "{repository}" "{self.backup_home()}"')
        command = [self.offscreen, "--load", "repeat", "--progress", self.progress_file("backup"), "--",
                   "/bin/sh", "-c", script]
        environment = dict(os.environ, RESTIC_PASSWORD="saturation")
        self.running.append(subprocess.Popen(placed(group, command, settings), env=environment,
                                             stdout=subprocess.DEVNULL, stderr=open(self.out / "s5-backup.log", "w")))

    def start_index(self):
        """S5's index: the search service as the session runs it (its child, so in the session's
        scope: ADR 0051 decision 13) building a new index of INDEX_FILES files, again each time it
        is done."""
        root = self.out / "s5-index"
        home = root / "home"
        if not (root / ".made").exists():
            shutil.rmtree(root, ignore_errors=True)
            home.mkdir(parents=True)
            self.search_home(home, INDEX_FILES)
            (root / ".made").write_text("")
        stop = threading.Event()
        rounds = {"done": 0, "cpu_ms": 0}
        progress = self.progress_file("index")

        def loop():
            while not stop.is_set():
                for kept in ("cache", "data", "config"):
                    shutil.rmtree(root / kept, ignore_errors=True)
                service = self.search_service(self.offscreen, root, prefix=placed(self.fixture.session, []))
                try:
                    while not stop.wait(0.5):
                        status = service.ask({"status": {}})["status"]
                        used = int(service.cpu_ns() / 1e6)
                        progress.write_text(json.dumps({"work": rounds["done"] * INDEX_FILES + status["items"],
                                                        "cpu_ms": rounds["cpu_ms"] + used}))
                        if not status["indexing"] and status["items"] >= INDEX_FILES:
                            rounds["done"] += 1
                            rounds["cpu_ms"] += used
                            break
                finally:
                    service.stop()

        thread = threading.Thread(target=loop, daemon=True)
        thread.start()
        self.search = {"stop": stop, "thread": thread}

    def progress_files(self, load):
        names = {"S1": ["processes"], "S2": ["threads"], "S3": ["build"], "S5": ["backup", "index"],
                 "S6": ["memory"], "S7": ["other"]}.get(load, [])
        return {name: self.progress_file(name) for name in names}


def scenario_of(interaction):
    if isinstance(interaction, dict):
        return json.loads(json.dumps(interaction))
    return json.loads((SCENARIOS / interaction).read_text())


# What a paced step's expectations count of its frames: the numbers this benchmark measures, which a
# load is expected to change, never a reason to drop a run (what the steps do is still checked).
COUNTED = {"scrolled": ("presented_min",), "moved": ("presented_min", "latency_max_ms"),
           "typed": ("presented_min",), "drag": ("composed_min",)}


def relaxed(scenario):
    """`scenario` with its paced steps' frame counts left to the measure."""
    for step in scenario["steps"]:
        expect = step.get("expect", {})
        for key, counts in COUNTED.items():
            for count in counts:
                expect.get(key, {}).pop(count, None)
    return scenario


def measure(options, out, run_scenario, fixture, loads, load, interaction):
    """One interaction's runs under `load`: every input's device latency pooled, the frames, the
    host's JavaScript, the compositor's wait, pressure and the loads' progress."""
    name, source, steps, key, what = interaction
    latencies, compose, js, late, frames = [], [], [], 0, 0
    waits, pressures, progress, failed, turns = [], [], [], [], []
    kills, levels, swapped = {"machine": 0, "session": 0}, [], []
    for run in range(options.runs):
        scenario = relaxed(scenario_of(source))
        # The application under the hand is one the session launched, with its id (its launch
        # record), wherever the session puts what it launches.
        scenario.setdefault("app", {"id": f"so.oxy.perf.{name}", "title": name.capitalize()})
        applications = loads.applications(load)
        if applications:
            scenario["also_launch"] = applications
            scenario["also_launch_lead_ms"] = LEAD_MS
        run_out = out / load / f"{name}-{run + 1}"
        run_out.mkdir(parents=True, exist_ok=True)
        path = run_out / "scenario.json"
        path.write_text(json.dumps(scenario, indent=1))
        for file in loads.progress_files(load).values():
            if load not in ("S5", "S7"):
                file.unlink(missing_ok=True)
        watch = {}

        def during(sampler, _out, watch=watch):
            watch["it"] = Watch(fixture, sampler, loads.progress_files(load))

        before = {"machine": oom_kills(fixture.machine), "session": oom_kills(fixture.session)}
        try:
            summary, report, _, _ = run_scenario(options, path, run_out, during=during, cgroup=fixture.session)
        except SystemExit as failure:
            # A run the load broke (the compositor or the application killed, a step never settled)
            # is a result, not the benchmark's end.
            failed.append(f"run {run + 1}: {str(failure).splitlines()[0][:300]}")
            continue
        finally:
            if "it" in watch:
                watch["it"].stop()
            fixture.clear()
            # What the kernel ended for want of memory, the session's own among it; and the levels
            # the applications were told (docs/adr/0066, decision 7).
            kills["machine"] += oom_kills(fixture.machine) - before["machine"]
            kills["session"] += oom_kills(fixture.session) - before["session"]
            levels.append(told_pressure(run_out / "director.log"))
        measured = [summary["steps"][index - 1] for index in steps]
        start, end = measured[0]["t_inject"], measured[-1]["t_settled"]
        for step in measured:
            record = step.get(key) or {}
            latencies += record.get("device_latency_ms", [])
            paced = step.get("drag_paced") or step.get("scroll_paced") or step.get("move_paced")
            composed = step.get("composed", [])
            if paced:
                span = paced[-2] / paced[-1] * 1000 + REFRESH_MS
                composed = [frame for frame in composed if frame[0] <= span]
            if key == "drag":
                gaps = [b[0] - a[0] for a, b in itertools.pairwise(composed)]
                late += sum(1 for gap in gaps if gap > 1.5 * REFRESH_MS)
            frames += len(composed)
            compose += [frame[1] for frame in composed]
            js += [line["js_ms"] for line in report if line["kind"] == "frame" and line.get("surface") == 1
                   and step["t_inject"] <= line["t"] <= step["t_settled"] and "js_ms" in line]
        window = watch["it"].between(start, end)
        if "compositor_wait_us" in window:
            waits.append(window["compositor_wait_us"])
        if window.get("turn"):
            turns.append(window["turn"])
        pressures.append({where: window.get(f"{where}_pressure", {}) for where in ("session", "apps")})
        swapped.append({kind: window.get(f"{kind}_mib") for kind in ("zswap", "swap")})
        progress.append(window.get("loads", {}))

    def mean_of(entries, *path):
        values = []
        for entry in entries:
            for part in path:
                entry = (entry or {}).get(part)
            if entry is not None:
                values.append(entry)
        return round(statistics.mean(values), 3) if values else None

    loads_progress = {}
    for name_ in sorted({n for p in progress for n in p}):
        loads_progress[name_] = {"work_per_s": mean_of(progress, name_, "work_per_s"),
                                 "cores": mean_of(progress, name_, "cores")}
    return {
        "what": what,
        "runs": options.runs,
        "latency_ms": percentiles(latencies),
        "frames": frames,
        "late_frames": late if key == "drag" else None,
        "compose_ms": percentiles(compose),
        "js_ms": percentiles(js),
        "compositor_wait_us": round(statistics.mean(waits), 1) if waits else None,
        "pressure": {where: {kind: mean_of(pressures, where, kind) for kind in ("cpu", "memory", "io")}
                     for where in ("session", "apps")},
        "loads": loads_progress,
        "failed": failed,
        "oom_kills": kills,
        # The most of the machine's memory in zswap's pool and in swap while the input was measured.
        "zswap_mib": max((s["zswap"] for s in swapped if s.get("zswap") is not None), default=None),
        "swap_mib": max((s["swap"] for s in swapped if s.get("swap") is not None), default=None),
        "memory_pressure_told": levels,
        # The turn thread as the last run left it: its nice, policy and slice.
        "turn": turns[-1] if turns else None,
    }


class PlayedRtkit:
    """rtkit as the session asks it (docs/adr/0066, decision 2), played for the run where the tree
    has it (`tools/rn-host-spike/played-rtkit.sh`): on a bus of the run's own, in a unit of its own
    with CAP_SYS_NICE alone, changing only the threads of this run's control groups. The development
    compositor asks it (`OXYOS_RTKIT_BUS`) as the session asks the system's; a tree without it runs
    the compositor as it starts."""

    SCRIPT = HERE / "played-rtkit.sh"

    def __init__(self, offscreen, fixture, out):
        self.bus = self.unit = None
        if not self.SCRIPT.exists():
            return
        # Started by the run's wrapper, outside its unit, where sudo can still run it: perf-device.sh,
        # whose unit runs under strace's seccomp filter, so nothing in it gains a privilege.
        if os.environ.get("OXYOS_RTKIT_BUS"):
            return
        out.mkdir(parents=True, exist_ok=True)
        self.bus = subprocess.Popen(placed(fixture.root / "harness", ["dbus-daemon", "--session", "--nofork",
                                                                      "--print-address=1"]),
                                    stdout=subprocess.PIPE, text=True)
        address = self.bus.stdout.readline().strip()
        started = subprocess.run([str(self.SCRIPT), "start", offscreen, address, str(fixture.root), str(out)],
                                 capture_output=True, text=True)
        if started.returncode != 0:
            self.stop()
            raise SystemExit(f"the played rtkit did not start: {started.stderr.strip()[-500:]}")
        self.unit = started.stdout.strip().splitlines()[-1]
        os.environ["OXYOS_RTKIT_BUS"] = address

    def stop(self):
        if self.unit is None and self.bus is None:
            return
        if self.unit:
            subprocess.run([str(self.SCRIPT), "stop", self.unit], check=False)
            self.unit = None
        if self.bus:
            self.bus.terminate()
            self.bus.wait()
            self.bus = None
        os.environ.pop("OXYOS_RTKIT_BUS", None)


def seat_weights(offscreen, fixture):
    """The users' slices weighted as the machine weighs them (docs/adr/0066, decision 8): the
    accounts' service's own rule (`oxyos-accountd --apply-seat-weights USERS SEAT`, beside the
    development compositor) given the run's `user.slice`, the person at the seat's slice named. A
    build without it leaves every user's slice at 100, as today's machine does. What it wrote."""
    accountd = pathlib.Path(offscreen).resolve().parent / "oxyos-accountd"
    users = fixture.machine / "user.slice"
    if not accountd.exists():
        return {"applied": False, "why": f"no {accountd.name} beside the development compositor"}
    done = subprocess.run([str(accountd), "--apply-seat-weights", str(users), "user-seat.slice"],
                          capture_output=True, text=True)
    if done.returncode != 0:
        return {"applied": False, "why": (done.stderr.strip() or done.stdout.strip())[-300:]}
    weights = dict(line.split()[:2] for line in done.stdout.splitlines() if len(line.split()) >= 2)
    return {"applied": True, "weights": weights}


def run(options, out, run_scenario, search):
    """Every chosen load and interaction (`--loads`, `--interactions`); `search` is perf.py's search
    service and its home's maker, for S5's index."""
    fixture = Fixture()
    chosen_loads = [(id, what) for id, what in LOADS if not options.loads or id in options.loads.split(",")]
    chosen = [i for i in INTERACTIONS if not options.interactions or i[0] in options.interactions.split(",")]
    os.environ["OXYOS_CGROUP_FIXTURE"] = str(fixture.manager)
    # The machine's memory pressure, as the session follows /proc/pressure/memory: the fixture's
    # machine's (docs/adr/0066, decision 7).
    os.environ["OXYOS_MEMORY_PRESSURE_FIXTURE"] = str(fixture.machine / "memory.pressure")
    loads = Loads(options, fixture, out / "saturation", search)
    rtkit = PlayedRtkit(loads.offscreen, fixture, out / "saturation" / "rtkit")
    results = {"cpus": fixture.cpus, "cgroup": str(fixture.root), "machine_mib": MACHINE_MIB,
               "kernel": platform.release(), "loads": {}, "what": dict(LOADS)}
    results["played_rtkit"] = rtkit.unit is not None
    results["seat_weights"] = seat_weights(loads.offscreen, fixture)
    results["image_units"] = apply_image_units(fixture, MACHINE_MIB << 20)
    results["memory"] = zswap_state()
    try:
        for load, what in chosen_loads:
            print(f"saturation: {load}, {what}", file=sys.stderr, flush=True)
            loads.start(load)
            try:
                results["loads"][load] = {name: measure(options, out / "saturation", run_scenario, fixture, loads,
                                                        load, interaction)
                                          for interaction in chosen for name in [interaction[0]]}
            finally:
                loads.stop()
    finally:
        rtkit.stop()
    return results


def show(results, table):
    def spread(stats):
        return "—" if not stats else f"{stats['p50']} / {stats['p95']} / {stats['p99']} / {stats['max']}"

    def loads(entry):
        def each(name, p):
            work = f"{p['work_per_s']}/s, " if p.get("work_per_s") else ""
            return f"{name}: {work}{p['cores']} cores"
        return ", ".join(each(name, p) for name, p in entry["loads"].items()) or "—"

    def memory(entry):
        # What the kernel ended for want of memory, and the highest level the applications were told.
        kills = entry.get("oom_kills") or {}
        told = {level for run in entry.get("memory_pressure_told") or [] for level in run}
        highest = next((level for level in ("critical", "warning") if level in told), None)
        said = []
        if kills.get("machine"):
            said.append(f"{kills['machine']} OOM kills ({kills.get('session', 0)} of the session's)")
        if highest:
            said.append(f"told {highest}")
        if entry.get("swap_mib") or entry.get("zswap_mib"):
            said.append(f"zswap {entry.get('zswap_mib') or 0} MiB, swap {entry.get('swap_mib') or 0} MiB")
        return ("; " + ", ".join(said)) if said else ""

    rows = []
    for load, interactions in results["loads"].items():
        for name, entry in interactions.items():
            pressure = entry["pressure"]["session"]
            rows.append([load, name, entry["latency_ms"]["n"] if entry["latency_ms"] else 0,
                         spread(entry["latency_ms"]),
                         "—" if entry["late_frames"] is None else entry["late_frames"],
                         entry["compose_ms"]["p95"] if entry["compose_ms"] else "—",
                         entry["js_ms"]["p95"] if entry["js_ms"] else "—",
                         entry["compositor_wait_us"] if entry["compositor_wait_us"] is not None else "—",
                         " / ".join(str(pressure.get(kind, "—")) for kind in ("cpu", "memory", "io")),
                         loads(entry) + memory(entry)
                         + (f"; {len(entry['failed'])} runs failed" if entry.get("failed") else "")])
    turn = next((entry["turn"] for interactions in results["loads"].values() for entry in interactions.values()
                 if entry.get("turn")), None)
    said = "" if not turn else (f", the compositor's turn at nice {turn['nice']}, policy {turn['policy']}, "
                                f"a {turn['slice_ns'] / 1e6:.2f} ms slice")
    seat = results.get("seat_weights") or {}
    users = (", users' slices " + " ".join(f"{k} {v}" for k, v in seat["weights"].items())
             if seat.get("applied") else ", every user's slice at 100")
    memory = results.get("memory") or {}
    if memory.get("zswap"):
        said += (f", zswap {memory['zswap']} ({memory['compressor']}), MGLRU min_ttl_ms {memory['mglru_min_ttl_ms']}"
                 f"{', the image’s memory protections' if results.get('image_units') else ''}")
    print(f"saturation on {len(results['cpus'])} CPUs ({results['kernel']}){said}"
          f"{', rtkit played' if results.get('played_rtkit') else ''}{users}:")
    print(table(rows, ["load", "interaction", "inputs", "input to present ms p50 / p95 / p99 / max", "late frames",
                       "compose ms p95", "app JS ms p95", "compositor wait µs a run", "session pressure % cpu / mem / io",
                       "the load's progress"]))
    print()


def check(results):
    """The pass rule: under every load, an interaction's p95 at most its unloaded (S0) p95 plus a
    refresh, and no late frame in a drag."""
    over = []
    base = results["loads"].get("S0", {})
    for load, interactions in results["loads"].items():
        for name, entry in interactions.items():
            unloaded = (base.get(name) or {}).get("latency_ms")
            measured = entry["latency_ms"]
            if load != "S0" and unloaded and measured and measured["p95"] > unloaded["p95"] + REFRESH_MS:
                over.append(f"saturation {load} {name}: input to present {measured['p95']} ms at p95, over the "
                            f"unloaded {unloaded['p95']} ms and a refresh")
            for failure in entry.get("failed", []):
                over.append(f"saturation {load} {name}: {failure}")
            if entry.get("late_frames"):
                over.append(f"saturation {load} {name}: {entry['late_frames']} late frames")
    return over
