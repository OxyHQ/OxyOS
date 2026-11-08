"""regress.py's own checks, on records written by hand: `python3 tools/rn-host-spike/test_regress.py`.

Each test gives `check_input_delivery` the development compositor's record of what it routed
(`deliveries`) and the hosts' reports of what they received, as a run writes them, so what a check
decides about an ordering is decided the same way every time, whatever the machine's load."""

import importlib.util
import pathlib
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

if __name__ == "__main__":
    unittest.main()
