"""The gate watchdog's choice of superseded merge queue runs: `python3 tools/gate/test_watchdog.py`.

On recorded answers of GitHub (testdata/): the merge queue's runs and the queue as they were on
10 November 2026 at 13:40, when a queue rebuilt at 13:32 had left the four Scenarios runs of its
12:37 groups holding four of the five merge runners (the
first group's, whose failure at 13:31 rebuilt the queue, had ended) (tools/gate/watchdog.py, docs/developing.mdx).
The recorded queue's built entries carry the numbers of their pull requests, which their runs'
branches give; its unbuilt ones were recorded without."""

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
        self.entries = watchdog.queue_entries_of(QUEUE)

    def test_the_rebuilt_queues_old_runs_and_only_they(self):
        self.assertEqual(ids(watchdog.superseded(RUNS, self.entries, NOW)), STALE)

    def test_never_a_live_groups_run(self):
        built = {commit for _, commit in self.entries if commit}
        live = [run for run in RUNS if run["head_sha"] in built]
        self.assertTrue(live)
        self.assertFalse(ids(watchdog.superseded(RUNS, self.entries, NOW)) & ids(live))

    def test_never_the_run_asking(self):
        own = min(STALE)
        self.assertNotIn(own, ids(watchdog.superseded(RUNS, self.entries, NOW, own=str(own))))
        self.assertEqual(ids(watchdog.superseded(RUNS, self.entries, NOW, own=str(own))), STALE - {own})

    def test_never_another_workflow_or_an_ended_run(self):
        taken = watchdog.superseded(RUNS, [], NOW)
        self.assertTrue(taken)
        self.assertTrue(all(run["name"] == "Scenarios" and run["status"] != "completed" for run in taken))

    def test_never_a_run_younger_than_the_grace(self):
        early = datetime.datetime(2026, 11, 10, 13, 33, tzinfo=datetime.timezone.utc)
        young = [run for run in RUNS if run["created_at"] >= "2026-11-10T13:32"]
        self.assertEqual(watchdog.superseded(young, [], early), [])

    def test_never_the_run_of_an_entry_whose_group_is_being_built(self):
        # #543 on 10 November 2026: an entry taken out ahead of it, the queue built its groups again;
        # read meanwhile, #543's entry had no group yet, and its previous group's run, cancelled, was
        # still the one GitHub counted for it: GitHub took #543 out for failed checks.
        run = {"id": 1, "name": "Scenarios", "event": "merge_group", "status": "in_progress",
               "head_branch": "gh-readonly-queue/main/pr-543-dd1862c6a7bcec326102a3afbf958de4cc90b1aa",
               "head_sha": "0e22e71ae1000000000000000000000000000000", "created_at": "2026-11-10T13:00:00Z"}
        self.assertEqual(watchdog.superseded([run], [(571, "9c54c908c4"), (543, None)], NOW), [])
        # Once its group is built again, the old one's run is superseded; out of the queue, too.
        self.assertEqual(ids(watchdog.superseded([run], [(543, "a" * 40)], NOW)), {1})
        self.assertEqual(ids(watchdog.superseded([run], [(571, "9c54c908c4")], NOW)), {1})
        # An unbuilt entry the answer left the number of may be its: left.
        self.assertEqual(watchdog.superseded([run], [(571, "9c54c908c4"), (None, None)], NOW), [])

    def test_the_pull_request_of_a_queue_branch(self):
        self.assertEqual(watchdog.pull_request_of(
            {"head_branch": "gh-readonly-queue/main/pr-543-dd1862c6a7bcec326102a3afbf958de4cc90b1aa"}), 543)
        self.assertIsNone(watchdog.pull_request_of({"head_branch": "main"}))

    def test_an_unknown_queue_takes_nothing(self):
        self.assertEqual(watchdog.superseded(RUNS, None, NOW), [])


class QueueAnswer(unittest.TestCase):
    def test_every_entry_the_unbuilt_without_a_commit(self):
        entries = watchdog.queue_entries_of(QUEUE)
        self.assertEqual(len(entries), 14)
        self.assertIn((558, "30d99f4619eeccda1b3c81081f90982ced211103"), entries)
        self.assertEqual(sum(1 for _, commit in entries if commit is None), 9)

    def test_an_empty_queue_is_known_empty(self):
        answer = {"data": {"repository": {"mergeQueue": {"entries": {"nodes": []}}}}}
        self.assertEqual(watchdog.queue_entries_of(answer), [])

    def test_an_answer_that_does_not_say_is_unknown(self):
        for answer in ({"data": {"repository": {"mergeQueue": None}}},
                       {"data": None, "errors": [{"message": "Resource not accessible by integration"}]},
                       {"data": {"repository": None}},
                       {"data": {"repository": {"mergeQueue": {"entries": {"nodes": []}}}},
                        "errors": [{"message": "partial"}]},
                       {}):
            self.assertIsNone(watchdog.queue_entries_of(answer), answer)


if __name__ == "__main__":
    unittest.main()
