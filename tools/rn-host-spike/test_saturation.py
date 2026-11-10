"""saturation.py's own checks (`python3 tools/rn-host-spike/test_saturation.py`): the unit files read
as systemd reads them, their settings written as systemd writes them, a process started in its
control group with the unit's settings, the windows the latencies and the loads' progress are taken
over, and the pass rule; on files and processes of the test's own, no control group needed."""

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import saturation  # noqa: E402  (beside this file)

TREE = pathlib.Path(__file__).resolve().parents[2]


class UnitFiles(unittest.TestCase):
    def test_the_backup_service_unit_says_where_its_processes_run(self):
        settings = saturation.unit_settings(TREE / "tools/oxyos-backupd/oxyos-backupd.service")
        # Whatever step 5 makes them, the benchmark reads the unit's own (docs/adr/0066).
        self.assertIn("CPUSchedulingPolicy", settings)
        self.assertIn("IOSchedulingClass", settings)

    def test_only_the_service_section_and_the_last_value(self):
        with tempfile.NamedTemporaryFile("w", suffix=".service", delete=False) as unit:
            unit.write("[Unit]\nNice=5\n[Service]\n# Nice=3\nNice=10\nCPUWeight=idle\nNice=19\n[Install]\nNice=1\n")
        try:
            self.assertEqual(saturation.unit_settings(unit.name), {"Nice": "19", "CPUWeight": "idle"})
        finally:
            os.unlink(unit.name)

    def test_sizes_as_systemd_reads_them(self):
        total = 4 << 30
        self.assertEqual(saturation.size_bytes("512M", total), str(512 << 20))
        self.assertEqual(saturation.size_bytes("1.5G", total), str(3 << 29))
        self.assertEqual(saturation.size_bytes("25%", total), str(1 << 30))
        self.assertEqual(saturation.size_bytes("infinity", total), "max")
        self.assertEqual(saturation.size_bytes("4096", total), "4096")


class Generated(unittest.TestCase):
    def test_a_tree_without_generators_writes_nothing(self):
        self.assertEqual(saturation.generated("oxyos-test-nothing.service"), {})


class UnitSettingsWritten(unittest.TestCase):
    def test_each_setting_to_its_file(self):
        with tempfile.TemporaryDirectory() as group:
            group = pathlib.Path(group)
            written = saturation.apply_unit(group, {"CPUWeight": "idle", "IOWeight": "10", "MemoryHigh": "50%",
                                                    "AllowedCPUs": "8-15", "Nice": "19"}, 2 << 30)
            self.assertEqual((group / "cpu.idle").read_text(), "1")
            self.assertEqual((group / "io.weight").read_text(), "default 10")
            self.assertEqual((group / "memory.high").read_text(), str(1 << 30))
            self.assertEqual((group / "cpuset.cpus").read_text(), "8-15")
            # A process's settings are not the group's.
            self.assertNotIn("nice", " ".join(written))
            saturation.apply_unit(group, {"CPUWeight": "20"}, 2 << 30)
            self.assertEqual((group / "cpu.weight").read_text(), "20")


class Placed(unittest.TestCase):
    def test_a_process_joins_its_group_with_its_units_settings(self):
        with tempfile.TemporaryDirectory() as group:
            command = [sys.executable, "-c",
                       "import os; print(os.getpriority(os.PRIO_PROCESS, 0), os.sched_getscheduler(0))"]
            settings = {"Nice": "19", "CPUSchedulingPolicy": "batch"}
            out = subprocess.run(saturation.placed(group, command, settings), capture_output=True, text=True,
                                 check=True).stdout.split()
            self.assertEqual(out, ["19", str(os.SCHED_BATCH)])
            # What it wrote to join the group: its own pid, which became the command's.
            joined = (pathlib.Path(group) / "cgroup.procs").read_text()
            self.assertTrue(joined.isdigit())


class ImageMemory(unittest.TestCase):
    """The image's memory protections (docs/adr/0066, decision 7) written to the fixture's groups
    each drop-in stands for, and what a run records of the kernel's kills and the levels told."""

    def test_each_drop_in_on_the_groups_it_stands_for(self):
        with tempfile.TemporaryDirectory() as root:
            root = pathlib.Path(root)
            machine = root / "machine"
            seat = machine / "user.slice/user-seat.slice"
            manager = seat / "user@.service"
            fixture = type("Fixture", (), {"root": root, "machine": machine, "session": seat / "session.scope",
                                           "other": machine / "user.slice/user-other.slice",
                                           "manager": manager})()
            for group in (fixture.session, fixture.other, manager / "session.slice", manager / "background.slice",
                          manager / "app.slice"):
                group.mkdir(parents=True)
            written = saturation.apply_image_units(fixture, 4 << 30)
            self.assertEqual((fixture.session / "memory.min").read_text(), str(512 << 20))
            self.assertEqual((seat / "memory.min").read_text(), str(512 << 20))
            self.assertEqual((fixture.other / "memory.low").read_text(), str(128 << 20))
            self.assertEqual((machine / "user.slice/memory.min").read_text(), str(1 << 30))
            self.assertEqual((manager / "memory.low").read_text(), str(128 << 20))
            self.assertEqual((manager / "session.slice/memory.low").read_text(), str(128 << 20))
            # A fifth of the machine's memory.
            self.assertEqual((manager / "background.slice/memory.high").read_text(), str(int((4 << 30) * 0.2)))
            # oomd's settings are no control group's files: nothing written for app.slice.
            self.assertNotIn("machine/user.slice/user-seat.slice/user@.service/app.slice", written)

    def test_kills_and_levels_told(self):
        with tempfile.TemporaryDirectory() as root:
            root = pathlib.Path(root)
            (root / "memory.events").write_text("low 0\nhigh 3\nmax 0\noom 1\noom_kill 2\noom_group_kill 0\n")
            self.assertEqual(saturation.oom_kills(root), 2)
            self.assertEqual(saturation.oom_kills(root / "gone"), 0)
            log = root / "director.log"
            log.write_text("[session] memory pressure: Warning\n[launch] x\n[session] memory pressure: Critical\n"
                           "[session] memory pressure: no trigger \"some\" on /x: refused\n"
                           "[session] memory pressure: Normal\n")
            self.assertEqual(saturation.told_pressure(log), ["warning", "critical", "normal"])


class Windows(unittest.TestCase):
    def watch(self, samples):
        watch = saturation.Watch.__new__(saturation.Watch)
        watch.samples = samples
        return watch

    def test_from_before_the_first_input_to_after_the_last(self):
        sample = lambda t, ran, waited, cpu, work: {  # noqa: E731
            "t": t, "compositor": (0, waited, ran), "session": {"cpu": cpu, "memory": 0, "io": 0}, "apps": {},
            "loads": {"spin": {"work": work, "cpu_ms": work * 10}}}
        watch = self.watch([sample(0, 0, 0, 0, 0), sample(1_000_000_000, 100, 5_000_000, 100_000, 10),
                            sample(2_000_000_000, 300, 25_000_000, 600_000, 30),
                            sample(3_000_000_000, 400, 35_000_000, 700_000, 40)])
        window = watch.between(1_100_000_000, 1_900_000_000)
        # From the sample at 1 s to the one at 2 s: 200 runs that waited 20 ms.
        self.assertEqual(window["seconds"], 1.0)
        self.assertEqual(window["compositor_wait_us"], 100.0)
        self.assertEqual(window["session_pressure"]["cpu"], 50.0)
        self.assertEqual(window["loads"]["spin"], {"work_per_s": 20.0, "cores": 0.2})


class Relaxed(unittest.TestCase):
    def test_frame_counts_are_measured_not_required(self):
        scenario = saturation.relaxed(saturation.scenario_of("perf-window-drag.json"))
        drag = scenario["steps"][1]["expect"]["drag"]
        self.assertNotIn("composed_min", drag)
        self.assertEqual(drag["bytes_read_max"], 0)
        typing = saturation.relaxed(saturation.scenario_of(saturation.TYPING_SCENARIO))
        self.assertEqual(typing["steps"][1]["expect"]["typed"], {})
        self.assertIn("presented_min", saturation.TYPING_SCENARIO["steps"][1]["expect"]["typed"])


class PassRule(unittest.TestCase):
    def entry(self, p95, late=None):
        return {"latency_ms": {"n": 10, "p50": 1, "p95": p95, "p99": p95, "max": p95}, "late_frames": late,
                "failed": []}

    def test_a_refresh_over_the_unloaded_and_no_late_frame(self):
        results = {"loads": {"S0": {"scroll": self.entry(10.0), "drag": self.entry(5.0, 0)},
                             "S1": {"scroll": self.entry(26.0), "drag": self.entry(5.0, 0)},
                             "S2": {"scroll": self.entry(27.0), "drag": self.entry(5.0, 2)}}}
        over = saturation.check(results)
        self.assertEqual(len(over), 2, over)
        self.assertIn("S2 scroll", over[0])
        self.assertIn("2 late frames", over[1])

    def test_the_loads_are_the_applications_the_session_launches(self):
        class Options:
            offscreen = "/bin/true"
        with tempfile.TemporaryDirectory() as out:
            fixture = type("Fixture", (), {"cpus": [0, 1, 2, 3]})()
            loads = saturation.Loads(Options(), fixture, pathlib.Path(out), (None, None))
            spin = loads.applications("S2")[0]
            self.assertEqual(spin["exec"][:4], ["{offscreen}", "--load", "spin", "8"])
            hidden = loads.applications("S4")
            self.assertEqual(len(hidden), 6)
            self.assertTrue(all(app["minimized"] for app in hidden))
            self.assertEqual(loads.applications("S7"), [])
            json.dumps(loads.applications("S3"))


if __name__ == "__main__":
    unittest.main()
