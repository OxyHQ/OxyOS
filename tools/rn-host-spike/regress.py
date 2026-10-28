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
  (Pointer enter/leave, which follow every motion, are left out of `react`.)

A step's `expect_devices` replaces expectations of its `expect` when its input comes through the
virtual devices (`--uinput`), where a keyboard types one key at a time and a wheel scrolls one
notch at a time: what React and the client receive is then counted per key and per notch. A
`commit` step is an input method's committed text, which no keyboard types, so it is the same on
both paths.

A scenario's `final` expectations (texts / absent / focus_input) are checked on the last frames,
once every step has run; a scenario may have no steps at all.

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


# The fields of a director record that repeat the step's own instruction (oxy-offscreen launch.rs).
STEP_KEYS = {"press", "press_popup", "scroll", "type", "commit", "preedit", "drag", "key", "move_window",
             "resize_window", "scale", "a11y_action", "expect_popup"}


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
        # A code, or a chord as a list of codes pressed in order (Shift+Right is [42, 106]).
        return args + ["--key", "+".join(map(str, value)) if isinstance(value, list) else str(value)]
    if kind == "drag":
        return args + ["--drag", ",".join(map(str, value))]
    if kind in ("preedit", "commit"):
        return args + [f"--{kind}", value]
    if kind in ("move-window", "resize-window"):
        return args + [f"--{kind}", ",".join(map(str, value))]
    if kind == "a11y-action":
        return args + ["--a11y-action", f"{value[0]},{value[1]}"]
    if kind == "scale":
        return args + ["--scale", str(value)]
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


def check_step(index, step, result, report, start, failures, through_devices=False):
    expect = step.get("expect", {})
    if through_devices:
        expect = {**expect, **step.get("expect_devices", {})}
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
            # Hover follows every pointer motion; a step checks it only under `hover`.
            if line["kind"] == "deliver" and start <= line["t"] <= end and not line["event"].startswith("pointer"):
                label = line.get("within") or line["target"]
                key = f"{line['event']}@{label}"
                got[key] = got.get(key, 0) + 1
        if got != expect["react"]:
            fail(f"React received {got}, expected exactly {expect['react']}")
    if "a11y_events" in expect:
        # Accessibility events JavaScript sent (focus moves), as `event@testID` counts.
        got = {}
        for line in report:
            if line["kind"] == "a11y_event" and start <= line["t"] <= end:
                key = f"{line['event']}@{line.get('within') or line['target']}"
                got[key] = got.get(key, 0) + 1
        if got != expect["a11y_events"]:
            fail(f"accessibility events {got}, expected exactly {expect['a11y_events']}")
    if "delivered" in expect:
        delivered = dict(result.get("delivered", {}))
        leaked = delivered.pop("leaked", 0)
        if leaked:
            fail(f"{leaked} input events reached another client")
        if delivered != expect["delivered"]:
            fail(f"compositor delivered {delivered}, expected exactly {expect['delivered']}")
    if "popup_events" in expect:
        # Popup lifecycle (open, reposition, close, dismissed) in this step, exactly.
        got = {}
        for line in report:
            if line["kind"] == "popup" and start <= line["t"] <= end:
                key = f"{line['event']}@{line.get('view', line['surface'])}"
                got[key] = got.get(key, 0) + 1
        if got != expect["popup_events"]:
            fail(f"popup events {got}, expected exactly {expect['popup_events']}")
    if "popup_size" in expect:
        rects = result.get("popups", [])
        sizes = [rect[2:] for rect in rects]
        if sizes != [expect["popup_size"]]:
            fail(f"popup sizes {sizes}, expected [{expect['popup_size']}]")
    if "scroll_offset" in expect:
        # The offset the host gave each scroll view in this step, by testID.
        offsets = {}
        for line in report:
            if line["kind"] == "deliver" and line["event"] == "scroll" and start <= line["t"] <= end:
                offsets[line.get("within") or line["target"]] = [line["x"], line["y"]]
        if offsets != expect["scroll_offset"]:
            fail(f"scroll offsets {offsets}, expected exactly {expect['scroll_offset']}")
    if "a11y" in expect or "a11y_nodes" in expect or "a11y_order" in expect:
        # The latest accessibility tree of the window and of each open popup (accessibility.h).
        trees = {}
        for line in report:
            if line["kind"] == "a11y" and line["t"] <= end and line["surface"] not in closed:
                trees[line["surface"]] = line["nodes"]

        def surfaces(role):
            return [nodes for s, nodes in trees.items() if (s == 1) == (role == "window")]

        # Each listed node is present: every key it gives matches.
        for role, wanted in expect.get("a11y", {}).items():
            nodes = [n for tree in surfaces(role) for n in tree]
            for node in wanted:
                if not any(all(n.get(k) == v for k, v in node.items()) for n in nodes):
                    fail(f"no {role} accessibility node {node}")
        # The surface's nodes, exactly and in reading order, as [role, name] pairs.
        for role, wanted in expect.get("a11y_nodes", {}).items():
            got = [[[n["role"], n["name"]] for n in tree] for tree in surfaces(role)]
            if got != [wanted]:
                fail(f"{role} accessibility nodes {got}, expected [{wanted}]")
        # These names are read in this order (other nodes may come between them).
        for role, wanted in expect.get("a11y_order", {}).items():
            names = [n["name"] for tree in surfaces(role) for n in tree]
            at = 0
            for label in wanted:
                at = names.index(label, at) + 1 if label in names[at:] else -1
                if at < 0:
                    fail(f"{role} reading order {names} does not read {wanted} in order")
                    break
    if "at" in expect:
        # What the assistive technology has: the compositor's committed snapshots (ADR 0003).
        held = result.get("a11y", {})
        for role, wanted in expect["at"].items():
            nodes = [n for s, tree in held.items() if (s == "1") == (role == "window") for n in tree]
            for node in wanted:
                if not any(all(n.get(k) == v for k, v in node.items()) for n in nodes):
                    fail(f"the assistive technology has no {role} node {node}")
    if "never" in expect:
        # Text that must appear nowhere: not drawn, not reported by the host, not held by the
        # compositor for an assistive technology. The step's own instruction is left out.
        held = json.dumps({k: v for k, v in result.items() if k not in STEP_KEYS}, ensure_ascii=False)
        lines = [json.dumps(line, ensure_ascii=False) for line in report if line["t"] <= end]
        for text in expect["never"]:
            if text in held or any(text in line for line in lines):
                fail(f"{text!r} leaked (host report or compositor)")
    if "frame_px" in expect:
        # The size in pixels of the latest frame of the window and of each open popup.
        for role, size in expect["frame_px"].items():
            got = [[f["width"], f["height"]] for s, f in frames.items() if (s == 1) == (role == "window")]
            if got != [size]:
                fail(f"{role} frames are {got} px, expected [{size}]")
    if "window_rect" in expect and result.get("window") != expect["window_rect"]:
        fail(f"window at {result.get('window')}, expected {expect['window_rect']}")
    if "popup_rect" in expect and result.get("popups") != [expect["popup_rect"]]:
        fail(f"popups at {result.get('popups')}, expected [{expect['popup_rect']}]")
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
    parser.add_argument("--uinput", action="store_true",
                        help="point, scroll and type through virtual devices read by libinput")
    parser.add_argument("--layout", default=None, help="XKB layout for typing (default us)")
    parser.add_argument("--kms", default=None,
                        help="show every composed frame on this DRM card (needs DRM master)")
    parser.add_argument("--dump-frames", default=None,
                        help="write the frame after each step as layers (oxy-gpu's replay example)")
    parser.add_argument("--gpu", action="store_true",
                        help="also compose every frame on a Vulkan device; any difference fails")
    options = parser.parse_args()

    scenario = json.loads(pathlib.Path(options.scenario).read_text())
    out = pathlib.Path(options.out or tempfile.mkdtemp(prefix="oxy-regress-"))
    out.mkdir(parents=True, exist_ok=True)
    report_path = out / "host-report.jsonl"
    bundle = pathlib.Path(options.bundle).resolve() / "index.js"
    width, height = scenario.get("window", [1024, 640])

    command = [options.offscreen, "--launch", "--out", str(out)] + (["--gpu"] if options.gpu else [])
    if options.uinput:
        command += ["--uinput"]
    if options.layout:
        command += ["--layout", options.layout]
    if options.kms:
        command += ["--kms", options.kms]
    if options.dump_frames:
        command += ["--dump-frames", options.dump_frames]
    for step in scenario["steps"]:
        command += director_args(step)
    command += ["--", options.host, str(bundle), scenario.get("module", "OxyBloom"), "--window", str(width), str(height)]
    # Steps checked one by one below: refuse a scenario whose steps the director would not run.
    if not all(isinstance(step.get("do"), list) for step in scenario["steps"]):
        raise SystemExit("every step needs `do`: [kind, value]")
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
        start = check_step(index, step, result, report, start, failures, options.uinput)
    if "final" in scenario:
        last = max(line["t"] for line in report)
        check_step(len(scenario["steps"]), {"name": "final", "do": ["final"], "expect": scenario["final"]},
                   {"t_settled": last}, report, start, failures)

    renders = [line["render_ms"] for line in report if line["kind"] == "frame"]
    latencies = [s["latency_ms"] for s in summary["steps"] if s.get("latency_ms") is not None]
    metrics = {
        "steps": len(summary["steps"]),
        "first_frame_ms": summary["first_frame_ms"],
        "gpu": summary.get("gpu"),
        "display": summary.get("drm"),
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
