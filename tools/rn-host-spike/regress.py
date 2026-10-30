#!/usr/bin/env python3
"""Regression scenarios for the native Bloom host on the offscreen compositor.

Runs `oxyos-offscreen --launch` with a scenario's steps, with the host writing its report
(OXYOS_HOST_REPORT), then checks every step's EXPECTED result, not only that pixels changed:

- texts / absent: text the application shows (or must not) once the step settled, across its
  window and popups (each surface's latest presented frame);
- focus_input: the text input holding focus (null for none), from the latest frame;
- focus_surface: the application surface with the compositor's keyboard focus;
- react: the exact events delivered to React during the step, as "event@name" -> count, where
  name is the target's testID or the nearest ancestor's (`within`). Any delivered event the step
  does not list fails it, which catches duplicated and leaked events;
- delivered: compositor input routed to the application per surface (exact), plus `leaked`
  (what the session's shell takes is its own, never leaked),
  input routed to any other client;
- popups: how many popups of the application are shown after the step;
- shell: with the scenario's `shell` module run as the session's shell (ADR 0009), its surfaces
  ("surfaces": output rectangles, compared as a set; "top": the topmost), the "work_area"
  they leave, exactly, and whether the session is "locked" (ADR 0010);
- at / at_absent: nodes the session's assistive technology holds (or must not), as it received
  the trees (ADR 0003): by "window", "shell" (the shell's surfaces, the lock screen's among
  them) or the application's popups, each node matching the keys given ({} matches any): role,
  name, value, state, actions (what it offers beyond activate and focus), custom (its custom
  actions' labels), selection;
- announcements: what the assistive technology would have said in the step, exactly and in
  order: an application's announcements and live regions' changes, each matching the keys
  given (politeness, text, live);
- spoken: with the scenario's `reader`, what the screen reader said in the step (ADR 0014),
  exactly and in order: each a text, or keys matching an utterance (text, interrupting);
- output_pixels: as pixels, in output coordinates rather than the window's;
- window_shown: whether the application's window is on the screen (a minimized one is not);
- animated: what Animated's native driver set on views in the step (native-animated.h), frame by
  frame, as "testID" -> {prop: [from, to]} ("transform.translateX" for a transform's entry; a
  colour as "#rrggbb"): the values went from `from` to exactly `to`, monotonically in each colour
  channel, over at least `animated_frames` frames (8 by default); with `animated_linear_ms`, every
  frame's value is where a linear animation of that duration is at that frame's time, within one
  frame. A view it does not list must not move: the driver may set it (a node's first frame sets
  its start), but always to the same values;
- transitions: what the compositor animated in the step (ADR 0011): "frames_min", the fewest
  frames composed while a transition ran; "seen", the transitions it must have run, each matched
  by a distinct one having every key given ("kind": window, popup, overlay, panel,
  notification, lock; "change": arrived, gone, minimized, restored, workspace_in_forward,
  workspace_in_back, workspace_out_forward, workspace_out_back; "arriving"; "first" and "last",
  the opacities of the first and last frames drawn; "ended");
  "count", how many it ran, exactly;
- field: what the focused surface told the compositor of its text field (ADR 0008), each key
  given matching (text, anchor, cursor, purpose, hints, caret...), or null for none;
- im: with the scenario's `input_method`, what the session's input method was told of the field
  and the keys it received in the step (active, purpose, text, cursor, keys, given_back...);
- latency_ms: budget from injecting the input to the first changed frame;
- pixels: colours of the output once the step settled, "x,y" (window-local logical pixels at
  scale 1) -> "#rrggbb", each channel within `pixel_tolerance` (3 by default).
  (Pointer enter/leave, which follow every motion, are left out of `react`.)

A step's `expect_devices` replaces expectations of its `expect` when its input comes through the
virtual devices (`--uinput`), where a keyboard types one key at a time and a wheel scrolls one
notch at a time: what React and the client receive is then counted per key and per notch. A
scenario's `layout` (an XKB layout, `us` by default) is the one those expectations are written
for, and the devices type with it. A `commit` step is an input method's committed text, which
no keyboard types, so it is the same on both paths; so is a `transaction` step, an input
method's whole change (ADR 0008).

A scenario with `input_method` runs with an input method the session plays: `keyboard` steps type
through it (it gives the keys back), `compose_keys` steps have it compose them (Enter commits),
and `im_change` steps send a change of its own, for the field's latest state or (`stale`) the one
before.

A scenario with `wallpaper` ({folder, tz, now}) has the shell show the solar wallpaper's frame for
that moment where that time zone is (ADR 0012).

A scenario with `reader` ({"locale": LOCALE}) runs the session's screen reader (`oxyos-reader`,
beside the offscreen compositor unless `--reader` names it) in that locale, its voice a record of
what it says (ADR 0014); the reader's keys are `key` steps (Ctrl+Alt+Right is [29, 56, 106]).

Every scenario runs with a preferences file of its own, light unless it says otherwise. A scenario
with `settings` ({"app": ID, "preferences": {KEY: VALUE}}) has that file written with those lines (docs/adr/0019-settings.mdx), and the application ID granted
`settings` as a session grants an administrator's entry. A step's `preferences` expectation checks
what the session last told the application of how to draw (dark, reduceMotion,
reduceTransparency, accent) by the time the step settled, each key given; the `final`
`preferences_file` expectation checks the file holds each KEY = VALUE given once the run ended.

A scenario with `lock_password` has the development compositor's lock fixture unlock with that
password (ADR 0010; never PAM).

A scenario's `final` expectations (texts / absent / focus_input) are checked on the last frames,
once every step has run; a scenario may have no steps at all.

Also reports render times per frame and the launch time. Exit status 0 only if every
expectation holds.

Usage: regress.py SCENARIO.json --offscreen BIN --host BIN --bundle DIR [--out DIR]
"""

import argparse
import json
import re
import struct
import zlib
import os
import pathlib
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


# The fields of a director record that repeat the step's own instruction (oxyos-offscreen launch.rs).
STEP_KEYS = {"press", "press_popup", "press_output", "scroll", "pinch", "type", "commit", "preedit", "transaction", "keyboard",
             "compose_keys", "im_change", "drag", "drag_output", "grab", "key", "move_window", "resize_window", "minimize_window", "caps_lock", "scale",
             "a11y_action",
             "expect_popup", "switch_terminal"}


def director_args(step):
    kind, value = step["do"]
    args = ["--allow-unchanged"] if step.get("allow_unchanged") else []
    if kind in ("press", "press-popup"):
        return args + [f"--{kind}", f"{value[0]},{value[1]}"]
    if kind in ("press_popup", "press_output"):
        return args + [f"--{kind.replace('_', '-')}", f"{value[0]},{value[1]}"]
    if kind == "scroll":
        return args + ["--scroll", f"{value[0]},{value[1]},{value[2]}"]
    if kind == "pinch":
        return args + ["--pinch", ",".join(map(str, value))]
    if kind == "type":
        return args + ["--type", value]
    if kind == "key":
        # A code, or a chord as a list of codes pressed in order (Shift+Right is [42, 106]).
        return args + ["--key", "+".join(map(str, value)) if isinstance(value, list) else str(value)]
    if kind == "drag":
        return args + ["--drag", ",".join(map(str, value))]
    if kind == "drag_output":
        return args + ["--drag-output", ",".join(map(str, value))]
    if kind == "grab":
        # A press held while the application answers it, then a drag by (dx, dy): [x, y, dx, dy].
        return args + ["--grab", ",".join(map(str, value))]
    if kind in ("preedit", "commit"):
        return args + [f"--{kind}", value]
    if kind in ("keyboard", "compose_keys"):
        return args + ["--" + kind.replace("_", "-"), value]
    if kind == "im_change":
        # The scenario's input method (`input_method`): {"delete", "commit", "preedit", "stale"}.
        delete = value.get("delete", [0, 0])
        return args + ["--im-change", f"{delete[0]},{delete[1]}", value.get("commit", ""), value.get("preedit", ""),
                       "stale" if value.get("stale") else "current"]
    if kind == "minimize_window":
        return args + ["--minimize-window"]
    if kind == "caps_lock":
        # A press of Caps Lock, which turns it on or off.
        return args + ["--caps-lock"]
    if kind == "transaction":
        # An input method's change (ADR 0008): {"delete": [before, after], "commit", "preedit",
        # "cursor": [begin, end], "segments": [[begin, end, "composing"|"converting"|"error"]]}.
        delete = value.get("delete", [0, 0])
        preedit = value.get("preedit", "")
        cursor = value.get("cursor", [len(preedit.encode()), len(preedit.encode())])
        segments = ";".join(f"{b}-{e}:{style}" for b, e, style in value.get("segments", [])) or "-"
        return args + ["--transaction", f"{delete[0]},{delete[1]}", value.get("commit", ""), preedit,
                       f"{cursor[0]},{cursor[1]}", segments]
    if kind in ("move-window", "resize-window"):
        return args + [f"--{kind}", ",".join(map(str, value))]
    if kind == "a11y-action":
        return args + ["--a11y-action", f"{value[0]},{value[1]}"]
    if kind == "scale":
        return args + ["--scale", str(value)]
    if kind == "switch-terminal":
        return args + ["--switch-terminal", str(value)]
    if kind == "expect-popup":
        return ["--expect-popup"]
    raise ValueError(f"unknown step {kind}")


class ProbeServer(BaseHTTPRequestHandler):
    """The routes scenarios/network.json's probe calls (sample/NetworkProbe.tsx)."""

    def log_message(self, *args):
        pass

    def reply(self, status, body, content_type="text/plain", headers=()):
        data = body if isinstance(body, bytes) else body.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/json":
            self.reply(200, '{"hello":"OxyOS","n":3}', "application/json", [("X-OxyOS", "yes")])
        elif self.path == "/text":
            self.reply(200, "plain text from the server")
        elif self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/json")
            self.send_header("Content-Length", "0")
            self.end_headers()
        elif self.path == "/bytes":
            self.reply(200, bytes(range(256)), "application/octet-stream")
        elif self.path == "/header":
            self.reply(200, self.headers.get("X-Probe", "none"))
        elif self.path == "/split":
            # "año" in two writes, the second "ñ"'s bytes split between them.
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", "4")
            self.end_headers()
            self.wfile.write(b"a\xc3")
            self.wfile.flush()
            time.sleep(0.3)
            self.wfile.write(b"\xb1o")
        elif self.path == "/slow":
            time.sleep(3)
            self.reply(200, "late")
        else:
            self.reply(404, "missing")

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
        content_type = self.headers.get("Content-Type", "")
        if content_type.startswith("multipart/form-data"):
            # The fields' names and values, in order.
            fields = []
            for part in body.split("--")[1:]:
                head, _, value = part.partition("\r\n\r\n")
                if 'name="' in head:
                    fields.append(head.split('name="')[1].split('"')[0] + "=" + value.rstrip("\r\n"))
            body = " ".join(fields)
            content_type = "multipart/form-data"
        self.reply(200, f"POST {content_type} {body}")


def latest_frames(report, until, closed_before):
    """Each surface's latest frame presented by `until`, leaving out closed popups."""
    frames = {}
    for line in report:
        if line["kind"] == "frame" and line["t"] <= until:
            frames[line["surface"]] = line
    return {s: f for s, f in frames.items() if s not in closed_before}


def png_pixel_reader(path):
    """A function giving the RGB colour at (x, y) of a PNG the director wrote (oxyos-offscreen's
    png.rs): 8-bit RGB rows, none filtered. Any other PNG is refused."""
    data = pathlib.Path(path).read_bytes()
    width, height, depth, kind = struct.unpack(">IIBB", data[16:26])
    if depth != 8 or kind != 2:
        raise ValueError(f"{path}: not the director's PNG (depth {depth}, colour type {kind})")
    compressed, at = b"", 8
    while at < len(data):
        length, chunk = struct.unpack(">I4s", data[at:at + 8])
        if chunk == b"IDAT":
            compressed += data[at + 8:at + 8 + length]
        at += 12 + length
    raw, stride = zlib.decompress(compressed), width * 3 + 1

    def pixel(x, y):
        if not (0 <= x < width and 0 <= y < height):
            raise ValueError(f"{path}: ({x}, {y}) is outside {width}x{height}")
        if raw[y * stride] != 0:
            raise ValueError(f"{path}: row {y} is filtered")
        at = y * stride + 1 + x * 3
        return tuple(raw[at:at + 3])

    return pixel


def check_pixels(name, expect, screenshot, window, fail):
    """The step's `pixels`, against its screenshot of the output, the window at `window`."""
    pixel = png_pixel_reader(screenshot)
    tolerance = expect.get("pixel_tolerance", 3)
    for point, colour in expect["pixels"].items():
        x, y = (int(value) for value in point.split(","))
        want = tuple(int(colour[i:i + 2], 16) for i in (1, 3, 5))
        got = pixel(window[0] + x, window[1] + y)
        if any(abs(a - b) > tolerance for a, b in zip(got, want)):
            fail(f"pixel {point} is #{''.join(f'{c:02x}' for c in got)}, expected {colour}")


def gesture_events(report, start, end):
    """What gesture handlers sent JavaScript in a step (gesture-handler.h), in order: state changes
    as `handler@view OLD>NEW`, a run of updates as `handler@view update xN`, a run of touch events
    as `handler@view touches TYPE xN`, and the gesture taken from React Native (`intercept`,
    `handler cancels the responder`)."""
    events = []
    for line in report:
        if line["kind"] != "gesture" or not start <= line["t"] <= end:
            continue
        # A view without a testID or label is named by its component: its tag is arbitrary.
        who = f"{line.get('handler')}@{re.sub(r'#[0-9]+$', '', str(line.get('view')))}"
        if line["event"] == "state":
            events.append(f"{who} {line['oldState']}>{line['state']}")
        elif line["event"] == "update":
            run = f"{who} update x"
            if events and events[-1].startswith(run):
                events[-1] = f"{run}{int(events[-1][len(run):]) + 1}"
            else:
                events.append(f"{run}1")
        elif line["event"] == "touch":
            run = f"{who} touches {line['type']} x"
            if events and events[-1].startswith(run):
                events[-1] = f"{run}{int(events[-1][len(run):]) + 1}"
            else:
                events.append(f"{run}1")
        elif line["event"] == "intercept":
            events.append("intercept")
        else:
            events.append(f"{line.get('handler')} cancels the responder")
    return events


def animated_values(report, start, end):
    """What Animated's native driver set in a step, by view and prop: [(t, [channels])], a colour
    (an ARGB integer) as its red, green and blue, a transform's entry as `transform.KEY`."""
    values = {}
    for line in report:
        if line["kind"] != "animated" or not start <= line["t"] <= end:
            continue
        for prop, value in line["props"].items():
            entries = [(f"transform.{key}", v) for entry in value for key, v in entry.items()] \
                if prop == "transform" else [(prop, value)]
            for key, v in entries:
                channels = [(v & 0xffffffff) >> shift & 0xff for shift in (16, 8, 0)] if "Color" in key or key == "color" \
                    else [v]
                values.setdefault(line["target"], {}).setdefault(key, []).append((line["t"], channels))
    return values


def check_animated(expect, report, start, end, fail):
    """The step's `animated` (see the module's documentation)."""
    got = animated_values(report, start, end)
    for view in sorted(set(got) - set(expect["animated"])):
        for prop, frames in got[view].items():
            if any(value != frames[0][1] for _, value in frames):
                fail(f"the native driver moved {view} {prop} ({[value for _, value in frames]}), which the step does not list")
    least, linear_ms = expect.get("animated_frames", 8), expect.get("animated_linear_ms")

    def channels(value):
        return [int(value[i:i + 2], 16) for i in (1, 3, 5)] if isinstance(value, str) else [value]

    for view, props in expect["animated"].items():
        for prop, (first, last) in props.items():
            frames = got.get(view, {}).get(prop, [])
            name = f"{view} {prop}"
            if len(frames) < least:
                fail(f"{name} was set on {len(frames)} frames, expected at least {least}")
                continue
            low, high = channels(first), channels(last)
            shown = [[round(c, 3) for c in value] for _, value in frames]
            if [round(c, 3) for c in frames[-1][1]] != [round(c, 3) for c in high]:
                fail(f"{name} ended at {shown[-1]}, expected {high}")
            for c, (a, b) in enumerate(zip(low, high)):
                path = [value[c] for _, value in frames]
                if any((y < x) if b > a else (y > x) for x, y in zip(path, path[1:])):
                    fail(f"{name} went {shown}, not monotonically from {low} to {high}")
                    break
            if linear_ms:
                # Every frame's value is where one linear animation of `animated_linear_ms` is at that
                # frame's time. Its start is when the frames in between say it started (a frame that
                # changes nothing sets nothing, so the first frame is not always reported): those
                # starts agree within a frame, and every frame is then where it should be, within one.
                frame_ns = 1e9 / 60
                c = max(range(len(low)), key=lambda i: abs(high[i] - low[i]))
                starts = [t - (value[c] - low[c]) / (high[c] - low[c]) * linear_ms * 1e6 for t, value in frames
                          if 0 < (value[c] - low[c]) / (high[c] - low[c]) < 1]
                if len(starts) < 2 or max(starts) - min(starts) > frame_ns:
                    spread = (max(starts) - min(starts)) / 1e6 if starts else 0
                    fail(f"{name} went {shown}: not one linear {linear_ms} ms animation (its frames start "
                         f"{spread:.1f} ms apart)")
                    continue
                t0 = statistics.median(starts)
                for t, value in frames:
                    progress = min(1.0, max(0.0, (t - t0) / (linear_ms * 1e6)))
                    wrong = [i for i, (a, b) in enumerate(zip(low, high))
                             if abs(value[i] - (a + (b - a) * progress)) >
                             abs(b - a) * frame_ns / (linear_ms * 1e6) + (1 if isinstance(first, str) else 1e-6)]
                    if wrong:
                        fail(f"{name} was {[round(v, 3) for v in value]} {(t - t0) / 1e6:.1f} ms after it started, "
                             f"expected {[round(a + (b - a) * progress, 3) for a, b in zip(low, high)]} within a frame")
                        break


def check_step(index, step, result, report, start, failures, through_devices=False, screenshot=None,
               shell_report=None, spoken=None, preferences=None):
    expect = step.get("expect", {})
    if through_devices:
        # What differs through the devices; a key set to null is not checked there.
        expect = {key: value for key, value in {**expect, **step.get("expect_devices", {})}.items()
                  if value is not None}
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
    if "pointer_shape" in expect and result.get("pointer_shape") != expect["pointer_shape"]:
        # The pointer's shape the compositor shows after the step, as the surface under it asked.
        fail(f"the pointer's shape is {result.get('pointer_shape')}, expected {expect['pointer_shape']}")
    if "modal_animations" in expect:
        # Modal windows that started animating in or out (modal.h), as `in slide@testID`, `out fade@testID`.
        got = [f"{line['event']} {line['animation']}@{line.get('within') or line['target']}" for line in report
               if line["kind"] == "modal_animation" and start <= line["t"] <= end]
        if got != expect["modal_animations"]:
            fail(f"modal windows animated {got}, expected exactly {expect['modal_animations']}")
        # Each one moved monotonically from where its animation starts to where it ends: a slide in
        # from the window's height (`height`) to 0 and back out, a fade in from alpha 0 to 1 and out.
        for entry in got:
            event, rest = entry.split(" ", 1)
            animation, within = rest.split("@", 1)
            lines = [line for line in report if line["kind"] == "modal_presented" and line["event"] == event
                     and line["within"] == within and start <= line["t"] <= end]
            height = lines[0]["height"] if lines else 0
            key, first, last = {("in", "slide"): ("offset", height, 0), ("out", "slide"): ("offset", 0, height),
                                ("in", "fade"): ("alpha", 0, 1), ("out", "fade"): ("alpha", 1, 0)}[(event, animation)]
            values = [line[key] for line in lines]
            steps = [b - a for a, b in zip(values, values[1:])]
            rising = last > first
            if (len(values) < 3 or abs(values[0] - first) > abs(last - first) * 0.25 or values[-1] != last
                    or any((d < 0) if rising else (d > 0) for d in steps)):
                fail(f"{entry}: {key} went {[round(v, 2) for v in values]}, expected from {first} to {last} monotonically")
    if "animated" in expect:
        check_animated(expect, report, start, end, fail)
    if "window_frames" in expect:
        # The window's frames presented in the step: [at least, at most]. An animation draws several.
        low, high = expect["window_frames"]
        count = sum(1 for line in report if line["kind"] == "frame" and line.get("surface") == 1
                    and line.get("presented", True) and start <= line["t"] <= end)
        if not low <= count <= high:
            fail(f"the window presented {count} frames, expected between {low} and {high}")
    if "gestures" in expect:
        got = gesture_events(report, start, end)
        if got != expect["gestures"]:
            fail(f"gesture handlers sent {got}, expected exactly {expect['gestures']}")
    if "gesture_states" in expect:
        # The same without updates and touch moves, whose count depends on how frames fall
        # between the moves (a view moving under a still pointer is a touch move, as on Android).
        got = [event for event in gesture_events(report, start, end)
               if " update x" not in event and " touches move x" not in event]
        if got != expect["gesture_states"]:
            fail(f"gesture handlers changed states {got}, expected exactly {expect['gesture_states']}")
    if "in_shell" in expect:
        # The session's shell's own texts, React deliveries and gestures, from its report.
        shell_step = {"name": f"{step.get('name', step['do'][0])}, in the shell", "do": step["do"],
                      "expect": expect["in_shell"]}
        check_step(index, shell_step, result, shell_report or [], start, failures)
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
        # The session's shell takes its own input (its panels, the pointer leaving them).
        delivered.pop("shell", None)
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
    if "text_scroll" in expect:
        # How far each text input's text was scrolled in this step, by testID: the last value.
        scrolls = {}
        for line in report:
            if line["kind"] == "text_scroll" and start <= line["t"] <= end:
                scrolls[line["target"]] = [line["x"], line["y"]]
        if scrolls != expect["text_scroll"]:
            fail(f"text scrolls {scrolls}, expected exactly {expect['text_scroll']}")
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
    # What the assistive technology holds, as it received the trees (ADR 0003), by where they
    # are: "window" the application's window, "shell" the session's shell's surfaces (the lock
    # screen's among them), anything else the application's popups.
    def held_nodes(role):
        def place(surface):
            return "window" if surface == "1" else "shell" if surface.startswith("shell-") else "popup"
        wanted = role if role in ("window", "shell") else "popup"
        return [n for s, tree in result.get("a11y", {}).items() if place(s) == wanted for n in tree]
    if "at" in expect:
        for role, wanted in expect["at"].items():
            nodes = held_nodes(role)
            for node in wanted:
                if not any(all(n.get(k) == v for k, v in node.items()) for n in nodes):
                    fail(f"the assistive technology has no {role} node {node}")
    if "announcements" in expect:
        said = result.get("announcements", [])
        wanted = expect["announcements"]
        if len(said) != len(wanted) or not all(all(a.get(k) == v for k, v in w.items())
                                               for a, w in zip(said, wanted)):
            fail(f"the assistive technology would have said {said}, expected {wanted}")
    if "spoken" in expect:
        # What the screen reader said between the step's input and its settling.
        said = [u for u in spoken or [] if result.get("t_inject", 0) <= u["t"] <= end]
        wanted = [w if isinstance(w, dict) else {"text": w} for w in expect["spoken"]]
        if len(said) != len(wanted) or not all(all(u.get(k) == v for k, v in w.items())
                                               for u, w in zip(said, wanted)):
            fail(f"the screen reader said {[(u['text'], u['interrupting']) for u in said]}, expected {expect['spoken']}")
    if "at_absent" in expect:
        # Nodes it must not hold ({} matches any node: none at all there).
        for role, unwanted in expect["at_absent"].items():
            nodes = held_nodes(role)
            for node in unwanted:
                if any(all(n.get(k) == v for k, v in node.items()) for n in nodes):
                    fail(f"the assistive technology holds a {role} node {node}")
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
    if "transitions" in expect:
        ran = result.get("transitions") or {"frames": 0, "seen": []}
        wanted = expect["transitions"]
        if ran["frames"] < wanted.get("frames_min", 0):
            fail(f"{ran['frames']} frames composed in transitions, expected at least {wanted['frames_min']}")
        if "count" in wanted and len(ran["seen"]) != wanted["count"]:
            fail(f"{len(ran['seen'])} transitions ran, expected {wanted['count']}: {ran['seen']}")
        left = list(ran["seen"])
        for transition in wanted.get("seen", []):
            match = next((t for t in left if all(t.get(k) == v for k, v in transition.items())), None)
            if match is None:
                fail(f"no transition {transition} among {ran['seen']}")
            else:
                left.remove(match)
    if "window_shown" in expect and result.get("window_shown") != expect["window_shown"]:
        fail(f"window shown: {result.get('window_shown')}, expected {expect['window_shown']}")
    if "preferences" in expect:
        told = [line for line in report if line["kind"] == "input" and line["event"] == "preferences"
                and line["t"] <= end]
        last = told[-1] if told else {}
        for key, value in expect["preferences"].items():
            if last.get(key) != value:
                fail(f"preferences told: {key} = {last.get(key)!r}, expected {value!r}")
    if "preferences_file" in expect:
        kept = {}
        for line in (preferences.read_text().splitlines() if preferences else []):
            if "=" in line and not line.lstrip().startswith("#"):
                key, value = line.split("=", 1)
                kept[key.strip()] = value.strip()
        for key, value in expect["preferences_file"].items():
            if kept.get(key) != value:
                fail(f"preferences file: {key} = {kept.get(key)!r}, expected {value!r}")
    if "window_rect" in expect and result.get("window") != expect["window_rect"]:
        fail(f"window at {result.get('window')}, expected {expect['window_rect']}")
    if "popup_rect" in expect and result.get("popups") != [expect["popup_rect"]]:
        fail(f"popups at {result.get('popups')}, expected [{expect['popup_rect']}]")
    if "popups" in expect:
        count = len(result.get("popups", [result["popup"]] if "popup" in result else []))
        if count != expect["popups"]:
            fail(f"{count} popups shown, expected {expect['popups']}")
    if "shell" in expect:
        # The shell's surfaces (output rectangles, bottom to top) and the work area they leave.
        held = result.get("shell") or {}
        for key, wanted in expect["shell"].items():
            got = held.get(key)
            # A press raises its panel within the panel layer: surfaces compare as a set; "top"
            # is the topmost of them.
            if key == "top":
                got = (held.get("surfaces") or [None])[-1]
            if key == "surfaces" and got is not None:
                got, wanted = sorted(got), sorted(wanted)
            if got != wanted:
                fail(f"shell {key} is {got}, expected {wanted}")
    if "output_pixels" in expect:
        # Colours anywhere on the output, "x,y" in output pixels (the shell's surfaces included).
        check_pixels(name, {**expect, "pixels": expect["output_pixels"]}, screenshot, [0, 0], fail)
    if "im" in expect:
        im, wanted = result.get("im") or {}, expect["im"]
        differing = {k: im.get(k) for k, v in wanted.items() if im.get(k) != v}
        if differing:
            fail(f"the input method has {differing}, expected {wanted}")
    if "field" in expect:
        field, wanted = result.get("field"), expect["field"]
        if wanted is None or field is None:
            if field != wanted:
                fail(f"text field {field}, expected {wanted}")
        else:
            differing = {k: field.get(k) for k, v in wanted.items() if field.get(k) != v}
            if differing:
                fail(f"text field has {differing}, expected {wanted}")
    if "pixels" in expect:
        check_pixels(name, expect, screenshot, result.get("window"), fail)
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
    parser.add_argument("--reader", default=None,
                        help="the screen reader for scenarios with `reader` (oxyos-reader beside --offscreen)")
    parser.add_argument("--seat", action="store_true",
                        help="with --kms: run in the caller's logind session, devices through logind")
    parser.add_argument("--uinput", action="store_true",
                        help="point, scroll and type through virtual devices read by libinput")
    parser.add_argument("--kms", default=None,
                        help="show every composed frame on this DRM card (needs DRM master)")
    parser.add_argument("--dump-frames", default=None,
                        help="write the frame after each step as layers (oxyos-gpu's replay example)")
    parser.add_argument("--gpu", action="store_true",
                        help="also compose every frame on a Vulkan device; any difference fails")
    parser.add_argument("--check-damage", action="store_true",
                        help="the host draws every frame whole too; a frame drawn from its damage that differs fails")
    parser.add_argument("--gpu-only", action="store_true",
                        help="compose on the Vulkan device alone, with no CPU reference (measures it)")
    options = parser.parse_args()

    scenario = json.loads(pathlib.Path(options.scenario).read_text())
    out = pathlib.Path(options.out or tempfile.mkdtemp(prefix="oxyos-regress-"))
    out.mkdir(parents=True, exist_ok=True)
    report_path = out / "host-report.jsonl"
    bundle = pathlib.Path(options.bundle).resolve() / "index.js"
    width, height = scenario.get("window", [1024, 640])

    command = [options.offscreen, "--launch", "--out", str(out)] + (["--gpu"] if options.gpu else [])
    if options.gpu_only:
        command += ["--gpu-only"]
    if options.uinput:
        command += ["--uinput"]
    if options.seat:
        command += ["--seat"]
    if options.uinput:
        # The device expectations are the scenario's layout's: its dead keys, its Shift keys.
        command += ["--layout", scenario.get("layout", "us")]
    if scenario.get("shell"):
        # The session's shell (ADR 0009), a Bloom host of the same bundle started before the
        # application with the shell grant; its report is the application's with ".shell".
        command += ["--shell", f"{options.host},{bundle},{scenario['shell']}"]
    # The user's preferences (ADR 0019), in a file of the run's own: light unless the scenario says
    # otherwise, so what a scenario sees never depends on the time of day it runs at (an automatic
    # appearance follows the sun). With `settings`, the application the session would grant it to.
    preferences = out / "preferences"
    given = {"appearance": "light", **scenario.get("settings", {}).get("preferences", {})}
    preferences.write_text("".join(f"{key} = {value}\n" for key, value in given.items()))
    command += ["--preferences", str(preferences)]
    if "settings" in scenario:
        command += ["--grant-settings", scenario["settings"]["app"]]
    if scenario.get("lock_password"):
        # The lock screen's password (ADR 0010): the development compositor's fixture unlocks with
        # it, never PAM.
        command += ["--lock-password", scenario["lock_password"]]
    if "reader" in scenario:
        # The session's screen reader (ADR 0014), saying into a record, in the scenario's locale.
        reader = options.reader or str(pathlib.Path(options.offscreen).resolve().parent / "oxyos-reader")
        command += ["--reader", reader, "--reader-locale", scenario["reader"].get("locale", "en_US.UTF-8")]
    if scenario.get("input_method"):
        # The session plays an input method the scenario's steps drive (ADR 0008).
        command += ["--input-method"]
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
    env = dict(os.environ, OXYOS_HOST_REPORT=str(report_path), OXYOS_SCRIPT_URL=f"file://{bundle}")
    if scenario.get("shell"):
        # The shell's applications are the development registry's alone (ADR 0013), not the
        # machine's.
        env["XDG_DATA_DIRS"] = str(pathlib.Path(__file__).resolve().parent / "applications")
        env["XDG_DATA_HOME"] = str(out / "xdg-data")
    # The application's title and id, as a session's launch records give them (ADR 0009).
    for key, name in (("title", "OXYOS_APP_TITLE"), ("id", "OXYOS_APP_ID")):
        if key in scenario.get("app", {}):
            env[name] = scenario["app"][key]
    # An application the session did not start (ADR 0013): its id is only its claim.
    if scenario.get("app", {}).get("recorded") is False:
        env["OXYOS_APP_UNRECORDED"] = "1"
    # A local server for the scenario's probe, for as long as the run lasts, and the port after
    # it held bound without listening, so that connecting there is refused at once and nothing
    # else can answer (an unbound port may instead drop the connection, as WSL's firewall does).
    if "http" in scenario:
        port = scenario["http"]["port"]
        # The probe opens every request at once; socketserver's backlog of 5 would drop the rest's
        # connections for TCP to retry a second later, longer than a step takes to settle.
        ThreadingHTTPServer.request_queue_size = 64
        server = ThreadingHTTPServer(("127.0.0.1", port), ProbeServer)
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        refusing = socket.socket()
        refusing.bind(("127.0.0.1", port + 1))
    if options.check_damage:
        env["OXYOS_HOST_DAMAGE_CHECK"] = "1"
    if scenario.get("wallpaper"):
        # The solar wallpaper (ADR 0012) at a moment and a time zone the scenario fixes: the
        # wallpaper's folder (from the repository's root), "TZ" and "now" (unix seconds).
        wallpaper = scenario["wallpaper"]
        env["OXYOS_WALLPAPER"] = str(pathlib.Path(__file__).resolve().parents[2] / wallpaper["folder"])
        env["TZ"] = wallpaper["tz"]
        env["OXYOS_SOLAR_NOW"] = str(wallpaper["now"])
    run = subprocess.run(command, env=env, capture_output=True, text=True)
    (out / "director.log").write_text(run.stdout + run.stderr)
    if run.returncode != 0:
        print(f"FAIL: the director failed ({run.returncode}); see {out / 'director.log'}")
        print("\n".join(run.stderr.strip().splitlines()[-5:]))
        return 1
    summary = json.loads(next(line for line in run.stdout.splitlines() if line.startswith('{"mode"')))
    report = [json.loads(line) for line in report_path.read_text().splitlines() if line.strip()]
    shell_path = report_path.with_name(report_path.name + ".shell")
    shell_report = ([json.loads(line) for line in shell_path.read_text().splitlines() if line.strip()]
                    if shell_path.exists() else [])

    speech_path = out / "speech.jsonl"
    spoken = ([json.loads(line) for line in speech_path.read_text().splitlines() if line.strip()]
              if "reader" in scenario else None)

    failures = []
    start = summary["t_launch"]
    for index, (step, result) in enumerate(zip(scenario["steps"], summary["steps"])):
        start = check_step(index, step, result, report, start, failures, options.uinput,
                           out / f"launch-{index + 2}.png", shell_report, spoken,
                           out / "preferences" if "settings" in scenario else None)
    # A frame the host drew from its damage that differs from the frame drawn whole.
    for line in report:
        if line["kind"] == "damage_mismatch":
            failures.append(f"the host's frame drawn from its damage differs in {line['pixels']} pixels")
    if "final" in scenario:
        last = max(line["t"] for line in report)
        # The output after the last step (or at launch, without steps), the window where it was then.
        window = (summary["steps"][-1] if summary["steps"] else summary)["window"]
        check_step(len(scenario["steps"]), {"name": "final", "do": ["final"], "expect": scenario["final"]},
                   {"t_settled": last, "window": window}, report, start, failures,
                   screenshot=out / f"launch-{len(scenario['steps']) + 1}.png",
                   preferences=out / "preferences" if "settings" in scenario else None)

    # Frames presented; a change that drew nothing (a new focus) is reported, not presented.
    renders = [line["render_ms"] for line in report if line["kind"] == "frame" and line.get("presented", True)]
    # The share of each window frame the host drew again (1 when drawn whole).
    damaged = [line.get("damaged", 1.0) for line in report
               if line["kind"] == "frame" and line["surface"] == 1 and line.get("presented", True)]
    latencies = [s["latency_ms"] for s in summary["steps"] if s.get("latency_ms") is not None]
    # On a display: input to the kernel's flip of the first frame showing the answer.
    flips = [s["flip_latency_ms"] for s in summary["steps"] if s.get("flip_latency_ms") is not None]
    metrics = {
        "steps": len(summary["steps"]),
        "first_frame_ms": summary["first_frame_ms"],
        "gpu": summary.get("gpu"),
        "display": summary.get("drm"),
        "frames": len(renders),
        "damaged": {
            "median": round(statistics.median(damaged), 3) if damaged else None,
            "whole": sum(1 for share in damaged if share >= 1.0),
        },
        "render_ms": {
            "median": round(statistics.median(renders), 2) if renders else None,
            "max": round(max(renders), 2) if renders else None,
        },
        "latency_ms": {
            "median": round(statistics.median(latencies), 2) if latencies else None,
            "max": round(max(latencies), 2) if latencies else None,
        },
        "flip_latency_ms": {
            "median": round(statistics.median(flips), 2) if flips else None,
            "max": round(max(flips), 2) if flips else None,
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
