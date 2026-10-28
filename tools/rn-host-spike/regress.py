#!/usr/bin/env python3
"""Regression scenarios for the native Bloom host on the offscreen compositor.

Runs `oxy-offscreen --launch` with a scenario's steps, with the host writing its report
(OXY_HOST_REPORT), then checks every step's EXPECTED result, not only that pixels changed:

- texts / absent: text the application shows (or must not) once the step settled, across its
  window and popups (each surface's latest presented frame);
- focus_input: the text input holding focus (null for none), from the latest frame;
- focus_surface: the application surface with the compositor's keyboard focus;
- react: the exact events delivered to React during the step, as "event@name" -> count, where
  name is the target's testID or the nearest ancestor's (`within`). Any delivered event the step
  does not list fails it, which catches duplicated and leaked events;
- delivered: compositor input routed to the application per surface (exact), plus `leaked`,
  input routed to any other client;
- popups: how many popups of the application are shown after the step;
- latency_ms: budget from injecting the input to the first changed frame.

Also reports render times per frame and the launch time. Exit status 0 only if every
expectation holds.

Usage: regress.py SCENARIO.json --offscreen BIN --host BIN --bundle DIR [--out DIR]
"""

import argparse
import json
import os
import pathlib
import statistics
import subprocess
import sys
import tempfile


def director_args(step):
    kind, value = step["do"]
    args = ["--allow-unchanged"] if step.get("allow_unchanged") else []
    if kind in ("press", "press-popup"):
        return args + [f"--{kind}", f"{value[0]},{value[1]}"]
    if kind == "scroll":
        return args + ["--scroll", f"{value[0]},{value[1]},{value[2]}"]
    if kind == "type":
        return args + ["--type", value]
    if kind == "key":
        return args + ["--key", str(value)]
    if kind == "expect-popup":
        return ["--expect-popup"]
    raise ValueError(f"unknown step {kind}")


def latest_frames(report, until, closed_before):
    """Each surface's latest frame presented by `until`, leaving out closed popups."""
    frames = {}
    for line in report:
        if line["kind"] == "frame" and line["t"] <= until:
            frames[line["surface"]] = line
    return {s: f for s, f in frames.items() if s not in closed_before}


def check_step(index, step, result, report, start, failures):
    expect = step.get("expect", {})
    name = f"step {index + 1} ({step.get('name', step['do'][0])})"
    end = result.get("t_settled")
    # A popup surface id is never reused: its last lifecycle event says whether it is open.
    lifecycle = {}
    for line in report:
        if line["kind"] == "popup" and line["t"] <= end:
            lifecycle[line["surface"]] = line["event"]
    closed = {surface for surface, event in lifecycle.items() if event in ("close", "dismissed")}
    frames = latest_frames(report, end, closed)
    shown = [text for frame in frames.values() for text in frame["texts"]]
    window = frames.get(1, {})

    def fail(message):
        failures.append(f"{name}: {message}")

    for text in expect.get("texts", []):
        if not any(text in shown_text for shown_text in shown):
            fail(f"expected text {text!r} is not shown")
    for text in expect.get("absent", []):
        if any(text in shown_text for shown_text in shown):
            fail(f"text {text!r} is shown but must not be")
    if "focus_input" in expect and window.get("focus") != expect["focus_input"]:
        fail(f"focused input is {window.get('focus')!r}, expected {expect['focus_input']!r}")
    if "focus_surface" in expect and result.get("focus") != expect["focus_surface"]:
        fail(f"compositor focus is on surface {result.get('focus')}, expected {expect['focus_surface']}")
    if "react" in expect:
        got = {}
        for line in report:
            if line["kind"] == "deliver" and start <= line["t"] <= end:
                label = line.get("within") or line["target"]
                key = f"{line['event']}@{label}"
                got[key] = got.get(key, 0) + 1
        if got != expect["react"]:
            fail(f"React received {got}, expected exactly {expect['react']}")
    if "delivered" in expect:
        delivered = dict(result.get("delivered", {}))
        leaked = delivered.pop("leaked", 0)
        if leaked:
            fail(f"{leaked} input events reached another client")
        if delivered != expect["delivered"]:
            fail(f"compositor delivered {delivered}, expected exactly {expect['delivered']}")
    if "popups" in expect:
        count = len(result.get("popups", [result["popup"]] if "popup" in result else []))
        if count != expect["popups"]:
            fail(f"{count} popups shown, expected {expect['popups']}")
    if "latency_ms" in expect and result.get("latency_ms") is not None:
        if result["latency_ms"] > expect["latency_ms"]:
            fail(f"latency {result['latency_ms']} ms over the {expect['latency_ms']} ms budget")
    return end


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("scenario")
    parser.add_argument("--offscreen", required=True)
    parser.add_argument("--host", required=True)
    parser.add_argument("--bundle", required=True, help="directory holding index.js and its assets")
    parser.add_argument("--out", default=None)
    options = parser.parse_args()

    scenario = json.loads(pathlib.Path(options.scenario).read_text())
    out = pathlib.Path(options.out or tempfile.mkdtemp(prefix="oxy-regress-"))
    out.mkdir(parents=True, exist_ok=True)
    report_path = out / "host-report.jsonl"
    bundle = pathlib.Path(options.bundle).resolve() / "index.js"
    width, height = scenario.get("window", [1024, 640])

    command = [options.offscreen, "--launch", "--out", str(out)]
    for step in scenario["steps"]:
        command += director_args(step)
    command += ["--", options.host, str(bundle), scenario.get("module", "OxyBloom"), "--window", str(width), str(height)]
    env = dict(os.environ, OXY_HOST_REPORT=str(report_path), OXY_SCRIPT_URL=f"file://{bundle}")
    run = subprocess.run(command, env=env, capture_output=True, text=True)
    (out / "director.log").write_text(run.stdout + run.stderr)
    if run.returncode != 0:
        print(f"FAIL: the director failed ({run.returncode}); see {out / 'director.log'}")
        print("\n".join(run.stderr.strip().splitlines()[-5:]))
        return 1
    summary = json.loads(next(line for line in run.stdout.splitlines() if line.startswith('{"mode"')))
    report = [json.loads(line) for line in report_path.read_text().splitlines() if line.strip()]

    failures = []
    start = summary["t_launch"]
    for index, (step, result) in enumerate(zip(scenario["steps"], summary["steps"])):
        start = check_step(index, step, result, report, start, failures)

    renders = [line["render_ms"] for line in report if line["kind"] == "frame"]
    latencies = [s["latency_ms"] for s in summary["steps"] if s.get("latency_ms") is not None]
    metrics = {
        "steps": len(summary["steps"]),
        "first_frame_ms": summary["first_frame_ms"],
        "frames": len(renders),
        "render_ms": {
            "median": round(statistics.median(renders), 2) if renders else None,
            "max": round(max(renders), 2) if renders else None,
        },
        "latency_ms": {
            "median": round(statistics.median(latencies), 2) if latencies else None,
            "max": round(max(latencies), 2) if latencies else None,
        },
        "failures": len(failures),
        "out": str(out),
    }
    for failure in failures:
        print(f"FAIL: {failure}")
    print(json.dumps(metrics))
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
