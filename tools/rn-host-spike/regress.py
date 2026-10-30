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
  does not list fails it, which catches duplicated and leaked events; a count may be [least, most]
  for events one a frame (a glide's scroll events);
- delivered: compositor input routed to the application per surface (exact), plus `leaked`
  (what the session's shell takes is its own, never leaked),
  input routed to any other client; and, given `launched`, what the applications the shell
  launched received, by kind (exact; "close.requested" among them);
- popups: how many popups of the application are shown after the step;
- gpu_frames: the application's frames of GPU buffers (DMA-BUFs, docs/adr/0031-gpu-buffers.mdx)
  the compositor latched since the run began, at least "latched_min", and at least "waited_min"
  of them held until their acquire point signalled: frames that took the GPU path, not shared
  memory;
- shell: with the scenario's `shell` module run as the session's shell (ADR 0009), its surfaces
  ("surfaces": output rectangles, compared as a set; "top": the topmost), the "work_area"
  they leave on the primary and "work_areas" on every output (by id), exactly, and whether the
  session is "locked" (ADR 0010);
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
- shaken: a view (by testID) committed left and right of its place in the step, and back at the
  end: a wrong password's field shaking;
- output_pixels: as pixels, in output coordinates rather than the window's (the primary's);
- smooth: a region of the output ("region": [x0, y0, x1, y1] in its pixels) showing a smooth
  gradient without banding: neighbouring pixels differ by at most "max_step" levels (2), and down
  its columns a level holds for at most "median_run_max" rows (3) as the median of its runs: a
  gradient quantized to 8 bits without dithering holds each level for a band of rows;
- screen_pixels: with the scenario's `outputs`, each output's by id: "x,y" in its own pixels
  (at its scale) -> "#rrggbb";
- window_shown: whether the application's window is on the screen (a minimized one is not);
- controls: with the scenario's `controls` (a machine the session's hardware keys and Control
  Center act on, ADR 0025: `output_volume`, `display_brightness`, `battery`, `wifi`, `ssid`...),
  the machine as it is once the step settled, each key given matching;
- animated: what Animated's native driver set on views in the step (native-animated.h), frame by
  frame, as "testID" -> {prop: [from, to]} ("transform.translateX" for a transform's entry; a
  colour as "#rrggbb"): the values went from `from` to exactly `to`, monotonically in each colour
  channel, over at least `animated_frames` frames (8 by default); with `animated_linear_ms`, every
  frame's value is where a linear animation of that duration is at that frame's time, within one
  frame of the display (the scenario's refresh: see `outputs`). A view it does not list must not move: the driver may set it (a node's first frame sets
  its start), but always to the same values. `animated_frames` may be [least, most];
- committed: the same for what React's commits changed in the step (opacity and translation of
  views with a testID, as mounted: a JavaScript-driven Animated's frames among them), with
  `committed_frames` and `committed_linear_ms`; views it does not list are not checked;
- animation_frames: requestAnimationFrame's frames in the step (animation-frames.h): "count",
  [least, most]; "interval_ms", the least time between two; "median_interval_ms", [least, most]
  for the median time between them (a frame of the display: 8.3 at 120 Hz); their timestamps rise
  and are performance.now()'s clock (within a millisecond of the report's, CLOCK_MONOTONIC);
- transitions: what the compositor animated in the step (ADR 0011): "frames_min", the fewest
  frames composed while a transition ran; "seen", the transitions it must have run, each matched
  by a distinct one having every key given ("kind": window, popup, overlay, backdrop, panel,
  notification, lock; "change": arrived, gone, minimized, restored, workspace_in_forward,
  workspace_in_back, workspace_out_forward, workspace_out_back; "arriving"; "first" and "last",
  the opacities of the first and last frames drawn; "ended"), and its curve, from each frame it
  drew: "eased", its opacity and its size and place each only ever going toward their end and,
  in the first half of its time, half again ahead of a linear motion of the same length (eased
  out, as a spring from rest is; a linear one fails); "lasts_ms", [least, most] for its last frame's time;
  "frame_gap_ms", [median, most] for the time between its frames (at display rate: 16.7 at 60 Hz,
  8.3 at 120); "still",
  its size and place never changing (reduce motion's crossfade);
  "count", how many it ran, exactly ("change" is also "zoomed": a window's frame zooming, ADR 0016);
- drag: for a `drag_paced` step (a press moved on the output in N motions at a rate, the compositor
  serving the application between them, as a hand drags a window's edge): "unanswered_max", the
  most configures sent that no frame shown answered; "presented_min", the fewest frames the window
  showed; "bounded", no more configures than frames shown; "steady", the window's edges ("left",
  "top", "right", "bottom") that must not move in any frame shown; "composed_min", the fewest frames
  composed in the drag (a window moved is a frame for each motion, or for each refresh of a paced
  output slower than the hand); "composed_gap_ms", [least, most] for the median time between the
  frames composed while the hand moved (a paced output's refresh); "bytes_read_max", the most
  of the application's pixels the compositor may read in the drag (0 for a window moved: it shows
  nothing new);
  and always, no frame composed in the drag drawing the window at another size than its
  buffer's ("scaled");
- glide: for a `flick` step (a `scroll_paced` whose fingers then lift), the scroll view
  ("view", by testID) the fingers flung, as Android's ScrollView flings: the velocity the host
  measured at the lift ("velocity": [least, most] in logical pixels a second, along the offset);
  the fling's distance, worked out here again from that velocity and the friction as Android's
  SplineOverScroller works it out (within half a pixel of the host's); the view's offset from the
  drag's end to the glide's end going only that way and covering that distance (or stopping at the
  end it reaches), within a pixel, over at least "frames_min" frames; and its events in order:
  scrollEndDrag, momentumScrollBegin, the glide's scrolls, momentumScrollEnd;
- scrolled: for a `scroll_paced` step (the pointer over the window scrolling by DY in N events at a
  rate, the compositor serving the application between them, as a touchpad scrolls a list):
  "presented_min", the fewest frames the window showed;
- react_hover: the hover events the host itself sent React in the step, exactly, as "event@name" ->
  count: a pointerleave as the pointer leaves the window; never a pointerenter, pointerover or
  pointerout, which React Native's pointer events processor makes from the host's pointermoves
  for the views that listen (`react` leaves them all out);
- moved: for a `move_paced` step (the mouse moved over the window from one point to another in N
  motions at a rate, no button held, the compositor serving the application between them, as a
  hand moves across a list): "presented_min", the fewest frames the window showed;
  "latency_max_ms", the longest from a motion to the frame that followed it;
- field: what the focused surface told the compositor of its text field (ADR 0008), each key
  given matching (text, anchor, cursor, purpose, hints, caret...), or null for none;
- im: with the scenario's `input_method`, what the session's input method was told of the field
  and the keys it received in the step (active, purpose, text, cursor, keys, given_back...);
- latency_ms: budget from injecting the input to the first changed frame;
- pixels: colours of the output once the step settled, "x,y" (window-local logical pixels at
  scale 1) -> "#rrggbb", each channel within `pixel_tolerance` (3 by default).
  (Pointer enter/leave, which follow every motion, are left out of `react`, and so are the W3C
  pointer presses, every mouse button's pointerdown and pointerup;)
- react_presses: those presses, exactly, as `react` checks its events;

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

A scenario with `client` (a list of arguments) runs a native OxyOS client that is not a Bloom
application instead of the host: the program `--client` names, with those arguments, `{out}`,
`{scenarios}`, `{width}` and `{height}` in them replaced by the run's directory, the scenarios'
directory and the scenario's window. It has no host report, so only what the compositor sees is
checked (pixels, delivered, focus_surface, popups, drag...). `astro.json` runs Astro this way
(ADR 0029), with no `--runtimes` or `--packages`.

Bloom scenarios run on the machine's applications as packages (docs/adr/0030-installing-applications.mdx):
`--packages DIR` holds one package per application (as `sample/packages.mjs` makes them: its
manifest, its source bundle, the `index.hbc` compiled from it) and `probes/`, the probes' development
bundle; `--runtimes DIR` holds the runtimes they were built for (`DIR/<abi>/oxyos-bloom-host`,
`runtime.json`, as `cmake --install` lays one out). A scenario's `module`, `shell` or the
development registry's applications that a package provides run from that package, on its
runtime's host, its bytecode known by its source bundle's URL; any other module is a probe, run
from `probes/` on the runtime it was built for. The development registry the shell lists is
`applications/` with each packaged application's bundle and runtime its package's.

A scenario's `installed` ([{"package": ID, "capabilities": {NAME: DECISION}, "entry": [LINE]}]) is the
install service's state for the run's user, as it would have it (docs/adr/0030-installing-applications.mdx,
decision 7): each package's version copied into `packages/<id>/<versionCode>/`, its entry the one the
service writes (`oxyos-pack entry`, beside the offscreen compositor) with the lines given added, and the
account's decisions on its capabilities; the development compositor takes it as the service's
(`--installed-fixture`). Its `applications_launched` is what the shell had started, exactly: each id,
where its entry came from (`system`, `installed N`, `development`) and whether it was given `settings`.

A scenario with `wallpaper` ({folder, tz, now}) has the shell show the solar wallpaper's frame for
that moment where that time zone is (ADR 0012).

A scenario with `reader` ({"locale": LOCALE}) runs the session's screen reader (`oxyos-reader`,
beside the offscreen compositor unless `--reader` names it) in that locale, its voice a record of
what it says (ADR 0014); the reader's keys are `key` steps (Ctrl+Alt+Right is [29, 56, 106]).

Every scenario runs with a preferences file of its own, light unless it says otherwise. A scenario
with `settings` ({"app": ID, "preferences": {KEY: VALUE}}) has that file written with those lines (docs/adr/0019-settings.mdx), and the application ID granted
`settings` as a session grants the system application the image's policy names. A step's `preferences` expectation checks
what the session last told the application of how to draw (dark, reduceMotion,
reduceTransparency, accent) by the time the step settled, each key given; the `final`
`preferences_file` expectation checks the file holds each KEY = VALUE given once the run ended.

A scenario with `home` runs with a home of the run's own (docs/adr/0028-file-access.mdx), made
afresh in its output directory: `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`
inside it, never the developer's. `home.items` maps each path in it to what it is: a folder ("/" at the end), a file's
text, or {"bytes": N} (N zero bytes) or {"png": [W, H, "#rrggbb"]} (a picture of one colour); each
modified at `home.modified` (unix seconds; 2026-01-02 10:00 UTC by default), so dates read the same
on every run. `home.later` is what another program does to it while the run goes on, each when the
application first shows a text: [{"when_shown": TEXT, "items": {...}}], items as above, made when a
frame of the application's window shows TEXT (so a step after that one sees the window follow a
change it did not make). A step's `home` expectation checks the home once the step settled: each path given
exists (true) or not (false), a file's text, and `trash` the names in the home's Trash (its
`files/`), each with a `.trashinfo` naming where it was. A scenario's `locale` sets `LANG` and
`LC_ALL` for the run, and its `tz` the time zone (`TZ`).

What applications keep (their data and cache directories by id, application.h) is under the run's
own XDG_DATA_HOME and XDG_CACHE_HOME. A scenario's `app_data` ({ID: {FILE: JSON}}) is written to an
application's data directory before it starts; the `final` `app_data` expectation checks each such
file holds that JSON once the run ended.

A scenario with `lock_password` has the development compositor's lock fixture unlock with that
password (ADR 0010; never PAM).

A scenario with `outputs` ("WxH[@SCALE][:HZ],..."; ADR 0018) runs on those outputs, arranged as a
session arranges its displays (left to right, top-aligned, the first the primary), with a shell
on each; steps' points are in the global space. An output given a refresh (":120") is paced as a
display at it (each frame shown at its next vblank, its surfaces told the rate, which the host
paces its frames by); one without is unpaced and said to be 60 Hz. The primary's refresh is the
scenario's: what "within a frame" means in its expectations.

A scenario with `displays` (ADR 0018) runs on the session's displays instead, on virtual connectors
in the card's order, each {identity, kind ("internal" or "external"), size_mm [w, h], modes
["WxH@MHZ", ... with "*" after the preferred one and "!" after one the card refuses], connector,
name (its EDID's, optional)}: shown, arranged and changed from Settings as a session's are, as the
preferences keep them (each at its native resolution and fastest refresh the card takes, unless
one was chosen), each paced at its mode's refresh. A step's
`displays` expectation checks what the session last told Settings of them by the time the step
settled: by output id, each key given of that display (x, y, width, height, scale, mode, primary,
name...), and `trial` (the mode on trial, {output, seconds}, or null).

A scenario with `login` is a login screen (docs/adr/0020-oxy-account-login.mdx): the development
compositor runs its `shell` module (`OxyOSLogin`) locked, with no application, and hands its
passwords to the login program on its development path (`oxyos-login --dev`, beside the offscreen
compositor), which checks them with PAM from a configuration of the run's own (`pam_exec` accepts
`login.password` alone) and records the session it would start instead of starting it. The
accounts' service is a stand-in answering `list` with `login.accounts`; Oxy's API is a refused port,
so the machine is offline. `login.record`: what the login program must have recorded once the run
ended, each line beginning so, in order (none: nobody signed in). A step's `shell.signed_in` says
whether the login screen was told someone signed in.

A scenario's `final` expectations (texts / absent / focus_input) are checked on the last frames,
once every step has run; a scenario may have no steps at all.

Also reports render times per frame and the launch time. Exit status 0 only if every
expectation holds.

Usage: regress.py SCENARIO.json --offscreen BIN --runtimes DIR --packages DIR [--out DIR]
"""

import argparse
import json
import math
import re
import shutil
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
STEP_KEYS = {"press", "press_popup", "press_output", "press_secondary", "double_press", "scroll", "pinch", "type", "commit", "preedit", "transaction", "keyboard",
             "compose_keys", "im_change", "drag", "drag_output", "drag_paced", "scroll_paced", "flick", "move_paced", "press_paced", "grab", "key", "key_down", "key_up", "move_window", "resize_window", "minimize_window", "caps_lock", "scale",
             "a11y_action",
             "expect_popup", "switch_terminal", "idle", "wait"}


def director_args(step):
    kind, value = step["do"]
    args = ["--allow-unchanged"] if step.get("allow_unchanged") else []
    if kind in ("press", "press-popup"):
        return args + [f"--{kind}", f"{value[0]},{value[1]}"]
    if kind in ("press_popup", "press_output", "press_secondary", "double_press"):
        return args + [f"--{kind.replace('_', '-')}", f"{value[0]},{value[1]}"]
    if kind == "scroll":
        return args + ["--scroll", f"{value[0]},{value[1]},{value[2]}"]
    if kind == "pinch":
        return args + ["--pinch", ",".join(map(str, value))]
    if kind == "type":
        return args + ["--type", value]
    if kind in ("key", "key_down", "key_up"):
        # A code, or a chord as a list of codes pressed in order (Shift+Right is [42, 106]);
        # `key_down` holds them pressed (Super through the application switcher, ADR 0024) until
        # a `key_up` releases them.
        flag = "--" + kind.replace("_", "-")
        return args + [flag, "+".join(map(str, value)) if isinstance(value, list) else str(value)]
    if kind == "drag":
        return args + ["--drag", ",".join(map(str, value))]
    if kind == "drag_output":
        return args + ["--drag-output", ",".join(map(str, value))]
    if kind == "drag_paced":
        # A drag on the output at a hand's pace: [x0, y0, x1, y1, motions, hz].
        return args + ["--drag-paced", ",".join(map(str, value))]
    if kind == "scroll_paced":
        # A touchpad's scroll at its pace over the window: [x, y, dy, events, hz].
        return args + ["--scroll-paced", ",".join(map(str, value))]
    if kind == "press_paced":
        # Presses at their pace, each carrying its time as a device's: [x, y, count, interval_ms].
        return args + ["--press-paced", ",".join(map(str, value))]
    if kind == "flick":
        # The same, then the fingers lifting: [x, y, dy, events, hz].
        return args + ["--flick", ",".join(map(str, value))]
    if kind == "move_paced":
        # The mouse moved over the window at its pace, no button held: [x0, y0, x1, y1, motions, hz].
        return args + ["--move-paced", ",".join(map(str, value))]
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
    if kind == "wait":
        # Milliseconds of real time with no input: what fades on its own clock (a bezel, ADR 0025).
        return args + ["--wait", str(value)]
    if kind == "idle":
        # Seconds with no input, which the idle lock counts as passed at once (ADR 0010).
        return args + ["--idle", str(value)]
    if kind == "switch-terminal":
        return args + ["--switch-terminal", str(value)]
    if kind == "expect-popup":
        return ["--expect-popup"]
    raise ValueError(f"unknown step {kind}")


def serve_accounts(path, accounts):
    """The accounts' service as the login screen and the login program ask it (`list` over
    SOCK_SEQPACKET, crates/oxyos-account's ipc.rs), answering `accounts` to anyone."""
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    listener.bind(str(path))
    listener.listen(16)

    def serve():
        while True:
            connection, _ = listener.accept()
            with connection:
                try:
                    if json.loads(connection.recv(16384)) == "list":
                        connection.send(json.dumps({"accounts": accounts}).encode())
                except (OSError, ValueError):
                    pass

    threading.Thread(target=serve, daemon=True).start()
    return listener


def login_program(out, offscreen, login):
    """The login program on its development path, its PAM configuration, the accounts' stand-in
    and a refused port for Oxy's API: (process, greeter socket, record, environment)."""
    pam = out / "pam"
    pam.mkdir(exist_ok=True)
    script = pam / "check-password"
    script.write_text("#!/bin/sh\n[ \"$(tr -d '\\000')\" = '%s' ]\n" % login["password"])
    script.chmod(0o700)
    (pam / "oxyos-login").write_text(
        f"auth [success=1 default=ignore] pam_exec.so quiet expose_authtok {script}\n"
        "auth requisite pam_deny.so\nauth required pam_permit.so\n"
        "account required pam_permit.so\nsession required pam_permit.so\n")
    # Sockets in a short directory of their own: a socket's path holds at most 107 bytes.
    sockets = pathlib.Path(tempfile.mkdtemp(prefix="oxyos-login-"))
    accountd, greeter, record = sockets / "accountd", sockets / "greeter", out / "login-record"
    record.unlink(missing_ok=True)
    serve_accounts(accountd, login.get("accounts", []))
    program = pathlib.Path(offscreen).resolve().parent / "oxyos-login"
    process = subprocess.Popen(
        [str(program), "--dev", "--pam-confdir", str(pam), "--record", str(record),
         "--socket", str(greeter), "--accountd", str(accountd)],
        stdout=subprocess.DEVNULL, stderr=open(out / "login.log", "w"))
    deadline = time.monotonic() + 10
    while not greeter.exists():
        if process.poll() is not None or time.monotonic() > deadline:
            raise SystemExit(f"the login program did not start; see {out / 'login.log'}")
        time.sleep(0.02)
    # Offline: Oxy's API is a port held bound without listening, so connecting is refused.
    refusing = socket.socket()
    refusing.bind(("127.0.0.1", 0))
    env = {"OXYOS_ACCOUNTD_SOCKET": str(accountd),
           "OXYOS_OXY_API": f"http://127.0.0.1:{refusing.getsockname()[1]}/"}
    return process, greeter, record, env, (refusing, sockets)


class ProbeServer(BaseHTTPRequestHandler):
    """The routes the probes of scenarios/network.json and file-system.json call
    (sample/NetworkProbe.tsx, sample/FileSystemProbe.tsx)."""

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
        elif self.path == "/report":
            # A download into a folder takes the name the response gives it.
            self.reply(200, "quarterly numbers", "text/plain",
                       [("Content-Disposition", 'attachment; filename="Q3 report.txt"')])
        else:
            self.reply(404, "missing")

    def do_PUT(self):
        # An upload whose body is the file: its length as sent, and what arrived.
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()
        self.reply(200, f"PUT {length} {body}")

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


def png_bytes(width, height, colour):
    """A PNG of one colour (8-bit RGB, rows unfiltered)."""
    rgb = bytes(int(colour[i:i + 2], 16) for i in (1, 3, 5))
    raw = b"".join(b"\x00" + rgb * width for _ in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def make_home(out, home):
    """The scenario's home, made afresh in the run's directory: its items, each modified at the
    same moment. Returns its path."""
    root = (out / "home").resolve()
    if root.exists():
        subprocess.run(["chmod", "-R", "u+rwx", str(root)], check=False)
        shutil.rmtree(root)
    root.mkdir(parents=True)
    modified = home.get("modified", 1767348000)
    write_items(root, home.get("items", {}), modified)
    os.utime(root, (modified, modified))
    return root


def write_items(root, items, modified):
    """Items as `home.items` describes them, each modified at `modified`."""
    made = []
    for relative, what in items.items():
        path = root / relative.rstrip("/")
        if relative.endswith("/"):
            path.mkdir(parents=True, exist_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            if isinstance(what, str):
                path.write_text(what)
            elif "bytes" in what:
                path.write_bytes(bytes(what["bytes"]))
            else:
                path.write_bytes(png_bytes(*what["png"]))
        made.append(path)
    for path in sorted(made, key=lambda p: -len(p.parts)):
        os.utime(path, (modified, modified))


def change_home_later(home_root, home, report_path):
    """Another program's changes to the home (`home.later`), each made once the application's
    window first shows its text, from a thread reading the host's report as it is written."""
    later = list(home.get("later", []))
    if not later:
        return None
    modified = home.get("modified", 1767348000)

    def watch():
        seen = 0
        deadline = time.monotonic() + 120
        while later and time.monotonic() < deadline:
            lines = report_path.read_text().splitlines() if report_path.exists() else []
            for line in lines[seen:]:
                try:
                    entry = json.loads(line)
                except ValueError:
                    continue
                if entry.get("kind") != "frame" or entry.get("surface") != 1:
                    continue
                while later and any(later[0]["when_shown"] in text for text in entry.get("texts", [])):
                    write_items(home_root, later.pop(0)["items"], modified)
            seen = len(lines)
            time.sleep(0.02)

    thread = threading.Thread(target=watch, daemon=True)
    thread.start()
    return thread


def home_state(root):
    """What the home holds: each path in it, a folder as None and a file as its text (or its
    size, past 4 KB)."""
    state = {}
    for path in root.rglob("*"):
        relative = str(path.relative_to(root))
        try:
            if path.is_dir() and not path.is_symlink():
                state[relative] = None
            else:
                size = path.stat().st_size
                state[relative] = path.read_text(errors="replace") if size <= 4096 else size
        except OSError:
            pass
    return state


class HomeRecorder:
    """The home as it was through the run, from a thread looking at it every 10 ms: each state
    with when it was first seen, on the monotonic clock the host's report uses, so each step is
    checked against the home as it was when the step settled."""

    def __init__(self, root):
        self.root = root
        self.states = []
        self.running = True
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while self.running:
            state = home_state(self.root)
            if not self.states or self.states[-1][1] != state:
                self.states.append((time.monotonic_ns(), state))
            time.sleep(0.01)

    def stop(self):
        self.running = False
        self.thread.join()
        state = home_state(self.root)
        if not self.states or self.states[-1][1] != state:
            self.states.append((time.monotonic_ns(), state))

    def at(self, t):
        """The home as it was at `t` (the last state seen by then)."""
        seen = [state for when, state in self.states if when <= t]
        return seen[-1] if seen else (self.states[0][1] if self.states else {})


def check_home(state, wanted, fail):
    """A step's `home`: what the home held once it settled."""
    for relative, want in wanted.items():
        if relative == "trash":
            files = ".local/share/Trash/files/"
            names = sorted(path[len(files):] for path in state if path.startswith(files) and "/" not in path[len(files):])
            if names != sorted(want):
                fail(f"the Trash holds {names}, expected {sorted(want)}")
            for name in names:
                text = state.get(f".local/share/Trash/info/{name}.trashinfo")
                if not isinstance(text, str) or not text.startswith("[Trash Info]\nPath=") or "\nDeletionDate=" not in text:
                    fail(f"{name} in the Trash has no info saying where it was")
            continue
        if isinstance(want, bool):
            if (relative in state) != want:
                fail(f"{relative} {'is missing' if want else 'is still there'}")
        elif state.get(relative) != want:
            fail(f"{relative} does not hold {want!r}")


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


def android_fling_distance(velocity, friction):
    """How far Android's SplineOverScroller flings at `velocity` dp a second with `friction`, for a
    device of 160 dots per inch (dp): its getSplineFlingDistance."""
    if velocity == 0:
        return 0.0
    inflexion, deceleration_rate = 0.35, math.log(0.78) / math.log(0.9)
    physical = 9.80665 * 39.37 * 160.0 * 0.84
    deceleration = math.log(inflexion * abs(velocity) / (friction * physical))
    distance = friction * physical * math.exp(deceleration_rate / (deceleration_rate - 1.0) * deceleration)
    return math.copysign(distance, velocity)


def check_glide(wanted, report, start, end, fail):
    """The step's `glide`: a scroll view flung by a touchpad's fingers, as Android flings it."""
    view = wanted["view"]
    lines = [line for line in report if start <= line["t"] <= end
             and (line.get("within") or line.get("target")) == view]
    flings = [line for line in lines if line["kind"] == "fling"]
    if len(flings) != 1:
        fail(f"{view} was flung {len(flings)} times, expected once")
        return
    fling = flings[0]
    lo, hi = wanted["velocity"]
    if not lo <= fling["vy"] <= hi:
        fail(f"{view} was flung at {fling['vy']:.0f} px/s, expected {lo} to {hi}")
    distance = android_fling_distance(fling["vy"], fling["friction"])
    if abs(distance - fling["dy"]) > 0.5:
        fail(f"{view}'s fling goes {fling['dy']:.1f} px, Android's goes {distance:.1f} at {fling['vy']:.0f} px/s")
    events = [line for line in lines if line["kind"] == "deliver"]
    names = [line["event"] for line in events]
    if "scrollEndDrag" not in names:
        fail(f"{view} had no scrollEndDrag")
        return
    after = events[names.index("scrollEndDrag"):]
    order = [line["event"] for line in after]
    glide = order[2:-1]
    if order[:2] != ["scrollEndDrag", "momentumScrollBegin"] or order[-1:] != ["momentumScrollEnd"] or \
            any(name != "scroll" for name in glide):
        fail(f"{view}'s events after the drag were {order}, expected scrollEndDrag, momentumScrollBegin, "
             f"scrolls, momentumScrollEnd")
        return
    offsets = [after[0]["y"]] + [line["y"] for line in after[2:-1]]
    steps = [b - a for a, b in zip(offsets, offsets[1:])]
    if any(step * distance <= 0 for step in steps):
        fail(f"{view} did not glide only one way: {steps}")
    if len(steps) < wanted.get("frames_min", 0):
        fail(f"{view} glided in {len(steps)} frames, expected at least {wanted['frames_min']}")
    covered, ended = offsets[-1] - offsets[0], after[-1].get("y")
    reached_end = wanted.get("to_end", False)
    if reached_end:
        if abs(covered) >= abs(distance) + 1:
            fail(f"{view} glided {covered:.1f} px, past the fling's {distance:.1f}")
        if abs(ended - wanted.get("end_offset", ended)) > 0.01:
            fail(f"{view} stopped at {ended}, expected its end {wanted['end_offset']}")
    elif abs(covered - distance) > 1:
        fail(f"{view} glided {covered:.1f} px, expected the fling's {distance:.1f}")


def check_smooth(name, wanted, screenshot, fail):
    """The step's `smooth`: a region of the output that shows a smooth gradient (a wide blur)
    without banding."""
    pixel = png_pixel_reader(screenshot)
    x0, y0, x1, y1 = wanted["region"]
    steepest, runs = 0, []
    for x in range(x0, x1):
        column = [pixel(x, y) for y in range(y0, y1)]
        run = 1
        for a, b in zip(column, column[1:]):
            steepest = max(steepest, max(abs(p - q) for p, q in zip(a, b)))
            if a == b:
                run += 1
            else:
                runs.append(run)
                run = 1
        runs.append(run)
        if x + 1 < x1:
            for y in range(y0, y1, 8):
                steepest = max(steepest, max(abs(p - q) for p, q in zip(pixel(x, y), pixel(x + 1, y))))
    if steepest > wanted.get("max_step", 2):
        fail(f"neighbouring pixels in {wanted['region']} differ by up to {steepest} levels, expected at most "
             f"{wanted.get('max_step', 2)}: a step, not a gradient")
    runs.sort()
    median = runs[len(runs) // 2]
    if median > wanted.get("median_run_max", 3):
        fail(f"down the columns of {wanted['region']} a level holds for {median} rows (median): bands, "
             f"expected at most {wanted.get('median_run_max', 3)} (a gradient dithered as it is written)")


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


def animated_values(report, start, end, kind="animated"):
    """What Animated's native driver set in a step, by view and prop: [(t, [channels])], a colour
    (an ARGB integer) as its red, green and blue, a transform's entry as `transform.KEY`."""
    values = {}
    for line in report:
        if line["kind"] != kind or not start <= line["t"] <= end:
            continue
        for prop, value in line["props"].items():
            entries = [(f"transform.{key}", v) for entry in value for key, v in entry.items()] \
                if prop == "transform" else [(prop, value)]
            for key, v in entries:
                channels = [(v & 0xffffffff) >> shift & 0xff for shift in (16, 8, 0)] if "Color" in key or key == "color" \
                    else [v]
                values.setdefault(line["target"], {}).setdefault(key, []).append((line["t"], channels))
    return values


def display_hz(scenario):
    """The refresh, in hertz, of the display the application's window is on: the primary output's
    (`outputs`' first; 60 for one without a refresh, said to be 60 Hz), or the primary display's own
    fastest at its native resolution that the card takes (`displays`: the first internal one, else
    the first); 60 for the development compositor's one output."""
    if scenario.get("outputs"):
        primary = scenario["outputs"].split(",")[0]
        return float(primary.split(":")[1]) if ":" in primary else 60.0
    if scenario.get("displays"):
        displays = scenario["displays"]
        primary = next((d for d in displays if d["kind"] == "internal"), displays[0])
        modes = [(m.rstrip("*!"), m) for m in primary["modes"]]
        native = next((m.split("@")[0] for m, raw in modes if raw.rstrip("!").endswith("*")), modes[0][0].split("@")[0])
        rates = [int(m.split("@")[1]) for m, raw in modes if m.split("@")[0] == native and not raw.endswith("!")]
        return max(rates) / 1000 if rates else 60.0
    return 60.0


def check_animated(expect, report, start, end, fail, kind="animated", refresh_hz=60.0):
    """The step's `animated` or `committed` (see the module's documentation), each frame within a
    frame of the display at `refresh_hz`."""
    got = animated_values(report, start, end, kind)
    if kind == "animated":
        for view in sorted(set(got) - set(expect[kind])):
            for prop, frames in got[view].items():
                if any(value != frames[0][1] for _, value in frames):
                    fail(f"the native driver moved {view} {prop} ({[value for _, value in frames]}), which the step does not list")
    bounds, linear_ms = expect.get(f"{kind}_frames", 8), expect.get(f"{kind}_linear_ms")
    least, most = bounds if isinstance(bounds, list) else (bounds, None)

    def channels(value):
        return [int(value[i:i + 2], 16) for i in (1, 3, 5)] if isinstance(value, str) else [value]

    for view, props in expect[kind].items():
        for prop, (first, last) in props.items():
            frames = got.get(view, {}).get(prop, [])
            name = f"{view} {prop}"
            if len(frames) < least or (most is not None and len(frames) > most):
                fail(f"{name} was set on {len(frames)} frames, expected {least} to {most}" if most is not None
                     else f"{name} was set on {len(frames)} frames, expected at least {least}")
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
                frame_ns = 1e9 / refresh_hz
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


CURVE_KEYS = ("eased", "still", "lasts_ms", "frame_gap_ms")


def check_curve(wanted, seen, fail, refresh_hz=60.0):
    """A transition's frames (`path`: [ms, opacity, moved in thousandths]) against its `eased`,
    `lasts_ms` and `frame_gap_ms` (see the module's documentation), on a display at `refresh_hz`."""
    if not any(key in wanted for key in CURVE_KEYS):
        return
    path = seen.get("path") or []
    name = f"the {seen['kind']} {seen['change']} transition"
    if len(path) < 3:
        fail(f"{name} drew {len(path)} frames, too few to tell its curve")
        return
    times = [ms for ms, _, _ in path]
    if "lasts_ms" in wanted:
        low, high = wanted["lasts_ms"]
        # Its last frame drawn in it, within a display frame of its end.
        if not low <= times[-1] <= high:
            fail(f"{name} drew its last frame {times[-1]} ms in, expected {low} to {high}")
    if "frame_gap_ms" in wanted:
        gaps = sorted(b - a for a, b in zip(times, times[1:]))
        median, most = gaps[len(gaps) // 2], gaps[-1]
        if median > wanted["frame_gap_ms"][0] or most > wanted["frame_gap_ms"][1]:
            fail(f"{name}'s frames came {median} ms apart (median), {most} at most; expected at most "
                 f"{wanted['frame_gap_ms']}: {times}")
    if wanted.get("eased"):
        arriving = seen["arriving"]
        span = times[-1] + 1000 / refresh_hz
        opacities = [o if arriving else 255 - o for _, o, _ in path]
        moved = [m for _, _, m in path]
        for label, values, end in (("opacity", opacities, 255), ("size and place", moved, 1000)):
            # Nothing of it changes (a zoom's opacity, a fade's size): nothing to ease.
            if len(set(values)) == 1:
                continue
            if any(b < a for a, b in zip(values, values[1:])):
                fail(f"{name}'s {label} went back: {values}")
            # Eased out: in the first half of its time, from 34 ms on (a spring released from rest
            # starts slower than a line, whatever the refresh), half again ahead of a linear motion
            # of the same length (a spring released from rest is
            # quick at first and slow to arrive: a critically damped one is 0.81 of the way a third
            # of the way through, a linear one 0.33).
            behind = [(ms, v) for (ms, _, _), v in zip(path, values)
                      if 34 <= ms <= span / 2 and v + end * 0.02 < min(end, 1.5 * end * ms / span)]
            if behind:
                fail(f"{name}'s {label} was behind a linear motion at {behind}: not eased out ({values})")
    if wanted.get("still") and any(m != 1000 for _, _, m in path):
        fail(f"{name} grew, shrank or moved ({[m for _, _, m in path]}): expected a crossfade alone")


def check_step(index, step, result, report, start, failures, through_devices=False, screenshot=None,
               shell_report=None, spoken=None, preferences=None, refresh_hz=60.0, home_root=None):
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
    closed = {surface for surface, event in lifecycle.items() if event in ("close", "dismissed", "shell.close")}
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
            # Hover follows every pointer motion: the W3C events it makes are checked by what the
            # application shows of them.
            if line["kind"] == "deliver" and start <= line["t"] <= end and not line["event"].startswith("pointer"):
                label = line.get("within") or line["target"]
                key = f"{line['event']}@{label}"
                got[key] = got.get(key, 0) + 1
        wanted = expect["react"]
        within = lambda count, want: count == want if isinstance(want, int) else want[0] <= count <= want[1]
        if set(got) != set(wanted) or not all(within(got[k], wanted[k]) for k in got):
            fail(f"React received {got}, expected exactly {wanted}")
    if "home" in expect:
        check_home(home_root.at(end) if home_root else {}, expect["home"], fail)
    if "react_hover" in expect:
        got = {}
        for line in report:
            if (line["kind"] == "deliver" and start <= line["t"] <= end
                    and line["event"] in ("pointerEnter", "pointerLeave", "pointerOver", "pointerOut")):
                key = f"{line['event']}@{line.get('within') or line['target']}"
                got[key] = got.get(key, 0) + 1
        if got != expect["react_hover"]:
            fail(f"the host sent React the hover events {got}, expected exactly {expect['react_hover']}")
    if "react_presses" in expect:
        # The W3C pointer presses (pointerdown, pointerup) React received in the step, which `react`
        # leaves out with the hover: exactly, as "event@name" -> count.
        got = {}
        for line in report:
            if (line["kind"] == "deliver" and start <= line["t"] <= end
                    and line["event"] in ("pointerDown", "pointerUp")):
                key = f"{line['event']}@{line.get('within') or line['target']}"
                got[key] = got.get(key, 0) + 1
        if got != expect["react_presses"]:
            fail(f"React received the presses {got}, expected exactly {expect['react_presses']}")
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
        check_animated(expect, report, start, end, fail, refresh_hz=refresh_hz)
    if "committed" in expect:
        check_animated(expect, report, start, end, fail, "committed", refresh_hz)
    if "animation_frames" in expect:
        wanted = expect["animation_frames"]
        frames = [line for line in report if line["kind"] == "animation_frame" and start <= line["t"] <= end]
        low, high = wanted.get("count", [0, float("inf")])
        if not low <= len(frames) <= high:
            fail(f"requestAnimationFrame ran {len(frames)} frames, expected {low} to {high}")
        stamps = [line["timestamp"] for line in frames]
        gaps = [b - a for a, b in zip(stamps, stamps[1:])]
        if gaps and min(gaps) < wanted.get("interval_ms", 0):
            fail(f"requestAnimationFrame's frames came {min(gaps):.2f} ms apart, expected at least {wanted['interval_ms']}")
        if "median_interval_ms" in wanted:
            low, high = wanted["median_interval_ms"]
            median = statistics.median(gaps) if gaps else None
            if median is None or not low <= median <= high:
                fail(f"requestAnimationFrame's frames came {median} ms apart (median), expected {low} to {high}")
        if any(gap <= 0 for gap in gaps):
            fail("requestAnimationFrame's timestamps did not rise")
        if any(abs(line["t"] / 1e6 - line["timestamp"]) > 1 for line in frames):
            fail("requestAnimationFrame's timestamps are not on performance.now()'s clock")
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
    if "shaken" in expect:
        # A view shaken in the step (a wrong password's field): committed to the left and the right
        # of where it stays, and back there at the end.
        moves = [entry["translateX"] for line in report
                 if line["kind"] == "committed" and line["target"] == expect["shaken"] and start <= line["t"] <= end
                 for entry in line["props"].get("transform", []) if "translateX" in entry]
        if not moves or min(moves) > -5 or max(moves) < 5 or moves[-1] != 0:
            fail(f"{expect['shaken']} moved {[round(x, 1) for x in moves]}, not a shake")
    if "in_shell" in expect:
        # The session's shell's own texts, React deliveries and gestures, from its report.
        shell_step = {"name": f"{step.get('name', step['do'][0])}, in the shell", "do": step["do"],
                      "expect": expect["in_shell"]}
        check_step(index, shell_step, result, shell_report or [], start, failures, refresh_hz=refresh_hz)
    if "in_launched" in expect and screenshot is not None:
        # An application the session launched (a store's open, the shell's dock), from its own
        # report beside the director's (`.launched`): its texts and React deliveries.
        path = pathlib.Path(screenshot).parent / "host-report.jsonl.launched"
        launched = ([json.loads(line) for line in path.read_text().splitlines() if line.strip()]
                    if path.exists() else [])
        launched_step = {"name": f"{step.get('name', step['do'][0])}, in the launched application",
                         "do": step["do"], "expect": expect["in_launched"]}
        check_step(index, launched_step, result, launched, start, failures, refresh_hz=refresh_hz)
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
        # Applications the shell launched (ADR 0013): checked, exactly, only when the step says.
        launched = delivered.pop("launched", {})
        wanted = dict(expect["delivered"])
        if "launched" in wanted and launched != wanted.pop("launched"):
            fail(f"the launched applications were delivered {launched}, expected exactly {expect['delivered']['launched']}")
        if leaked:
            fail(f"{leaked} input events reached another client")
        if delivered != wanted:
            fail(f"compositor delivered {delivered}, expected exactly {wanted}")
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
        shown = [{k: v for k, v in t.items() if k != "path"} for t in ran["seen"]]
        for transition in wanted.get("seen", []):
            plain = {k: v for k, v in transition.items() if k not in CURVE_KEYS}
            match = next((t for t in left if all(t.get(k) == v for k, v in plain.items())), None)
            if match is None:
                fail(f"no transition {transition} among {shown}")
            else:
                left.remove(match)
                check_curve(transition, match, fail, refresh_hz)
    if "drag" in expect:
        # A drag at a hand's pace (`drag_paced`): what the window showed meanwhile.
        drag, wanted = result.get("drag") or {}, expect["drag"]
        unanswered = drag.get("configures", 0) - drag.get("answered", 0)
        if unanswered > wanted.get("unanswered_max", unanswered):
            fail(f"{unanswered} of the {drag.get('configures')} configures sent in the drag were never answered by "
                 f"a frame shown, expected at most {wanted['unanswered_max']}")
        if drag.get("presented", 0) < wanted.get("presented_min", 0):
            fail(f"the window showed {drag.get('presented', 0)} frames in the drag, expected at least "
                 f"{wanted['presented_min']}")
        if drag.get("composed", 0) < wanted.get("composed_min", 0):
            fail(f"{drag.get('composed', 0)} frames were composed in the drag, expected at least {wanted['composed_min']}")
        if "composed_gap_ms" in wanted:
            low, high = wanted["composed_gap_ms"]
            median = (drag.get("composed_gap_ms") or {}).get("median")
            if median is None or not low <= median <= high:
                fail(f"the frames composed in the drag came {median} ms apart (median), expected {low} to {high}")
        if drag.get("bytes_read", 0) > wanted.get("bytes_read_max", float("inf")):
            fail(f"the compositor read {drag['bytes_read']} bytes of the application's pixels in the drag, expected at most "
                 f"{wanted['bytes_read_max']}")
        if drag.get("configures", 0) > drag.get("presented", 0) and wanted.get("bounded"):
            fail(f"{drag.get('configures')} configures for {drag.get('presented', 0)} frames shown")
        # A window resizing is never drawn scaled: every frame composed draws its buffer 1:1.
        if drag.get("scaled", 0) > 0:
            fail(f"{drag['scaled']} of the {drag.get('composed')} frames composed in the drag drew the window "
                 f"scaled, not at its buffer's size")
        for edge in wanted.get("steady", []):
            low, high = (drag.get("edges") or {}).get(edge) or [None, None]
            if low is None or low != high:
                fail(f"the window's {edge} edge moved in the drag: {low} to {high}")
    if "glide" in expect:
        check_glide(expect["glide"], report, start, end, fail)
    if "scrolled" in expect:
        # A paced scroll (`scroll_paced`): "presented_min", the fewest frames the window showed
        # while it scrolled.
        scrolled, wanted = result.get("scrolled") or {}, expect["scrolled"]
        if scrolled.get("presented", 0) < wanted.get("presented_min", 0):
            fail(f"the window showed {scrolled.get('presented', 0)} frames in the scroll, expected at least "
                 f"{wanted['presented_min']}")
    if "moved" in expect:
        # A paced pointer motion (`move_paced`): the frames the window showed while the mouse
        # moved over it, and how soon each followed its motion.
        moved, wanted = result.get("moved") or {}, expect["moved"]
        if moved.get("presented", 0) < wanted.get("presented_min", 0):
            fail(f"the window showed {moved.get('presented', 0)} frames as the pointer moved, expected at least "
                 f"{wanted['presented_min']}")
        worst = (moved.get("latency_ms") or {}).get("max")
        if "latency_max_ms" in wanted and (worst is None or worst > wanted["latency_max_ms"]):
            fail(f"a frame came {worst} ms after the motion it followed, expected at most {wanted['latency_max_ms']}")
    if "controls" in expect:
        # The played machine once the step settled (ADR 0025): what the session did to it, each
        # key given matching (levels within 1/65536 of all of it).
        machine = result.get("controls") or {}
        for key, value in expect["controls"].items():
            got = machine.get(key)
            if isinstance(value, float) and isinstance(got, (int, float)) and not isinstance(got, bool):
                matches = abs(got - value) <= 1 / 65536
            else:
                matches = got == value
            if not matches:
                fail(f"the machine's {key} is {got!r}, expected {value!r}")
    if "window_shown" in expect and result.get("window_shown") != expect["window_shown"]:
        fail(f"window shown: {result.get('window_shown')}, expected {expect['window_shown']}")
    if "preferences" in expect:
        told = [line for line in report if line["kind"] == "input" and line["event"] == "preferences"
                and line["t"] <= end]
        last = told[-1] if told else {}
        for key, value in expect["preferences"].items():
            if last.get(key) != value:
                fail(f"preferences told: {key} = {last.get(key)!r}, expected {value!r}")
    if "displays" in expect:
        told = [line for line in report if line["kind"] == "input" and line["event"] == "displays"
                and line["t"] <= end]
        last = told[-1] if told else {}
        shown = {str(display["output"]): display for display in last.get("displays", [])}
        for output, wanted in expect["displays"].items():
            if output == "trial":
                if last.get("trial") != wanted:
                    fail(f"the mode on trial told: {last.get('trial')!r}, expected {wanted!r}")
                continue
            display = shown.get(output)
            if display is None:
                fail(f"display {output} was not told (told {sorted(shown)})")
                continue
            for key, value in wanted.items():
                if display.get(key) != value:
                    fail(f"display {output}: {key} = {display.get(key)!r}, expected {value!r}")
    if "preferences_file" in expect:
        kept = {}
        for line in (preferences.read_text().splitlines() if preferences else []):
            if "=" in line and not line.lstrip().startswith("#"):
                key, value = line.split("=", 1)
                kept[key.strip()] = value.strip()
        for key, value in expect["preferences_file"].items():
            if kept.get(key) != value:
                fail(f"preferences file: {key} = {kept.get(key)!r}, expected {value!r}")
    for app, files in expect.get("app_data", {}).items():
        # What the application kept, as JSON, once the run ended.
        for name, content in files.items():
            run = preferences.parent
            data_home = run / "home" / ".local/share" if (run / "home").is_dir() else run / "xdg-data"
            path = data_home / "oxyos" / "apps" / app / name
            kept = json.loads(path.read_text()) if path.exists() else None
            if kept != content:
                fail(f"{app}'s {name} holds {kept!r}, expected {content!r}")
    if "window_rect" in expect and result.get("window") != expect["window_rect"]:
        fail(f"window at {result.get('window')}, expected {expect['window_rect']}")
    if "popup_rect" in expect and result.get("popups") != [expect["popup_rect"]]:
        fail(f"popups at {result.get('popups')}, expected [{expect['popup_rect']}]")
    if "popups" in expect:
        count = len(result.get("popups", [result["popup"]] if "popup" in result else []))
        if count != expect["popups"]:
            fail(f"{count} popups shown, expected {expect['popups']}")
    if "gpu_frames" in expect:
        got = result.get("gpu_frames", {"latched": 0, "waited": 0})
        for key in ("latched", "waited"):
            least = expect["gpu_frames"].get(f"{key}_min", 0)
            if got[key] < least:
                fail(f"{got[key]} frames of GPU buffers {key}, expected at least {least}")
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
    if "smooth" in expect:
        check_smooth(name, expect["smooth"], screenshot, fail)
    for output, pixels in expect.get("screen_pixels", {}).items():
        # Another output's colours, in its own pixels, from its own snapshot beside the primary's.
        shot = pathlib.Path(screenshot).with_suffix(f".output-{output}.png")
        check_pixels(name, {**expect, "pixels": pixels}, shot, [0, 0],
                     lambda message: fail(f"output {output}: {message}"))
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




def installed_fixture(root, installed, machine, offscreen):
    """The install service's state for this user, as `installed` describes it (see the header)."""
    import pwd
    import shutil
    user = pwd.getpwuid(os.getuid()).pw_name
    pack = pathlib.Path(offscreen).resolve().parent / "oxyos-pack"
    applications = root / "users" / user / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    for app in installed:
        package = machine.by_id[app["package"]]
        manifest = package["manifest"]
        version = root / "packages" / manifest["id"] / str(manifest["versionCode"])
        shutil.copytree(package["dir"], version)
        entry = subprocess.run([str(pack), "entry", str(version)], check=True, capture_output=True,
                               text=True).stdout
        (applications / f"{manifest['id']}.desktop").write_text(entry + "".join(f"{line}\n" for line in app.get("entry", [])))
        (root / "users" / user / f"{manifest['id']}.json").write_text(json.dumps({
            "format": 1, "id": manifest["id"], "uid": os.getuid(), "user": user,
            "current": manifest["versionCode"], "shown": False, "unshownExits": 0, "bad": [],
            "capabilities": app.get("capabilities", {})}))
    return root

class Machine:
    """The runtimes and the applications' packages a scenario runs on (ADR 0030)."""

    def __init__(self, runtimes, packages):
        self.runtimes = runtimes.resolve()
        self.packages = packages.resolve()
        # Each package by its entry's module and by its id, from its manifest.
        self.by_module, self.by_id = {}, {}
        for manifest_path in sorted(self.packages.glob("*/manifest.json")):
            manifest = json.loads(manifest_path.read_text())
            package = {"dir": manifest_path.parent, "manifest": manifest}
            self.by_module[manifest["entry"]["module"]] = package
            self.by_id[manifest["id"]] = package
        self.probes = self.packages / "probes"

    def host(self, abi):
        host = self.runtimes / str(abi) / "oxyos-bloom-host"
        if not host.is_file():
            raise SystemExit(f"runtime {abi} is not in {self.runtimes}")
        return host

    def run(self, module):
        """The host, the bundle and the script URL `module` runs with: its package's, else a probe's."""
        package = self.by_module.get(module)
        if package:
            directory = package["dir"]
            return (self.host(package["manifest"]["runtime"]["abi"]), directory / "index.hbc",
                    directory / package["manifest"]["entry"]["bundle"])
        level = json.loads((self.probes / "runtime.json").read_text())
        return self.host(level["abi"]), self.probes / "index.js", self.probes / "index.js"

    def registry(self, into):
        """The development registry (`applications/`) in `into`: each packaged application's entry the
        one oxyos-installd writes for it (`packages/applications/`, `oxyos-pack entry`), with the
        development registry's place in the dock; any other entry as it is, its icon where its is."""
        source = pathlib.Path(__file__).resolve().parent / "applications" / "oxyos" / "applications"
        target = into / "oxyos" / "applications"
        target.mkdir(parents=True, exist_ok=True)
        for entry in sorted(source.glob("*.desktop")):
            text = entry.read_text().splitlines()
            installed = self.packages / "applications" / entry.name
            if entry.stem in self.by_id and installed.is_file():
                lines = installed.read_text().splitlines()
                lines += [line for line in text if line.startswith("X-OxyOS-Pinned=")]
            else:
                lines = [("Icon=" + str((source / line[5:]).resolve()))
                         if line.startswith("Icon=") and not line[5:].startswith("/") else line
                         for line in text]
            (target / entry.name).write_text("\n".join(lines) + "\n")
        return into

def install_service(spec, machine, offscreen):
    """A real oxyos-installd for a scenario (docs/adr/0030-installing-applications.mdx), in its
    development mode (unprivileged, serving this user alone) on directories of its own: a store
    repository holding each package of `publish` (oxyos-pack publish, signed with the packages'
    development key, as the store would publish them), its fetcher, and its state, which the
    development compositor reads as the install service's (`--installed-fixture`). Its socket and
    state; stopped and removed when the run ends."""
    import atexit
    import pwd
    import shutil
    import tempfile
    tools = pathlib.Path(offscreen).resolve().parent
    pack, installd = tools / "oxyos-pack", tools / "oxyos-installd"
    # A Unix socket's path is at most 107 bytes: a short directory of the run's own.
    base = pathlib.Path(tempfile.mkdtemp(prefix="oxi", dir="/tmp"))
    keys, repository, state = base / "keys", base / "repository", base / "state"
    run = lambda *args: subprocess.run([str(a) for a in args], check=True, capture_output=True)
    run(pack, "store-keys", keys)
    run(pack, "repository", repository, keys)
    developer = machine.packages / "development.key"
    for id in spec.get("publish", []):
        run(pack, "publish", machine.by_id[id]["dir"], repository, keys, developer)
    me = pwd.getpwuid(os.getuid())
    (base / "passwd").write_text(f"{me.pw_name}:x:{me.pw_uid}:{me.pw_gid}::{me.pw_dir}:/bin/sh\n")
    logs = open(base / "installd.log", "w")
    processes = [subprocess.Popen([str(installd), "--fetch", "--development", "--repository",
                                   f"file://{repository}", "--socket", str(base / "f")],
                                  stdout=logs, stderr=logs)]
    wait_for(base / "f")
    processes.append(subprocess.Popen([str(installd), "--development", "--state", str(state), "--anchor",
                                       str(repository / "metadata/1.root.json"), "--runtimes",
                                       str(machine.runtimes), "--socket", str(base / "i"), "--fetcher",
                                       str(base / "f"), "--passwd", str(base / "passwd")],
                                      stdout=logs, stderr=logs))
    wait_for(base / "i")
    # Installed before the run, as an account that installed it earlier has it.
    for id in spec.get("installed", []):
        installd_ask(base / "i", [{"prepare": {"id": id}}, "commit"])
    # Versions the store publishes afterwards: the same package's files, its manifest's
    # versionCode, version and capabilities changed, packed and signed with the same key.
    for later in spec.get("later", []):
        source = pathlib.Path(machine.by_id[later["id"]]["dir"])
        manifest = json.loads((source / "manifest.json").read_text())
        dir = base / f"{later['id']}-{later['versionCode']}"
        shutil.copytree(source, dir, ignore=shutil.ignore_patterns("manifest.json", "manifest.sig", "index.hbc"))
        description = {key: value for key, value in manifest.items() if key not in ("files", "runtime")}
        description["versionCode"] = later["versionCode"]
        description["version"] = f"0.{later['versionCode']}.0"
        description.pop("capabilities", None)
        if later.get("capabilities"):
            description["capabilities"] = [{"name": name, "purpose": {"en": "Scenario."}}
                                           for name in later["capabilities"]]
        (base / "description.json").write_text(json.dumps(description))
        run(pack, "pack", dir, base / "description.json", machine.runtimes / str(manifest["runtime"]["abi"]) / "runtime.json", developer)
        run(pack, "publish", dir, repository, keys, developer)
    INSTALLD.update(state=state, log=base / "installd.log")

    def stop():
        for process in processes:
            process.terminate()
            process.wait(timeout=5)
        shutil.rmtree(base, ignore_errors=True)
    atexit.register(stop)
    return base / "i", state


# The run's install service (`install_service`): its state and log, for the final checks.
INSTALLD = {}


def installd_ask(socket_path, requests):
    """Requests on one connection to the install service (a plan is its connection's); "commit"
    commits the last plan. The answers, each request's last."""
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    connection.connect(str(socket_path))
    answers, plan = [], None
    for request in requests:
        if request == "commit":
            request = {"commit": {"planId": plan}}
        connection.send(json.dumps(request).encode())
        while True:
            answer = json.loads(connection.recv(1 << 20))
            if "progress" not in answer:
                break
        if "plan" in answer:
            plan = answer["plan"]["planId"]
        if "refused" in answer:
            raise RuntimeError(f"the install service refused {request}: {answer}")
        answers.append(answer)
    connection.close()
    return answers


def check_installd(wanted, failures):
    """The scenario's `installd_final`: the versions the service keeps by id, and how many times
    it read the store (checks asked at once make one)."""
    state = INSTALLD.get("state")
    if "packages" in wanted:
        kept = {}
        packages = state / "packages" if state else None
        if packages and packages.is_dir():
            for app in sorted(packages.iterdir()):
                kept[app.name] = sorted(int(v.name) for v in app.iterdir() if v.name.isdigit())
        if kept != wanted["packages"]:
            failures.append(f"the install service keeps {kept}, expected {wanted['packages']}")
    if "store_reads" in wanted:
        text = INSTALLD["log"].read_text() if INSTALLD.get("log") else ""
        reads = [int(n) for n in re.findall(r"the store read (\d+) times", text)]
        got = max(reads, default=0)
        if got != wanted["store_reads"]:
            failures.append(f"the install service read the store {got} times, expected {wanted['store_reads']}")


def wait_for(path, seconds=10):
    deadline = time.monotonic() + seconds
    while not path.exists():
        if time.monotonic() > deadline:
            raise RuntimeError(f"{path} did not appear")
        time.sleep(0.05)


def install_arguments(scenario, out, machine=None, offscreen=None):
    """System grants (`grants`: {ID: [GRANT, ...]}) as the image's policy gives them, and the
    install service the session asks (`installd`: null for a socket no service is at, or
    {"publish": [ID, ...]} for a real one, `install_service`), docs/adr/0030-installing-
    applications.mdx."""
    arguments = []
    for app, grants in scenario.get("grants", {}).items():
        arguments += ["--grant", f"{app}={','.join(grants)}"]
    if "installd" in scenario:
        spec = scenario["installd"]
        if spec is None:
            # A socket no service is at (a short path: a Unix socket's is at most 107 bytes).
            arguments += ["--installd", "/nonexistent/oxyos-installd.socket"]
        else:
            socket, state = install_service(spec, machine, offscreen)
            arguments += ["--installd", str(socket), "--installed-fixture", str(state)]
    return arguments


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("scenario")
    parser.add_argument("--offscreen", required=True)
    parser.add_argument("--runtimes", help="the runtimes (every scenario but a `client` one): DIR/<abi>/")
    parser.add_argument("--packages", help="the applications' packages, and probes/ (sample/packages.mjs)")
    parser.add_argument("--client", default=None,
                        help="the native OxyOS client a scenario with `client` runs instead of the Bloom host")
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
    if "client" in scenario:
        if not options.client:
            raise SystemExit("a scenario with `client` needs --client: the program it runs")
        if scenario.get("shell") or "login" in scenario:
            raise SystemExit("a `client` scenario runs one native client: no `shell` or `login`")
    elif not options.runtimes or not options.packages:
        raise SystemExit("--runtimes and --packages are needed: the runtimes and the applications")
    if "client" not in scenario:
        machine = Machine(pathlib.Path(options.runtimes), pathlib.Path(options.packages))
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
    if scenario.get("outputs"):
        # Several outputs (ADR 0018), a shell on each.
        command += ["--outputs", scenario["outputs"]]
    if scenario.get("displays"):
        # The session's displays (ADR 0018), on virtual connectors described in a file of the run's
        # own: identity, kind, size in mm, modes (the preferred one marked), connector and name.
        displays = out / "displays"
        displays.write_text("".join(
            " ".join([display["identity"], display["kind"], "x".join(map(str, display["size_mm"])),
                      ",".join(display["modes"]), display["connector"]]
                     + ([display["name"]] if display.get("name") else [])) + "\n"
            for display in scenario["displays"]))
        command += ["--displays", str(displays)]
    if scenario.get("shell"):
        # The session's shell (ADR 0009), its package's Bloom host and bundle started before the
        # application with the shell grant; its report is the application's with ".shell" (another
        # output's shell's with ".shell.ID").
        shell_host, shell_bundle, _ = machine.run(scenario["shell"])
        command += ["--shell", f"{shell_host},{shell_bundle},{scenario['shell']}"]
    # The user's preferences (ADR 0019), in a file of the run's own: light unless the scenario says
    # otherwise, so what a scenario sees never depends on the time of day it runs at (an automatic
    # appearance follows the sun). With `settings`, the application the session would grant it to.
    preferences = out / "preferences"
    given = {"appearance": "light", **scenario.get("settings", {}).get("preferences", {})}
    preferences.write_text("".join(f"{key} = {value}\n" for key, value in given.items()))
    command += ["--preferences", str(preferences)]
    if "settings" in scenario:
        command += ["--grant-settings", scenario["settings"]["app"]]
    command += install_arguments(scenario, out, machine if "client" not in scenario else None, options.offscreen)
    if "controls" in scenario:
        # The machine the hardware keys and the Control Center act on (ADR 0025), played as the
        # scenario describes it, in a file of the run's own the session writes back.
        controls = out / "controls"
        controls.write_text("".join(f"{key} = {str(value).lower() if isinstance(value, bool) else value}\n"
                                    for key, value in scenario["controls"].items()))
        command += ["--controls", str(controls)]
    if scenario.get("installed"):
        command += ["--installed-fixture", str(installed_fixture(out / "installed", scenario["installed"],
                                                                  machine, options.offscreen))]
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
    login = None
    if "login" in scenario:
        # A login screen: no application, its passwords the login program's.
        login = login_program(out, options.offscreen, scenario["login"])
        command += ["--login", str(login[1])]
    for step in scenario["steps"]:
        command += director_args(step)
    if "client" in scenario:
        # A native OxyOS client that is not a Bloom application (ADR 0029: Astro, through Chromium's
        # Ozone platform `oxyos`), with its arguments: {out} is the run's directory, {scenarios} this
        # file's, {width} and {height} the scenario's window.
        values = {"out": str(out), "scenarios": str(pathlib.Path(options.scenario).resolve().parent),
                  "width": str(width), "height": str(height)}
        command += ["--", options.client] + [argument.format(**values) for argument in scenario["client"]]
    elif login is None:
        app_host, app_bundle, app_script = machine.run(scenario.get("module", "OxyBloom"))
        command += ["--", str(app_host), str(app_bundle), scenario.get("module", "OxyBloom"), "--window", str(width),
                    str(height)]
    # Steps checked one by one below: refuse a scenario whose steps the director would not run.
    if not all(isinstance(step.get("do"), list) for step in scenario["steps"]):
        raise SystemExit("every step needs `do`: [kind, value]")
    env = dict(os.environ, OXYOS_HOST_REPORT=str(report_path))
    # glibc fills what it frees (and what it hands out) with this byte, so a use of freed memory
    # crashes the run every time instead of only when the memory happened to be reused: a host
    # destroying JavaScript's pending timers after its runtime crashed one exit in tens, and with
    # this every exit that does it.
    env["MALLOC_PERTURB_"] = "165"
    if "client" not in scenario:
        # The runtimes the applications' entries name (ADR 0030), the scenario's in place of the image's.
        command[2:2] = ["--runtimes", str(machine.runtimes)]
        if login is None:
            env["OXYOS_SCRIPT_URL"] = f"file://{app_script}"
    # What applications keep (their data and cache directories, application.h) is the run's own,
    # never the user's; a scenario's `app_data` is there before the application starts.
    for kept in ("xdg-data", "xdg-cache"):
        shutil.rmtree(out / kept, ignore_errors=True)
    env["XDG_DATA_HOME"] = str(out / "xdg-data")
    env["XDG_CACHE_HOME"] = str(out / "xdg-cache")
    if scenario.get("shell"):
        # The shell's applications are the development registry's alone (ADR 0013), not the
        # machine's, each packaged one from its package.
        env["XDG_DATA_DIRS"] = str(machine.registry(out / "registry"))
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
    if login is not None:
        env.update(login[3])
    home_root = None
    if "home" in scenario:
        # A home of the run's own (docs/adr/0028-file-access.mdx), never the developer's.
        home_root = make_home(out, scenario["home"])
        env["HOME"] = str(home_root)
        env["XDG_DATA_HOME"] = str(home_root / ".local/share")
        env["XDG_CONFIG_HOME"] = str(home_root / ".config")
        env["XDG_CACHE_HOME"] = str(home_root / ".cache")
    # Its data directory is under the run's XDG_DATA_HOME, its home's in a scenario with one.
    for app, files in scenario.get("app_data", {}).items():
        directory = pathlib.Path(env["XDG_DATA_HOME"]) / "oxyos" / "apps" / app
        directory.mkdir(parents=True, exist_ok=True)
        for name, content in files.items():
            (directory / name).write_text(json.dumps(content))
    if "locale" in scenario:
        env["LANG"] = env["LC_ALL"] = scenario["locale"]
    if "tz" in scenario:
        env["TZ"] = scenario["tz"]
    recorder = None
    if home_root is not None:
        report_path.unlink(missing_ok=True)
        change_home_later(home_root, scenario["home"], report_path)
        recorder = HomeRecorder(home_root)
    run = subprocess.run(command, env=env, capture_output=True, text=True)
    if recorder is not None:
        recorder.stop()
    (out / "director.log").write_text(run.stdout + run.stderr)
    recorded = []
    if login is not None:
        login[0].terminate()
        login[0].wait(timeout=20)
        recorded = login[2].read_text().splitlines() if login[2].exists() else []
        shutil.rmtree(login[4][1], ignore_errors=True)
    if run.returncode != 0:
        print(f"FAIL: the director failed ({run.returncode}); see {out / 'director.log'}")
        print("\n".join(run.stderr.strip().splitlines()[-5:]))
        return 1
    summary = json.loads(next(line for line in run.stdout.splitlines() if line.startswith('{"mode"')))
    # A login screen has no application, and so no application's report.
    report = ([json.loads(line) for line in report_path.read_text().splitlines() if line.strip()]
              if report_path.exists() else [])
    shell_path = report_path.with_name(report_path.name + ".shell")
    shell_report = ([json.loads(line) for line in shell_path.read_text().splitlines() if line.strip()]
                    if shell_path.exists() else [])

    speech_path = out / "speech.jsonl"
    spoken = ([json.loads(line) for line in speech_path.read_text().splitlines() if line.strip()]
              if "reader" in scenario else None)

    failures = []
    start = summary["t_launch"]
    refresh_hz = display_hz(scenario)
    for index, (step, result) in enumerate(zip(scenario["steps"], summary["steps"])):
        start = check_step(index, step, result, report, start, failures, options.uinput,
                           out / f"launch-{index + 2}.png", shell_report, spoken,
                           out / "preferences", refresh_hz, recorder)
    # A frame the host drew from its damage that differs from the frame drawn whole.
    for line in report:
        if line["kind"] == "damage_mismatch":
            failures.append(f"the host's frame drawn from its damage differs in {line['pixels']} pixels")
    if login is not None:
        # What the login program would have started, in order: each line begins as expected.
        wanted = scenario["login"].get("record", [])
        if len(recorded) != len(wanted) or any(not line.startswith(w) for line, w in zip(recorded, wanted)):
            failures.append(f"the login program recorded {recorded}, expected lines beginning {wanted}")
        password = scenario["login"]["password"]
        if any(password in line for line in recorded):
            failures.append("the password is in the login program's record")
    if "installd_final" in scenario:
        check_installd(scenario["installd_final"], failures)
    if "applications_launched" in scenario and summary.get("applications") != scenario["applications_launched"]:
        failures.append(f"the shell started {summary.get('applications')}, expected exactly "
                        f"{scenario['applications_launched']}")
    if "final" in scenario:
        last = max(line["t"] for line in report + shell_report)
        # The output after the last step (or at launch, without steps), the window where it was then.
        window = (summary["steps"][-1] if summary["steps"] else summary)["window"]
        check_step(len(scenario["steps"]), {"name": "final", "do": ["final"], "expect": scenario["final"]},
                   {"t_settled": last, "window": window}, report, start, failures,
                   screenshot=out / f"launch-{len(scenario['steps']) + 1}.png", shell_report=shell_report,
                   preferences=out / "preferences", refresh_hz=refresh_hz, home_root=recorder)

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
