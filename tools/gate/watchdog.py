#!/usr/bin/env python3
"""The gate's watchdog (docs/developing.mdx, "The gate's machine"): ends Scenarios jobs that can no
longer end by themselves, so a runner GitHub still counts busy takes work again.

A job whose runner went away (its service restarted, its registration replaced) stays in progress
for hours: `timeout-minutes` ends a job its runner runs, not one no runner runs, and GitHub keeps that
runner busy, so every queued job, the merge queue's among them, waits behind it. From GitHub, such a
job is one of these, each force-cancelled:
  - in progress longer than the job's timeout-minutes and a quarter of an hour;
  - an older job on a runner that has started a newer one (a runner runs one job at a time).
A step's age says nothing: the gate's scenarios are one step, which takes over an hour when the
runners' gates run together (a runtime built again, five gates taking turns for the timing
scenarios); taking that for a lost runner cancelled live runs and took their entries out of the
merge queue (#572, #573 and #574 on 10 November 2026). A runner that came back starts a newer job
at once, and one that did not is past its timeout soon enough.
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


def queue_entries():
    """The merge queue's entries (queue_entries_of), or None if the queue cannot be read (then no run
    is taken for superseded)."""
    owner, name = os.environ["GITHUB_REPOSITORY"].split("/")
    query = ('{repository(owner:"%s",name:"%s"){mergeQueue(branch:"main"){entries(first:100)'
             '{nodes{headCommit{oid} pullRequest{number}}}}}}' % (owner, name))
    request = urllib.request.Request(
        "https://api.github.com/graphql", data=json.dumps({"query": query}).encode(), method="POST",
        headers={"Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return queue_entries_of(json.loads(response.read()))
    except (OSError, ValueError):
        return None


def queue_entries_of(answer):
    """The entries in a GraphQL answer for the merge queue, each (its pull request's number, the
    commit its group is tested on), either None when the answer leaves it out; or None when the
    answer does not say what the queue holds (an error, no queue, an unreadable one): an empty list
    would take every merge queue run for superseded.

    An entry without a commit is waiting for its group to be built, as it is while GitHub builds
    the queue's groups again, and until it is, the run of its previous group may still be the one
    GitHub counts for it."""
    try:
        queue = answer["data"]["repository"]["mergeQueue"]
        nodes = queue["entries"]["nodes"]
    except (KeyError, TypeError):
        return None
    if answer.get("errors") or not isinstance(nodes, list):
        return None
    return [((node.get("pullRequest") or {}).get("number"), (node.get("headCommit") or {}).get("oid"))
            for node in nodes if node]


def when(text):
    return datetime.datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def pull_request_of(run):
    """The number of the pull request a merge queue run tests last (its branch is
    gh-readonly-queue/main/pr-NUMBER-BASE), or None."""
    match = re.search(r"/pr-(\d+)-[0-9a-f]+$", run.get("head_branch") or "")
    return int(match.group(1)) if match else None


def superseded(runs, entries, now, own=None):
    """The merge queue's Scenarios runs, of those listed, that no entry of the queue waits for.

    `entries` is the queue (queue_entries), read after `runs` was listed: a run listed for a group
    still in the queue then has its commit among the entries'. A run is taken only when the queue
    says it is no entry's: its commit is no entry's group's, and its pull request is out of the queue
    or its entry's group is another, already built. A run whose entry is waiting for its group to be
    built is left: GitHub may still count it for that entry, and a run cancelled then fails the
    entry and takes it out of the queue (#543 on 10 November 2026, when the queue was built again
    behind an entry taken out). So is a run younger than GRACE, in case GitHub's queue answer lags
    its runs, and `own`, the run asking (GITHUB_RUN_ID)."""
    if entries is None:
        return []
    built = {commit for _, commit in entries if commit}
    known = {number for number, _ in entries if number is not None}
    unbuilt = {number for number, commit in entries if not commit}

    def no_entrys(run):
        number = pull_request_of(run)
        if run["head_sha"] in built or number is None or number in unbuilt:
            return False
        # An entry without a group whose number the answer left out may be this run's.
        return number in known or None not in unbuilt

    return [run for run in runs
            if run.get("event") == "merge_group" and run["name"] == WORKFLOW
            and run["status"] in ("queued", "in_progress", "waiting", "pending", "requested")
            and no_entrys(run)
            and str(run["id"]) != str(own)
            and now - when(run["created_at"]) >= GRACE]


def main():
    dry = "--dry-run" in sys.argv
    now = datetime.datetime.now(datetime.timezone.utc)
    limit = datetime.timedelta(minutes=timeout_minutes() + 15)
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
        name = job.get("runner_name") or "no runner"
        why = None
        if now - started > limit:
            why = f"in progress {int((now - started).total_seconds() // 60)} min, past its timeout and 15"
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
    taken = superseded(runs, queue_entries(), now, os.environ.get("GITHUB_RUN_ID"))
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
