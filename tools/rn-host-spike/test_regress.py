"""regress.py's own checks, on records written by hand: `python3 tools/rn-host-spike/test_regress.py`.

Each test gives `check_input_delivery` the development compositor's record of what it routed
(`deliveries`) and the hosts' reports of what they received, as a run writes them, so what a check
decides about an ordering is decided the same way every time, whatever the machine's load."""

import importlib.util
import pathlib
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

# A run's processes as its /proc shows them: those whose command is `sleep 7.31` (a twin's).
TWIN = """
import os, subprocess, time
twin = subprocess.Popen(["sleep", "7.31"])
time.sleep(1.5)
seen = 0
for entry in os.listdir("/proc"):
    try:
        seen += open(f"/proc/{entry}/cmdline", "rb").read() == b"sleep\\x007.31\\x00"
    except OSError:
        pass
twin.kill()
print(seen)
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
        command = [sys.executable, "-c", TWIN]
        runs = [subprocess.Popen(regress.pid_namespace_command(command) if isolated else command,
                                 stdout=subprocess.PIPE, text=True) for _ in range(2)]
        return [int(run.communicate(timeout=30)[0]) for run in runs]

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


if __name__ == "__main__":
    unittest.main()


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
