#!/usr/bin/env python3
"""The gate's watchdog (docs/developing.mdx, "The gate's machine"): ends Scenarios jobs that can no
longer end by themselves, so a runner GitHub still counts busy takes work again.

A job whose runner went away (its service restarted, its registration replaced) stays in progress
for hours: `timeout-minutes` ends a job its runner runs, not one no runner runs, and GitHub keeps that
runner busy, so every queued job, the merge queue's among them, waits behind it. From GitHub, such a
job is one of these, each force-cancelled:
  - in progress longer than the job's timeout-minutes and a quarter of an hour;
  - its running step started more than STALLED minutes ago (the gate's longest step takes 25 to 40);
  - an older job on a runner that has started a newer one (a runner runs one job at a time).
It also cancels the merge queue's runs that no entry of the queue is waiting for any more (a group
superseded when the queue changed: an entry removed, another put ahead), which would otherwise hold
runners for nothing; GitHub leaves them running.

Run by .github/workflows/gate-watchdog.yml whenever the merge queue changes (and on a schedule GitHub
keeps loosely) on a GitHub-hosted runner with that run's GITHUB_TOKEN
(actions: write, this repository alone); nothing of it is on the gate's machine.
    GITHUB_TOKEN=... GITHUB_REPOSITORY=OxyHQ/OxyOS python3 tools/gate/watchdog.py [--dry-run]
"""
import datetime
import json
import os
import pathlib
import re
import sys
import urllib.error
import urllib.request

WORKFLOW = "Scenarios"
STALLED = 60  # minutes without a step starting or ending
GRACE = datetime.timedelta(minutes=2)  # a merge queue run this young is never taken for superseded
ROOT = pathlib.Path(__file__).resolve().parents[2]


def timeout_minutes():
    """The Scenarios job's timeout-minutes, from its workflow."""
    text = (ROOT / ".github/workflows/scenarios.yml").read_text()
    return int(re.search(r"^\s*timeout-minutes:\s*(\d+)", text, re.M).group(1))


def api(path, method="GET"):
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}", method=method,
        headers={"Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
                 "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"})
    # Three tries: a dropped connection is no reason to leave a stuck job for another ten minutes.
    for attempt in range(3):
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                body = response.read()
                return json.loads(body) if body else None
        except urllib.error.HTTPError:
            raise
        except OSError:
            if attempt == 2:
                raise


def queue_commits():
    """The commits the merge queue's entries are being tested on, or None if the queue cannot be
    read (then no run is taken for superseded)."""
    owner, name = os.environ["GITHUB_REPOSITORY"].split("/")
    query = ('{repository(owner:"%s",name:"%s"){mergeQueue(branch:"main"){entries(first:100)'
             '{nodes{headCommit{oid}}}}}}' % (owner, name))
    request = urllib.request.Request(
        "https://api.github.com/graphql", data=json.dumps({"query": query}).encode(), method="POST",
        headers={"Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return waited_commits(json.loads(response.read()))
    except (OSError, ValueError):
        return None


def waited_commits(answer):
    """The entries' commits in a GraphQL answer for the merge queue, or None when the answer does not
    say what the queue holds (an error, no queue, an unreadable one): an empty set would take every
    merge queue run for superseded."""
    try:
        queue = answer["data"]["repository"]["mergeQueue"]
        nodes = queue["entries"]["nodes"]
    except (KeyError, TypeError):
        return None
    if answer.get("errors") or not isinstance(nodes, list):
        return None
    # An entry without a commit is waiting for its group to be built: no run is its yet.
    return {node["headCommit"]["oid"] for node in nodes if node and node.get("headCommit")}


def when(text):
    return datetime.datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def superseded(runs, waited, now, own=None):
    """The merge queue's Scenarios runs, of those listed, that no entry of the queue waits for.

    `waited` is the queue's commits (queue_commits), read after `runs` was listed: a run listed for a
    group still in the queue then has its commit among them. A run younger than GRACE is left, in
    case GitHub's queue answer lags its runs, and so is `own`, the run asking (GITHUB_RUN_ID)."""
    if waited is None:
        return []
    return [run for run in runs
            if run.get("event") == "merge_group" and run["name"] == WORKFLOW
            and run["status"] in ("queued", "in_progress", "waiting", "pending", "requested")
            and run["head_sha"] not in waited
            and str(run["id"]) != str(own)
            and now - when(run["created_at"]) >= GRACE]


def main():
    dry = "--dry-run" in sys.argv
    now = datetime.datetime.now(datetime.timezone.utc)
    limit = datetime.timedelta(minutes=timeout_minutes() + 15)
    stalled = datetime.timedelta(minutes=STALLED)
    # First, what the merge queue's changes run this for: its superseded runs.
    superseded_runs = cancel_superseded(now, dry)
    jobs = []
    for run in api("actions/runs?status=in_progress&per_page=100")["workflow_runs"]:
        if run["name"] != WORKFLOW:
            continue
        for job in api(f"actions/runs/{run['id']}/jobs?per_page=100")["jobs"]:
            if job["status"] == "in_progress" and job["started_at"]:
                jobs.append((run, job))
    newest = {}
    for _run, job in jobs:
        name = job.get("runner_name")
        if name and (name not in newest or when(job["started_at"]) > newest[name]):
            newest[name] = when(job["started_at"])
    ended = 0
    for run, job in jobs:
        started = when(job["started_at"])
        steps = [when(s.get("started_at")) for s in job.get("steps", []) if s.get("started_at")]
        steps += [when(s.get("completed_at")) for s in job.get("steps", []) if s.get("completed_at")]
        last = max(steps, default=started)
        name = job.get("runner_name") or "no runner"
        why = None
        if now - started > limit:
            why = f"in progress {int((now - started).total_seconds() // 60)} min, past its timeout and 15"
        elif now - last > stalled:
            why = f"no step started or ended for {int((now - last).total_seconds() // 60)} min"
        elif job.get("runner_name") and newest[job["runner_name"]] > started:
            why = f"{name} has started a newer job"
        if not why:
            continue
        print(f"run {run['id']} ({run['head_branch']}), job on {name}: {why}: "
              f"{'would force-cancel' if dry else 'force-cancelled'}")
        if not dry:
            api(f"actions/runs/{run['id']}/force-cancel", method="POST")
        ended += 1
    print(f"watchdog: {len(jobs)} Scenarios jobs in progress, {ended} ended, {superseded_runs} superseded "
          f"merge queue runs cancelled")


def cancel_superseded(now, dry):
    """Cancels the merge queue's runs that no entry waits for (their jobs did nothing wrong)."""
    # The runs first, the queue after: a run listed for a live group has its commit in the queue read
    # after it, so only a group gone before that read is taken for superseded.
    runs = [run for status in ("queued", "in_progress")
            for run in api(f"actions/runs?status={status}&event=merge_group&per_page=100")["workflow_runs"]]
    taken = superseded(runs, queue_commits(), now, os.environ.get("GITHUB_RUN_ID"))
    for run in taken:
        print(f"run {run['id']} ({run['head_branch']}): no merge queue entry waits for it: "
              f"{'would cancel' if dry else 'cancelled'}")
        if not dry:
            try:
                api(f"actions/runs/{run['id']}/cancel", method="POST")
            except urllib.error.HTTPError as error:
                # 409: it ended meanwhile.
                if error.code != 409:
                    raise
    return len(taken)


if __name__ == "__main__":
    main()
