"""The gate watchdog's choice of superseded merge queue runs: `python3 tools/gate/test_watchdog.py`.

On recorded answers of GitHub (testdata/): the merge queue's runs and the queue as they were on
10 November 2026 at 13:40, when a queue rebuilt at 13:32 had left the four Scenarios runs of its
12:37 groups holding four of the five merge runners (the
first group's, whose failure at 13:31 rebuilt the queue, had ended) (tools/gate/watchdog.py, docs/developing.mdx)."""

import datetime
import json
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import watchdog  # noqa: E402

RUNS = json.loads((HERE / "testdata/merge-group-runs.json").read_text())["workflow_runs"]
QUEUE = json.loads((HERE / "testdata/merge-queue.json").read_text())
NOW = datetime.datetime(2026, 11, 10, 13, 40, tzinfo=datetime.timezone.utc)
# The 12:37 groups' Scenarios runs, which no entry waited for after 13:32.
STALE = {42817364461, 42817362036, 42817360795, 42817357622}


def ids(runs):
    return {run["id"] for run in runs}


class Superseded(unittest.TestCase):
    def setUp(self):
        self.waited = watchdog.waited_commits(QUEUE)

    def test_the_rebuilt_queues_old_runs_and_only_they(self):
        self.assertEqual(ids(watchdog.superseded(RUNS, self.waited, NOW)), STALE)

    def test_never_a_live_groups_run(self):
        live = [run for run in RUNS if run["head_sha"] in self.waited]
        self.assertTrue(live)
        self.assertFalse(ids(watchdog.superseded(RUNS, self.waited, NOW)) & ids(live))

    def test_never_the_run_asking(self):
        own = min(STALE)
        self.assertNotIn(own, ids(watchdog.superseded(RUNS, self.waited, NOW, own=str(own))))
        self.assertEqual(ids(watchdog.superseded(RUNS, self.waited, NOW, own=str(own))), STALE - {own})

    def test_never_another_workflow_or_an_ended_run(self):
        taken = watchdog.superseded(RUNS, set(), NOW)
        self.assertTrue(taken)
        self.assertTrue(all(run["name"] == "Scenarios" and run["status"] != "completed" for run in taken))

    def test_never_a_run_younger_than_the_grace(self):
        early = datetime.datetime(2026, 11, 10, 13, 33, tzinfo=datetime.timezone.utc)
        young = [run for run in RUNS if run["created_at"] >= "2026-11-10T13:32"]
        self.assertEqual(watchdog.superseded(young, set(), early), [])

    def test_an_unknown_queue_takes_nothing(self):
        self.assertEqual(watchdog.superseded(RUNS, None, NOW), [])


class QueueAnswer(unittest.TestCase):
    def test_the_entries_commits_without_the_unbuilt(self):
        waited = watchdog.waited_commits(QUEUE)
        self.assertEqual(len(waited), 5)
        self.assertIn("30d99f4619eeccda1b3c81081f90982ced211103", waited)

    def test_an_empty_queue_is_known_empty(self):
        answer = {"data": {"repository": {"mergeQueue": {"entries": {"nodes": []}}}}}
        self.assertEqual(watchdog.waited_commits(answer), set())

    def test_an_answer_that_does_not_say_is_unknown(self):
        for answer in ({"data": {"repository": {"mergeQueue": None}}},
                       {"data": None, "errors": [{"message": "Resource not accessible by integration"}]},
                       {"data": {"repository": None}},
                       {"data": {"repository": {"mergeQueue": {"entries": {"nodes": []}}}},
                        "errors": [{"message": "partial"}]},
                       {}):
            self.assertIsNone(watchdog.waited_commits(answer), answer)


if __name__ == "__main__":
    unittest.main()
