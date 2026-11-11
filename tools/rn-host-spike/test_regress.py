"""regress.py's own checks, on records written by hand: `python3 tools/rn-host-spike/test_regress.py`.

Each test gives `check_input_delivery` the development compositor's record of what it routed
(`deliveries`) and the hosts' reports of what they received, as a run writes them, so what a check
decides about an ordering is decided the same way every time, whatever the machine's load."""

import importlib.util
import os
import pathlib
import secrets
import subprocess
import sys
import unittest

_spec = importlib.util.spec_from_file_location("regress", pathlib.Path(__file__).with_name("regress.py"))
regress = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(regress)

APP, CHOOSER = 3663832, 3664353


def summary(deliveries):
    """A run of one step that settled after everything routed."""
    return {"t_launch": 0, "steps": [{"t_settled": 10_000}], "deliveries": deliveries}


def received(pid, *events):
    """A host's report: each (when, surface, kind, serial) it received, in that order."""
    return [{"kind": "received", "t": t, "pid": pid, "surface": surface, "event": kind, "serial": serial,
             "seq": seq} for seq, (t, surface, kind, serial) in enumerate(events, 1)]


def check(deliveries, reports):
    failures = []
    regress.check_input_delivery(summary(deliveries), reports, failures)
    return failures


class HostsReceivedWhatWasRouted(unittest.TestCase):
    # A file chooser's sheet the application withdraws (file-chooser.json's step 13): the scene
    # dismisses it, so the pointer leaves it and the keyboard focus goes back to the window, and the
    # session ends the chooser's process (SIGTERM) in the same turn.
    WITHDRAWN = [
        [50, 1, APP, 1, "focus.in", 0],
        [51, 1, APP, 1, "pointer.enter", 64],
        [100, 1, APP, 1, "focus.out", 0],
        [101, 7, CHOOSER, 1, "focus.in", 0],
        [102, 1, APP, 1, "pointer.leave", 65],
        [103, 7, CHOOSER, 1, "pointer.enter", 66],
        [5000, 7, CHOOSER, 1, "pointer.leave", 67],
        [5003, 7, CHOOSER, 1, "focus.out", 0],
        [5004, 1, APP, 1, "focus.in", 0],
        [5005, 1, APP, 1, "pointer.enter", 68],
        [5200, 7, CHOOSER, 0, "disconnected", 0],
    ]
    APP_READ = received(APP, (60, 1, "focus.in", 0), (61, 1, "pointer.enter", 64), (110, 1, "focus.out", 0), (111, 1, "pointer.leave", 65),
                        (5010, 1, "focus.in", 0), (5011, 1, "pointer.enter", 68))

    def test_a_chooser_ended_while_reading_left_the_rest_unread(self):
        # It read the leave after the focus out was routed, and was ended before reading that: it
        # stopped reading and went, and nothing was lost (7 November 2026, at a load of 17).
        chooser = received(CHOOSER, (102, 1, "focus.in", 0), (104, 1, "pointer.enter", 66),
                           (5010, 1, "pointer.leave", 67))
        self.assertGreater(chooser[-1]["t"], 5003)
        self.assertEqual(check(self.WITHDRAWN, {"app": self.APP_READ, "chooser.5": chooser}), [])

    def test_a_chooser_ended_before_reading_its_sheets_end(self):
        chooser = received(CHOOSER, (102, 1, "focus.in", 0), (104, 1, "pointer.enter", 66))
        self.assertEqual(check(self.WITHDRAWN, {"app": self.APP_READ, "chooser.5": chooser}), [])

    def test_a_chooser_that_read_everything(self):
        chooser = received(CHOOSER, (102, 1, "focus.in", 0), (104, 1, "pointer.enter", 66),
                           (5001, 1, "pointer.leave", 67), (5004, 1, "focus.out", 0))
        self.assertEqual(check(self.WITHDRAWN, {"app": self.APP_READ, "chooser.5": chooser}), [])

    def test_an_event_missing_before_one_received_is_lost(self):
        # The leave never read, the focus out after it read: lost, though the host then went.
        chooser = received(CHOOSER, (102, 1, "focus.in", 0), (104, 1, "pointer.enter", 66),
                           (5004, 1, "focus.out", 0))
        self.assertEqual(check(self.WITHDRAWN, {"app": self.APP_READ, "chooser.5": chooser}),
                         [f"input delivery, step 1: the host of pid {CHOOSER} never received pointer.leave 67 "
                          "on surface 1 (1 routed)"])

    def test_what_follows_a_lost_event_is_not_left_unread(self):
        # The enter lost and nothing read after it: not a host that stopped reading at the end of
        # what it read, so the leave and the focus out are lost too, though it went.
        deliveries = list(self.WITHDRAWN)
        deliveries.insert(6, [200, 7, CHOOSER, 1, "pointer.motion", 70])
        chooser = received(CHOOSER, (102, 1, "focus.in", 0), (201, 1, "pointer.motion", 70))
        failures = check(deliveries, {"app": self.APP_READ, "chooser.5": chooser})
        self.assertEqual(sorted(failures), sorted(
            f"input delivery, step 1: the host of pid {CHOOSER} never received {kind} on surface 1 (1 routed)"
            for kind in ("pointer.enter 66", "pointer.leave 67", "focus.out 0")))

    def test_a_host_still_connected_loses_what_it_never_read(self):
        connected = [d for d in self.WITHDRAWN if d[4] != "disconnected"]
        chooser = received(CHOOSER, (102, 1, "focus.in", 0), (104, 1, "pointer.enter", 66),
                           (5010, 1, "pointer.leave", 67))
        self.assertEqual(check(connected, {"app": self.APP_READ, "chooser.5": chooser}),
                         [f"input delivery, step 1: the host of pid {CHOOSER} never received focus.out 0 "
                          "on surface 1 (1 routed)"])



def deliver(t, event, target, within=""):
    """A delivery to React as the host reports it: its target's name and, for a scroll or a touch,
    the nearest testID (`within`)."""
    return {"kind": "deliver", "t": t, "event": event, "target": target, "within": within}


def scroll_names(report):
    failures = []
    regress.check_scroll_names(report, 0, 100, failures.append)
    return failures


class MachineFacts(unittest.TestCase):
    """About This Computer's facts as a scenario expects them: this machine's, as the host reads them."""

    def test_a_text_names_the_machines_facts(self):
        self.assertEqual(regress.machine_text("{machine:memory}"), regress.machine_fact("memory"))
        self.assertRegex(regress.machine_fact("memory"), r"^\d+ (GB|MB)$")
        self.assertEqual(regress.machine_text("Name: {machine:name}."), f"Name: {regress.machine_fact('name')}.")
        self.assertEqual(regress.machine_text("plain"), "plain")
        self.assertTrue(regress.same_text(regress.machine_fact("os"), "{machine:os}", 0))

    def test_an_unknown_fact_stops_the_run(self):
        with self.assertRaises(SystemExit):
            regress.machine_text("{machine:serial}")


class ViewsAreNamedNotTagged(unittest.TestCase):
    # settings-input-sources.json's step 8: the Add sheet's scroll view has no testID, nor has the
    # modal it is in; the page that declares the modal has.
    SHEET = [deliver(10, "scroll", "ScrollView#704", "settings-page"),
             deliver(20, "scroll", "ScrollView#704", "settings-page"),
             deliver(30, "focus", "input-source-add-pinyin", "input-source-add-pinyin")]

    def test_an_unnamed_scroll_view_is_its_component_in_its_container(self):
        self.assertEqual(regress.react_events(self.SHEET, 0, 100),
                         {"scroll@ScrollView in settings-page": 2, "focus@input-source-add-pinyin": 1})
        self.assertEqual(scroll_names(self.SHEET), [])

    def test_it_is_not_its_named_container(self):
        # The page's own scroll view, named settings-page, is another view.
        report = self.SHEET + [deliver(40, "scroll", "settings-page", "settings-page")]
        self.assertEqual(regress.react_events(report, 0, 100)["scroll@settings-page"], 1)
        self.assertEqual(scroll_names(report), [])

    def test_its_name_does_not_follow_its_tag(self):
        # A sidebar row mounted before it moves every later tag by the views it adds.
        moved = [dict(line, target=line["target"].replace("#704", "#716")) for line in self.SHEET]
        self.assertEqual(regress.react_events(moved, 0, 100), regress.react_events(self.SHEET, 0, 100))

    def test_two_scroll_views_under_one_name_fail(self):
        report = self.SHEET + [deliver(40, "scroll", "ScrollView#710", "settings-page")]
        self.assertEqual(scroll_names(report),
                         ["2 scroll views scrolled as ScrollView in settings-page (ScrollView#704, "
                          "ScrollView#710): a testID of their own tells them apart"])

    def test_one_name_may_stand_for_a_controls_texts(self):
        # A touch's target is often its text: one control, whichever of its views was touched.
        report = [deliver(10, "touchStart", "Text#40", "input-sources-add"),
                  deliver(20, "touchEnd", "RCTView#38", "input-sources-add")]
        self.assertEqual(scroll_names(report), [])
        self.assertEqual(regress.react_events(report, 0, 100),
                         {"touchStart@input-sources-add": 1, "touchEnd@input-sources-add": 1})

    def test_a_named_scroll_view_or_a_line_without_within_is_its_target(self):
        report = [deliver(10, "scroll", "list", "list"), deliver(20, "scroll", "ScrollView#3")]
        del report[1]["within"]
        self.assertEqual(regress.react_events(report, 0, 100), {"scroll@list": 1, "scroll@ScrollView#3": 1})


class AStallsMotionsAreOneTurns(unittest.TestCase):
    """`react`'s {"turns": count} (desktop-drag.json): a press's motions the loop read while
    JavaScript was busy (behind a stall) reach it together, in one turn of its; the drag it starts on
    the first takes the press, so no turn after delivers another."""

    def check(self, turns, wanted):
        report = [dict(deliver(10, "touchStart", "View#406", "desktop"), turn=1)]
        report += [dict(deliver(20 + i, "touchMove", "View#406", "desktop"), turn=turn) for i, turn in enumerate(turns)]
        report.append(dict(deliver(90, "touchCancel", "View#406", "desktop"), turn=9))
        step = {"name": "drag", "do": ["wait", 0], "expect": {"react": {
            "touchStart@desktop": 1, "touchMove@desktop": wanted, "touchCancel@desktop": 1}}}
        failures = []
        regress.check_step(0, step, {"t_settled": 100}, report, 0, failures)
        return failures

    def test_four_motions_read_at_once_are_one_turn(self):
        self.assertEqual(self.check([3, 3, 3, 3], {"turns": [1, 2]}), [])
        self.assertEqual(regress.react_events([dict(deliver(20, "touchMove", "V", "desktop"), turn=3)] * 4, 0, 100,
                                              turns=True), {"touchMove@desktop": 1})

    def test_a_motion_in_a_turn_after_the_drag_began_fails(self):
        failures = self.check([3, 3, 5, 7], {"turns": [1, 2]})
        self.assertEqual(len(failures), 1)
        self.assertIn("'touchMove@desktop': {'turns': 3, 'events': 4}", failures[0])

    def test_a_plain_count_still_counts_every_delivery(self):
        self.assertEqual(len(self.check([3, 3, 3], [1, 2])), 1)


class ActivityIsCountedPerStep(unittest.TestCase):
    """`activity` and `processes` (docs/adr/0063-activity-monitor.mdx): counts in the step, from the
    step before's settling, and the signals a known process received between them."""

    def results(self, before, now):
        return [{"t_settled": 100, "activity": before}, {"t_settled": 200, "activity": now}]

    def check(self, expect, before, now, processes=None):
        failures = []
        step = {"name": "hidden", "do": ["wait", 1], "expect": expect}
        results = self.results(before, now)
        regress.check_activity(1, step, results[1], results, processes or regress.ProcessRecorder(None, []),
                               failures)
        return failures

    def test_a_hidden_step_holds_its_counts_to_none(self):
        held = {"samples": 4, "sent": 4, "wakeups": 9, "host_wakeups": 30, "followed": True}
        self.assertEqual(self.check({"activity": {"samples": 0, "wakeups": 0, "followed": True}}, held, held), [])

    def test_a_count_in_the_step_is_the_difference(self):
        before = {"samples": 4, "sent": 4, "wakeups": 9, "host_wakeups": 30, "followed": True}
        now = {"samples": 5, "sent": 5, "wakeups": 11, "host_wakeups": 33, "followed": True}
        failures = self.check({"activity": {"samples": 0, "host_wakeups": [0, 5]}}, before, now)
        self.assertEqual(failures, ["step 2 (hidden): activity samples 1 in the step, expected 0"])

    def test_signals_are_those_of_the_step(self):
        directory = pathlib.Path(regress.tempfile.mkdtemp())
        record = directory / "idle.signals"
        record.write_text("50 started 7\n90 TERM\n150 TERM\n")

        class Known:
            processes = {"idle": (None, record)}
            ended = {"idle": 160}
            signals = regress.ProcessRecorder.signals
            running_at = regress.ProcessRecorder.running_at

        expect = {"processes": {"idle": {"signals": ["TERM"], "running": False}}}
        self.assertEqual(self.check(expect, {}, {}, Known()), [])
        expect = {"processes": {"idle": {"signals": [], "running": True}}}
        self.assertEqual(len(self.check(expect, {}, {}, Known())), 2)


class DeliveredCounts(unittest.TestCase):
    # What the compositor delivered is the step's exactly; a count may be a [least, most] (a pinch
    # through libinput, as many updates as it recognized frames of the fingers).
    def test_exact_counts_and_ranges(self):
        got = {"1": {"pinch.begin": 1, "pinch.update": 22, "pinch.end": 1}}
        self.assertTrue(regress.counts_match(got, {"1": {"pinch.begin": 1, "pinch.update": [10, 30], "pinch.end": 1}}))
        self.assertFalse(regress.counts_match(got, {"1": {"pinch.begin": 1, "pinch.update": [23, 30], "pinch.end": 1}}))
        self.assertFalse(regress.counts_match(got, {"1": {"pinch.begin": 1, "pinch.update": 21, "pinch.end": 1}}))
        # An event more, or a window more, is not what the step says.
        self.assertFalse(regress.counts_match(got, {"1": {"pinch.begin": 1, "pinch.update": [10, 30]}}))
        self.assertFalse(regress.counts_match({**got, "2": {"focus.in": 1}}, {"1": got["1"]}))
        self.assertTrue(regress.counts_match({}, {}))

    def test_an_event_given_none_at_least_may_not_have_come(self):
        # desktop-drag.json's Dropsea dragging out: its drag began over its window, or (late, on a
        # loaded machine) once the pointer had left it, never entering it.
        wanted = {"1": {"press": 1, "drag.enter": [0, 1], "drag.leave": [0, 1]}}
        self.assertTrue(regress.counts_match({"1": {"press": 1}}, wanted))
        self.assertTrue(regress.counts_match({"1": {"press": 1, "drag.enter": 1, "drag.leave": 1}}, wanted))
        self.assertFalse(regress.counts_match({"1": {"press": 1, "drag.enter": 2}}, wanted))
        self.assertFalse(regress.counts_match({"1": {"drag.enter": 1}}, wanted))


class SpokenExactly(unittest.TestCase):
    # What the reader said is the step's exactly: in order, or with `spoken_any_order` in either
    # (two processes speaking for one input), each utterance once.
    SAID = [{"t": 1, "text": "EE. UU.", "interrupting": False}, {"t": 2, "text": "ni", "interrupting": True}]

    def test_in_order(self):
        self.assertTrue(regress.spoken_matches(self.SAID, ["EE. UU.", {"text": "ni", "interrupting": True}]))
        self.assertFalse(regress.spoken_matches(self.SAID, ["ni", "EE. UU."]))

    def test_in_any_order_each_once(self):
        wanted = [{"text": "ni", "interrupting": True}, {"text": "EE. UU.", "interrupting": False}]
        self.assertTrue(regress.spoken_matches(self.SAID, wanted, any_order=True))
        self.assertFalse(regress.spoken_matches(self.SAID, ["ni", "ni"], any_order=True))
        self.assertFalse(regress.spoken_matches(self.SAID + self.SAID[:1], wanted, any_order=True))
        self.assertFalse(regress.spoken_matches(self.SAID, [{"text": "ni", "interrupting": False}, "EE. UU."],
                                                any_order=True))


class CrashReportsAreTheHomesOldestFirst(unittest.TestCase):
    """docs/adr/0061: the reports in the run's home, each its application's, oldest first."""

    STATE = {
        ".local/state/oxyos/crash-reports": None,
        ".local/state/oxyos/crash-reports/so.oxy.crash_probe_2026-11-08-101200-2.crash": "second at 10:12\nboom",
        ".local/state/oxyos/crash-reports/so.oxy.crash_probe_2026-11-08-101200.crash": "first at 10:12",
        ".local/state/oxyos/crash-reports/so.oxy.allo_2026-11-08-091000.crash": "Allo, at nine",
        ".local/state/oxyos/elsewhere.crash": "not a report",
    }

    def check(self, wanted):
        failures = []
        regress.check_crash_reports(self.STATE, wanted, failures.append)
        return failures

    def test_each_application_in_order_with_what_it_says(self):
        self.assertEqual(self.check([
            {"application": "so.oxy.allo"},
            {"application": "so.oxy.crash_probe", "contains": ["first"]},
            {"application": "so.oxy.crash_probe", "contains": ["second", "boom"]},
        ]), [])

    def test_another_order_count_or_text_fails(self):
        self.assertTrue(self.check([{"application": "so.oxy.allo"}]))
        self.assertTrue(self.check([
            {"application": "so.oxy.crash_probe"}, {"application": "so.oxy.crash_probe"}, {"application": "so.oxy.allo"},
        ]))
        self.assertTrue(self.check([
            {"application": "so.oxy.allo", "contains": ["Mention"]},
            {"application": "so.oxy.crash_probe"}, {"application": "so.oxy.crash_probe"},
        ]))

# A twin: a run that starts `sleep MARKER`, says "ready" once the kernel shows that command, and
# on "count" says how many processes its /proc shows with exactly that command, then on "end"
# ends its sleep. The test tells both twins to count only once both are ready, and to end only
# once both have counted, so each counts while both sleeps run, however loaded the machine; and
# MARKER is this test's own, so a test run beside it (another checkout's, another agent's) never
# shows a sleep of the same command.
TWIN = """
import os, subprocess, sys
marker = sys.argv[1]
command = b"sleep\\0" + marker.encode() + b"\\0"
twin = subprocess.Popen(["sleep", marker])
# The kernel names a process before it lays out its arguments: ready once they are there.
while True:
    if twin.poll() is not None:
        sys.exit(f"sleep {marker} ended at once")
    try:
        if open(f"/proc/{twin.pid}/cmdline", "rb").read() == command:
            break
    except OSError:
        pass
print("ready", flush=True)
sys.stdin.readline()
seen = 0
for entry in os.listdir("/proc"):
    try:
        seen += open(f"/proc/{entry}/cmdline", "rb").read() == command
    except OSError:
        pass
print(seen, flush=True)
sys.stdin.readline()
twin.kill()
twin.wait()
"""


class ARunSeesOnlyItsOwnProcesses(unittest.TestCase):
    """A scenario whose session lists the machine's processes (Activity Monitor's) runs in a PID
    namespace of its own: two at once, one user's, never see each other's processes."""

    @classmethod
    def setUpClass(cls):
        refused = regress.pid_namespace_refused()
        if refused:
            raise unittest.SkipTest(f"this machine gives no PID namespace of a run's own: {refused}")

    def twins(self, isolated):
        """What each of two twins at once counted of their two sleeps."""
        # Seconds sleep takes, unique to this test (a day, and this process's id and a nonce as
        # its fraction).
        marker = f"86400.{os.getpid()}{secrets.randbelow(10**9):09d}"
        command = [sys.executable, "-c", TWIN, marker]
        runs = [subprocess.Popen(regress.pid_namespace_command(command) if isolated else command,
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) for _ in range(2)]
        try:
            for run in runs:
                self.assertEqual(run.stdout.readline(), "ready\n")
            for run in runs:
                run.stdin.write("count\n")
                run.stdin.flush()
            counts = [int(run.stdout.readline()) for run in runs]
            for run in runs:
                run.stdin.write("end\n")
                run.stdin.flush()
            for run in runs:
                self.assertEqual(run.wait(timeout=30), 0)
            return counts
        finally:
            for run in runs:
                if run.poll() is None:
                    run.kill()
                    run.wait()

    def test_two_runs_at_once_each_see_only_their_own(self):
        self.assertEqual(self.twins(isolated=False), [2, 2], "without namespaces each sees both")
        self.assertEqual(self.twins(isolated=True), [1, 1])

    def test_which_scenarios_list_processes(self):
        self.assertTrue(regress.lists_processes({"processes": [{"name": "oxyos-test-idle"}]}))
        self.assertTrue(regress.lists_processes({"grants": {"so.oxy.activitymonitor": ["activity"]}}))
        self.assertFalse(regress.lists_processes({"grants": {"so.oxy.settings": ["settings"]}}))

    def test_the_init_reaps_orphans_and_ends_with_its_child(self):
        # An orphan left to the init, then the child's status 3: no zombie listed, 3 returned.
        orphan = ("import os, subprocess, sys, time\n"
                  "subprocess.Popen(['sh', '-c', 'sleep 0.2 & exit 0']).wait()\n"
                  "time.sleep(1)\n"
                  "zombies = [e for e in os.listdir('/proc') if e.isdigit() and"
                  " open(f'/proc/{e}/stat').read().rsplit(')', 1)[1].split()[0] == 'Z']\n"
                  "print(len(zombies)); sys.exit(3)")
        run = subprocess.run(regress.pid_namespace_command([sys.executable, "-c", orphan]),
                             capture_output=True, text=True, timeout=30)
        self.assertEqual((run.returncode, run.stdout.strip()), (3, "0"), run.stderr)

class LaunchToContent(unittest.TestCase):
    """`content_ms_max`: from the host's `main` to the first frame showing every one of the step's
    texts, across its surfaces, a closed popup's left out."""

    MS = 1_000_000

    def report(self, *frames):
        lines = [{"t": 5 * self.MS, "kind": "stage", "name": "main"}]
        for at, surface, texts in frames:
            lines.append({"t": (5 + at) * self.MS, "kind": "frame", "surface": surface, "texts": texts})
        return lines

    def check(self, report, budget=1500, texts=("notas.txt", "3 elementos")):
        failures = []
        step = {"name": "launch", "do": ["wait", 2500], "expect": {"texts": list(texts), "content_ms_max": budget}}
        result = {"t_settled": max(line["t"] for line in report)}
        regress.check_step(0, step, result, report, 0, failures)
        return result.get("content_ms"), failures

    def test_its_listing_within_the_budget(self):
        shown, failures = self.check(self.report((40, 1, []), (284, 1, ["Inicio", "notas.txt", "3 elementos"])))
        self.assertEqual((shown, failures), (284.0, []))

    def test_every_text_counts_on_any_surface(self):
        shown, failures = self.check(self.report((100, 1, ["notas.txt"]), (200, 1, ["notas.txt"]),
                                                 (300, 2, ["3 elementos"])))
        self.assertEqual((shown, failures), (300.0, []))

    def test_a_late_listing_fails(self):
        shown, failures = self.check(self.report((40, 1, []), (5200, 1, ["notas.txt", "3 elementos"])))
        self.assertEqual(shown, 5200.0)
        self.assertIn("over the 1500 ms budget", failures[0])

    def test_a_blank_window_fails(self):
        shown, failures = self.check(self.report((40, 1, []), (2600, 1, [])))
        self.assertIsNone(shown)
        self.assertTrue(any("never all shown" in failure for failure in failures))

    def test_a_closed_popups_text_is_not_shown(self):
        report = self.report((100, 2, ["3 elementos"]), (150, 1, []))
        report.append({"t": 200 * self.MS, "kind": "popup", "event": "close", "surface": 2})
        report += self.report((300, 1, ["notas.txt"]))[1:]
        self.assertIsNone(regress.content_shown_ms(report, ["notas.txt", "3 elementos"]))

    def test_under_collect_it_is_measured_not_held(self):
        regress.COLLECT = True
        try:
            shown, failures = self.check(self.report((5200, 1, ["notas.txt", "3 elementos"])))
        finally:
            regress.COLLECT = False
        self.assertEqual((shown, failures), (5200.0, []))

    def test_a_budget_with_no_texts_is_refused(self):
        with self.assertRaises(ValueError):
            self.check(self.report((40, 1, ["notas.txt"])), texts=())




class GlidesOnlySlow(unittest.TestCase):
    """`glide`'s check that a fling only slows (regress.glide_speeding), on frames written by hand."""

    @staticmethod
    def frames(*points):
        return [{"t": int(ms * 1e6), "y": y} for ms, y in points]

    def test_a_spline_sampled_unevenly_only_slows(self):
        # Android's fling slows; frames late or early change each frame's step, not its trend.
        self.assertEqual(regress.glide_speeding(self.frames((0, 0), (16, 40), (40, 95), (48, 112), (80, 170), (96, 195))), [])

    def test_rounding_to_pixels_near_its_end_is_no_speeding(self):
        # At 1.25 offsets fall on 0.8 px: a slow end alternates steps of 0.8 and 1.6.
        self.assertEqual(regress.glide_speeding(self.frames((0, 100), (16, 101.6), (32, 102.4), (48, 104.0), (64, 104.8))), [])

    def test_a_glide_begun_again_fails(self):
        self.assertEqual(regress.glide_speeding(self.frames((0, 0), (16, 40), (32, 70), (48, 92), (64, 140))),
                         [(3, 1375, 3000)])

    def test_upwards_too(self):
        self.assertEqual(regress.glide_speeding(self.frames((0, 500), (16, 460), (32, 430), (48, 380))),
                         [(2, -1875, -3125)])


class GlidesMeetInput(unittest.TestCase):
    """`glide`'s check of a fling (regress.check_glide) and of the input that meets it, on reports
    written by hand: input on the glide's surface stops it where it is, input on another surface of
    the application (a popup's) leaves it gliding to its end."""

    VELOCITY, FRICTION, FRAME = 1200.0, 0.015, 16

    def glide(self, axis="y", stop_at=None):
        """A report of a drag's end and its glide along `axis`, decelerating as a spline does, stopped
        at frame `stop_at` if given: its lines, and the offset and time where its momentum ended."""
        distance = regress.android_fling_distance(self.VELOCITY, self.FRICTION)
        frames = 30
        other = "x" if axis == "y" else "y"

        def line(ms, event, offset):
            return {"kind": "deliver", "t": int(ms * 1e6), "event": event, "target": "list", "surface": 1,
                    axis: offset, other: 0}

        fling = {"kind": "fling", "t": int(0.5e6), "target": "list", "friction": self.FRICTION, "v" + axis: self.VELOCITY,
                 "d" + axis: distance, "v" + other: 0.0, "d" + other: 0.0}
        lines = [line(0, "scrollEndDrag", 100), line(0.2, "momentumScrollBegin", 100), fling]
        offset = 100
        last = frames if stop_at is None else stop_at
        for i in range(1, last + 1):
            share = i / frames
            offset = round(100 + distance * (1 - (1 - share) ** 2), 2)
            lines.append(line(i * self.FRAME, "scroll", offset))
        ended = last * self.FRAME + (5 if stop_at is not None else 0.1)
        lines.append(line(ended, "momentumScrollEnd", offset))
        return lines, offset, ended, line

    def check(self, wanted, report):
        failures = []
        regress.check_glide({"view": "list", "velocity": [1000, 1400], **wanted}, report, 0, int(10e9),
                            failures.append)
        return failures

    @staticmethod
    def scroll_input(ms, surface, source, dx=0, dy=0, stop=False):
        return {"kind": "input", "t": int(ms * 1e6), "event": "scroll", "surface": surface, "source": source,
                "dx": dx, "dy": dy, "stop": stop}

    def test_a_glide_to_its_distance_down_or_across(self):
        for axis in ("y", "x"):
            lines, _, _, _ = self.glide(axis)
            self.assertEqual(self.check({"axis": axis}, lines), [], axis)
        # Across, read as down, glides nowhere.
        lines, _, _, _ = self.glide("x")
        self.assertTrue(self.check({}, lines))

    def test_a_wheels_notch_on_its_surface_stops_it_then_scrolls(self):
        lines, offset, ended, line = self.glide(stop_at=10)
        notch = self.scroll_input(ended - 0.5, 1, "wheel", dy=40)
        self.assertEqual(self.check({"stopped_by": "wheel"}, lines + [notch, line(ended + 0.2, "scroll", offset + 40)]), [])
        # The notch's scroll missing, or by another distance, fails; so does a glide scroll after it.
        self.assertTrue(self.check({"stopped_by": "wheel"}, lines + [notch]))
        self.assertTrue(self.check({"stopped_by": "wheel"}, lines + [notch, line(ended + 0.2, "scroll", offset + 15)]))
        # A notch on another surface does not stop it here.
        elsewhere = dict(notch, surface=2)
        self.assertIn("the wheel's notch came on surface 2, not the glide's (1)",
                      self.check({"stopped_by": "wheel"}, lines + [elsewhere, line(ended + 0.2, "scroll", offset + 40)]))

    def test_a_glide_the_notch_did_not_stop_fails(self):
        lines, _, ended, _ = self.glide()
        notch = self.scroll_input(200, 1, "wheel", dy=40)
        failures = self.check({"stopped_by": "wheel"}, lines + [notch])
        self.assertTrue(any("did not stop it" in failure for failure in failures), failures)

    def test_new_fingers_stop_it_and_drag_it_from_there(self):
        lines, offset, ended, line = self.glide(stop_at=10)
        fingers = [self.scroll_input(ended - 0.5 + 16 * i, 1, "finger", dy=10) for i in range(3)]
        lifted = self.scroll_input(ended + 150, 1, "finger", stop=True)
        drag = [line(ended + 0.2, "scrollBeginDrag", offset)] + \
               [line(ended + 0.3 + 16 * i, "scroll", offset + 10 * (i + 1)) for i in range(3)] + \
               [line(ended + 150, "scrollEndDrag", offset + 30), line(ended + 150.1, "momentumScrollBegin", offset + 30),
                line(ended + 166, "momentumScrollEnd", offset + 30)]
        rested = {"kind": "fling", "t": int((ended + 150.2) * 1e6), "target": "list", "friction": self.FRICTION,
                  "vx": 0.0, "vy": 0.0, "dx": 0.0, "dy": 0.0}
        self.assertEqual(self.check({"stopped_by": "fingers"}, lines + fingers + [lifted] + drag + [rested]), [])
        # Their lift flinging it again fails.
        flung = dict(rested, vy=300.0, dy=40.0)
        self.assertTrue(self.check({"stopped_by": "fingers"}, lines + fingers + [lifted] + drag + [flung]))
        # A drag going the other way fails.
        back = drag[:1] + [dict(event, y=offset - 10 * (i + 1)) for i, event in enumerate(drag[1:4])] + drag[4:]
        self.assertTrue(self.check({"stopped_by": "fingers"}, lines + fingers + [lifted] + back + [rested]))

    def test_resting_fingers_and_a_click_as_before(self):
        lines, _, ended, _ = self.glide(stop_at=10)
        hold = {"kind": "input", "t": int((ended - 0.5) * 1e6), "event": "hold.begin", "surface": 1}
        press = dict(hold, event="pointer.press")
        self.assertEqual(self.check({"held": True}, lines + [hold]), [])
        self.assertEqual(self.check({"pressed": True}, lines + [press]), [])
        self.assertTrue(self.check({"held": True}, lines + [press]))

    def test_input_on_another_surface_leaves_it_gliding_to_its_end(self):
        lines, _, _, _ = self.glide()
        for kind, inputs in (("wheel", [self.scroll_input(200, 2, "wheel", dy=40)]),
                             ("fingers", [self.scroll_input(200 + 16 * i, 2, "finger", dy=10) for i in range(3)]),
                             ("press", [{"kind": "input", "t": int(200e6), "event": "pointer.press", "surface": 2}])):
            self.assertEqual(self.check({"kept_by": kind}, lines + inputs), [], kind)
        # On its own surface, or after its end, it is no such input.
        self.assertTrue(self.check({"kept_by": "wheel"}, lines + [self.scroll_input(200, 1, "wheel", dy=40)]))
        self.assertTrue(self.check({"kept_by": "wheel"}, lines + [self.scroll_input(900, 2, "wheel", dy=40)]))
        # Stopped by it all the same fails.
        cut, _, ended, _ = self.glide(stop_at=10)
        self.assertTrue(self.check({"kept_by": "wheel"}, cut + [self.scroll_input(ended - 0.5, 2, "wheel", dy=40)]))

    def test_anything_after_its_momentum_ended_fails(self):
        lines, offset, ended, line = self.glide()
        self.assertTrue(self.check({}, lines + [line(ended + 20, "scroll", offset + 5)]))


class JavaScriptHeldWhileSomethingMoves(unittest.TestCase):
    """A step's `stall` (docs/adr/0067-the-ui-thread.mdx): what moves is drawn through a block of
    JavaScript's, every run half the block's refreshes, with --speed-budgets each refresh."""

    @staticmethod
    def report(frames_ms, block=(1000.0, 1500.0), motion="animated", target="native"):
        lines = [{"kind": "frame", "surface": 1, "presented": True, "t": int(t * 1e6)} for t in frames_ms]
        lines += [{"kind": motion, "target": target, "t": int(t * 1e6)} for t in frames_ms]
        since, until = block
        lines.append({"kind": "js_timing", "t": int(until * 1e6), "work_ms": until - since, "trigger": "a timer"})
        return sorted(lines, key=lambda line: line["t"])

    def check(self, report, motion="animated:native", strict=False):
        failures = []
        held = regress.SPEED_BUDGETS
        regress.SPEED_BUDGETS = strict
        try:
            regress.check_stall({"block_ms_min": 450, "motion": motion}, report, 0, int(10e9), failures.append, 60.0)
        finally:
            regress.SPEED_BUDGETS = held
        return failures

    def test_a_frame_each_refresh_through_the_block_holds(self):
        frames = [900 + i * 1000 / 60 for i in range(48)]
        self.assertEqual(self.check(self.report(frames)), [])
        self.assertEqual(self.check(self.report(frames), strict=True), [])

    def test_frames_held_up_with_javascript_fail(self):
        frames = [900 + i * 1000 / 60 for i in range(7)] + [1505 + i * 1000 / 60 for i in range(10)]
        self.assertTrue(self.check(self.report(frames)))

    def test_a_gap_of_three_refreshes_fails_only_the_budget(self):
        frames = [t for t in (900 + i * 1000 / 60 for i in range(48)) if not 1200 < t < 1250]
        self.assertEqual(self.check(self.report(frames)), [])
        self.assertTrue(self.check(self.report(frames), strict=True))

    def test_frames_without_the_motion_fail(self):
        frames = [900 + i * 1000 / 60 for i in range(48)]
        self.assertTrue(self.check(self.report(frames, target="other")))
        self.assertEqual(self.check(self.report(frames), motion="frames"), [])

    def test_no_block_fails(self):
        frames = [900 + i * 1000 / 60 for i in range(48)]
        self.assertTrue(self.check(self.report(frames, block=(1000.0, 1200.0))))


class TypedTextEchoedAsTheFieldTookIt(unittest.TestCase):
    """`spoken`'s "typed": text typed at once is echoed whole or in pieces, as the field took each key
    (a field takes a key once JavaScript answered it, docs/adr/0067-the-ui-thread.mdx)."""

    wanted = [{"typed": "ada", "interrupting": True}, {"text": "Ese nombre ya está en uso", "interrupting": True}]

    @staticmethod
    def said(*texts, interrupting=True):
        return [{"text": text, "interrupting": interrupting, "t": 0} for text in texts]

    def test_whole_or_in_pieces(self):
        for pieces in (["ada"], ["a", "d", "a"], ["ad", "a"], ["a", "da"]):
            self.assertTrue(regress.spoken_as_wanted(self.said(*pieces, "Ese nombre ya está en uso"), self.wanted))

    def test_pieces_that_do_not_join_to_it(self):
        for pieces in (["a", "d"], ["a", "d", "a", "a"], ["ad", "d"]):
            self.assertFalse(regress.spoken_as_wanted(self.said(*pieces, "Ese nombre ya está en uso"), self.wanted))

    def test_each_piece_matches_the_other_keys(self):
        said = self.said("a") + self.said("d", interrupting=False) + self.said("a", "Ese nombre ya está en uso")
        self.assertFalse(regress.spoken_as_wanted(said, self.wanted))


class EachThreadsDiagnostics(unittest.TestCase):
    """check_diagnostics with JavaScript on its own thread: each frame counts its stretches."""

    @staticmethod
    def stretch(t, work, cpu=None):
        cpu = work if cpu is None else cpu
        return {"kind": "js_timing", "t": t, "work_ms": work, "cpu_ms": cpu, "blocked_ms": work - cpu,
                "preempted_ms": 0.0, "slow": work > 16.7, "refresh_ms": 16.7, "frames": 0, "dropped": 0,
                "trigger": "a timer", "tasks": 1}

    @staticmethod
    def frame(t, js_work, stretches):
        part = {"other": 1.0}
        return {"kind": "timing", "t": t, "presented": True, "work_ms": 1.0, "cpu_ms": 1.0, "blocked_ms": 0.0,
                "preempted_ms": 0.0, "wait_ms": 0.0, "late_ms": 1.0, "refresh_ms": 16.7, "slow": False,
                "dropped": 0, "latency_ms": None, "parts": part, "cpu": part, "blocked": {"other": 0.0},
                "preempted": {"other": 0.0},
                "js": {"work_ms": js_work, "cpu_ms": js_work, "blocked_ms": 0.0, "preempted_ms": 0.0,
                       "stretches": stretches}}

    def check(self, report):
        failures = []
        regress.check_diagnostics(report, "", failures)
        return failures

    def test_a_frame_counts_the_stretches_since_the_frame_before(self):
        report = [self.stretch(1, 2.0), self.stretch(2, 3.0), self.frame(3, 5.0, 2), self.frame(4, 0.0, 0)]
        self.assertEqual(self.check(report), [])

    def test_a_stretch_counted_twice_or_not_at_all_fails(self):
        self.assertTrue(self.check([self.stretch(1, 2.0), self.frame(2, 2.0, 1), self.frame(3, 2.0, 1)]))
        self.assertTrue(self.check([self.stretch(1, 2.0), self.frame(2, 0.0, 0)]))

    def test_a_stretch_said_slow_exactly_past_a_refresh(self):
        line = self.stretch(1, 20.0)
        self.assertEqual(self.check([line]), [])
        self.assertTrue(self.check([{**line, "slow": False}]))

    def test_the_summary_counts_the_stretches_since_the_one_before(self):
        report = [self.stretch(1, 2.0), self.stretch(2, 30.0), self.frame(3, 32.0, 2)]
        summary = {"kind": "timing_summary", "t": 4, "frames": 1, "work_p50": 1.0, "work_p95": 1.0, "work_max": 1.0,
                   "latency_p50": None, "latency_p95": None, "inputs": 0, "slow": 0, "dropped": 0,
                   "js_stretches": 2, "js_slow": 1, "js_ms": 32.0, "js_longest_ms": 30.0}
        self.assertEqual(self.check(report + [summary]), [])
        self.assertTrue(self.check(report + [{**summary, "js_stretches": 1}]))


class SignalSteps(unittest.TestCase):
    def test_a_part_is_sent_a_signal_that_ends_it(self):
        self.assertEqual(regress.director_args({"do": ["signal", ["SEGV", "reader"]]}, None),
                         ["--signal", "SEGV", "reader"])
        self.assertIn("signal", regress.STEP_KEYS)

    def test_only_signals_that_end_a_part_and_only_parts_the_session_keeps(self):
        for value in (["STOP", "reader"], ["SEGV", "dock"], ["HUP", "reader"], ["HUP", "shell"]):
            with self.assertRaises(ValueError):
                regress.director_args({"do": ["signal", value]}, None)

    def test_the_shell_is_ended_or_hung(self):
        for signal in ("SEGV", "KILL", "STOP"):
            self.assertEqual(regress.director_args({"do": ["signal", [signal, "shell"]]}, None),
                             ["--signal", signal, "shell"])


if __name__ == "__main__":
    unittest.main()
