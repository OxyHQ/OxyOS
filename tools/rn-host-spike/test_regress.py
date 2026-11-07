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


if __name__ == "__main__":
    unittest.main()
