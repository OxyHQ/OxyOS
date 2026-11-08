#!/usr/bin/env python3
"""Regression scenarios for the native Bloom host on the offscreen compositor.

Runs `oxyos-offscreen --launch` with a scenario's steps, with the host writing its report
(OXYOS_HOST_REPORT), then checks every step's EXPECTED result, not only that pixels changed:

- texts / absent: text the application shows (or must not) once the step settled, across its
  window and popups (each surface's latest presented frame); texts_matching, regular expressions
  each some shown text matches (re.search): what follows the wall clock (the top bar's clock, in
  the locale's format);
- focus_input: the text input holding focus (null for none), from the latest frame;
- focus_surface: the application surface with the compositor's keyboard focus;
- keyboard_focus: the control the keyboard's navigation focused (Tab, docs/adr/0039-keyboard-access.mdx)
  as the host last moved it: null for none, else {"target": testID, "ring": whether its ring shows,
  "color": the colour it was last drawn in, "#rrggbb", the session's accent's};
- react: the exact events delivered to React during the step, as "event@name" -> count, where
  name is the target's testID or the nearest ancestor's (`within`); a scroll view without a
  testID (a sheet's, a sidebar's) is its component in that ancestor, "ScrollView in settings-rail",
  never its tag, which every view mounted before it moves; two scroll views under one name in a
  step fail it (as in scroll_offset, scroll_moved and glide). Any delivered event the step does not list fails it, which catches
  duplicated and leaked events; a count may be [least, most] for events one a frame (a glide's
  scroll events), and [0, most] for events that may not come;
- delivered: compositor input routed to the application per surface (exact), plus `leaked`
  (what the session's shell takes is its own, never leaked),
  input routed to any other client; and, given `launched`, what the applications the shell
  launched received, by kind (exact; "close.requested" among them; a list of them: any one, for
  an order of two clients' requests neither decides);
- popups: how many popups of the application are shown after the step; popup_rects, where each
  is on the output ([x, y, width, height]), exactly and in the order they opened (a submenu's
  popup beside its menu's);
- gpu_frames: the application's frames of GPU buffers (DMA-BUFs, docs/adr/0031-gpu-buffers.mdx)
  the compositor latched since the run began, at least "latched_min", and at least "waited_min"
  of them held until their acquire point signalled: frames that took the GPU path, not shared
  memory;
- menus: the window's menus as the shell is told them (ADR 0021): null, or by an entry's path
  ("View > Appearance > Dark") the keys given (shortcut, disabled, checked), null for an entry that
  must not be there, and "bar", the menus of the bar exactly; in `in_companion`, the companion's
  window's. A `menu` step chooses a command of the active window's menus by its path, as the
  shell delivers the person's choice;
- pointer_image: the client's image the pointer shows (ADR 0029), each key given matching (width,
  height, hotspot, scale_120, middle as "#aarrggbb"), or null for a shape;
- clipboard: what the clipboard holds once the step settled, as the session reads it from its
  owner through a pipe as any receiver pastes (ADR 0007): null, or "types" exactly and by type its
  text, {"contains": text}, or {"bytes": N, "png": [w, h]} for other bytes. A `clipboard` step
  ({type: content, "base64:..." for bytes}) is another application copying;
- chooser: with the scenario's `file_chooser` module run as the file chooser the session starts
  (ADR 0029, decision 8), its sheet once the step settled: null, or each key given matching
  ("sheet": [x, y, width, height] on the output, "focused": it has the keyboard, "window_active":
  the application's window is the active one); `in_chooser` checks its own texts and React
  deliveries, as `in_shell` the shell's, and `delivered`'s "chooser" what it was delivered;
- companion: with the scenario's `companion` ({"module", "window", "at", "app": the id it claims,
  the application's own making both windows one application's}) started as a second application
  once the first's window is shown (ADR 0035, the other side of a drag between applications),
  `in_companion` checks its own texts and React deliveries, `delivered`'s "companion" what the
  compositor delivered it, and "shell_drags" the drag events the shell's surfaces received;
- shell: with the scenario's `shell` module run as the session's shell (ADR 0009), its surfaces
  ("surfaces": output rectangles, compared as a set; "top": the topmost), the "work_area"
  they leave on the primary and "work_areas" on every output (by id), exactly, whether the
  session is "locked" (ADR 0010), and "candidates", where the compositor shows the shell's
  candidate window (ADR 0041: [x, y, width, height] on the output, or null while hidden);
- at / at_absent: nodes the session's assistive technology holds (or must not), as it received
  the trees (ADR 0003): by "window", "shell" (the shell's surfaces, the lock screen's among
  them), "launched" (the applications the session launched, docs/adr/0039-keyboard-access.mdx)
  or the application's popups, each node matching the keys given ({} matches any): role,
  name, value, state, actions (what it offers beyond activate and focus), custom (its custom
  actions' labels), selection, hint (what activating it does, ADR 0039) and position ([place,
  count] among its collection's items, a list's row; null for none);
- at_group: groups in the window's tree as the assistive technology received it, nested as the
  application nested it (each node's parent): each {"group": node, "items": [node, ...]} has
  exactly one node matching "group", and each item matches exactly one node in the whole window,
  nested in that group (at any depth), the items in order, each to the right of or below the last,
  inside the group's rectangle: one control, never an empty group beside loose buttons, nor a second copy
  elsewhere. With "only": true the buttons the group holds are exactly its items;
- at_hit: what a press lands on, by the tree's geometry: "x,y" (the window's coordinates, as
  `press` takes them) -> the node matching the keys given is the innermost one whose rectangle holds it;
- announcements: what the assistive technology would have said in the step, exactly and in
  order: an application's announcements and live regions' changes, each matching the keys
  given (politeness, text, live);
- spoken: with the scenario's `reader`, what the screen reader said in the step (ADR 0014),
  exactly and in order: each a text, or keys matching an utterance (text, interrupting, voice:
  the eSpeak NG voice it was said with, `en` or `es`, the session's language's, ADR 0040); in a
  text, here and in `at`'s, "{clock}" is the shell's clock as said when it was said (`clock_said`,
  in the scenario's language: "martes, 3 de noviembre, 14:05", "Tuesday, November 3, 2:05 PM", the
  minute it was said or the one before);
- shaken: a view (by testID) committed left and right of its place in the step, and back at the
  end: a wrong password's field shaking;
- output_pixels: as pixels, in output coordinates rather than the window's (the primary's);
  output_pixels_any, a list of such maps of which one holds at every point (one of a folder's
  photos, whichever a shuffle picked);
- smooth: a region of the output ("region": [x0, y0, x1, y1] in its pixels) showing a smooth
  gradient without banding: neighbouring pixels differ by at most "max_step" levels (2), and down
  its columns a level holds for at most "median_run_max" rows (3) as the median of its runs: a
  gradient quantized to 8 bits without dithering holds each level for a band of rows;
- screen_pixels: with the scenario's `outputs`, each output's by id: "x,y" in its own pixels
  (at its scale) -> "#rrggbb";
- window_shown: whether the application's window is on the screen (a minimized one is not);
- minimize_target: where the application's window minimizes into, as the shell's dock says it
  (ADR 0011's genie): [x, y, width, height] in the global space (logical pixels), or null where
  no shell surface shown says, exactly;
- sounds: the system sounds the session played in the step (docs/adr/0044-system-sounds.mdx),
  exactly and in order, by their names in the sound theme (`file-trash`, `trash-empty`,
  `message-new-instant`, `screen-capture`): recorded as the session would play them, after the person's settings and
  the output's mute; a step that plays one and does not say so fails;
- files_opened: the files the shell had opened with an application in the step (ADR 0034,
  OpenFiles), exactly and in order, each {"name": the file's name, "application": its id};
- urls_opened: the URLs the shell opened in the step (ADR 0034), and those applications opened (the
  platform's Settings, `oxyos-settings:`, docs/adr/0019-settings.mdx), exactly and in order, each
  {"url", "application"}: the application started for it ("{home}" in a URL being the run's home's
  file:// URI as encodeURIComponent writes it, a desktop folder opened in Dropsea, ADR 0047); a step
  that opens one and does not say so fails;
- signed_out: whether Settings signed out in the step (`{"signOut": {}}`,
  docs/adr/0020-oxy-account-login.mdx): the session would end there, back to the login screen; the
  development compositor records it and goes on. A step that signs out and does not say so fails;
- screenshots: with the scenario's `screenshots` (files written as the session writes them, in the
  run's own Desktop and Documents, docs/adr/0024-system-shortcuts.mdx), the files saved in the
  step, exactly and in order ("saved": each its "name" as a regular expression, "folder", "output",
  "width", "height", "alpha" and "pixels" read from the PNG, "x,y" -> "#rrggbb" or "#rrggbbaa"),
  and the screenshots the shell opened ("opened": each the "application" started for it); a step
  that saves one and does not say so fails;
- network: with the scenario's `network` (a NetworkManager played by oxyos-fake-networkmanager on a
  bus of the run's own, docs/adr/0038-networks.mdx), what the daemon holds once the step settled
  (its State: "wifi_enabled", "active", "connections" with each one's "ssid", "psk" and
  "permissions", "attempts", "scans", "ethernet"), each key given matching: a dict key by key, a
  list item by item and of the same length;
- sound: with the scenario's `sound` (a machine's outputs and inputs played by the development
  compositor, docs/adr/0056-sound-devices.mdx: `outputs`, `inputs`, `configured_outputs`...),
  what it is once the step settled ("default_output", "default_input", "configured_outputs",
  "metering", each device's "volume", "balance" and "muted", and "calls", every change the
  session made, in order), each key given matching: a dict key by key, a list item by item and of
  the same length;
- bluetooth: with the scenario's `bluetooth` (a BlueZ played by oxyos-fake-bluez on a bus of the
  run's own, docs/adr/0055-bluetooth.mdx), what it holds once the step settled ("adapter" with its
  "powered", "discovering", "discoverable", "discovery_clients"; "agent" with its "capability" and
  "default"; "devices" by address, each "present", "paired", "trusted", "connected", "alias",
  "battery"; "log", the calls made of it in order), each key given matching as `network`'s;
- displays_off: whether the displays are off for want of input once the step settled (ADR 0057);
- power: with the scenario's `power` (UPower, power-profiles-daemon and logind played on a bus of
  the run's own for the machine it describes, oxyos_power::played, docs/adr/0057-battery-and-
  energy.mdx), what they hold once the step settled ("on_ac", "battery" with its "percent",
  "state" and "charge_limit", "sysfs" with the battery's "start" and "end" thresholds as UPower
  wrote them, "active_profile", "holds", "suspends", "calls"), each key given matching;
- controls: with the scenario's `controls` (a machine the session's hardware keys and Control
  Center act on, ADR 0025: `output_volume`, `display_brightness`, `battery`, `wifi`, `ssid`...),
  the machine as it is once the step settled, each key given matching;
- animated: what Animated's native driver set on views in the step (native-animated.h), frame by
  frame, as "testID" -> {prop: [from, to]} ("transform.translateX" for a transform's entry; a
  colour as "#rrggbb"): the values went from `from` to exactly `to`, monotonically in each colour
  channel, over at least `animated_frames` frames (8 by default); with `animated_linear_ms`, every
  frame's value is where a linear animation of that duration is at that frame's time, within one
  frame of the display (the scenario's refresh: see `outputs`). Given three values [from, peak,
  to], the values went from `from` to exactly `peak`, monotonically, then from there to exactly
  `to`, monotonically (a view scaled up and back: the dock's Trash under a drag). A view it does not list must not move: the driver may set it (a node's first frame sets
  its start), but always to the same values. `animated_frames` may be [least, most], or such bounds by view ({testID: bounds});
- committed: the same for what React's commits changed in the step (opacity, translation, scale
  (`transform.scale`), the top corners' radii, `borderTopLeftRadius` and `borderTopRightRadius` as drawn,
  and `width` as laid out (a dock's place opening as icons slide apart), of views with a
  testID, as mounted: a JavaScript-driven Animated's and Reanimated's frames among them), with
  `committed_frames` and `committed_linear_ms`; views it does not list are not checked;
- animation_frames: requestAnimationFrame's frames in the step (animation-frames.h): "count",
  [least, most]; "interval_ms", the least time between two; "median_interval_ms", [least, most]
  for the median time between them (a frame of the display: 8.3 at 120 Hz); their timestamps rise
  and are performance.now()'s clock (within a millisecond of the report's, CLOCK_MONOTONIC);
- window_views: the windows the shell's surfaces show live once the step settled (ADR 0022), by
  their numbers: "windows", exactly those; "moved", each moved into its view (App Exposé, ADR 0045)
  or each a copy; "apart", no two overlapping; "within", each inside [x, y, width, height] (logical
  pixels of the global space); "below", those windows each below every other one shown (App
  Exposé's minimized row);
- flights: the windows that flew to their views and back in the step (ADR 0045): "arriving" and
  "going", exactly those windows; "none", nothing flew; and always, each flight in more than one
  frame, every edge only ever going toward its end, an arriving one beginning where its window is
  and ending exactly in its view, a going one beginning in its view's place the step before and
  ending exactly where its window is (its last frame the one that ended it, at its end, whatever
  frames were dropped before);
- window_states: every window as the shell is told of it once the step settled, by its number,
  exactly those: each key given of "app", "workspace", "minimized", "hidden" and "focused"
  matching (the dock's press, docs/adr/0034-the-dock.mdx, decision 9);
- windows_placed_as: every window where it was once step N settled (1-based): nothing of App
  Exposé's moved a window;
- swiped: a share (0 to 1) of a swipe's way (ADR 0046): every window moved into a view drawn,
  in the frame composed last, exactly that share of the way from where it is to its view, edge by
  edge (within a pixel), as the fingers have it;
- slid: [position, width]: the desktops slid to `position` (in desktops) by a swipe across, every
  window drawn where it is moved by (its desktop - position) × width, within a pixel;
- drawn_home: every window drawn exactly where it is (nothing flying, nothing slid);
- workspaces: [count, active] once the step settled;
- transitions: what the compositor animated in the step (ADR 0011): "frames_min", the fewest
  frames composed while a transition ran; "seen", the transitions it must have run, each matched
  by a distinct one having every key given ("kind": window, popup, overlay, backdrop, panel,
  notification, lock; "change": arrived, gone, minimized, restored, workspace_in_forward,
  workspace_in_back, workspace_out_forward, workspace_out_back; "arriving"; "first" and "last",
  the opacities of the first and last frames drawn; "ended"), and its curve, from each frame it
  drew: "eased", its opacity and its size and place each only ever going toward their end and,
  in the first half of its time, half again ahead of a linear motion of the same length (eased
  out, as a spring from rest is; a linear one fails), or with "eased": "place" only its size and
  place so (its opacity only never going back: a fade delayed to its end); "lasts_ms", [least, most] for how long it
  lasts, judged by time, not by its frames: when its clock ends it (`ends_ms`), every frame drawn in it before then (or
  held at its end: a zoom waiting for its window's new frame) and the frame that ended it (`ended_ms`, drawing it at
  its end) at or after, however many were dropped;
  "frame_gap_ms", [median, most] for the time between its frames (at display rate: 16.7 at 60 Hz,
  8.3 at 120); "linear_ms", every frame's size and place where a linear motion of that length is
  at the frame's time (a window into the dock: the frame clock drives it, whatever the load);
  "refresh_frames", [least, most] it drew, the most always (one a refresh at most, which no load adds) and
  the least only with --speed-budgets, each frame at a later time and a further place than the
  one before; "still",
  its size and place never changing (reduce motion's crossfade);
  "count", how many it ran, exactly ("change" is also "zoomed": a window's frame zooming, ADR 0016);
- drag_icons: the drag icon surfaces of every client once the step settled (ADR 0035): "count",
  how many there are, exactly, and "shown", where each shown one is on the output ([x, y, width,
  height] in the global space's logical pixels), exactly and in order: a drag shows one, and none
  is left once it ended. A transition `seen` of a drag's image going back ("kind": "dragicon",
  "change": "returned") has "to", the point its centre goes back to (the item's own place);
- drag: for a `drag_paced` step (a press moved on the output in N motions at a rate, the compositor
  serving the application between them, as a hand drags a window's edge): "unanswered_max", the
  most configures sent that no frame shown answered; "presented_min", the fewest frames the window
  showed; "bounded", no more configures than frames shown; "steady", the window's edges ("left",
  "top", "right", "bottom") that must not move in any frame shown; "composed_min", the fewest frames
  composed in the drag (a window moved is a frame for each motion, or for each refresh of a paced
  output slower than the hand; with --speed-budgets, else two); "composed_gap_ms", [least, most]
  for the median time between the frames composed while the hand moved (a paced output's refresh;
  with --speed-budgets); "bytes_read_max", the most
  of the application's pixels the compositor may read in the drag (0 for a window moved: it shows
  nothing new);
  and always, no frame composed in the drag drawing the window at another size than its
  buffer's ("scaled");
- resize_work: what a window resizing cost the application's host, from its report's frames in
  the step: "react_commits_max", the most commits of React's (re-renders; a resize's own layout is
  native and is none) in the whole step, one for each threshold the window's size crosses that
  changes what the application shows; "js_ms_p95_max", the most JavaScript a frame may run on the
  host's CPU at the 95th percentile (the events a resize delivers, onLayout and the dimensions,
  and nothing they re-render); "busy_refreshes_p95_max", the most of a refresh of the display
  (`display_hz`) a frame's whole work may take at the 95th percentile (both budgets on speed, held
  only with --speed-budgets); "zoom_presented_ms_max", for a zoom (the
  green bubble), the most time from its being asked to its frame presented, as the host's
  diagnostics log it ("zoom (…): its configure … its frame presented N ms after");
- stretch: a scroll view ("view", by testID) stretched past an end in the step, as Android 12's
  ScrollView stretches (EdgeEffect's stretch, overscroll in input.h), from the host's report of
  each change of the stretch drawn along its axis ("axis", "y" by default; a share of its size,
  negative past its start): "none", that it never stretched; "pulled", how far it is pulled at the
  step's end, a share of its size, and "held", the stretch drawn then, worked out here again from
  that distance as EdgeEffect's dampStretchVector works it out (within 1e-4); "peak", [least,
  most] for the largest stretch drawn (by magnitude, signed by "side", -1 or 1); "rests", that it
  went back to none by the step's end, only ever shrinking once it began to (a spring from rest
  that does not overshoot), in at least "frames_min" changes; "caught_ms", that it was held still while it
  showed for at least that long (fingers resting on the touchpad catching it as it sprang back);
- scrollbars: the scroll views' overlay scrollbars (scrollbars.h) changing in the step, as the host
  reported each change, counted exactly as "event@testID": "shown" as they appear, "wide-x" or
  "wide-y" as the pointer over one brings it wide, "hidden" once faded out; "scrollbar_thumbs",
  where each view's thumbs were in the window (logical pixels) as last reported in the step,
  {testID: {"x"|"y": [x, y, width, height]}};
- glide: for a `flick` step (a `scroll_paced` whose fingers then lift), the scroll view
  ("view", by testID) the fingers flung, as Android's ScrollView flings: the velocity the host
  measured at the lift ("velocity": [least, most] in logical pixels a second, along the offset);
  the fling's distance, worked out here again from that velocity and the friction as Android's
  SplineOverScroller works it out (within half a pixel of the host's); the view's offset from the
  drag's end to the glide's end going only that way and covering that distance (or stopping at the
  end it reaches), within a pixel, over at least "frames_min" frames; and its events in order:
  scrollEndDrag, momentumScrollBegin, the glide's scrolls, momentumScrollEnd. With "held" (a
  `flick_hold` step: fingers coming to rest on the touchpad while it glides), the glide instead
  stops where the resting fingers find it, as macOS stops a scroll under resting fingers: short of
  the fling's distance, its momentumScrollEnd once, in the frame the host handled their hold's
  begin (`hold.begin`), and no scroll after it;
- scroll_times: "dispatch": every scroll event React received in the step (scroll, the drag's and
  the momentum's) carries as its `timestamp` when the host dispatched it, within 2 ms, as React
  Native's Android ScrollEvent takes `SystemClock.uptimeMillis()` when it is obtained: never the
  device's time of the input behind it, nor nothing;
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
- transactions: the text-input transactions the application was given in the step (ADR 0008),
  exactly and in order, each {"delete": [before, after], "commit", "preedit", "cursor": [begin,
  end], "segments": [[begin, end, "composing"|"converting"|"error"]]}, its parts left out empty;
- im: with the scenario's `input_method`, what the session's input method was told of the field
  and the keys it received in the step (active, purpose, text, cursor, keys, given_back,
  composing...); with `im_candidates`, the page of candidates it hands the system ("candidates":
  {page, highlighted, list}, null while none) and the shell's presses in the step ("chosen": the
  candidates' indices, "turned": each page asked for, forward or not);
- input_sources: with the scenario's `input_sources` (below), the source in effect once the step
  settled ("current"), the sources chosen in the step ("selected", in order: the shell's top bar's
  `selectInputSource`) and the emoji pickers asked for ("emoji", `showEmoji`), each key given
  matching;
- latency_ms: budget from injecting the input to the first changed frame;
- pixels: colours of the output once the step settled, "x,y" (window-local logical pixels at
  scale 1) -> "#rrggbb", each channel within `pixel_tolerance` (3 by default); "!#rrggbb", any
  colour further than that from it.
- pixels_match: pairs of points, as `pixels` gives them, that show the same colour (each channel
  within `pixel_tolerance`): an edge drawn one way matching the same edge drawn another (a child
  clipped to a rounded box and a rounded background, anti-aliased alike); `output_pixels_match`
  the same in output pixels, as `output_pixels` gives them;
  (Pointer enter/leave, which follow every motion, are left out of `react`, and so are the W3C
  pointer presses, every mouse button's pointerdown and pointerup;)
- react_presses: those presses, exactly, as `react` checks its events;
- diagnostics: the application's frame diagnostics in the step (frame-diagnostics.h), from its
  report's `timing` and `log` lines: "slow", [least, most] slow lines written to the log;
  "slow_frames", [least, most] frames (or work that drew nothing) slower than a refresh;
  "slow_line", a slow line written in the step having each key given ("subject", a pattern it
  matches whole; "javascript_min", "work_min", "dropped_min"; "skipped", the slow frames it says went
  unsaid); "summaries", [least, most] summaries written; "fast_frames_min", the fewest frames
  presented within a refresh; "logged", patterns (regular expressions) each
  matching a line written in the step. Every run also checks, for each host's report, that every
  line its diagnostics wrote is in the log (the hosts' stderr), that a frame is slow exactly when
  its work (and its wait beyond a refresh) passes a refresh, that a slow frame's line names the
  work its `timing` line has, and that each summary's numbers are those of the frames presented
  since the summary before;
- js_work: what the host's JavaScript cost in the step, from its report's `timing` lines (a frame's
  work, and work that drew none when it ran JavaScript): "js_ms_max", the most JavaScript any one
  of them ran on the host's CPU (its thread's CPU time, which the machine's load does not
  lengthen; an event the shell is told that changes nothing it shows, another window's zoom,
  runs next to none); "js_blocked_ms_max", the most time any one of them was blocked while running
  JavaScript (waiting for another thread, a lock, a file), both budgets on speed held only with
  --speed-budgets; "presented", [least, most] frames presented (0: the step changed nothing
  the host draws); "js_wall_ms_max", the most JavaScript any one of them ran by the wall clock, a
  backstop for a gross regression set far above what the work takes. In `in_shell`, the shell's;
- renders: what React re-rendered in the step, from the host's report's `react_render` lines (each
  commit's components where a re-render began, with the components each rendered; report.h), a
  count of work the machine's load cannot change: "tops", exactly the components the step's
  re-renders began at, in any order ([] for none: nothing re-rendered); "components_max", the most
  components they may render in all; "each_minute", what the wall clock's minute turning renders
  (the shell's clock), not counted when a commit of exactly that comes in the two seconds after a
  minute began. In `in_shell`, the shell's;

A step with `until_sheet` asks the session for something it confirms in a system sheet (ADR 0030):
it settles once the session has a sheet waiting and the shell shows a surface it did not show before
the step, however long after the step's first answer that is (an update is staged, downloaded and
verified, before its consent is asked), never after a set time.

A step with `until_named` ([name, ms]) is answered on the application's own clock (an install the
person confirmed in a system sheet, done once the machine has fetched and compiled it): it settles only
once the assistive technology holds a node so named on a shown surface (as `wait_named` waits, within
`ms`), so that everything said on the way is the step's own, never split by when compiling ends.

A `strokes` step types keys as a person does on the session's keyboard (a list of codes or chords,
[40, 18] being ´ then e on the Spanish layout): through the session's keymap (the scenario's
`layout`) and compose table (the image's, `config/`), so a dead key composes as on the machine.
A step's `host_input` expectation counts what the host reported handling in it, by name
({"caret.dark": 10}).

A step's `expect_devices` replaces expectations of its `expect` when its input comes through the
virtual devices (`--uinput`, or a scenario's `uinput`: true, which runs only that way and as
root, the devices being the kernel's), where a keyboard types one key at a time and a wheel scrolls one
notch at a time: what React and the client receive is then counted per key and per notch. A
`scroll` step ([x, y, dy] a wheel's notches, [x, y, dy, events] a touchpad's two fingers in that
many events, then lifted) is then the hand's, as `scroll_named` and `scroll_output` are: the
wheel turned toward the person, or the fingers moved down, for a positive dy, which libinput turns
as the session's natural scrolling says (docs/adr/0042-scrolling.mdx, decision 6); a step's
`scroll_input` ({"way": "down" or "up", "events": [least, most]}) checks the scrolls the host
received in it, every one that moved (a touchpad's lift moves nothing) that way (down: its dy
above 0, the view moving down) and that many of them (in `in_shell`, the shell's: the dock's
icons', ADR 0045), and `scroll_moved` ({testID: "down" or
"up"}) that each scroll view's offset only went that way in it, and went. A
scenario's `layout` (an XKB layout, `us` by default) is the one those expectations are written
for, and the devices type with it. A `commit` step is an input method's committed text, which
no keyboard types, so it is the same on both paths; so is a `transaction` step, an input
method's whole change (ADR 0008).

A scenario with `input_methods` runs the session's input methods (ADR 0041): the real engine
(fcitx5 and its engines, `tools/input-methods/build.sh`, given with `--input-methods PREFIX` or
`OXYOS_INPUT_METHODS`), with the scenario's `preferences` choosing the input sources; `strokes`
steps type through it as the seat's keyboard does.

A scenario with `input_method` runs with an input method the session plays: `keyboard` steps type
through it (it gives the keys back), `compose_keys` steps have it compose them (Enter commits),
and `im_change` steps send a change of its own, for the field's latest state or (`stale`) the one
before. With `im_candidates` ("horizontal" or "vertical") it also hands the system candidates for
what it composes (ADR 0041: twelve, five a page, which the session's shell draws at the caret):
`strokes` keys reach it as the keymap types them, the arrows moving the highlight along the list
and turning the page across it, a digit choosing that candidate of the page, Space the highlighted
one, Escape cancelling; the shell's presses choose one or turn the page.

A `drag_hold` step ([x0, y0, x1, y1, motions, hz]) is a `drag_paced` whose button stays held: the
drag goes on into the steps after it, each step's frame showing it midway (`move_output` moves it,
`key_down` holds a modifier that changes its action), until a `release_output` ([x, y]) moves the
pointer there and releases it, dropping it or ending it over nothing.

A `swipe` step ([fingers, dx, dy, steps]) is three or four fingers swiped on the touchpad, the
system's (Mission Control up, App Exposé down, ADR 0045); a `scroll_named` step ([name, dy] a
wheel's notch, [name, dy, events] a touchpad's two fingers in that many events, then lifted)
scrolls on the node of that accessible name (a dock icon), and a `scroll_output` step ([x, y, dy]
or [x, y, dy, events]) at a point of the output.

A step may aim at what is on screen by its accessible name instead of a point: `press_named`,
`press_secondary_named`, `double_press_named` (an icon on the shell's desktop opened, ADR 0047) and
`move_named` (a name) and `hold_named` ([name, ms]) are the output's
press, secondary press, motion and long press at the middle of the node so named that the
session's assistive technology holds on the topmost shown surface that has one (its rectangle in
global coordinates, ADR 0003), wherever it is when the step runs: a dock's icon by its label
(`Inbox`), so a change of the dock's row moves no scenario's press; `drag_paced_named`
([from, to, motions, hz] and optionally the motion Escape follows, 0 for none, and how long the hand
rests at `to` before that Escape and the release, in ms) is a paced drag from one node to another (a
file onto the dock's Trash, ADR 0035; held over it, docs/adr/0044-system-sounds.mdx). A `wait_named` step
([name, ms]) waits, the session served all along, until the assistive technology holds a node so
named on a shown surface, and fails if none is within `ms`: an answer that comes on the application's
own clock (a store that is down, asked again until the application says so) is waited for as it
shows, never for a set time it might outlast.

A step settles once what the session shows stops changing: the director's application's frames,
the companion's, the shell's, and the windows of the applications the session launched (their place
and size, and a frame for their latest configure, as a zoom from their title bar gives them), not
each of their frames, which may animate for as long as they like (a storefront's skeleton while it
loads).

A scenario's `launch_settle_ms` is how long the application's first frames must stay unchanged
before its steps begin (750 by default): a browser shows its window before its interface draws in
it, for longer than that on a loaded machine.

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
`applications/` with each packaged application's bundle and runtime its package's: its entry is
written for each run where the package is (`oxyos-pack entry`, beside the offscreen compositor), so
packages built on one machine run where they were copied. Whatever XDG directories a run gives (its
registry, its data, its home), the Vulkan drivers are the machine's (`VK_ADD_DRIVER_FILES`, the
folders the loader looks in under the caller's environment): the hosts draw on the GPU the
compositor names, as in a session.

A scenario's `installed` ([{"package": ID, "capabilities": {NAME: DECISION}, "entry": [LINE]}]) is the
install service's state for the run's user, as it would have it (docs/adr/0030-installing-applications.mdx,
decision 7): each package's version copied into `packages/<id>/<versionCode>/`, its entry the one the
service writes (`oxyos-pack entry`, beside the offscreen compositor) with the lines given added, and the
account's decisions on its capabilities; the development compositor takes it as the service's
(`--installed-fixture`). Its `applications_launched` is what the shell had started, exactly: each id,
where its entry came from (`system`, `installed N`, `development`) and whether it was given `settings`.

Every run's wallpapers are the repository's catalog (`OXYOS_WALLPAPERS`,
config/includes.chroot_after_packages/usr/share/oxyos/wallpapers, as the image installs it,
docs/adr/0048-wallpapers.mdx); a choice is a preference as any other (`preferences`: `wallpaper`,
`wallpaper_all_desktops`, and the session's own lines, `wallpaper.DISPLAY` and
`wallpaper.DISPLAY.N`; `{repo}` in a value is the repository's root, for a photo's path). A
scenario with `wallpaper` ({tz, now}) fixes the moment the wallpaper
follows (`OXYOS_WALLPAPER_NOW`, unix seconds, which never moves on) and the time zone whose sun a
`sun` wallpaper's Automatic follows (ADR 0012).

A scenario with `reader` ({}) runs the session's screen reader (`oxyos-reader`, beside the
offscreen compositor unless `--reader` names it), which speaks the session's language (the
scenario's `locale`, docs/adr/0040-language-and-region.mdx), its voice a record of what it says
(ADR 0014); the reader's keys are `key` steps (Ctrl+Alt+Right is [29, 56, 106]).
With "off": true the reader is the session's but off, as `oxyos-session --reader` without
`--reader-on` has it: Super+F5 typed on the keymap (a `strokes` step, [[125, 63]]) starts it and
stops it again, locked or not (ADR 0014, decision 6).

Every scenario runs with a preferences file of its own, light and with carets that do not blink
unless it says otherwise (`caret_blink`, ADR 0037): a scenario's `preferences` ({KEY: VALUE}) are
written in it, with no grant given. A scenario
with `settings` ({"app": ID, "preferences": {KEY: VALUE}}) has that file written with those lines (docs/adr/0019-settings.mdx), and the application ID granted
`settings` as a session grants the system application the image's policy names. A step's `preferences` expectation checks
what the session last told the application of how to draw (dark, reduceMotion,
reduceTransparency, accent) by the time the step settled, each key given; a step's
`preferences_file` expectation checks the file holds each KEY = VALUE given as that step left it,
the `final` one once the run ended. In it and in `app_data`'s, "{home}" is the run's home (`home`):
a path the person chose there (a wallpaper's photo, docs/adr/0048-wallpapers.mdx).

A scenario with `input_sources` ({"available": [METHOD]}) has the development compositor tell
Settings and the shell the input sources as the session does (docs/adr/0041-input-methods.mdx,
decision 6), from the run's preferences and its `layout`, its engine having the input methods named
(an empty list: none installed). It answers the shell's top bar as the session does: a source chosen
is kept as the `input_source` preference, and the emoji picker asked for is counted.

A scenario with `home` runs with a home of the run's own (docs/adr/0028-file-access.mdx), made
afresh in its output directory: `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`
inside it, never the developer's. `home.items` maps each path in it to what it is: a folder ("/" at the end), a file's
text, or {"bytes": N} (N zero bytes) or {"png": [W, H, "#rrggbb"]} (a picture of one colour), or for a
folder {"tree": [FOLDERS, FILES]} (FOLDERS folders in it, FILES empty files in each: a
`node_modules` of thousands of files, made without listing each); each
modified at `home.modified` (unix seconds; 2026-01-02 10:00 UTC by default), so dates read the same
on every run. `home.later` is what another program does to it while the run goes on, each when the
application first shows a text: [{"when_shown": TEXT, "items": {...}, "xattrs": {...}}], items as
above, made when a
frame of the application's window shows TEXT (of any of the shell's surfaces with `"in_shell": true`:
a file written while Spotlight shows, docs/adr/0051-search.mdx) (so a step after that one sees the window follow a
change it did not make), or `after_ms` after it first does (a change coming while the application
sits idle). `home.xattrs` (and a `later`'s) sets extended attributes, as `setfattr` would (file tags,
docs/adr/0049-file-tags.mdx): {PATH: {NAME: VALUE}}, a value text (UTF-8) or {"bplist": [TEXT, ...]}
(a Mac's binary property list of strings, written by Python's plistlib). A step's `home` expectation
checks the home once the step settled: each path given
exists (true) or not (false), a file's text, and `trash` the names in the home's Trash (its
`files/`), each with a `.trashinfo` naming where it was; with a `shell`, the dock's Trash follows
that home's Trash (ADR 0034). Its `xattrs` ({PATH: {NAME: VALUE | false}}) checks an item's `user.`
attributes: a value as above, or false for none. The home's attributes are read only in a scenario
that sets or checks some. Every scenario runs in one fixed language, never the machine's
(docs/adr/0040-language-and-region.mdx, decision 8): `LANG` and `LC_ALL` are `en_US.UTF-8`, or the
scenario's `locale`, the session's language and region (what the shell, Settings, the login screen
and the reader speak, unless its preferences choose another). Its `tz` is the time zone (`TZ`), else
its wallpaper's, else Europe/Bucharest (FIXED_TZ), never the machine's.

What applications keep (their data and cache directories by id, application.h) is under the run's
own XDG_DATA_HOME and XDG_CACHE_HOME. A scenario's `app_data` ({ID: {FILE: JSON}}) is written to an
application's data directory before it starts; the `final` `app_data` expectation checks each such
file holds that JSON once the run ended.

A scenario with `lock_password` has the development compositor's lock fixture unlock with that
password (ADR 0010; never PAM), and with `lock_pin` with that PIN, said as one
(docs/adr/0020-oxy-account-login.mdx decision 2). The played session socket's `account.local`
({"pin", "password"}) is the account's local sign-in it tells the shell's lock screen and Settings,
which changes it (`setPin`, `removePin`, `setPasswordSignIn`) proved by `account.local_password` or
`account.local_pin`.

A scenario with `store` runs offline but for a store of its own (docs/adr/0030-installing-applications.mdx,
decision 14): the machine reaches Oxy's API (`api.oxy.so`) only through a proxy of the run's own
(`HTTPS_PROXY`), which hands it to a server of the run's own holding a certificate for that name from a
certificate authority of the run's own, the one the run trusts (`SSL_CERT_FILE`); every other host is
refused, so nothing of the network outside reaches the run. The server answers the store's public reads
as oxy-api does (`{"data": …}`, `{"data": [...], "pagination": …}`): `store.categories`, `store.apps`
(summaries; `?category=` filters them) and `store.listings` (each slug's page; any other is a 404),
reviews empty; `store.unavailable_first` ({PATH: N}) answers the first N requests for a path 503, as a
store that is down; `store.files` ({NAME: TEXT}) are files the run writes for it (an app's icon, an SVG
document, as a listing's `icon` may be a file URL or path), and `{store_files}` in any answer is the
directory holding them. It also answers NetInfo's check that the Internet is reachable
(`clients3.google.com/generate_204`, 204), so the machine reads as online and a store that is down is
told from a machine that is offline. Applications reach it as they reach the real one, with their own
builds.

The store also serves images, as Oxy's servers serve an app's icon or a person's avatar (images.h's
remote images): `store.hosts` are more names it answers for (its certificate holds them, the proxy
hands them to it: `cloud.oxy.so`), and `store.images` ({PATH: IMAGE}) what it answers for a path
on any of them: {"png": [W, H, "#rrggbb"(, "#rrggbb" its right half)]}, {"gif": [W, H, [["#rrggbb",
hundredths], ...](, loops)]} (an animated GIF, a frame of one colour each) or {"svg": TEXT}, with
"status" (200), "headers" (its caching headers: `Cache-Control`, `ETag`, `Last-Modified`; a
request whose `If-None-Match` or `If-Modified-Since` matches them is answered 304), "trickle_ms"
(its body sent 64 bytes at a time, that long apart: an image arriving slowly) and "delay_ms" (its
body sent whole after that long, its headers at once: an image arriving late). Each image answered
is recorded when its answer ends (`store/images.jsonl`), and a step's `served` expectation is what
the store answered in it, exactly, in any order: "STATUS PATH", with " cut" when the client went
away before the body ended (a request cancelled). `store.offline_when_shown` (a text) takes the
machine offline once the application's window first shows it: from then on the proxy refuses
everything, as a machine whose network went away. A step's `images` expectation is what the
host's image loader reported in it ("fetch URI" and "cancelled URI" -> count, exactly).

A step's `worklets` expectation is what worklets' UI runtime made in it (the first step's, from the
launch), each worklet the first time it ran there: `ahead_min`, at least that many the runtime's
framework compiled ahead (`framework/worklets.hbc`, sample/worklets.mjs); `from_source_max`, at most
that many compiled from their text then.

A scenario's `app.origin` and `app.capabilities` are what the session tells an application it
launches (OXYOS_APP_ORIGIN, OXYOS_APP_CAPABILITIES, docs/adr/0030-installing-applications.mdx,
decision 8): an installed version with only the capabilities its account granted.

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
so the machine is offline. With `login.oxy` Oxy's API is played instead (`played_oxy`: the SDK's own
sign-in by the code sent by email, `oxy.code`, or by password, adding a password confirmed with that
code, or by its QR, a request `oxy.qr.approve` has a phone approve by the time the screen first asks
(true) or `oxy.qr.approve_after_ms` after the screen joined Oxy's Socket.IO `/auth-session`
namespace over a WebSocket, which tells the screen at once ("socket"; its frames logged as `WS ...`
requests with status 101),
for the one account it plays, committing the device credential `oxy.device`; every request
logged by its method and path alone, one not played answered 404), and the accounts' service names
it with a client id (`platform`); `login.enroll` plays the setup's conversation (the credential must
be `oxy.device`'s, the password `enroll.password`, a second factor `enroll.code` when given) and
lists `enroll.account` once enrolled; `enroll.password` null plays an account without one, refused
`passwordRequired` as the service refuses it, and a `pin` given with the setup becomes the account's
PIN. An account's PIN (docs/adr/0020-oxy-account-login.mdx decision 2; `login.pins`: {USER: PIN},
listed `pin: set`; `login.pin_failures`: {USER: N} wrong ones before the run) is checked by `oxyos-pin-check`, which the login program's PIN service runs
(`oxyos-login-pin`, pam_exec, as the real one is), asking the stand-in's `verifyPin`: the tenth
wrong one in a row locks it out (listed `locked`). A step's `accounts_asked` is what the setup asked the service
in it ("ASKED: ANSWERED", exactly, in order) and `oxy_requests` requests Oxy's API answered in it
("METHOD PATH STATUS", each at least once), `oxy_requests_times` how many times each was answered
({"METHOD PATH STATUS": N}, exactly). No credential of anyone's is in a scenario: every one
is the run's own. `login.record`: what the login program must have recorded once the run
ended, each line beginning so, in order (none: nobody signed in). A step's `shell.signed_in` says
whether the login screen was told someone signed in.

A scenario with `account` runs against a played accounts' service's session socket
(docs/adr/0020-oxy-account-login.mdx, decision 6; `OXYOS_ACCOUNTD_SESSION_SOCKET`, which every run
points at a socket of its own, so none asks the machine's): the application ids it knows as the
session's launches (`account.registered`; any other process is refused `notPermitted`), its
snapshot (`account.snapshot`, over a signed-in Ada Lovelace, @ada, online), and its changes while
the run goes on (`account.later`: [{"when_shown": TEXT, "snapshot": {...}}], pushed once the
application's window first shows TEXT). A token it mints names the application it is for and is no
credential. Erasing the computer (docs/adr/0054-recovery-and-erasing.mdx), Settings' alone:
`account.erase` is what `erasePreview` answers (over an available computer whose asking account is
its administrator, Ada Lovelace alone, online) and `account.erase.password` the administrator's
password, which `eraseAll` takes (`erasing`, every account signed out online where its preview says
so); a wrong one is refused `wrongPassword`, and from the `account.erase.throttle_after`th wrong one
on, `throttled` with `retryAfter` 30. A step's `account_session` expectation is what it was asked in
the step, exactly, in any order: "APPLICATION ASKED: ANSWERED" ("-" for a process it refused;
`token boot: token`, `token preflight: no token`, `subscribe: snapshot`, `remove: removed`,
`erasePreview: preview`, `eraseAll: erasing` or the reason it was refused).

Every scenario, every step, whatever it says (docs/adr/0016-windows.mdx, decision 11): the person's
input reached exactly whom it should, once. The development compositor records every input event
it routes (the pointer's, motion among them, the keyboard's and what it types, touches, scrolls,
pinches, resting fingers, an assistive technology's actions, menu commands, focus changes, a drag's
events: `deliveries`, each with its serial, client, process and surface), and each Bloom host every
one it receives (`received`) and the input that caused each React event it delivers; the run fails
when the keyboard's input reached a surface without the keyboard focus or two surfaces had it at
once, when the pointer's reached a surface it is not over (its Enter, no Leave since) or two were
entered at once, when a release reached a surface with no press, when an event was routed twice,
when a step settled with the compositor's keyboard or pointer focus not where the events routed
left them (`input_focus`) or with the keyboard focus on a surface not shown, when a host received an event other than once, when one input made a
host deliver the same React event to the same view twice, or made the pointer's input reach a view
of another surface, and when any input reached a client that is neither the application, its shell,
companion or file chooser, nor one the shell launched (`leaked`, which `delivered` checks too).

A scenario's `final` expectations (texts / absent / focus_input) are checked on the last frames,
once every step has run; a scenario may have no steps at all.

Also reports render times per frame and the launch time. Exit status 0 only if every
expectation holds.

Usage: regress.py SCENARIO.json --offscreen BIN --runtimes DIR --packages DIR [--out DIR]
"""

import argparse
import atexit
import base64
import collections
import hashlib
import json
import math
import re
import shutil
import struct
import zlib
import urllib.parse
import os
import pathlib
import plistlib
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# The language every scenario runs in unless it names its own `locale` (docs/adr/0040-language-and-region.mdx).
FIXED_LOCALE = "en_US.UTF-8"


# The fields of a director record that repeat the step's own instruction (oxyos-offscreen launch.rs).
STEP_KEYS = {"press", "press_popup", "press_output", "move_output", "press_secondary", "press_secondary_output", "hold_output", "named", "double_press", "double_press_output", "triple_press", "double_drag", "scroll", "pinch", "type", "commit", "preedit", "transaction", "keyboard",
             "compose_keys", "im_change", "drag", "drag_output", "drag_paced", "drag_hold", "release_output", "drag_paced_named", "scroll_paced", "flick", "flick_hold", "move_paced", "press_paced", "double_click", "grab", "key", "key_down", "key_up", "move_window", "resize_window", "minimize_window", "caps_lock", "scale",
             "a11y_action",
             "expect_popup", "switch_terminal", "idle", "wait", "wait_named", "clipboard", "menu", "transition_clock",
             "scroll_output", "swipe", "swipe_begin", "swipe_move", "swipe_end"}


# CLOCK_REALTIME less CLOCK_MONOTONIC, the clock the hosts' reports are stamped with: the wall
# clock's time of a report's line, for what follows the wall clock (the shell's minute).
WALL_OFFSET_NS = time.time_ns() - time.monotonic_ns()

# The time zone the run's applications tell the time in: a scenario's `tz`, else FIXED_TZ.
FIXED_TZ = "Europe/Bucharest"
RUN_TZ = None
# The run's locale (its `locale`, else FIXED_LOCALE): the language the shell says its clock in.
RUN_LOCALE = FIXED_LOCALE
SPANISH_WEEKDAYS = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"]
SPANISH_MONTHS = ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto",
                  "septiembre", "octubre", "noviembre", "diciembre"]


def clock_said(t_ns):
    """The shell's clock as the screen reader says it at a report's time `t_ns` (its `clockSaid`,
    apps/shell/src/clock.ts): the date and time in whole words in the run's language, "martes, 3 de
    noviembre, 14:05" in Spain, "Tuesday, November 3, 2:05 PM" in the United States, written here
    from the calendar rather than by the shell's Intl. Other locales are not written here."""
    import datetime
    import zoneinfo
    zone = zoneinfo.ZoneInfo(RUN_TZ) if RUN_TZ else None
    at = datetime.datetime.fromtimestamp((t_ns + WALL_OFFSET_NS) / 1e9, zone)
    if zone is None:
        at = at.astimezone()
    region = RUN_LOCALE.split(".")[0]
    if region == "es_ES":
        return f"{SPANISH_WEEKDAYS[at.weekday()]}, {at.day} de {SPANISH_MONTHS[at.month - 1]}, {at:%H:%M}"
    if region == "en_US":
        # ICU's narrow no-break space before the day period.
        return f"{at:%A}, {at:%B} {at.day}, {at.hour % 12 or 12}:{at:%M}\u202f{'AM' if at.hour < 12 else 'PM'}"
    raise SystemExit(f"{{clock}} is not written for {RUN_LOCALE} (regress.py's clock_said)")


# The applications' packages (--packages), whose sizes "{size:ID}" in an expected text names.
PACKAGES = None


def package_size(package):
    """The package `package`'s size as Atlas says it in English (its lib/format.ts: decimal units,
    one decimal under 10): the bytes of the files its manifest lists, which the machine fetches."""
    manifest = json.loads((PACKAGES / package / "manifest.json").read_text())
    value, unit, units = float(sum(f["size"] for f in manifest["files"])), 0, ["bytes", "KB", "MB", "GB"]
    while value >= 1000 and unit < len(units) - 1:
        value, unit = value / 1000, unit + 1
    digits = 0 if unit == 0 or value >= 10 else 1
    return f"{value:,.{digits}f} {units[unit]}"


def same_text(got, wanted, t_ns):
    """Whether a text is the one expected, "{clock}" in it the shell's clock as said (clock_said) at
    `t_ns`, or in the minute before it (the clock said as its minute turned), and "{size:ID}" the
    size of the package ID as built (package_size), which every change to its bundle moves."""
    if isinstance(wanted, str) and "{size:" in wanted:
        wanted = re.sub(r"\{size:([^}]+)\}", lambda match: package_size(match.group(1)), wanted)
    if not isinstance(wanted, str) or "{clock}" not in wanted:
        return got == wanted
    return any(got == wanted.replace("{clock}", clock_said(t_ns - back * 60_000_000_000)) for back in (0, 1))


# Whether the steps' budgets on speed are held (--speed-budgets): how much time the work takes on
# this machine, which its load changes, and the display's rate (how many frames fit in a motion,
# how far apart they come: a drag's `composed_min` and `composed_gap_ms`, a transition's
# `frames_min` and `frame_gap_ms`, `window_frames`', `animated_frames`' and `committed_frames`'
# least, `animation_frames`' count and median interval), which perf.py's `refresh` judges on the
# machine itself. What a step must do (what it re-renders, a backstop far above any machine's time,
# a motion in more than one frame, the window told the display's refresh, each frame answered once
# and in order: check_pacing) is always held (docs/performance.mdx, "What a step re-renders").
SPEED_BUDGETS = False


def frames_least(least):
    """The fewest frames a step may show for a claim of `least`: all of them under --speed-budgets
    (how many fit in the time is the machine's rate, which a loaded one does not hold), else two:
    what moved did so in more than one frame, not one jump, which no load changes."""
    return least if SPEED_BUDGETS else min(least, 2)

# Whether the steps' budgets on time are left to the caller (--collect, perf.py's): every one of
# them, latency and backstops too, is measured over several runs and judged at the median there,
# so a run checks what its steps do and enforces no time.
COLLECT = False

# Whether the hosts count what React re-renders (OXYOS_HOST_REPORT_RENDERS): for a scenario holding
# `renders`, unless --no-render-counts, whose `renders` are then not checked.
RENDER_COUNTS = False


def holds_renders(value):
    """Whether a scenario's steps (and the hosts' within them, `in_shell` and the rest) hold `renders`."""
    if isinstance(value, dict):
        return "renders" in value or any(holds_renders(v) for v in value.values())
    if isinstance(value, list):
        return any(holds_renders(v) for v in value)
    return False


# A slow frame's line in the session's log (frame-diagnostics.cpp, FrameDiagnostics::finish).
SLOW_LINE = re.compile(
    r"\[host (?P<app>\S+)\] (?P<subject>.+?) took (?P<work>[\d.]+) ms, (?P<dropped>\d+) dropped "
    r"\(a refresh is (?P<refresh>[\d.]+) ms\): events (?P<events>[\d.]+), JavaScript (?P<javascript>[\d.]+) "
    r"\(commits (?P<commits>[\d.]+), of which Yoga layout (?P<layout>[\d.]+) and text (?P<text>[\d.]+)\), "
    r"animation (?P<animation>[\d.]+), mount (?P<mount>[\d.]+), drawing (?P<draw>[\d.]+), "
    r"presenting (?P<present>[\d.]+)(?P<painter>| on the GPU \(sync [\d.]+\)| from the CPU), "
    r"other (?P<other>[\d.]+); of it on the CPU (?P<cpu>[\d.]+) \(JavaScript (?P<javascript_cpu>[\d.]+)\), "
    r"blocked (?P<blocked>[\d.]+) \(JavaScript (?P<javascript_blocked>[\d.]+)\), preempted (?P<preempted>[\d.]+); "
    r"waiting for the compositor (?P<wait>[\d.]+); (?P<turns>\d+) turns?"
    r"(?:; input to present (?P<latency>[\d.]+) ms)?(?:; (?P<skipped>\d+) slow not shown since the last)?$")


def percentile(ordered, p):
    """The percentile as the host's summary picks it (FrameDiagnostics::percentile)."""
    return ordered[min(len(ordered) * p // 100, len(ordered) - 1)] if ordered else None


def check_pacing(report, hz, end, failures):
    """A display paced at a refresh (`outputs`' ":HZ"): what no load changes, where the rate itself
    is a speed budget (--speed-budgets). The window is told the display's refresh before its first
    frame, and the client's frames, numbered across its surfaces, follow on one by one (none
    skipped, none twice), each answered once by the compositor (shown, or discarded as the protocol
    says: drawn for a configure since replaced, or superseded), in order."""
    refresh = [line for line in report if line["kind"] == "refresh"]
    presents = [line for line in report if line["kind"] == "present" and "sequence" in line]
    feedback = [line for line in report if line["kind"] == "feedback"]
    if not refresh or refresh[0]["mhz"] != round(hz * 1000):
        failures.append(f"the window was told a refresh of {refresh[0]['mhz'] if refresh else None} mHz, "
                        f"the display's being {round(hz * 1000)}")
    elif presents and refresh[0]["t"] > presents[0]["t"]:
        failures.append("the window was told the display's refresh after its first frame")
    sequences = [line["sequence"] for line in presents]
    if any(b != a + 1 for a, b in zip(sequences, sequences[1:])):
        failures.append(f"the client's frames skipped or repeated a number: {sequences}")
    answers = collections.Counter(line["sequence"] for line in feedback)
    # A frame presented as the run ends may go unanswered: the host is stopped first.
    unanswered = [line["sequence"] for line in presents if line["t"] <= end and answers[line["sequence"]] != 1]
    if unanswered:
        failures.append(f"frames not answered exactly once by the compositor: {unanswered} "
                        f"({[answers[n] for n in unanswered]} times)")
    answered = [line["sequence"] for line in feedback]
    if answered != sorted(answered):
        failures.append(f"the compositor answered the client's frames out of order: {answered}")


def check_diagnostics(report, log_text, failures, who="the application"):
    """A host's frame diagnostics (frame-diagnostics.h) against its own report: each line written to
    the log is there, a slow frame's line names the work its `timing` line has, each part's time by
    the wall clock is its time on the CPU, blocked and preempted (thread-clock.h), and each summary
    sums up exactly the frames presented since the summary before."""
    logged = set(log_text.splitlines())
    frames, last_slow = [], None
    for line in report:
        if line["kind"] == "timing":
            # Slow exactly when its work, and its wait beyond a refresh, pass a refresh: a fast
            # frame is never said to be slow, however loaded the machine.
            late = line["work_ms"] + max(0.0, line["wait_ms"] - line["refresh_ms"])
            if abs(late - line["late_ms"]) > 1e-6 or line["slow"] != (late > line["refresh_ms"]):
                failures.append(f"{who}'s frame of {line['work_ms']:.2f} ms (waiting {line['wait_ms']:.2f}, a refresh "
                                f"{line['refresh_ms']:.2f}) is {'' if line['slow'] else 'not '}said to be slow")
            # Each part by the wall clock is exactly its CPU's, blocked and preempted time (each rounded
            # to 0.01 ms), so that nothing off the CPU goes unsaid.
            for part, wall in line["parts"].items():
                split = line["cpu"][part] + line["blocked"][part] + line["preempted"][part]
                if abs(split - wall) > 0.031:
                    failures.append(f"{who}'s {part} of {wall:.2f} ms is {line['cpu'][part]:.2f} on the CPU, "
                                    f"{line['blocked'][part]:.2f} blocked and {line['preempted'][part]:.2f} preempted")
            if abs(line["cpu_ms"] + line["blocked_ms"] + line["preempted_ms"] - line["work_ms"]) > 1e-6:
                failures.append(f"{who}'s frame of {line['work_ms']:.2f} ms is {line['cpu_ms']:.2f} on the CPU, "
                                f"{line['blocked_ms']:.2f} blocked and {line['preempted_ms']:.2f} preempted")
            if line["presented"]:
                frames.append(line)
            if line["slow"]:
                last_slow = line
        elif line["kind"] == "log":
            if line["text"] not in logged:
                failures.append(f"{who}'s diagnostics wrote {line['text']!r} to its report, not to the log")
            slow = SLOW_LINE.match(line["text"])
            if " took " in line["text"] and " dropped (a refresh is " in line["text"] and not slow:
                failures.append(f"{who}'s slow line {line['text']!r} is not as frame-diagnostics.cpp writes it")
            elif slow and (last_slow is None or abs(float(slow["work"]) - last_slow["work_ms"]) > 0.051
                           or abs(float(slow["cpu"]) - last_slow["cpu_ms"]) > 0.051
                           or abs(float(slow["javascript_cpu"]) - last_slow["cpu"]["javascript"]) > 0.051):
                failures.append(f"{who}'s slow line {line['text']!r} does not name its frame's work "
                                f"({last_slow and last_slow['work_ms']})")
        elif line["kind"] == "timing_summary":
            work = sorted(frame["work_ms"] for frame in frames)
            latency = sorted(frame["latency_ms"] for frame in frames if frame["latency_ms"] is not None)
            slow = [frame for frame in frames if frame["slow"]]
            wanted = {"frames": len(frames), "work_p50": percentile(work, 50), "work_p95": percentile(work, 95),
                      "work_max": work[-1] if work else None, "latency_p50": percentile(latency, 50),
                      "latency_p95": percentile(latency, 95), "inputs": len(latency), "slow": len(slow),
                      "dropped": sum(frame["dropped"] for frame in slow)}
            got = {key: line[key] for key in wanted}
            if got != wanted:
                failures.append(f"{who}'s summary says {got}, its frames since the one before {wanted}")
            frames = []


def director_args(step, out):
    kind, value = step["do"]
    args = ["--allow-unchanged"] if step.get("allow_unchanged") else []
    if step.get("until_sheet"):
        args.append("--until-sheet")
    if step.get("until_named"):
        name, ms = step["until_named"]
        args += ["--until-named", str(ms), name]
    if kind in ("press", "press-popup"):
        return args + [f"--{kind}", f"{value[0]},{value[1]}"]
    if kind in ("press_popup", "press_output", "move_output", "release_output", "press_secondary", "press_secondary_output",
                "double_press", "double_press_output", "triple_press"):
        return args + [f"--{kind.replace('_', '-')}", f"{value[0]},{value[1]}"]
    if kind == "double_drag":
        # A double click whose second press is held and dragged: [x0, y0, x1, y1].
        return args + ["--double-drag", ",".join(map(str, value))]
    if kind == "scroll":
        # A wheel's notches, [x, y, dy], or a touchpad's two fingers, [x, y, dy, events].
        return args + ["--scroll", ",".join(map(str, value))]
    if kind == "scroll_named":
        # A scroll by DY on the node of this accessible name (a dock icon's, ADR 0045): [name, dy]
        # a wheel's notch, or [name, dy, events] a touchpad's two fingers in that many events.
        events = value[2] if len(value) > 2 else 0
        return args + ["--scroll-named", f"{value[1]},{events}", value[0]]
    if kind == "scroll_output":
        # The same at a point of the output: [x, y, dy] or [x, y, dy, events].
        events = value[3] if len(value) > 3 else 0
        return args + ["--scroll-output", f"{value[0]},{value[1]},{value[2]},{events}"]
    if kind == "swipe":
        # Fingers swiped on the touchpad and lifted (Mission Control, App Exposé, the desktops, ADR
        # 0045, ADR 0046): [fingers, dx, dy, steps] or [fingers, dx, dy, steps, hz].
        return args + ["--swipe", ",".join(map(str, value))]
    if kind == "swipe_begin":
        # The same, the fingers left down for the steps after it: a swipe partway.
        return args + ["--swipe-begin", ",".join(map(str, value))]
    if kind == "swipe_move":
        # The fingers left down moved further: [dx, dy, steps] or [dx, dy, steps, hz].
        return args + ["--swipe-move", ",".join(map(str, value))]
    if kind == "swipe_end":
        # The fingers left down lifted, this many milliseconds after their last motion.
        return args + ["--swipe-end", str(value)]
    if kind == "pinch":
        return args + ["--pinch", ",".join(map(str, value))]
    if kind == "type":
        return args + ["--type", value]
    if kind == "strokes":
        # Keys typed on the session's keymap and compose table (the scenario's `layout`), one
        # after the other: each a code or a chord (a list of codes pressed in order), [40, 18]
        # being ´ then e on the Spanish layout.
        return args + ["--strokes", " ".join("+".join(map(str, c)) if isinstance(c, list) else str(c)
                                             for c in value)]
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
    if kind == "drag_hold":
        # The same, its button still held at the end: the drag goes on into the next steps
        # (`move_output`, `key_down`, `release_output`), each step's frame showing it midway.
        return args + ["--drag-hold", ",".join(map(str, value))]
    if kind == "scroll_paced":
        # A touchpad's scroll at its pace over the window: [x, y, dy, events, hz].
        return args + ["--scroll-paced", ",".join(map(str, value))]
    if kind == "drag_paced_named":
        # A paced drag from the node of one accessible name to the node of another (a file onto the
        # dock's Trash): [from, to, motions, hz] and, after that many motions, Escape (ADR 0035),
        # then how long it rests there and 1 to keep the button held (`drag_hold`'s). `to` may be a
        # point of the output, "@x,y" (an icon dragged off the dock, ADR 0034's decision 10).
        return args + ["--drag-paced-named", ",".join(map(str, value[2:])), value[0], value[1]]
    if kind in ("press_named", "press_secondary_named", "double_press_named", "move_named"):
        # The output's step at the middle of the node of this accessible name (a dock's icon by
        # its label, an icon on the desktop by its name), wherever it is when the step runs.
        return args + ["--" + kind.replace("_", "-"), value]
    if kind == "hold_named":
        # A long press on the node of this name: [name, ms].
        return args + ["--hold-named", str(value[1]), value[0]]
    if kind == "hold_output":
        # A long press on the output, held still: [x, y, ms] (a dock icon's menu, ADR 0034).
        return args + ["--hold-output", ",".join(map(str, value))]
    if kind == "press_paced":
        # Presses at their pace, each carrying its time as a device's: [x, y, count, interval_ms].
        return args + ["--press-paced", ",".join(map(str, value))]
    if kind == "double_click":
        # A hand's double click on a title area: the first press held until the application begins
        # a move with it (a drag region), the second after a gap, held hold_ms: [x, y, hold_ms, gap_ms].
        return args + ["--double-click", ",".join(map(str, value))]
    if kind == "flick":
        # The same, then the fingers lifting: [x, y, dy, events, hz].
        return args + ["--flick", ",".join(map(str, value))]
    if kind == "flick_hold":
        # A flick, then two fingers resting on the touchpad that long after the lift and lifting
        # 150 ms later: [x, y, dy, events, hz, after_ms].
        return args + ["--flick-hold", ",".join(map(str, value))]
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
    if kind == "transition_clock":
        # The clock the compositor's transitions run on (ADR 0011): "hold" where it is now, so many
        # milliseconds after where it was held (a transition looked at at an exact point of it,
        # whatever the machine's load), or "run" with the monotonic clock again.
        return args + ["--transition-clock", str(value)]
    if kind == "wait_named":
        # Real time until the assistive technology holds a node of this name, at most so many
        # milliseconds: [name, ms] (an answer on the application's own clock).
        return args + ["--wait-named", str(value[1]), value[0]]
    if kind == "idle":
        # Seconds with no input, which the idle lock counts as passed at once (ADR 0010).
        return args + ["--idle", str(value)]
    if kind == "switch-terminal":
        return args + ["--switch-terminal", str(value)]
    if kind == "expect-popup":
        return ["--expect-popup"]
    if kind == "menu":
        # The person chooses a command in the session's bar (ADR 0021), by its path:
        # "View > Show Full URL".
        return args + ["--menu", value]
    if kind == "sound":
        # The played sound changes as the world would (ADR 0056): {"plug": {"direction", "device"}},
        # {"unplug": {"id"}}, {"change": {"id", "kind", "name", "volume", "muted", "level"}}.
        return args + ["--sound-act", json.dumps(value)]
    if kind == "network":
        # The played network daemon changes the machine (ADR 0038): {"ethernet": "plugged"},
        # {"access_point": {...}}, {"remove_access_point": ssid}, {"hardware_enabled": false}.
        return args + ["--network", json.dumps(value)]
    if kind == "bluetooth":
        # The played BlueZ changes the machine (ADR 0055): {"appear": {...}}, {"pair_request":
        # address}, {"typed": address}, {"connect": address}, {"battery": {...}}, {"blocked": true}.
        return args + ["--bluetooth", json.dumps(value)]
    if kind == "power":
        # The played power services change the machine (ADR 0057): {"on_ac": false},
        # {"percent": 9, "warning": "low"}, {"state": "charging", "time_to_full": 3600}.
        return args + ["--power", json.dumps(value)]
    if kind == "clipboard":
        # Another application copies (ADR 0007): {type: content}, a text or "base64:..." bytes,
        # the most faithful type first. The director reads each from a file the list names.
        directory = pathlib.Path(tempfile.mkdtemp(prefix="clipboard-", dir=out))
        lines = []
        for index, (mime, content) in enumerate(value.items()):
            path = directory / str(index)
            if content.startswith("base64:"):
                import base64
                path.write_bytes(base64.b64decode(content[len("base64:"):]))
            else:
                path.write_text(content)
            lines.append(f"{mime}\t{path}")
        listing = directory / "list"
        listing.write_text("\n".join(lines))
        return args + ["--clipboard", str(listing)]
    raise ValueError(f"unknown step {kind}")


def serve_accounts(path, accounts, login=None, log=None):
    """The accounts' service as the login screen and the login program ask it (SOCK_SEQPACKET,
    crates/oxyos-account's ipc.rs), answering anyone: `list` with the accounts; `platform` with
    `login.platform` (refused `notConfigured` without one); and a setup's conversation (`enroll`,
    then `secondFactor` while asked) as `login.enroll` plays it: the device credential handed over
    must be `login.enroll.device` (the played Oxy API's), the password `login.enroll.password`, a
    second factor `login.enroll.code` when it has one; enrolled, the account (`login.enroll.account`)
    is listed from then on. What the setup was asked is logged, never what it was sent."""
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    listener.bind(str(path))
    listener.listen(16)
    login = login or {}
    accounts = [dict(account) for account in accounts]
    # The accounts' PINs, as the real service's verifiers are: the played run's own, never logged.
    pins = dict(login.get("pins", {}))
    # Wrong PINs in a row before the run (`login.pin_failures`): ten have locked it out.
    failures = dict(login.get("pin_failures", {}))
    for account in accounts:
        if account["user"] in pins:
            account.setdefault("pin", "locked" if failures.get(account["user"], 0) >= 10 else "set")
        account.setdefault("pin", "none")
        account.setdefault("password", True)

    def record(asked, answered):
        if log:
            with open(log, "a") as out:
                out.write(json.dumps({"t": time.monotonic_ns(), "asked": asked, "answered": answered}) + "\n")

    def enroll(connection, message):
        played = login.get("enroll")
        request = message["enroll"]
        pin = request.get("pin")
        if not played or request.get("deviceId") != played["device"]["deviceId"] \
                or request.get("deviceSecret") != played["device"]["deviceSecret"]:
            answer = {"refused": {"reason": "credentialRefused"}}
        elif pin is not None and (not re.fullmatch(r"[0-9]{4,16}", pin) or pin == request.get("password")):
            answer = {"refused": {"reason": "unusablePin"}}
        elif request.get("password") is None:
            # The platform credential is issued only to the platform's password sign-in.
            answer = {"refused": {"reason": "passwordRequired"}}
        elif request.get("password") != played["password"]:
            answer = {"refused": {"reason": "wrongPassword"}}
        elif played.get("code"):
            answer = "secondFactorRequired"
        else:
            answer = None
        while True:
            if answer is None:
                listed = dict(played["account"], password=True)
                if pin is not None:
                    pins[listed["user"]] = pin
                    failures.pop(listed["user"], None)
                listed["pin"] = "set" if listed["user"] in pins else "none"
                accounts[:] = [account for account in accounts if account["user"] != listed["user"]]
                accounts.append(listed)
                answer = {"enrolled": {"accountId": listed["accountId"], "user": listed["user"], "uid": 1001,
                                       "profile": {"id": listed["accountId"], "username": listed["name"],
                                                   "displayName": None, "avatar": None}}}
            record("secondFactor" if record.turn else "enroll",
                   answer if isinstance(answer, str) else next(iter(answer)))
            record.turn += 1
            connection.send(json.dumps(answer).encode())
            if answer != "secondFactorRequired" and answer != {"refused": {"reason": "secondFactorInvalid"}}:
                return
            packet = connection.recv(16384)
            if not packet:
                return
            code = json.loads(packet).get("secondFactor", {}).get("code")
            answer = None if code == played["code"] else {"refused": {"reason": "secondFactorInvalid"}}

    def serve():
        while True:
            connection, _ = listener.accept()
            with connection:
                try:
                    message = json.loads(connection.recv(16384))
                    if message == "list":
                        connection.send(json.dumps({"accounts": accounts}).encode())
                    elif isinstance(message, dict) and "check" in message:
                        # The login program's online check, as an offline service answers it.
                        connection.send(json.dumps({"checked": {"result": message["check"]["local"]}}).encode())
                    elif message == "platform":
                        platform = login.get("platform")
                        record("platform", "platform" if platform else "refused")
                        connection.send(json.dumps({"platform": platform} if platform
                                                   else {"refused": {"reason": "notConfigured"}}).encode())
                    elif isinstance(message, dict) and "enroll" in message:
                        record.turn = 0
                        enroll(connection, message)
                    elif isinstance(message, dict) and "verifyPin" in message:
                        # oxyos-pin-check's, for PAM: the PIN checked here, a wrong one counted.
                        user, pin = message["verifyPin"]["user"], message["verifyPin"]["pin"]
                        listed = next((a for a in accounts if a["user"] == user), None)
                        if user not in pins or listed is None:
                            result = "none"
                        elif failures.get(user, 0) >= 10:
                            result = "locked"
                        elif pin == pins[user]:
                            failures.pop(user, None)
                            result = "accepted"
                        else:
                            failures[user] = failures.get(user, 0) + 1
                            result = "locked" if failures[user] >= 10 else "rejected"
                        if listed is not None and user in pins:
                            listed["pin"] = "locked" if failures.get(user, 0) >= 10 else "set"
                        record("verifyPin", result)
                        connection.send(json.dumps({"pin": {"result": result}}).encode())
                except (OSError, ValueError, KeyError, TypeError):
                    pass

    record.turn = 0
    threading.Thread(target=serve, daemon=True).start()
    return listener


# The played accounts' service's session socket (`account`), what it was asked and its state.
ACCOUNT = {}


def account_snapshot(spec):
    """The snapshot the played service starts with (`account.snapshot`), as the service tells it."""
    return dict({"status": "signed_in", "revision": 1, "online": True,
                 "account": {"id": "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", "handle": "ada",
                             "displayName": "Ada Lovelace"}}, **spec.get("snapshot", {}))


def erase_preview(spec):
    """What the played service says erasing would do (`account.erase`, the password left out)."""
    told = {"available": True, "administrator": True, "online": True,
            "accounts": [{"name": "Ada Lovelace", "handle": "ada", "signsOutOnline": True}]}
    told.update({key: value for key, value in spec.get("erase", {}).items()
                 if key not in ("password", "throttle_after")})
    return told


def played_token(application, reason):
    """A token the played service mints: no credential, only what it names (the application it is
    for, as a real one's `azp` does), so a probe can say whose it is without showing a token."""
    import base64
    claims = json.dumps({"azp": application, "reason": reason}).encode()
    return "played." + base64.urlsafe_b64encode(claims).decode().rstrip("=")


def serve_account_session(path, spec, log):
    """The accounts' service's session socket (docs/adr/0020-oxy-account-login.mdx, decision 6:
    SOCK_SEQPACKET, one JSON message a packet) as applications ask it. The service knows a peer by
    its launch, which the session registers (`account.registered`, the application ids the played
    service holds as launched; the session's registration itself is the service's and the
    session's, not this stand-in's): the peer's process (SO_PEERCRED) is looked up by the id its
    launch gave it (OXYOS_APP_ID, never a process the session did not start, OXYOS_APP_UNRECORDED).
    Anyone else is refused `notPermitted`. `snapshot` and `subscribe` (pushed again at each change),
    `token` (a played token naming the application, or none offline or signed out), and Settings'
    `remove`, `erasePreview` and `eraseAll` (the erase's password checked against
    `account.erase.password`, never logged). Each request is logged ({t, application, asked,
    answered})."""
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    listener.bind(str(path))
    listener.listen(16)
    lock = threading.Lock()
    subscribers = []
    ACCOUNT["snapshot"] = account_snapshot(spec)
    ACCOUNT["log"] = log
    registered = set(spec.get("registered", []))
    wrong_passwords = [0]
    # The account's local sign-in (`account.local`) and its PIN (`account.local_pin`), as Settings
    # changes them; the secrets are the run's own and never logged.
    local_sign_in = dict({"pin": "none", "password": True}, **spec.get("local", {}))
    pins_set = [spec.get("local_pin")]

    def record(application, asked, answered):
        with lock, open(log, "a") as out:
            out.write(json.dumps({"t": time.monotonic_ns(), "application": application,
                                  "asked": asked, "answered": answered}) + "\n")

    def application_of(connection):
        pid, _, _ = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                                                struct.calcsize("3i")))
        try:
            environ = pathlib.Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
        except OSError:
            return None
        names = dict(item.split(b"=", 1) for item in environ if b"=" in item)
        # The session's shell, which the session registers as the system's `so.oxy.shell`.
        try:
            if b"OxyOSShell" in pathlib.Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0"):
                return "so.oxy.shell"
        except OSError:
            pass
        if b"OXYOS_APP_UNRECORDED" in names:
            return None
        application = names.get(b"OXYOS_APP_ID", b"").decode() or None
        return application if application in registered else None

    def push(snapshot):
        with lock:
            ACCOUNT["snapshot"] = snapshot
            for connection in list(subscribers):
                try:
                    connection.send(json.dumps({"snapshot": snapshot}).encode())
                except OSError:
                    subscribers.remove(connection)

    ACCOUNT["push"] = push

    def converse(connection):
        application = application_of(connection)
        with connection:
            while True:
                try:
                    packet = connection.recv(16384)
                except OSError:
                    return
                if not packet:
                    with lock:
                        if connection in subscribers:
                            subscribers.remove(connection)
                    return
                try:
                    message = json.loads(packet)
                except ValueError:
                    message = None
                asked = message if isinstance(message, str) else (
                    next(iter(message)) if isinstance(message, dict) and len(message) == 1 else "invalid")
                if asked == "token":
                    asked = f"token {message['token'].get('reason')}"
                if application is None:
                    record(None, asked, "refused")
                    connection.send(json.dumps({"refused": {"reason": "notPermitted"}}).encode())
                    return
                snapshot = ACCOUNT["snapshot"]
                if message in ("snapshot", "subscribe"):
                    record(application, asked, "snapshot")
                    with lock:
                        connection.send(json.dumps({"snapshot": snapshot}).encode())
                        if message == "subscribe":
                            subscribers.append(connection)
                elif asked.startswith("token "):
                    if snapshot["status"] == "signed_in" and snapshot["online"] and not spec.get("refuse_tokens"):
                        token = {"accessToken": played_token(application, message["token"]["reason"]),
                                 "expiresAt": "2026-11-06T12:05:00.000Z"}
                        record(application, asked, "token")
                    else:
                        token = None
                        record(application, asked, "no token")
                    connection.send(json.dumps({"token": token}).encode())
                elif message == {"erasePreview": {}} and application == "so.oxy.settings":
                    record(application, asked, "preview")
                    connection.send(json.dumps({"erasePreview": erase_preview(spec)}).encode())
                elif (asked == "eraseAll" and application == "so.oxy.settings"
                      and isinstance(message["eraseAll"], dict)):
                    erase = spec.get("erase", {})
                    preview = erase_preview(spec)
                    throttle_after = erase.get("throttle_after")
                    if not preview["available"]:
                        answer = {"refused": {"reason": "unavailable"}}
                    elif not preview["administrator"]:
                        answer = {"refused": {"reason": "notAdministrator"}}
                    elif throttle_after is not None and wrong_passwords[0] >= throttle_after:
                        answer = {"refused": {"reason": "throttled", "retryAfter": 30}}
                    elif message["eraseAll"].get("password") != erase.get("password"):
                        wrong_passwords[0] += 1
                        answer = {"refused": {"reason": "wrongPassword"}}
                    else:
                        accounts = preview["accounts"]
                        answer = {"erasing": {
                            "signedOut": [a["name"] for a in accounts if a["signsOutOnline"]],
                            "notSignedOut": [a["name"] for a in accounts if not a["signsOutOnline"]]}}
                    record(application, asked,
                           "erasing" if "erasing" in answer else answer["refused"]["reason"])
                    # The real service then restarts the computer; the run goes on.
                    connection.send(json.dumps(answer).encode())
                elif message == "localSignIn" and application in ("so.oxy.shell", "so.oxy.settings"):
                    # The account's PIN and password on this computer (ADR 0020 decision 2),
                    # `account.local`: what the lock screen and Settings are told.
                    record(application, asked, f"pin {local_sign_in['pin']}")
                    connection.send(json.dumps({"localSignIn": local_sign_in}).encode())
                elif asked in ("setPin", "removePin", "setPasswordSignIn") and application == "so.oxy.settings":
                    # Settings' changes, proved by the password (`account.local_password`) or the
                    # current PIN (`account.local_pin`), as the real service checks them.
                    change = message[asked]
                    password_ok = local_sign_in["password"] and change.get("password") is not None \
                        and change.get("password") == spec.get("local_password")
                    pin_ok = change.get("currentPin") is not None and change.get("currentPin") == pins_set[0]
                    if asked == "setPasswordSignIn" and change.get("enabled"):
                        proved = change.get("password") == spec.get("local_password")
                    else:
                        proved = password_ok or pin_ok
                    if not proved:
                        answer = {"refused": {"reason": "wrongPin" if "currentPin" in change else "wrongPassword"}}
                    elif asked == "setPin" and not re.fullmatch(r"[0-9]{4,16}", change.get("pin", "")):
                        answer = {"refused": {"reason": "unusablePin"}}
                    elif asked == "removePin" and not local_sign_in["password"]:
                        answer = {"refused": {"reason": "passwordSignInOff"}}
                    elif asked == "setPasswordSignIn" and not change.get("enabled") and local_sign_in["pin"] == "none":
                        answer = {"refused": {"reason": "noPin"}}
                    else:
                        if asked == "setPin":
                            pins_set[0] = change["pin"]
                            local_sign_in["pin"] = "set"
                        elif asked == "removePin":
                            pins_set[0] = None
                            local_sign_in["pin"] = "none"
                        else:
                            local_sign_in["password"] = bool(change.get("enabled"))
                        answer = {"localSignIn": dict(local_sign_in)}
                    record(application, asked, next(iter(answer)) if "localSignIn" in answer
                           else answer["refused"]["reason"])
                    connection.send(json.dumps(answer).encode())
                elif message == {"remove": {}} and application == "so.oxy.settings":
                    record(application, asked, "removed")
                    # The real service then ends the account's sessions, this one with them.
                    connection.send(json.dumps("removed").encode())
                else:
                    record(application, asked, "refused")
                    connection.send(json.dumps({"refused": {"reason": "notPermitted"}}).encode())
                    return

    def serve():
        while True:
            connection, _ = listener.accept()
            threading.Thread(target=converse, args=(connection,), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()
    return listener


def account_later(changes, report_path):
    """The played service's snapshot changing while the run goes on (`account.later`): each
    [{"when_shown": TEXT, "snapshot": {...}}] once the application's window first shows TEXT, the
    revision one past the last."""
    started = time.monotonic_ns()

    def watch():
        waiting = list(changes)
        seen = 0
        deadline = time.monotonic() + 180
        while waiting and time.monotonic() < deadline:
            lines = report_path.read_text().splitlines() if report_path.exists() else []
            for line in lines[seen:]:
                try:
                    entry = json.loads(line)
                except ValueError:
                    continue
                if waiting and entry.get("kind") == "frame" and entry.get("surface") == 1 \
                        and entry.get("t", 0) >= started \
                        and any(waiting[0]["when_shown"] in shown for shown in entry.get("texts", [])):
                    change = waiting.pop(0)
                    current = ACCOUNT["snapshot"]
                    ACCOUNT["push"](dict(current, **change["snapshot"], revision=current["revision"] + 1))
            seen = len(lines)
            time.sleep(0.02)

    threading.Thread(target=watch, daemon=True).start()


def played_oxy(spec, log):
    """Oxy's API as the login screen's setup reaches it (`login.oxy`): the SDK's own sign-in and the
    calls around it, answered in the API's shapes for the one account the scenario plays
    (`identifier`, `password`, `id`, `username`, `has_password`, `has_email`) and the device credential its
    sign-in commits (`device`: {deviceId, deviceSecret}, the run's own, no credential of anyone's).
    Every request is logged by its method and path, never its body; one not played is answered 404
    and logged so. Returns its base URL and the server.

    As the API does, a sign-in's access token is the only way into the account's routes (`/users/me`
    and what is under it, `/accounts`, `/session/device/*`): a request without it, or with one the
    API no longer takes, is answered 401. A sign-in puts the account on the sign-in's device (the
    claim's `finalizeDeviceLogin`), so the device's state and directory list it from then on, as
    `POST /session/device/add`, `GET /session/device/state` and `GET /session/device/directory`
    answer them. `qr.token_refused_at` ("METHOD PATH") plays what production did on 2026-11-08: the
    first QR sign-in's token refused when it first reaches that request (the API task that approved
    the request held the session as it was before the claim bound it, OxyHQ/oxy#2224), and never
    taken again."""
    import secrets as _secrets

    state = {"has_password": spec.get("has_password", True), "requests": {}, "tokens": {},
             "on_device": False, "revision": 0, "qr_claims": 0}
    lock = threading.Lock()

    class Oxy(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def reply(self, status, body):
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            with lock, open(log, "a") as out:
                # A device sign-in's secret token is in its status path: logged as `{token}`.
                logged = re.sub(r"^/auth/session/status/.*", "/auth/session/status/{token}", self.path.split("?")[0])
                out.write(json.dumps({"t": time.monotonic_ns(), "request": f"{self.command} {logged}",
                                      "status": status}) + "\n")

        def body(self):
            length = int(self.headers.get("Content-Length") or 0)
            try:
                return json.loads(self.rfile.read(length) or b"{}")
            except ValueError:
                return {}

        def session(self, refused_at=None):
            """A sign-in's session: its token taken by the account's routes, and the account on the
            sign-in's device."""
            # Shaped as the API's tokens are (a JWT the SDK reads its expiry from; an opaque one it
            # would not send), signed by nobody: the played API knows its tokens by value.
            def part(value):
                return base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip("=")
            now = int(time.time())
            token = ".".join((part({"alg": "none", "typ": "JWT"}),
                              part({"sub": spec["id"], "sid": "played-session", "type": "access",
                                    "iat": now, "exp": now + 900, "jti": _secrets.token_hex(8)}),
                              "played"))
            with lock:
                state["tokens"][token] = {"refused_at": refused_at, "refused": False}
                if not state["on_device"]:
                    state["on_device"] = True
                    state["revision"] += 1
            return {"sessionId": "played-session", "deviceId": spec["device"]["deviceId"],
                    "deviceSecret": spec["device"]["deviceSecret"],
                    "expiresAt": "2026-11-13T12:00:00.000Z", "accessToken": token,
                    "user": {"id": spec["id"], "username": spec["username"]}}

        def signed_in(self, request):
            """Whether the request carries a token the API takes; answers 401 when it does not."""
            header = self.headers.get("Authorization", "")
            token = header[len("Bearer "):] if header.startswith("Bearer ") else None
            with lock:
                held = state["tokens"].get(token)
                if held and held["refused_at"] == request:
                    held["refused"] = True
                taken = held is not None and not held["refused"]
            if not taken:
                self.reply(401, {"error": "invalid_token", "message": "Invalid or expired session"})
            return taken

        def device_state(self):
            """The sign-in's device as `GET /session/device/state` answers it: the account on it,
            active, once a sign-in put it there."""
            with lock:
                on, revision = state["on_device"], state["revision"]
            accounts = [{"accountId": spec["id"], "sessionId": "played-session", "authuser": 0}] if on else []
            return {"deviceId": spec["device"]["deviceId"], "accounts": accounts,
                    "activeAccountId": spec["id"] if on else None, "revision": revision,
                    "updatedAt": 1794096000000}

        def device_directory(self):
            """The device's directory (`GET /session/device/directory`): its one person, acting as
            themself, once a sign-in put them on it."""
            device = self.device_state()
            profile = {"id": spec["id"], "username": spec["username"], "name": self.user()["name"],
                       "avatar": None, "color": None}
            principals = [{"id": "played-principal", "userId": spec["id"], "authuser": 0, "user": profile,
                           "contexts": [{"id": "played-context", "accountId": spec["id"], "kind": "personal",
                                         "relationship": "self", "account": profile, "onDevice": True,
                                         "available": True, "active": True, "lastUsedAt": None}]}] \
                if device["accounts"] else []
            return {"deviceId": device["deviceId"], "revision": device["revision"],
                    "activeContextId": "played-context" if principals else None, "principals": principals,
                    "updatedAt": device["updatedAt"]}

        def user(self):
            return {"id": spec["id"], "username": spec["username"],
                    "name": {"first": spec.get("first", spec["username"]), "last": spec.get("last", ""),
                             "full": spec.get("name", spec["username"])}}

        def log(self, what):
            with lock, open(log, "a") as out:
                out.write(json.dumps({"t": time.monotonic_ns(), "request": what, "status": 101}) + "\n")

        def socket_io(self):
            """Socket.IO (Engine.IO 4) over a WebSocket, as Oxy's API serves it to the SDK: its
            `/auth-session` namespace, which a QR's screen joins with its secret token (logged as
            `{token}`) to be told the instant a phone approves (`auth_update`, a signal the screen
            answers by asking the status). With `qr.approve` "socket" the phone approves once the
            screen joined: only the socket can tell it before the SDK's next status check, twelve
            seconds on."""
            websocket_accept(self)
            self.log("WS /socket.io/")
            # Frames are written from the approval's thread too.
            writing = threading.Lock()
            websocket_send(self.wfile, 0x1, '0{"sid":"played","upgrades":[],"pingInterval":25000,'
                                            '"pingTimeout":20000,"maxPayload":1000000}')
            while (frame := websocket_read(self.rfile)) is not None:
                opcode, payload = frame
                if opcode == 0x8:
                    with writing:
                        websocket_send(self.wfile, 0x8, payload)
                    return
                if opcode != 0x1:
                    continue
                packet = payload.decode()
                if packet.startswith("40") and not packet.startswith("40/"):
                    # The device's own namespace, which a signed-in SessionClient joins with its
                    # token: connected while the API takes the token, else refused (`44`).
                    try:
                        auth = json.loads(packet[2:] or "{}")
                    except ValueError:
                        auth = {}
                    with lock:
                        held = state["tokens"].get(auth.get("token") if isinstance(auth, dict) else None)
                        taken = held is not None and not held["refused"]
                    self.log("WS / connect" if taken else "WS / refused")
                    with writing:
                        websocket_send(self.wfile, 0x1, '40{"sid":"played-device"}' if taken
                                       else '44{"message":"Authentication error"}')
                elif packet.startswith("40/auth-session"):
                    self.log("WS /auth-session connect")
                    with writing:
                        websocket_send(self.wfile, 0x1, '40/auth-session,{"sid":"played-auth"}')
                elif packet.startswith("42/auth-session,"):
                    event = json.loads(packet[len("42/auth-session,"):])
                    if event[:1] != ["join"] or len(event) < 2:
                        continue
                    self.log("WS /auth-session join {token}")
                    if spec.get("qr", {}).get("approve") == "socket":
                        threading.Thread(target=self.approve, args=(event[1], writing), daemon=True).start()

        def approve(self, token, writing):
            """The phone's approval, `qr.approve_after_ms` (default 0) after the screen joined, told
            on the socket at once."""
            time.sleep(spec.get("qr", {}).get("approve_after_ms", 0) / 1000)
            with lock:
                request = state["requests"].get(token)
                approved = request is not None and request["status"] == "pending"
                if approved:
                    request["status"] = "authorized"
            if not approved:
                return
            try:
                with writing:
                    websocket_send(self.wfile, 0x1, '42/auth-session,["auth_update",{"status":"authorized"}]')
                self.log("WS /auth-session auth_update")
            except OSError:
                pass

        def do_GET(self):
            path = self.path.split("?")[0]
            if path == "/socket.io/" and self.headers.get("Upgrade", "").lower() == "websocket":
                try:
                    self.socket_io()
                except (OSError, ValueError, struct.error):
                    pass
            elif path in ("/users/me/sign-in-methods", "/users/me", "/accounts") or path.startswith("/session/device/"):
                if not self.signed_in(f"GET {path}"):
                    pass
                elif path == "/users/me/sign-in-methods":
                    self.reply(200, {"hasEmail": spec.get("has_email", True), "hasPassword": state["has_password"], "totpEnabled": False,
                                     "backupCodesRemaining": 0})
                elif path == "/users/me":
                    self.reply(200, self.user())
                elif path == "/accounts":
                    self.reply(200, {"accounts": []})
                elif path == "/session/device/state":
                    self.reply(200, {"state": self.device_state(), "activeToken": None})
                elif path == "/session/device/directory":
                    self.reply(200, self.device_directory())
                else:
                    self.reply(404, {"error": "not_found", "message": "not played"})
            elif path == "/auth/oauth/client/oxy_pk_played":
                # OxyOS's registry entry as the API answers it: Oxy's own, official, so its sign-in
                # takes the device lane (an application answered with less is classified a third
                # party's and signed in on an isolated OAuth session, which production never is).
                self.reply(200, {"application": {"id": "played-oxyos", "name": "OxyOS", "type": "first_party",
                                                 "isOfficial": True, "isInternal": False, "scopes": [],
                                                 "description": "Official Oxy desktop shell for Linux.",
                                                 "websiteUrl": "https://oxy.so/os"}})
            elif path in ("/", "/health"):
                self.reply(200, {"ok": True})
            elif path.startswith("/auth/session/status/"):
                # The screen asking whether its QR was approved. With `qr.approve` a phone approved
                # it by the time the screen first asks: the stand-in for a person scanning it with
                # Commons and pressing Allow (Oxy's approval itself is Oxy's, never the machine's).
                with lock:
                    request = state["requests"].get(path.rsplit("/", 1)[1])
                    if request and spec.get("qr", {}).get("approve") is True and request["status"] == "pending":
                        request["status"] = "authorized"
                if not request:
                    self.reply(404, {"error": "not_found", "message": "Session not found"})
                elif request["status"] == "authorized":
                    self.reply(200, {"authorized": True, "status": "authorized", "sessionId": "played-session",
                                     "purpose": "device_sign_in"})
                else:
                    self.reply(200, {"authorized": False, "status": request["status"], "purpose": "device_sign_in"})
            else:
                self.reply(404, {"error": "not_found", "message": "not played"})

        def do_POST(self):
            path = self.path.split("?")[0]
            body = self.body()
            if path == "/auth/signin/email/start":
                self.reply(200, {"requestId": "played-request", "requestSecret": "p" * 43,
                                 "expiresAt": int(time.time() * 1000) + 600000})
            elif path == "/auth/signin/email/collect":
                self.reply(200, {"status": "pending", "expiresAt": int(time.time() * 1000) + 600000})
            elif path == "/auth/signin/email/confirm":
                if body.get("code") == spec["code"]:
                    self.reply(200, self.session())
                else:
                    self.reply(400, {"error": "invalid_code", "code": "INVALID_CODE", "message": "Invalid code"})
            elif path == "/auth/session/create":
                # The sign-in's QR (`qr`): a request a phone approves, its secret token the screen's.
                token, code = body.get("sessionToken"), _secrets.token_hex(8)
                if not token or body.get("clientId") != "oxy_pk_played":
                    self.reply(400, {"error": "invalid_request", "message": "Invalid request"})
                else:
                    with lock:
                        state["requests"][token] = {"code": code, "status": "pending"}
                    self.reply(200, {"authorizeCode": code, "qrPayload": f"oxycommons://approve?code={code}",
                                     "approvalUrl": f"https://auth.oxy.so/device?user_code={code}",
                                     "expiresAt": body.get("expiresAt"), "status": "pending"})
            elif path.startswith("/auth/session/deny/"):
                # A request the screen withdrew (the QR left the screen while still approvable).
                code = path.rsplit("/", 1)[1]
                with lock:
                    for request in state["requests"].values():
                        if request["code"] == code and request["status"] == "pending":
                            request["status"] = "cancelled"
                self.reply(200, {"success": True})
            elif path == "/auth/session/claim":
                # The approved request exchanged once for the session, and the device credential.
                with lock:
                    request = state["requests"].get(body.get("sessionToken"))
                    ok = request is not None and request["status"] == "authorized"
                    if ok:
                        request["status"] = "consumed"
                if ok:
                    with lock:
                        state["qr_claims"] += 1
                        first = state["qr_claims"] == 1
                    refused_at = spec.get("qr", {}).get("token_refused_at") if first else None
                    self.reply(200, dict(self.session(refused_at), user=self.user()))
                else:
                    self.reply(400, {"error": "invalid_request", "message": "Invalid or expired session"})
            elif path == "/users/me/reauth/email":
                if self.signed_in(f"POST {path}"):
                    self.reply(200, {"verificationId": "played-reauth",
                                     "expiresAt": int(time.time() * 1000) + 600000})
            elif path == "/session/device/add":
                # The account is on the device already (the sign-in put it there): the device as is.
                if self.signed_in(f"POST {path}"):
                    self.reply(200, {"state": self.device_state(), "activeToken": None})
            elif path == "/auth/signin/password":
                if body.get("identifier") == spec["identifier"] and body.get("password") == spec.get("password") \
                        and state["has_password"]:
                    self.reply(200, self.session())
                else:
                    self.reply(401, {"error": "invalid_credentials", "code": "INVALID_CREDENTIALS",
                                     "message": "Invalid credentials"})
            else:
                self.reply(404, {"error": "not_found", "message": "not played"})

        def do_PUT(self):
            path = self.path.split("?")[0]
            body = self.body()
            if path == "/users/me/password":
                # The SDK's own flow of adding a password, confirmed with a code sent by email.
                if not self.signed_in(f"PUT {path}"):
                    pass
                elif json.dumps(body.get("reauth", {})).find(spec["code"]) >= 0 and body.get("newPassword"):
                    state["has_password"] = True
                    self.reply(200, {"success": True})
                else:
                    self.reply(400, {"error": "invalid_code", "code": "INVALID_CODE", "message": "Invalid code"})
            else:
                self.reply(404, {"error": "not_found", "message": "not played"})

        do_DELETE = do_PUT

    ThreadingHTTPServer.request_queue_size = 64
    server = ThreadingHTTPServer(("127.0.0.1", 0), Oxy)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}", server


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
    atexit.register(shutil.rmtree, sockets, True)
    accountd, greeter, record = sockets / "accountd", sockets / "greeter", out / "login-record"
    # A PIN (docs/adr/0020-oxy-account-login.mdx decision 2): its own service, as the image's
    # (tools/oxyos-login/pam.d/oxyos-login-pin), oxyos-pin-check asking the accounts' stand-in.
    pin_check = pathlib.Path(offscreen).resolve().parent / "oxyos-pin-check"
    (pam / "oxyos-login-pin").write_text(
        f"auth [success=1 default=ignore] pam_exec.so quiet expose_authtok {pin_check} --socket {accountd}\n"
        "auth requisite pam_deny.so\nauth required pam_permit.so\n"
        "account required pam_permit.so\nsession required pam_permit.so\n")
    record.unlink(missing_ok=True)
    # Online, Oxy's API is played (`login.oxy`) and the accounts' service names it, with a client id.
    for log in ("oxy-api.jsonl", "login-accounts.jsonl"):
        (out / log).unlink(missing_ok=True)
    ACCOUNT["setup_log"] = out / "login-accounts.jsonl"
    ACCOUNT["oxy_log"] = out / "oxy-api.jsonl"
    oxy = played_oxy(login["oxy"], out / "oxy-api.jsonl") if "oxy" in login else None
    if oxy:
        login = dict(login, platform={"api": oxy[0], "clientId": "oxy_pk_played"})
    serve_accounts(accountd, login.get("accounts", []), login, out / "login-accounts.jsonl")
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
           "OXYOS_OXY_API": oxy[0] if oxy else f"http://127.0.0.1:{refusing.getsockname()[1]}/"}
    return process, greeter, record, env, (refusing, sockets)


# The server side of a WebSocket (RFC 6455) for the scenarios' local servers: just what the probes
# need of it (an upgrade answered, frames read unmasked and written, a fragmented message joined),
# not a client, and nothing an application runs. The client under test is the host's, tungstenite's.
WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def websocket_accept(handler, protocol=None):
    """Answers `handler`'s upgrade request (101), choosing `protocol`; the connection is the socket's
    from here."""
    key = handler.headers.get("Sec-WebSocket-Key", "")
    accept = base64.b64encode(hashlib.sha1((key + WEBSOCKET_GUID).encode()).digest()).decode()
    lines = ["HTTP/1.1 101 Switching Protocols", "Upgrade: websocket", "Connection: Upgrade",
             f"Sec-WebSocket-Accept: {accept}"]
    if protocol:
        lines.append(f"Sec-WebSocket-Protocol: {protocol}")
    handler.wfile.write(("\r\n".join(lines) + "\r\n\r\n").encode())
    handler.wfile.flush()
    handler.close_connection = True


def websocket_read(rfile):
    """The next message or control frame: (opcode, payload), a fragmented message joined; None once
    the connection ends."""
    message, kind = b"", None
    while True:
        head = rfile.read(2)
        if len(head) < 2:
            return None
        fin, opcode, masked, length = head[0] & 0x80, head[0] & 0x0F, head[1] & 0x80, head[1] & 0x7F
        if length == 126:
            length = struct.unpack("!H", rfile.read(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", rfile.read(8))[0]
        mask = rfile.read(4) if masked else b"\0\0\0\0"
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(rfile.read(length)))
        if opcode >= 0x8:
            return opcode, payload
        kind = kind if opcode == 0 else opcode
        message += payload
        if fin:
            return kind, message


def websocket_send(wfile, opcode, payload=b""):
    """One unmasked frame, as a server sends it."""
    payload = payload.encode() if isinstance(payload, str) else payload
    length = len(payload)
    if length < 126:
        head = struct.pack("!BB", 0x80 | opcode, length)
    elif length < 1 << 16:
        head = struct.pack("!BBH", 0x80 | opcode, 126, length)
    else:
        head = struct.pack("!BBQ", 0x80 | opcode, 127, length)
    wfile.write(head + payload)
    wfile.flush()


def websocket_close(wfile, code, reason=""):
    websocket_send(wfile, 0x8, struct.pack("!H", code) + reason.encode())


class ProbeServer(BaseHTTPRequestHandler):
    """The routes the probes of scenarios/network.json, websocket.json and file-system.json call
    (sample/NetworkProbe.tsx, sample/WebSocketProbe.tsx, sample/FileSystemProbe.tsx)."""

    def log_message(self, *args):
        pass

    def websocket(self):
        """A WebSocket's routes (sample/WebSocketProbe.tsx): /echo sends back each message and
        answers a ping with a pong and the text "ping heard"; /header says the X-Probe header it was
        opened with, then echoes; /count sends "0" to "49"; /close closes at once with 4001; /slow
        answers its upgrade three seconds late. The first protocol offered is chosen."""
        path = self.path.split("?")[0]
        if path == "/slow":
            time.sleep(3)
        offered = [p.strip() for p in self.headers.get("Sec-WebSocket-Protocol", "").split(",") if p.strip()]
        try:
            websocket_accept(self, offered[0] if offered else None)
            if path == "/header":
                websocket_send(self.wfile, 0x1, self.headers.get("X-Probe", "none"))
            elif path == "/count":
                for n in range(50):
                    websocket_send(self.wfile, 0x1, str(n))
            elif path == "/close":
                websocket_close(self.wfile, 4001, "the server says bye")
            while (frame := websocket_read(self.rfile)) is not None:
                opcode, payload = frame
                if opcode in (0x1, 0x2):
                    websocket_send(self.wfile, opcode, payload)
                elif opcode == 0x9:
                    websocket_send(self.wfile, 0xA, payload)
                    websocket_send(self.wfile, 0x1, "ping heard")
                elif opcode == 0x8:
                    # The close answered with its own code and reason, then the connection ended,
                    # as a server ends it.
                    websocket_send(self.wfile, 0x8, payload)
                    return
        except (OSError, struct.error):
            return

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
        if self.headers.get("Upgrade", "").lower() == "websocket":
            self.websocket()
        elif self.path == "/json":
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


def gif_bytes(width, height, frames, loops=0):
    """An animated GIF of `frames` ([["#rrggbb", hundredths], ...]), each all one colour, played
    `loops` times (0: forever; NETSCAPE2.0), each frame replacing the one before."""
    colours = sorted({colour for colour, _ in frames})
    bits = max(1, (len(colours) - 1).bit_length())
    palette = b"".join(bytes.fromhex(c[1:]) for c in colours) + b"\0\0\0" * ((1 << bits) - len(colours))
    out = bytearray(b"GIF89a" + struct.pack("<HHBBB", width, height, 0x80 | (bits - 1), 0, 0) + palette)
    out += b"\x21\xff\x0bNETSCAPE2.0\x03\x01" + struct.pack("<H", loops) + b"\x00"
    def lzw(indices, minimum):
        # A clear code before every pixel keeps every code at minimum + 1 bits: valid, if not small.
        clear, end, size = 1 << minimum, (1 << minimum) + 1, minimum + 1
        codes = [code for index in indices for code in (clear, index)] + [end]
        bitstream, count, data = 0, 0, bytearray()
        for code in codes:
            bitstream |= code << count
            count += size
            while count >= 8:
                data.append(bitstream & 0xff)
                bitstream >>= 8
                count -= 8
        if count:
            data.append(bitstream & 0xff)
        return bytes(data)
    minimum = max(2, bits)
    for colour, delay in frames:
        out += b"\x21\xf9\x04" + struct.pack("<BHB", 1 << 2, delay, 0) + b"\x00"
        out += b"\x2c" + struct.pack("<HHHHB", 0, 0, width, height, 0)
        data = lzw([colours.index(colour)] * (width * height), minimum)
        out.append(minimum)
        for at in range(0, len(data), 255):
            block = data[at:at + 255]
            out += bytes([len(block)]) + block
        out += b"\x00"
    return bytes(out + b"\x3b")


def png_bytes(width, height, colour, right=None):
    """A PNG of one colour (8-bit RGB, rows unfiltered), or with `right` its right half another."""
    rgb = lambda text: bytes(int(text[i:i + 2], 16) for i in (1, 3, 5))
    row = rgb(colour) * width if right is None else rgb(colour) * (width // 2) + rgb(right) * (width - width // 2)
    raw = b"".join(b"\x00" + row for _ in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def with_home(value, preferences):
    """An expected value with "{home}" in its texts the run's home (`home`, made afresh in the run's
    directory beside its preferences): a path the person chose there, which no two runs share."""
    if isinstance(value, str):
        return value.replace("{home}", str((preferences.parent / "home").resolve())) if preferences else value
    if isinstance(value, list):
        return [with_home(item, preferences) for item in value]
    if isinstance(value, dict):
        return {key: with_home(item, preferences) for key, item in value.items()}
    return value


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
    write_xattrs(root, home.get("xattrs", {}))
    os.utime(root, (modified, modified))
    return root


def xattr_bytes(value):
    """An extended attribute's value as `home.xattrs` gives it: text, or {"bplist": [TEXT]}."""
    if isinstance(value, str):
        return value.encode()
    return plistlib.dumps(value["bplist"], fmt=plistlib.FMT_BINARY)


def write_xattrs(root, xattrs):
    """Extended attributes as `home.xattrs` describes them, as another program (setfattr) sets
    them; an item's modification time is kept."""
    for relative, attributes in xattrs.items():
        path = root / relative.rstrip("/")
        for name, value in attributes.items():
            os.setxattr(path, name, xattr_bytes(value))


def wants_xattrs(scenario):
    """Whether a scenario sets or checks extended attributes, so the home's are recorded."""
    home = scenario.get("home", {})
    return bool(home.get("xattrs") or any(later.get("xattrs") for later in home.get("later", []))
                or any("xattrs" in step.get("expect", {}).get("home", {}) for step in scenario.get("steps", [])))


def write_items(root, items, modified):
    """Items as `home.items` describes them, each modified at `modified`."""
    made = []
    for relative, what in items.items():
        path = root / relative.rstrip("/")
        if relative.endswith("/"):
            path.mkdir(parents=True, exist_ok=True)
            if isinstance(what, dict) and "tree" in what:
                folders, files = what["tree"]
                for folder in range(folders):
                    inside = path / f"folder-{folder}"
                    inside.mkdir()
                    for file in range(files):
                        (inside / f"file-{file}.js").touch()
                        made.append(inside / f"file-{file}.js")
                    made.append(inside)
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
    window first shows its text (or, with `in_shell`, once the shell shows it), from a thread
    reading the hosts' reports as they are written."""
    later = list(home.get("later", []))
    if not later:
        return None
    modified = home.get("modified", 1767348000)
    shell_path = report_path.with_name(report_path.name + ".shell")

    def watch():
        seen = {report_path: 0, shell_path: 0}
        shown = {report_path: [], shell_path: []}
        due = None
        deadline = time.monotonic() + 180
        while later and time.monotonic() < deadline:
            for path in (report_path, shell_path):
                lines = path.read_text().splitlines() if path.exists() else []
                for line in lines[seen[path]:]:
                    try:
                        entry = json.loads(line)
                    except ValueError:
                        continue
                    # The application's window, or any of the shell's surfaces.
                    if entry.get("kind") != "frame" or (path == report_path and entry.get("surface") != 1):
                        continue
                    shown[path] = entry.get("texts", [])
                    # A change made as its text shows, before the window draws anything after it.
                    while later and "after_ms" not in later[0] and \
                            any(later[0]["when_shown"] in text for text in shown[watched(later[0])]):
                        change = later.pop(0)
                        write_items(home_root, change.get("items", {}), modified)
                        write_xattrs(home_root, change.get("xattrs", {}))
                seen[path] = len(lines)
            # One made a while after its text showed, which comes while the window may draw nothing.
            if later and due is None and any(later[0]["when_shown"] in text for text in shown[watched(later[0])]):
                due = time.monotonic() + later[0].get("after_ms", 0) / 1000
            if later and due is not None and time.monotonic() >= due:
                change = later.pop(0)
                write_items(home_root, change.get("items", {}), modified)
                write_xattrs(home_root, change.get("xattrs", {}))
                due = None
            time.sleep(0.02)

    def watched(change):
        return shell_path if change.get("in_shell") else report_path

    thread = threading.Thread(target=watch, daemon=True)
    thread.start()
    return thread


def offline_when_shown(text, report_path):
    """The machine going offline (`store.offline_when_shown`) once the application's window first
    shows `text`, from a thread reading the host's report as it is written."""

    # Only this run's frames: a report left by a run before may still be there.
    started = time.monotonic_ns()

    def watch():
        seen = 0
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            lines = report_path.read_text().splitlines() if report_path.exists() else []
            for line in lines[seen:]:
                try:
                    entry = json.loads(line)
                except ValueError:
                    continue
                if entry.get("kind") == "frame" and entry.get("surface") == 1 and entry.get("t", 0) >= started \
                        and any(text in shown for shown in entry.get("texts", [])):
                    STORE["go_offline"]()
                    return
            seen = len(lines)
            time.sleep(0.02)

    threading.Thread(target=watch, daemon=True).start()


class HomeState(dict):
    """What the home holds: each path in it, a folder as None and a file as its text (or its
    size, past 4 KB); and `xattrs`, each item's `user.` extended attributes ({PATH: {NAME:
    BYTES}}, items with none left out) when they are read."""

    def __init__(self, items=(), xattrs=None):
        super().__init__(items)
        self.xattrs = xattrs or {}

    def __eq__(self, other):
        return dict.__eq__(self, other) and self.xattrs == getattr(other, "xattrs", {})

    def __ne__(self, other):
        return not self == other


def home_state(root, xattrs=False):
    """What the home holds now (`HomeState`), its items' attributes too with `xattrs`."""
    state = HomeState()
    for path in root.rglob("*"):
        relative = str(path.relative_to(root))
        try:
            if path.is_dir() and not path.is_symlink():
                state[relative] = None
            else:
                size = path.stat().st_size
                state[relative] = path.read_text(errors="replace") if size <= 4096 else size
            if xattrs and not path.is_symlink():
                held = {name: os.getxattr(path, name) for name in os.listxattr(path) if name.startswith("user.")}
                if held:
                    state.xattrs[relative] = held
        except OSError:
            pass
    return state


class HomeRecorder:
    """The home as it was through the run, from a thread looking at it every 10 ms: each state
    with when it was first seen, on the monotonic clock the host's report uses, so each step is
    checked against the home as it was when the step settled."""

    def __init__(self, root, xattrs=False):
        self.root = root
        self.xattrs = xattrs
        self.states = []
        self.running = True
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while self.running:
            state = home_state(self.root, self.xattrs)
            if not self.states or self.states[-1][1] != state:
                self.states.append((time.monotonic_ns(), state))
            time.sleep(0.01)

    def stop(self):
        self.running = False
        self.thread.join()
        state = home_state(self.root, self.xattrs)
        if not self.states or self.states[-1][1] != state:
            self.states.append((time.monotonic_ns(), state))

    def at(self, t):
        """The home as it was at `t` (the last state seen by then)."""
        seen = [state for when, state in self.states if when <= t]
        return seen[-1] if seen else (self.states[0][1] if self.states else HomeState())


def check_home(state, wanted, fail):
    """A step's `home`: what the home held once it settled."""
    for relative, want in wanted.items():
        if relative == "xattrs":
            for path, attributes in want.items():
                held = getattr(state, "xattrs", {}).get(path.rstrip("/"), {})
                for name, value in attributes.items():
                    if value is False:
                        if name in held:
                            fail(f"{path} still has {name}: {held[name]!r}")
                    elif name not in held:
                        fail(f"{path} has no {name}")
                    elif isinstance(value, dict) and "bplist" in value:
                        try:
                            got = plistlib.loads(held[name])
                        except Exception as error:
                            fail(f"{path}'s {name} is not a property list: {error}")
                            continue
                        if not held[name].startswith(b"bplist00") or got != value["bplist"]:
                            fail(f"{path}'s {name} holds {got!r}, not the binary property list {value['bplist']!r}")
                    elif held[name] != xattr_bytes(value):
                        fail(f"{path}'s {name} holds {held[name]!r}, not {value!r}")
            continue
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


def png_reader(path):
    """A function giving the colour at (x, y) of an 8-bit RGB or RGBA PNG, any filter (a
    screenshot the session wrote, docs/adr/0024-system-shortcuts.mdx): (r, g, b) or (r, g, b, a);
    and its width, height and colour type."""
    data = pathlib.Path(path).read_bytes()
    width, height, depth, kind = struct.unpack(">IIBB", data[16:26])
    if depth != 8 or kind not in (2, 6):
        raise ValueError(f"{path}: not 8-bit RGB or RGBA (depth {depth}, colour type {kind})")
    channels = 3 if kind == 2 else 4
    compressed, at = b"", 8
    while at < len(data):
        length, chunk = struct.unpack(">I4s", data[at:at + 8])
        if chunk == b"IDAT":
            compressed += data[at + 8:at + 8 + length]
        at += 12 + length
    raw, stride = zlib.decompress(compressed), width * channels
    rows, previous = [], bytearray(stride)
    for y in range(height):
        kind_of_filter = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up = previous[i]
            corner = previous[i - channels] if i >= channels else 0
            if kind_of_filter == 1:
                line[i] = (line[i] + left) & 0xff
            elif kind_of_filter == 2:
                line[i] = (line[i] + up) & 0xff
            elif kind_of_filter == 3:
                line[i] = (line[i] + (left + up) // 2) & 0xff
            elif kind_of_filter == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                line[i] = (line[i] + (left if pa <= pb and pa <= pc else up if pb <= pc else corner)) & 0xff
        rows.append(line)
        previous = line

    def pixel(x, y):
        if not (0 <= x < width and 0 <= y < height):
            raise ValueError(f"{path}: ({x}, {y}) is outside {width}x{height}")
        return tuple(rows[y][x * channels:(x + 1) * channels])

    return pixel, width, height, kind


def check_screenshots(wanted, result, fail):
    """The step's `screenshots` (docs/adr/0024-system-shortcuts.mdx, decision 10): the files the
    session saved in it, exactly and in order, each matching the keys given ("name", a regular
    expression for its file's name; "folder", the run's folder it is in; "output", "width",
    "height"; "alpha", whether it has an alpha channel (a window's); "pixels", "x,y" in its own
    pixels -> "#rrggbb" or "#rrggbbaa"), and the screenshots the shell opened, each the application
    started for it ("application")."""
    told = result.get("screenshots") or {"saved": [], "opened": []}
    saved = told["saved"]
    if "saved" in wanted and len(saved) != len(wanted["saved"]):
        fail(f"screenshots saved: {saved}, expected {len(wanted['saved'])}")
    for want, got in zip(wanted.get("saved", []), saved):
        if "error" in got:
            fail(f"a screenshot was not saved: {got['error']}")
            continue
        path = pathlib.Path(got["path"])
        if "name" in want and not re.fullmatch(want["name"], path.name):
            fail(f"screenshot named {path.name!r}, expected {want['name']!r}")
        if "folder" in want and path.parent.name != want["folder"]:
            fail(f"screenshot in {path.parent}, expected the run's {want['folder']}")
        for key in ("output", "width", "height"):
            if key in want and got.get(key) != want[key]:
                fail(f"screenshot {path.name}: {key} {got.get(key)}, expected {want[key]}")
        if not path.is_file():
            fail(f"screenshot {path} is not there")
            continue
        pixel, width, height, kind = png_reader(path)
        if (width, height) != (got["width"], got["height"]):
            fail(f"screenshot {path.name} is {width}x{height}, told {got['width']}x{got['height']}")
        if "alpha" in want and (kind == 6) != want["alpha"]:
            fail(f"screenshot {path.name}: alpha {kind == 6}, expected {want['alpha']}")
        tolerance = want.get("pixel_tolerance", 0)
        for point, colour in want.get("pixels", {}).items():
            x, y = (int(value) for value in point.split(","))
            expected = tuple(int(colour[i:i + 2], 16) for i in range(1, len(colour), 2))
            got_colour = pixel(x, y)[:len(expected)]
            if any(abs(a - b) > tolerance for a, b in zip(got_colour, expected)):
                fail(f"screenshot {path.name} pixel {point} is "
                     f"#{''.join(f'{c:02x}' for c in pixel(x, y))}, expected {colour}")
    opened = told["opened"]
    if "opened" in wanted:
        if [o.get("application") for o in opened] != [w.get("application") for w in wanted["opened"]]:
            fail(f"screenshots opened: {opened}, expected {wanted['opened']}")
        for got in opened:
            if not any(got["path"] == s.get("path") for s in saved) and not pathlib.Path(got["path"]).is_file():
                fail(f"opened {got['path']}, which is not a screenshot saved")


def check_pixels(name, expect, screenshot, window, fail):
    """The step's `pixels`, against its screenshot of the output, the window at `window`."""
    pixel = png_pixel_reader(screenshot)
    tolerance = expect.get("pixel_tolerance", 3)
    for point, colour in expect["pixels"].items():
        x, y = (int(value) for value in point.split(","))
        # "!#rrggbb": anything but that colour (a window gone from where it was, over whatever
        # is beneath it).
        unlike = colour.startswith("!")
        colour = colour.lstrip("!")
        want = tuple(int(colour[i:i + 2], 16) for i in (1, 3, 5))
        got = pixel(window[0] + x, window[1] + y)
        if any(abs(a - b) > tolerance for a, b in zip(got, want)) != unlike:
            fail(f"pixel {point} is #{''.join(f'{c:02x}' for c in got)}, expected "
                 f"{'anything but ' if unlike else ''}{colour}")


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


def edge_stretch(distance):
    """EdgeEffect's dampStretchVector: the stretch drawn for a pull of `distance` (a share of the
    view's size)."""
    sign = 1 if distance > 0 else -1
    distance = abs(distance)
    return sign * (0.016 * distance + 0.016 * (1 - math.exp(-distance * math.e / 0.33)))


def check_stretch(wanted, report, start, end, fail):
    """The step's `stretch`: a scroll view stretched past an end, as Android 12's ScrollView."""
    view, axis = wanted["view"], wanted.get("axis", "y")
    lines = [line for line in report if start <= line["t"] <= end and line["kind"] == "stretch"
             and line.get("target") == view]
    amounts = [line[axis] for line in lines]
    if wanted.get("none"):
        if any(amount != 0 for amount in amounts):
            fail(f"{view} stretched {max(amounts, key=abs):.4f} along {axis}, expected no stretch")
        return
    if not amounts:
        fail(f"{view} never stretched")
        return
    if "pulled" in wanted:
        pulled = lines[-1]["pulled_" + axis]
        if abs(pulled - wanted["pulled"]) > 1e-4:
            fail(f"{view} is pulled {pulled:.5f} of its size at the step's end, expected {wanted['pulled']}")
        expected = edge_stretch(pulled) if pulled else 0
        if abs(amounts[-1] - expected) > 1e-4 or "held" in wanted and abs(amounts[-1] - wanted["held"]) > 1e-4:
            fail(f"{view} is stretched {amounts[-1]:.5f} at the step's end, expected {expected:.5f} for that pull "
                 f"(EdgeEffect's), {wanted.get('held')} by the scenario")
    if "peak" in wanted:
        side = wanted.get("side", 1)
        peak = max(amounts, key=abs)
        lo, hi = wanted["peak"]
        if peak * side <= 0 or not lo <= abs(peak) <= hi:
            fail(f"{view}'s largest stretch was {peak:.5f}, expected {lo} to {hi} on side {side}")
    if "caught_ms" in wanted:
        # Held still while stretched (fingers resting caught it): the longest time between two
        # changes of a stretch that still showed.
        held = max((b["t"] - a["t"]) / 1e6 for a, b in zip(lines, lines[1:]) if a[axis] != 0) if len(lines) > 1 else 0
        if held < wanted["caught_ms"]:
            fail(f"{view}'s stretch was held still at most {held:.0f} ms, expected at least {wanted['caught_ms']} ms")
    if wanted.get("rests"):
        if amounts[-1] != 0:
            fail(f"{view} is still stretched {amounts[-1]:.5f} at the step's end")
        top = max(range(len(amounts)), key=lambda i: abs(amounts[i]))
        falling = [abs(a) for a in amounts[top:]]
        if any(b > a + 1e-6 for a, b in zip(falling, falling[1:])):
            fail(f"{view}'s stretch did not only shrink once it began to: {[round(a, 4) for a in amounts[top:]]}")
        if len(falling) < wanted.get("frames_min", 0):
            fail(f"{view}'s stretch went back in {len(falling) - 1} changes, expected at least {wanted['frames_min']}")


def offset_before(report, view, start):
    """Where a scroll view was before a step began: its last scroll's offset, else 0."""
    ys = [line["y"] for line in report if line["kind"] == "deliver" and line["event"] == "scroll"
          and line["t"] < start and scroll_view_name(line) == view]
    return ys[-1] if ys else 0


def check_glide(wanted, report, start, end, fail):
    """The step's `glide`: a scroll view flung by a touchpad's fingers, as Android flings it."""
    view = wanted["view"]
    lines = [line for line in report if start <= line["t"] <= end
             and scroll_view_name(line) == view]
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
    if wanted.get("held"):
        holds = [line for line in report if start <= line["t"] <= end and line["kind"] == "input"
                 and line.get("event") == "hold.begin"]
        if len(holds) != 1:
            fail(f"the host handled {len(holds)} hold begins, expected one")
            return
        held = holds[0]["t"]
        ends = [line for line in after if line["event"] == "momentumScrollEnd"]
        if len(ends) != 1:
            fail(f"{view} ended its momentum {len(ends)} times, expected once")
        if after[-1]["t"] < held or after[-1]["t"] - held > 2e6:
            fail(f"{view}'s momentum ended {(after[-1]['t'] - held) / 1e6:.1f} ms from the fingers' rest, "
                 f"expected as the host handled it")
        late = [line for line in after if line["event"] == "scroll" and line["t"] > held]
        if late:
            fail(f"{view} scrolled {len(late)} times after the fingers came to rest")
        if abs(covered) >= abs(distance) - 1:
            fail(f"{view} glided {covered:.1f} px of the fling's {distance:.1f}: the resting fingers did not stop it")
        return
    if reached_end:
        if abs(covered) >= abs(distance) + 1:
            fail(f"{view} glided {covered:.1f} px, past the fling's {distance:.1f}")
        if abs(ended - wanted.get("end_offset", ended)) > 0.01:
            fail(f"{view} stopped at {ended}, expected its end {wanted['end_offset']}")
    elif abs(covered - distance) > 1:
        fail(f"{view} glided {covered:.1f} px, expected the fling's {distance:.1f}")


def check_pixels_match(name, expect, screenshot, window, fail):
    """The step's `pixels_match`: pairs of points of the window showing the same colour."""
    pixel = png_pixel_reader(screenshot)
    tolerance = expect.get("pixel_tolerance", 3)
    hex_of = lambda c: "#" + "".join(f"{v:02x}" for v in c)
    for a, b in expect["pixels_match"]:
        (ax, ay), (bx, by) = ((int(v) for v in p.split(",")) for p in (a, b))
        got_a, got_b = pixel(window[0] + ax, window[1] + ay), pixel(window[0] + bx, window[1] + by)
        if any(abs(p - q) > tolerance for p, q in zip(got_a, got_b)):
            fail(f"pixel {a} is {hex_of(got_a)} and pixel {b} {hex_of(got_b)}, expected the same")


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


SCROLL_EVENTS = ("scroll", "scrollBeginDrag", "scrollEndDrag", "momentumScrollBegin", "momentumScrollEnd")
SCROLL_CHECKS = ("react", "scroll_offset", "scroll_moved", "glide")


def scroll_view_name(line):
    """A scroll view's name in a step's expectations: its testID (or accessibility label), else
    its component in the nearest view with a testID (`within`), "ScrollView in settings-rail";
    never its tag, which every view mounted before it moves. A line without `within` (a fling's, a
    stretch's) names a scroll view by its target."""
    tagged = re.fullmatch(r"(\w+)#[0-9]+", str(line.get("target")))
    if tagged and line.get("within"):
        return f"{tagged[1]} in {line['within']}"
    return line.get("target")


def react_events(report, start, end):
    """The events delivered to React in a step, as "event@name" -> count (`react`): a view named by
    its testID, else the nearest ancestor's (`within`), as the host reports them; a scroll view by
    `scroll_view_name`."""
    got = {}
    for line in report:
        # Hover follows every pointer motion: the W3C events it makes are checked by what the
        # application shows of them.
        if line["kind"] == "deliver" and start <= line["t"] <= end and not line["event"].startswith("pointer"):
            name = scroll_view_name(line) if line["event"] in SCROLL_EVENTS else line.get("within") or line["target"]
            key = f"{line['event']}@{name}"
            got[key] = got.get(key, 0) + 1
    return got


def check_scroll_names(report, start, end, fail):
    """Two scroll views scrolling under one name in a step (two unnamed ones in one container)
    fail it, rather than add up as one."""
    scrollers = {}
    for line in report:
        if line["kind"] == "deliver" and start <= line["t"] <= end and line["event"] in SCROLL_EVENTS:
            scrollers.setdefault(scroll_view_name(line), set()).add(line["target"])
    for name, targets in scrollers.items():
        if len(targets) > 1:
            fail(f"{len(targets)} scroll views scrolled as {name} ({', '.join(sorted(targets))}): "
                 f"a testID of their own tells them apart")


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
    every, linear_ms = expect.get(f"{kind}_frames", 8), expect.get(f"{kind}_linear_ms")

    def channels(value):
        return [int(value[i:i + 2], 16) for i in (1, 3, 5)] if isinstance(value, str) else [value]

    for view, props in expect[kind].items():
        # The frames' bounds, the same for every view or by view ({testID: bounds}, 8 for one not
        # named): a view that must not move beside one that must.
        bounds = every.get(view, 8) if isinstance(every, dict) else every
        least, most = bounds if isinstance(bounds, list) else (bounds, None)
        least = frames_least(least)
        for prop, path_wanted in props.items():
            frames = got.get(view, {}).get(prop, [])
            if len(path_wanted) == 3:
                # Up to a peak and back (the dock's Trash under a drag): each part monotonic, the
                # peak and the end reached exactly.
                first, peak, last = path_wanted
                name = f"{view} {prop}"
                values = [value[0] for _, value in frames]
                if len(values) < least:
                    fail(f"{name} was set on {len(values)} frames, expected at least {least}")
                    continue
                top = max(range(len(values)), key=lambda i: (values[i] - first) * (peak - first))
                up, down = values[:top + 1], values[top:]
                shown = [round(v, 3) for v in values]
                if round(values[top], 3) != round(peak, 3) or round(values[-1], 3) != round(last, 3):
                    fail(f"{name} went {shown}, expected up to exactly {peak} and back to exactly {last}")
                elif any((b < a) if peak > first else (b > a) for a, b in zip(up, up[1:])) or \
                        any((b > a) if peak > last else (b < a) for a, b in zip(down, down[1:])):
                    fail(f"{name} went {shown}, not monotonically from {first} to {peak} and to {last}")
                continue
            first, last = path_wanted
            name = f"{view} {prop}"
            if len(frames) < least or (most is not None and len(frames) > most):
                fail(f"{name} was set on {len(frames)} frames, expected {least} to {most}" if most is not None
                     else f"{name} was set on {len(frames)} frames, expected at least {least}")
                continue
            if not frames:
                # None allowed, none set (`{kind}_frames` [0, 0]: a view that must not move).
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


CURVE_KEYS = ("eased", "still", "lasts_ms", "frame_gap_ms", "linear_ms", "refresh_frames")


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
    ends, ended = seen.get("ends_ms"), seen.get("ended_ms")
    if "lasts_ms" in wanted:
        low, high = wanted["lasts_ms"]
        # Its end judged by time, not by its frames (a loaded machine drops some, its last ones
        # too): its clock ends it so long in, every frame drawn in it before then (or at its end,
        # held there: a zoom waiting for its window's new frame), and the frame that ended it,
        # drawing it at its end, at or after.
        held = [step for step in path if ends is not None and step[0] > ends and step[2] != 1000]
        if ends is None or ended is None:
            fail(f"{name} did not end by its clock (ends {ends} ms in, ended {ended} ms in)")
        elif not low <= ends <= high:
            fail(f"{name} lasts {ends} ms, expected {low} to {high}")
        elif held or ended < ends:
            fail(f"{name} ended {ended} ms in, drawing {held} past its end {ends} ms in: {times}")
    if "frame_gap_ms" in wanted and SPEED_BUDGETS:
        gaps = sorted(b - a for a, b in zip(times, times[1:]))
        median, most = gaps[len(gaps) // 2], gaps[-1]
        if median > wanted["frame_gap_ms"][0] or most > wanted["frame_gap_ms"][1]:
            fail(f"{name}'s frames came {median} ms apart (median), {most} at most; expected at most "
                 f"{wanted['frame_gap_ms']}: {times}")
    if wanted.get("eased"):
        arriving = seen["arriving"]
        # Its length, by its clock; else up to a display frame after its last frame.
        span = ends if ends is not None else times[-1] + 1000 / refresh_hz
        opacities = [o if arriving else 255 - o for _, o, _ in path]
        moved = [m for _, _, m in path]
        for label, values, end in (("opacity", opacities, 255), ("size and place", moved, 1000)):
            # Nothing of it changes (a zoom's opacity, a fade's size): nothing to ease.
            if len(set(values)) == 1:
                continue
            if any(b < a for a, b in zip(values, values[1:])):
                fail(f"{name}'s {label} went back: {values}")
            # "place": only its size and place eased out; its opacity only never goes back (a drag's
            # image sliding back fades only as it arrives, a delayed fade).
            if wanted["eased"] == "place" and label == "opacity":
                continue
            # Eased out: in the first half of its time, from 34 ms on (a spring released from rest
            # starts slower than a line, whatever the refresh), half again ahead of a linear motion
            # of the same length (a spring released from rest is
            # quick at first and slow to arrive: a critically damped one is 0.81 of the way a third
            # of the way through, a linear one 0.33).
            behind = [(ms, v) for (ms, _, _), v in zip(path, values)
                      if 34 <= ms <= span / 2 and v + end * 0.02 < min(end, 1.5 * end * ms / span)]
            if behind:
                fail(f"{name}'s {label} was behind a linear motion at {behind}: not eased out ({values})")
    if "linear_ms" in wanted:
        # Driven by the frame clock, linearly (a window into the dock, ADR 0011): every frame's
        # size and place are where a linear motion of that length is at the frame's time, to the
        # thousandth it is recorded in, whenever the frame came.
        # A frame's time is recorded in whole milliseconds, cut short (`Step::ms`): its place is
        # between where the motion is at that millisecond and the next, to the thousandth.
        length = wanted["linear_ms"]
        off = [(ms, m) for ms, _, m in path
               if not 1000 * ms / length - 1 <= m <= min(1000, 1000 * (ms + 1) / length) + 1]
        if off:
            fail(f"{name} was not where a linear motion of {length} ms is at its frames' times: {off}")
    if "refresh_frames" in wanted:
        # Composed for each refresh while it runs: no more frames than refreshes in it (no load
        # adds one), and, held only with --speed-budgets, at least so many (the machine's rate).
        least, most = wanted["refresh_frames"]
        count = seen.get("frames", len(path))
        if count > most or count < frames_least(least):
            fail(f"{name} drew {count} frames, expected {frames_least(least)} to {most}")
        if any(b[0] <= a[0] or b[2] <= a[2] for a, b in zip(path, path[1:])):
            fail(f"{name} drew a frame twice at one time or one place: {path}")
    if wanted.get("still") and any(m != 1000 for _, _, m in path):
        fail(f"{name} grew, shrank or moved ({[m for _, _, m in path]}): expected a crossfade alone")


def ended_report(path):
    """The report of a host the director ended (a file chooser, the companion, an application the
    session launched): its lines, but a last one cut short with no newline, which a host stopped
    while writing it leaves; the director's own report is complete when read."""
    if not path.exists():
        return []
    text = path.read_text()
    lines = text.split("\n")
    if not text.endswith("\n"):
        lines = lines[:-1]
    return [json.loads(line) for line in lines if line.strip()]


def overlaps(a, b):
    """Whether two [x, y, width, height] rectangles share any pixel."""
    return a[0] < b[0] + b[2] and b[0] < a[0] + a[2] and a[1] < b[1] + b[3] and b[1] < a[1] + a[3]


def within(inner, outer):
    return (inner[0] >= outer[0] and inner[1] >= outer[1] and inner[0] + inner[2] <= outer[0] + outer[2]
            and inner[1] + inner[3] <= outer[1] + outer[3])


def rounded(value):
    """A value rounded half away from zero, as Rust's f64::round."""
    return int(math.floor(value + 0.5)) if value >= 0 else -int(math.floor(-value + 0.5))


def between(start, end, share):
    """`start` `share` of the way to `end`, edge by edge, as the compositor draws a flight."""
    left = rounded(start[0] + (end[0] - start[0]) * share)
    top = rounded(start[1] + (end[1] - start[1]) * share)
    right = max(rounded(start[0] + start[2] + (end[0] + end[2] - start[0] - start[2]) * share), left + 1)
    bottom = max(rounded(start[1] + start[3] + (end[1] + end[3] - start[1] - start[3]) * share), top + 1)
    return [left, top, right - left, bottom - top]


# The keyboard's input: only ever to the surface the keyboard focus is on (its Focus in, no Focus
# out since).
KEYBOARD_KINDS = {"key.press", "key.release", "text", "preedit", "text_input"}
# The pointer's: only ever to the surface it is over (its Enter, no Leave since). A scroll, a pinch
# and resting fingers go on reaching the surface they began on until they end (ADR 0042, ADR 0046),
# wherever the pointer goes meanwhile, and are not among them.
POINTER_KINDS = {"pointer.motion", "pointer.press", "pointer.release", "pointer.cancel"}
# What the pointer causes in React: delivered only on the surface its input reached.
POINTER_CAUSES = POINTER_KINDS | {"pointer.enter", "pointer.leave", "scroll", "touch.down", "touch.motion",
                                  "touch.up", "touch.cancel", "pinch.begin", "pinch.update", "pinch.end",
                                  "pinch.cancel", "hold.begin", "hold.end", "hold.cancel"}


# What the compositor's record holds that no Bloom host receives: its markers of a surface or a
# client gone, and the keys it gives the input method.
HOST_UNSEEN = {"destroyed", "disconnected", "im.key.press", "im.key.release"}


def host_reports(out):
    """Every Bloom host's report of the run (the application's, the shell's on each output, the
    companion's, the file choosers', the launched applications'), each as its lines."""
    return {path.name: ended_report(path) for path in sorted(out.glob("host-report.jsonl*")) if path.is_file()}


def check_input_delivery(summary, reports, failures):
    """The person's input reached exactly whom it should, once (docs/adr/0016-windows.mdx, decision
    11), checked for every scenario and every step from the compositor's record of every
    input event it routed (`deliveries`: when, to which client and process, which surface, its kind
    and serial) and each Bloom host's of what it received (`received`) and delivered to React
    (`deliver`, with the input that caused it):

    - the compositor routed the keyboard's input only to the surface with the keyboard focus, as its
      Focus events told it, one surface at a time; the pointer's only to the surface it is over, as
      its Enter and Leave told it, one surface at a time; a release only to the surface that has the
      press, with no event routed twice;
    - once each step settled, where it says the keyboard focus and the pointer are is where those
      events left them;
    - every host received each event routed to it exactly once, and nothing that was not;
    - no input made a host deliver the same React event to the same view twice, and the pointer's
      input never made one reach a view of another surface than the one it reached."""
    deliveries = summary.get("deliveries")
    if deliveries is None:
        failures.append("input delivery: the director recorded no deliveries (an oxyos-offscreen without them)")
        return
    steps = summary["steps"]
    bounds = [summary["t_launch"]] + [result.get("t_settled") or summary["t_launch"] for result in steps]

    def when(t):
        if t <= bounds[0]:
            return "the launch"
        for index in range(1, len(bounds)):
            if t <= bounds[index]:
                return f"step {index}"
        return "after the last step"

    seen = set()
    failed = []

    def fail(t, message):
        failed.append(f"input delivery, {when(t)}: {message}")

    # The compositor's routing, as its clients were told it.
    focused = None
    entered = None
    pressed = collections.Counter()
    dead = set()
    for t, client, pid, surface, kind, serial in deliveries:
        key = (client, surface)
        who = f"client {client} (pid {pid}) surface {surface}"
        # A surface its client destroyed, or a client gone: nothing more reaches it, and what it
        # held is no longer anyone's.
        if kind in ("destroyed", "disconnected"):
            gone = (lambda k: k == key) if kind == "destroyed" else (lambda k: k[0] == client)
            dead.add(key if kind == "destroyed" else (client, None))
            if focused is not None and gone(focused):
                focused = None
            if entered is not None and gone(entered):
                entered = None
            for k in [k for k in pressed if gone(k)]:
                pressed.pop(k)
            continue
        if key in dead or (client, None) in dead:
            continue
        if serial:
            if (client, surface, kind, serial) in seen:
                fail(t, f"{kind} {serial} routed twice to {who}")
            seen.add((client, surface, kind, serial))
        if kind == "focus.in":
            if focused is not None and focused != key:
                fail(t, f"the keyboard focus came to {who} while client {focused[0]} surface {focused[1]} still had it")
            focused = key
        elif kind == "focus.out":
            if focused != key:
                fail(t, f"the keyboard focus left {who}, which did not have it")
            else:
                focused = None
        elif kind in KEYBOARD_KINDS and focused != key:
            fail(t, f"{kind} {serial} routed to {who}, which does not have the keyboard focus "
                    f"({'none' if focused is None else f'client {focused[0]} surface {focused[1]}'})")
        if kind == "pointer.enter":
            if entered is not None and entered != key:
                fail(t, f"the pointer entered {who} while client {entered[0]} surface {entered[1]} had not been left")
            entered = key
        elif kind == "pointer.leave":
            if entered != key:
                fail(t, f"the pointer left {who}, which it was not over")
            else:
                entered = None
            pressed.pop(key, None)
        elif kind in POINTER_KINDS and entered != key:
            fail(t, f"{kind} {serial} routed to {who}, which the pointer is not over "
                    f"({'none' if entered is None else f'client {entered[0]} surface {entered[1]}'})")
        if kind == "pointer.press":
            pressed[key] += 1
        elif kind == "pointer.release":
            if pressed[key] <= 0:
                fail(t, f"a release ({serial}) routed to {who}, which has no press")
            else:
                pressed[key] -= 1
        elif kind == "pointer.cancel":
            pressed.pop(key, None)
    # Where each step left them, against what was routed by then.
    for index, result in enumerate(steps):
        said = result.get("input_focus")
        end = result.get("t_settled")
        if said is None or end is None:
            continue
        focus_at, pointer_at = None, None
        for t, client, pid, surface, kind, serial in deliveries:
            if t > end:
                break
            if kind in ("destroyed", "disconnected"):
                gone = (lambda k: k == (client, surface)) if kind == "destroyed" else (lambda k: k[0] == client)
                focus_at = None if focus_at is not None and gone(focus_at) else focus_at
                pointer_at = None if pointer_at is not None and gone(pointer_at) else pointer_at
            elif kind == "focus.in":
                focus_at = (client, surface)
            elif kind == "focus.out" and focus_at == (client, surface):
                focus_at = None
            elif kind == "pointer.enter":
                pointer_at = (client, surface)
            elif kind == "pointer.leave" and pointer_at == (client, surface):
                pointer_at = None
        # The keyboard focus on a surface not shown (a window minimized, a menu gone) would take
        # the keys typed next to nobody.
        if said.get("keyboard") and said["keyboard"].get("shown") is False:
            failed.append(f"input delivery, step {index + 1}: the keyboard focus is on client "
                          f"{said['keyboard']['client']} surface {said['keyboard']['surface']}, which is not shown")
        for what, routed in (("keyboard", focus_at), ("pointer", pointer_at)):
            state = said.get(what)
            state = None if state is None else (state["client"], state["surface"])
            if state != routed:
                failed.append(f"input delivery, step {index + 1}: the compositor's {what} focus is "
                              f"{state}, but its clients were told {routed}")

    # Each host: what it received against what was routed to its process.
    received = collections.defaultdict(collections.Counter)
    for name, lines in reports.items():
        for line in lines:
            if line["kind"] == "received":
                received[line["pid"]][(line["surface"], line["event"], line["serial"])] += 1
    # When each process's connection went, if it did.
    gone_at = {pid: t for t, client, pid, surface, kind, serial in deliveries if kind == "disconnected"}
    routed = collections.defaultdict(collections.Counter)
    in_order = collections.defaultdict(list)
    for t, client, pid, surface, kind, serial in deliveries:
        # Not an event (a surface or client gone), or the input method's (not a Bloom host).
        if pid in received and kind not in HOST_UNSEEN:
            routed[pid][(surface, kind, serial)] += 1
            in_order[pid].append((surface, kind, serial))
    end = bounds[-1]
    for pid, got in received.items():
        sent = routed.get(pid, collections.Counter())
        for item, count in (got - sent).items():
            failures.append(f"input delivery: the host of pid {pid} received {item[1]} {item[2]} on surface "
                            f"{item[0]} {count} more time(s) than it was routed")
        # A host reads its connection in the order its events were routed: one that stopped reading
        # received exactly the first of them, and what it left unread is all that came after. Judged
        # by that order, never by comparing the host's clock of receiving one event with the
        # compositor's of routing another: a chooser the session ends as its request ends (SIGTERM,
        # its sheet's pointer leave and focus out routed a few microseconds before) can read the leave
        # after the focus out was routed and die before reading that (file-chooser's step 13 at a
        # load of 17, 7 November 2026).
        stopped_reading = collections.Counter(in_order[pid][:sum(got.values())]) == got
        for item, count in (sent - got).items():
            # Routed as the run ended (a host closed with it unread) is not a loss.
            times = [t for t, client, p, surface, kind, serial in deliveries
                     if p == pid and (surface, kind, serial) == item]
            del times[:-count]
            # Routed to a client that had stopped reading and then went (an application closing as
            # the menu that quit it gives it the keyboard back, a chooser the session ends): never
            # read, and never lost.
            left_unread = pid in gone_at and times and min(times) <= gone_at[pid] and stopped_reading
            if times and min(times) <= end and not left_unread:
                failures.append(f"input delivery, {when(min(times))}: the host of pid {pid} never received "
                                f"{item[1]} {item[2]} on surface {item[0]} ({count} routed)")

    # Each host's React events, by the input that caused them.
    for name, lines in reports.items():
        caused = collections.Counter()
        for line in lines:
            if line["kind"] != "deliver" or "input_serial" not in line:
                continue
            # The same event to the same view, the same in every way, for the same input.
            identity = json.dumps({k: v for k, v in line.items() if k != "t"}, sort_keys=True)
            caused[identity] += 1
            if caused[identity] == 2:
                fail(line["t"], f"{name}: {line['cause']} {line['input_serial']} delivered {line['event']} "
                                f"to {line['target']} twice")
            if (line["cause"] in POINTER_CAUSES and line.get("surface")
                    and line["surface"] != line.get("cause_surface")):
                fail(line["t"], f"{name}: {line['cause']} {line['input_serial']} on surface "
                                f"{line.get('cause_surface')} delivered {line['event']} to {line['target']} on "
                                f"surface {line['surface']}")
    failures.extend(failed)


def check_views(index, step, result, results, failures):
    """The windows the shell's surfaces show live, and windows flying to them (ADR 0022, ADR 0045),
    as the director reported them once the step settled: `window_views` ("windows", exactly those
    window numbers shown, in any order; "moved", each moved into its view or not; "apart", no two
    overlapping; "within", each inside [x, y, width, height]; "below", those windows each below
    every other one shown: App Exposé's minimized row), `flights` ("arriving" and "going", exactly
    the windows that flew to their views and back in the step; each flight's frames only ever going
    toward its end, edge by edge, in more than one frame; an arriving one beginning where its window
    is and ending in its view, a going one beginning in its view's place and ending where its window
    is; "none", nothing flew) and `windows_placed_as` (every window where it was once step N
    settled, 1-based: nothing moved)."""
    expect = step.get("expect", {})
    name = f"step {index + 1} ({step.get('name', step['do'][0])})"

    def fail(message):
        failures.append(f"{name}: {message}")

    views = result.get("window_views", [])
    placed = result.get("windows_placed", {})
    wanted = expect.get("window_views")
    if wanted is not None:
        shown = sorted(view["window"] for view in views)
        if "windows" in wanted and shown != sorted(wanted["windows"]):
            fail(f"the shell shows windows {shown} live, expected {sorted(wanted['windows'])}")
        if "moved" in wanted and any(view["moved"] != wanted["moved"] for view in views):
            fail(f"views moved {[view['moved'] for view in views]}, expected all {wanted['moved']}")
        if wanted.get("apart"):
            for i, a in enumerate(views):
                for b in views[i + 1:]:
                    if overlaps(a["rect"], b["rect"]):
                        fail(f"windows {a['window']} {a['rect']} and {b['window']} {b['rect']} overlap")
        if "within" in wanted:
            for view in views:
                if not within(view["rect"], wanted["within"]):
                    fail(f"window {view['window']} at {view['rect']} is outside {wanted['within']}")
        for number in wanted.get("below", []):
            mine = next((view for view in views if view["window"] == number), None)
            if mine is None:
                fail(f"window {number} is not shown")
                continue
            for other in views:
                if other["window"] != number and other["window"] not in wanted["below"] and \
                        mine["rect"][1] < other["rect"][1] + other["rect"][3]:
                    fail(f"window {number} at {mine['rect']} is not below window {other['window']} {other['rect']}")
    flights = result.get("flights", [])
    wanted = expect.get("flights")
    if wanted is not None:
        if wanted.get("none") and flights:
            fail(f"windows flew: {flights}")
        for kind, arriving in (("arriving", True), ("going", False)):
            if kind not in wanted:
                continue
            flew = sorted(flight["window"] for flight in flights if flight["arriving"] == arriving)
            if flew != sorted(wanted[kind]):
                fail(f"windows {flew} flew {'to their views' if arriving else 'back'}, expected {sorted(wanted[kind])}")
        before = results[index - 1] if index > 0 else {}
        for flight in flights:
            path = flight["path"]
            who = f"window {flight['window']}'s flight {'to its view' if flight['arriving'] else 'back'}"
            if len(path) < 2:
                fail(f"{who} drew {len(path)} frame(s)")
                continue
            first, last = path[0], path[-1]
            for edge in range(4):
                # Left, top, right and bottom.
                values = [(f[0], f[1], f[0] + f[2], f[1] + f[3])[edge] for f in path]
                rising = values[-1] >= values[0]
                if any((b < a) if rising else (b > a) for a, b in zip(values, values[1:])):
                    fail(f"{who} went back and forth: {path}")
                    break
            view = next((view for view in (views if flight["arriving"] else before.get("window_views", []))
                         if view["window"] == flight["window"]), None)
            home = (before if flight["arriving"] else result).get("windows_placed", {}).get(str(flight["window"]))
            start, end = (home, view and view["rect"]) if flight["arriving"] else (view and view["rect"], home)
            # Driven by fingers (a swipe, ADR 0046), its first frame is the first motion's: on its
            # way, within a fifth of it from where it began; else exactly there.
            swiped = step["do"][0] in ("swipe", "swipe_begin", "swipe_move", "swipe_end")
            if start is not None and end is not None and swiped:
                if any(abs(f - a) > abs(b - a) / 5 + 1 for f, a, b in zip(first, start, end)):
                    fail(f"{who} began at {first}, not near {start}")
            elif start is not None and first != start:
                fail(f"{who} began at {first}, not at {start}")
            # Its last frame is the one that ended it, exactly at its end, however many before it
            # were dropped.
            if end is not None and last != end:
                fail(f"{who} ended at {last}, not at {end}")
    if "windows_placed_as" in expect:
        then = results[expect["windows_placed_as"] - 1].get("windows_placed", {})
        if placed != then:
            fail(f"the windows are at {placed}, not where they were after step {expect['windows_placed_as']}: {then}")
    drawn = result.get("windows_drawn", {})

    def near(a, b):
        return a is not None and b is not None and all(abs(x - y) <= 1 for x, y in zip(a, b))

    if "swiped" in expect:
        share = expect["swiped"]
        moved = [view for view in views if view["moved"]]
        if not moved:
            fail("no window is moved into a view")
        for view in moved:
            home = placed.get(str(view["window"]))
            want = between(home, view["rect"], share) if home else None
            at = drawn.get(str(view["window"]), {}).get("rect")
            if not near(at, want):
                fail(f"window {view['window']} drawn at {at}, not {share} of the way from {home} to {view['rect']}: {want}")
    if "slid" in expect:
        position, width = expect["slid"]
        if not drawn:
            fail("no window drawn")
        for number, at in drawn.items():
            home = at["home"]
            dx = rounded((at["workspace"] - position) * width)
            want = [home[0] + dx, home[1], home[2], home[3]]
            if not near(at["rect"], want):
                fail(f"window {number} of desktop {at['workspace']} drawn at {at['rect']}, not {want} (slid to {position})")
    if expect.get("drawn_home"):
        for number, at in drawn.items():
            if at["rect"] != at["home"]:
                fail(f"window {number} drawn at {at['rect']}, not where it is, {at['home']}")
    if "workspaces" in expect and result.get("workspaces") != expect["workspaces"]:
        fail(f"the desktops are {result.get('workspaces')}, expected {expect['workspaces']}")


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
    for pattern in expect.get("texts_matching", []):
        if not any(re.search(pattern, shown_text) for shown_text in shown):
            fail(f"no text shown matches {pattern!r}")
    if "focus_input" in expect and window.get("focus") != expect["focus_input"]:
        fail(f"focused input is {window.get('focus')!r}, expected {expect['focus_input']!r}")
    if "keyboard_focus" in expect:
        # The control the keyboard's navigation focused (docs/adr/0039-keyboard-access.mdx), as the
        # host last moved it by the step's end: null for none, else its testID (or nearest
        # ancestor's) and whether its ring shows ({"target", "ring"}, each key given matching).
        moves = [line for line in report
                 if line["kind"] == "input" and line["event"] == "keyboard.focus" and line["t"] <= end]
        got = None
        if moves and moves[-1].get("target"):
            got = {"target": moves[-1]["target"], "ring": moves[-1].get("ring")}
            # The colour the ring was last drawn in ("#rrggbb"): the session's accent's.
            rings = [line for line in report
                     if line["kind"] == "input" and line["event"] == "keyboard.ring" and line["t"] <= end]
            if rings:
                got["color"] = rings[-1].get("color")
        wanted = expect["keyboard_focus"]
        if (wanted is None) != (got is None) or (wanted and any(got.get(k) != v for k, v in wanted.items())):
            fail(f"keyboard focus is {got}, expected {wanted}")
    if "focus_surface" in expect and result.get("focus") != expect["focus_surface"]:
        fail(f"compositor focus is on surface {result.get('focus')}, expected {expect['focus_surface']}")
    if any(key in expect for key in SCROLL_CHECKS):
        check_scroll_names(report, start, end, fail)
    if "react" in expect:
        got = react_events(report, start, end)
        wanted = expect["react"]
        within = lambda count, want: count == want if isinstance(want, int) else want[0] <= count <= want[1]
        # An event a range lets be absent ([0, most]) may not have come at all.
        if not set(got) <= set(wanted) or not all(within(got.get(k, 0), want) for k, want in wanted.items()):
            fail(f"React received {got}, expected exactly {wanted}")
    if "home" in expect:
        check_home(home_root.at(end) if home_root else HomeState(), expect["home"], fail)
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
    if "clipboard" in expect:
        # What the clipboard holds once the step settled, as the session read it from its owner
        # through a pipe (ADR 0007): null for nothing, or "types" (exactly, in order) and, by type,
        # its text exactly, {"contains": text}, or for other bytes {"bytes", "png": [w, h]}.
        wanted, got = expect["clipboard"], result.get("clipboard")
        if wanted is None or got is None:
            if wanted != got:
                fail(f"the clipboard holds {got}, expected {wanted}")
        else:
            contents = got.get("contents", {})
            for key, value in wanted.items():
                if key == "types":
                    if got.get("types") != value:
                        fail(f"the clipboard's types are {got.get('types')}, expected {value}")
                    continue
                have = contents.get(key)
                if isinstance(value, dict) and "contains" in value:
                    if not isinstance(have, str) or value["contains"] not in have:
                        fail(f"the clipboard's {key} is {have!r}, expected it to contain {value['contains']!r}")
                elif isinstance(value, dict):
                    if not isinstance(have, dict) or any(have.get(k) != v for k, v in value.items()):
                        fail(f"the clipboard's {key} is {have!r}, expected {value!r}")
                elif have != value:
                    fail(f"the clipboard's {key} is {have!r}, expected {value!r}")
    if "menus" in expect:
        # The window's menus as the shell is told them (ADR 0021): null for none, or by an
        # entry's path ("View > Appearance > Dark") the keys given (shortcut, disabled, checked),
        # null for an entry that must not be there; "bar" is the menus of the bar, exactly.
        wanted, got = expect["menus"], result.get("menus")
        if wanted is None or got is None:
            if wanted != got:
                fail(f"the window's menus are {got}, expected {wanted}")
        else:
            entries = {entry["path"]: entry for entry in got}
            for path, value in wanted.items():
                if path == "bar":
                    bar = [entry["path"] for entry in got if " > " not in entry["path"]]
                    if bar != value:
                        fail(f"the menu bar is {bar}, expected {value}")
                elif value is None:
                    if path in entries:
                        fail(f"the menus have {path!r}, expected none")
                elif path not in entries:
                    fail(f"the menus have no {path!r}")
                elif any(entries[path].get(key) != v for key, v in value.items()):
                    fail(f"the menus' {path!r} is {entries[path]}, expected {value}")
    if "pointer_image" in expect:
        # The client's image the pointer shows (ADR 0029), each key given matching (width,
        # height, hotspot, scale_120, middle as "#aarrggbb"), or null for a shape.
        wanted, got = expect["pointer_image"], result.get("pointer_image")
        if wanted is None or got is None:
            if wanted != got:
                fail(f"the pointer's image is {got}, expected {wanted}")
        elif any(got.get(key) != value for key, value in wanted.items()):
            fail(f"the pointer's image is {got}, expected {wanted}")
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
        if not frames_least(low) <= len(frames) <= high:
            fail(f"requestAnimationFrame ran {len(frames)} frames, expected {low} to {high}")
        stamps = [line["timestamp"] for line in frames]
        gaps = [b - a for a, b in zip(stamps, stamps[1:])]
        if gaps and min(gaps) < wanted.get("interval_ms", 0):
            fail(f"requestAnimationFrame's frames came {min(gaps):.2f} ms apart, expected at least {wanted['interval_ms']}")
        if "median_interval_ms" in wanted and SPEED_BUDGETS:
            low, high = wanted["median_interval_ms"]
            median = statistics.median(gaps) if gaps else None
            if median is None or not low <= median <= high:
                fail(f"requestAnimationFrame's frames came {median} ms apart (median), expected {low} to {high}")
        if any(gap <= 0 for gap in gaps):
            fail("requestAnimationFrame's timestamps did not rise")
        if any(abs(line["t"] / 1e6 - line["timestamp"]) > 1 for line in frames):
            fail("requestAnimationFrame's timestamps are not on performance.now()'s clock")
    if "diagnostics" in expect:
        wanted = expect["diagnostics"]
        logs = [line["text"] for line in report if line["kind"] == "log" and start <= line["t"] <= end]
        slow_lines = [m for m in map(SLOW_LINE.match, logs) if m]
        slow_frames = [line for line in report if line["kind"] == "timing" and line["slow"]
                       and start <= line["t"] <= end]
        summaries = [line for line in report if line["kind"] == "timing_summary" and start <= line["t"] <= end]
        fast_frames = [line for line in report if line["kind"] == "timing" and line["presented"] and not line["slow"]
                       and start <= line["t"] <= end]
        if len(fast_frames) < wanted.get("fast_frames_min", 0):
            fail(f"{len(fast_frames)} frames within a refresh, expected at least {wanted['fast_frames_min']}")
        for key, got in (("slow", len(slow_lines)), ("slow_frames", len(slow_frames)), ("summaries", len(summaries))):
            if key in wanted and not wanted[key][0] <= got <= wanted[key][1]:
                fail(f"the diagnostics wrote {got} {key.replace('_', ' ')}, expected {wanted[key][0]} to "
                     f"{wanted[key][1]}: {logs}")
        if "slow_line" in wanted:
            like = wanted["slow_line"]
            if not any(("subject" not in like or re.fullmatch(like["subject"], m["subject"]))
                       and ("skipped" not in like or int(m["skipped"] or 0) == like["skipped"])
                       and float(m["javascript"]) >= like.get("javascript_min", 0)
                       and float(m["work"]) >= like.get("work_min", 0)
                       and int(m["dropped"]) >= like.get("dropped_min", 0) for m in slow_lines):
                fail(f"no slow line like {like} was written: {logs}")
        for pattern in wanted.get("logged", []):
            if not any(re.search(pattern, text) for text in logs):
                fail(f"no line matching {pattern!r} was written: {logs}")
    if "js_work" in expect:
        # The host's work in the step as its diagnostics measured it (frame-diagnostics.cpp): every
        # frame, and the work that drew none but ran JavaScript.
        # JavaScript's work is its time on the loop thread's CPU, which the machine's load does not
        # lengthen (preempted, the host waits for a CPU another process has); its time blocked (on
        # another thread, a lock, a file) is held to a bound of its own, never hidden.
        wanted = expect["js_work"]
        work = [line for line in report if line["kind"] == "timing" and start <= line["t"] <= end]
        def js(line, measure):
            return line[measure].get("javascript", 0)
        costly = max(work, key=lambda line: js(line, "cpu"), default=None)
        most = js(costly, "cpu") if costly else 0.0
        if SPEED_BUDGETS and most > wanted.get("js_ms_max", float("inf")):
            fail(f"{costly['trigger']} ran {most:.2f} ms of JavaScript on the CPU"
                 f"{'' if costly['presented'] else ' (no frame)'} ({js(costly, 'parts'):.2f} by the clock: "
                 f"{js(costly, 'blocked'):.2f} blocked, {js(costly, 'preempted'):.2f} preempted), expected at most "
                 f"{wanted['js_ms_max']}")
        waited = max(work, key=lambda line: js(line, "blocked"), default=None)
        if SPEED_BUDGETS and waited and js(waited, "blocked") > wanted.get("js_blocked_ms_max", float("inf")):
            fail(f"{waited['trigger']}'s JavaScript was blocked {js(waited, 'blocked'):.2f} ms"
                 f"{'' if waited['presented'] else ' (no frame)'} (waiting for another thread, a lock or a file; "
                 f"{js(waited, 'cpu'):.2f} on the CPU), expected at most {wanted['js_blocked_ms_max']}")
        # A backstop by the wall clock, for a gross regression only (docs/performance.mdx, "What a
        # step re-renders"): set far above what the work takes, so that a loaded machine never
        # reaches it.
        slowest = max(work, key=lambda line: js(line, "parts"), default=None)
        if not COLLECT and slowest and js(slowest, "parts") > wanted.get("js_wall_ms_max", float("inf")):
            fail(f"{slowest['trigger']} ran {js(slowest, 'parts'):.2f} ms of JavaScript by the clock"
                 f"{'' if slowest['presented'] else ' (no frame)'} ({js(slowest, 'cpu'):.2f} on the CPU), expected at "
                 f"most {wanted['js_wall_ms_max']} (a backstop for a gross regression)")
        presented = sum(1 for line in work if line["presented"])
        if "presented" in wanted and not wanted["presented"][0] <= presented <= wanted["presented"][1]:
            fail(f"{presented} frames presented, expected {wanted['presented'][0]} to {wanted['presented'][1]}")
    if "memory" in expect:
        # What the host gave back once its window was idle (memory.h): the account its trim wrote by
        # the step's end, after the window's last frame, of the GPU buffers it kept at most one a
        # surface shown (`buffers_max`, when it draws on the GPU) and nothing left that Skia's cache
        # could purge (`skia_purgeable_max`, MiB). Counts, not sizes: the same on any machine
        # (perf.py holds the sizes on the ThinkPad).
        wanted = expect["memory"]
        accounts = [line for line in report if line["kind"] == "memory" and line["t"] <= end]
        last_frame = max((line["t"] for line in report if line["kind"] == "frame" and line["t"] <= end), default=0)
        if not accounts or accounts[-1]["t"] < last_frame:
            fail("the host wrote no memory account after its last frame: it gave nothing back once idle")
        else:
            account = accounts[-1]
            if "buffer_count" in account and account["buffer_count"] > wanted.get("buffers_max", float("inf")):
                fail(f"the host kept {account['buffer_count']} GPU buffers idle, expected at most {wanted['buffers_max']}")
            if account.get("skia_gpu_purgeable", 0) > wanted.get("skia_purgeable_max", float("inf")):
                fail(f"Skia kept {account['skia_gpu_purgeable']} MiB it could purge, expected at most "
                     f"{wanted['skia_purgeable_max']}")
    if "renders" in expect and RENDER_COUNTS:
        # What React re-rendered in the step, counted from its commits (report.h, `react_render`):
        # the components each re-render began at, exactly, the same on any machine however loaded.
        wanted = expect["renders"]
        commits = [line for line in report if line["kind"] == "react_render" and start <= line["t"] <= end]
        minute = sorted(wanted.get("each_minute", []))
        def on_the_minute(line):
            # The clock's own re-render (useMinuteClock): a commit of exactly what a minute's turn
            # renders, within two seconds after a minute of the wall clock began.
            since = (line["t"] + WALL_OFFSET_NS) % 60_000_000_000
            return bool(minute) and sorted(render[0] for render in line["renders"]) == minute and since < 2_000_000_000
        counted = [line for line in commits if not on_the_minute(line)]
        tops = sorted(render[0] for line in counted for render in line["renders"])
        components = sum(render[1] for line in counted for render in line["renders"])
        def rendered(render):
            names = sorted(render[2].items(), key=lambda item: (-item[1], item[0]))
            return f"{render[0]} ({render[1]}: " + ", ".join(f"{name} {n}" for name, n in names[:12]) + (", …" if len(names) > 12 else "") + ")"
        said = "; ".join(rendered(render) for line in counted for render in line["renders"]) or "nothing"
        if "tops" in wanted and tops != sorted(wanted["tops"]):
            fail(f"React re-rendered {said} (where each re-render began, with the components it rendered), "
                 f"expected re-renders beginning at exactly {sorted(wanted['tops'])}")
        if components > wanted.get("components_max", float("inf")):
            fail(f"React rendered {components} components ({said}), expected at most {wanted['components_max']}")
    if "accounts_asked" in expect:
        # What the login screen's setup asked the played accounts' service in the step (`login`):
        # "ASKED: ANSWERED" ("platform: platform", "enroll: secondFactorRequired",
        # "secondFactor: enrolled", "enroll: refused"), exactly, in order.
        log = ACCOUNT.get("setup_log", pathlib.Path("/nonexistent"))
        lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        got = [f"{line['asked']}: {line['answered']}" for line in lines if start <= line["t"] <= end]
        if got != expect["accounts_asked"]:
            fail(f"the accounts' service was asked {got}, expected exactly {expect['accounts_asked']}")
    if "oxy_requests" in expect:
        # Requests the played Oxy API answered in the step (`login.oxy`), "METHOD PATH STATUS":
        # each listed was made at least once.
        log = ACCOUNT.get("oxy_log", pathlib.Path("/nonexistent"))
        lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        got = {f"{line['request']} {line['status']}" for line in lines if start <= line["t"] <= end}
        missing = [wanted for wanted in expect["oxy_requests"] if wanted not in got]
        if missing:
            fail(f"Oxy's API was not asked {missing} (it was asked {sorted(got)})")
    if "oxy_requests_times" in expect:
        # How many times the played Oxy API answered each request in the step, exactly: a QR's
        # session claimed once ("POST /auth/session/claim 200": 1), a request never made (0).
        log = ACCOUNT.get("oxy_log", pathlib.Path("/nonexistent"))
        lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        asked = [f"{line['request']} {line['status']}" for line in lines if start <= line["t"] <= end]
        for wanted, times in expect["oxy_requests_times"].items():
            if asked.count(wanted) != times:
                fail(f"Oxy's API answered {wanted!r} {asked.count(wanted)} times, expected {times} "
                     f"(it was asked {asked})")
    if "account_session" in expect:
        # What the played accounts' service was asked in the step (`account`): "APPLICATION ASKED:
        # ANSWERED", exactly, in any order (an application none, "-").
        log = ACCOUNT.get("log", pathlib.Path("/nonexistent"))
        lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        got = sorted(f"{line['application'] or '-'} {line['asked']}: {line['answered']}"
                     for line in lines if start <= line["t"] <= end)
        if got != sorted(expect["account_session"]):
            fail(f"the accounts' service was asked {got}, expected exactly {sorted(expect['account_session'])}")
    if "served" in expect:
        # The images the store answered in the step (`store.images`), each when its answer ended.
        log = STORE.get("base", pathlib.Path("/nonexistent")) / "images.jsonl"
        lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        got = sorted(f"{line['status']} {line['path']}{' cut' if line['cut'] else ''}"
                     for line in lines if start <= line["t"] <= end)
        if got != sorted(expect["served"]):
            fail(f"the store answered {got}, expected exactly {sorted(expect['served'])}")
    if "images" in expect:
        # What the host's image loader did with remote images in the step (images.h).
        got = {}
        for line in report:
            if line["kind"] == "image" and start <= line["t"] <= end:
                key = f"{line['event']} {line['uri']}"
                got[key] = got.get(key, 0) + 1
        if got != expect["images"]:
            fail(f"the image loader reported {got}, expected exactly {expect['images']}")
    if "worklets" in expect:
        # The worklets worklets' UI runtime made in the step (the first step: the launch, from the
        # process's start), each the first time it ran there (worklets-host.cpp): `ahead_min`, at
        # least that many the framework compiled ahead; `from_source_max`, at most that many compiled
        # from their text then (an application's own, which no framework has).
        made = [line for line in report if line["kind"] == "worklet" and start <= line["t"] <= end]
        ahead = sum(1 for line in made if line["ahead"])
        from_source = sorted(line["url"] for line in made if not line["ahead"])
        wanted = expect["worklets"]
        if ahead < wanted.get("ahead_min", 0):
            fail(f"worklets' UI runtime made {ahead} worklets compiled ahead, expected at least {wanted['ahead_min']}")
        if len(from_source) > wanted.get("from_source_max", float("inf")):
            fail(f"worklets' UI runtime compiled {len(from_source)} worklets from their text ({', '.join(from_source[:8])}"
                 f"{', …' if len(from_source) > 8 else ''}), expected at most {wanted['from_source_max']}")
    if "image_layers" in expect:
        # What expo-image views drew in the step (images.h's `image_layers` lines, each time a view's
        # layers changed), by testID: "final", its layers at the step's end, bottom to top, each
        # matching the keys given (alpha within 0.01, placeholder); "fade", a cross-dissolve that ran
        # in the step: {"ms", "timing" (linear, ease-in, ease-out, ease-in-out: Android's
        # interpolators), "over": "placeholder" or "image" (what faded out), "frames_min"}: from its
        # first frame (the new image at alpha 0 over what fades out at its own) to its last (the new
        # image alone, opaque), every frame's alpha where the curve is at that frame's time (within
        # 0.02) and what fades out at its complement, lasting the duration; "animation": the frames of
        # its top layer's picture shown in the step: "changes_min" and "changes_max" times it moved to
        # another, each the next of "cycle" frames in order, "gap_ms" [least, most] after the one before.
        import math
        curves = {
            "linear": lambda t: t,
            "ease-in": lambda t: t * t,
            "ease-out": lambda t: 1 - (1 - t) * (1 - t),
            "ease-in-out": lambda t: math.cos((t + 1) * math.pi) / 2 + 0.5,
        }
        for target, wanted in expect["image_layers"].items():
            lines = [line for line in report
                     if line["kind"] == "image_layers" and line["target"] == target and start <= line["t"] <= end]
            seen = [[(round(layer["alpha"], 3), layer["placeholder"]) for layer in line["layers"]] for line in lines]
            if "final" in wanted:
                final = lines[-1]["layers"] if lines else None
                matches = final is not None and len(final) == len(wanted["final"]) and all(
                    all(abs(got[key] - value) <= 0.01 if key == "alpha" else got[key] == value
                        for key, value in want.items())
                    for got, want in zip(final, wanted["final"]))
                if not matches:
                    fail(f"{target} drew {final} at the step's end, expected {wanted['final']}")
            if "fade" in wanted:
                fade = wanted["fade"]
                curve = curves[fade.get("timing", "ease-in-out")]
                over_placeholder = fade.get("over", "placeholder") == "placeholder"
                begin = next((i for i, line in enumerate(lines)
                              if len(line["layers"]) == 2 and line["layers"][1]["alpha"] == 0
                              and line["layers"][0]["placeholder"] == over_placeholder), None)
                if begin is None:
                    fail(f"{target} began no cross-dissolve over a {fade.get('over', 'placeholder')} in the step: {seen}")
                    continue
                t0 = lines[begin]["t"]
                frames, ended = 0, None
                for line in lines[begin:]:
                    layers = line["layers"]
                    at = (line["t"] - t0) / 1e6 / fade["ms"]
                    if len(layers) == 1:
                        if layers[0]["placeholder"] or abs(layers[0]["alpha"] - 1) > 0.01:
                            fail(f"{target}'s cross-dissolve ended drawing {layers}, expected the image alone, opaque")
                        ended = line["t"]
                        break
                    frames += 1
                    want = curve(min(1, at))
                    if abs(layers[1]["alpha"] - want) > 0.02 or abs(layers[0]["alpha"] - (1 - want)) > 0.02:
                        fail(f"{target}'s cross-dissolve at {at * fade['ms']:.1f} ms drew the image at "
                             f"{layers[1]['alpha']:.3f} over {layers[0]['alpha']:.3f}, expected {want:.3f} "
                             f"over {1 - want:.3f} ({fade.get('timing', 'ease-in-out')})")
                        break
                if ended is None:
                    fail(f"{target}'s cross-dissolve did not end in the step: {seen[begin:]}")
                elif (ended - t0) / 1e6 < fade["ms"] - 1:
                    fail(f"{target}'s cross-dissolve lasted {(ended - t0) / 1e6:.1f} ms, expected {fade['ms']}")
                if frames < fade.get("frames_min", 8):
                    fail(f"{target}'s cross-dissolve drew {frames} frames, expected at least {fade.get('frames_min', 8)}")
            if "animation" in wanted:
                shown, times = [], []
                for line in lines:
                    frame = line["layers"][-1]["frame"] if line["layers"] else None
                    if frame is not None and (not shown or shown[-1] != frame):
                        shown.append(frame)
                        times.append(line["t"])
                animation = wanted["animation"]
                changes = len(shown) - 1 if shown else 0
                if changes < animation.get("changes_min", 0) or changes > animation.get("changes_max", float("inf")):
                    fail(f"{target} changed frames {max(changes, 0)} times ({shown}), expected "
                         f"{animation.get('changes_min', 0)} to {animation.get('changes_max', 'any')}")
                for i in range(1, len(shown)):
                    if "cycle" in animation and shown[i] != (shown[i - 1] + 1) % animation["cycle"]:
                        fail(f"{target} showed frame {shown[i]} after {shown[i - 1]}, expected its frames in order ({shown})")
                        break
                    gap = (times[i] - times[i - 1]) / 1e6
                    low, high = animation.get("gap_ms", [0, float("inf")])
                    if not low <= gap <= high:
                        fail(f"{target} showed frame {shown[i]} {gap:.1f} ms after frame {shown[i - 1]}, expected {low} to {high} ms")
                        break
    if "window_frames" in expect:
        # The window's frames presented in the step: [at least, at most]. An animation draws several.
        low, high = expect["window_frames"]
        count = sum(1 for line in report if line["kind"] == "frame" and line.get("surface") == 1
                    and line.get("presented", True) and start <= line["t"] <= end)
        if not frames_least(low) <= count <= high:
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
                      "expect": expect["in_shell"], "within": "shell"}
        check_step(index, shell_step, result, shell_report or [], start, failures, refresh_hz=refresh_hz)
    if "in_chooser" in expect and screenshot is not None:
        # The file chooser the session started (ADR 0029, decision 8), from its own report beside
        # the director's (`.chooser.N`, one a request): their texts and React deliveries, in time.
        chooser = sorted((line
                          for path in pathlib.Path(screenshot).parent.glob("host-report.jsonl.chooser.*")
                          for line in ended_report(path)),
                         key=lambda line: line["t"])
        chooser_step = {"name": f"{step.get('name', step['do'][0])}, in the file chooser",
                        "do": step["do"], "expect": expect["in_chooser"], "within": "chooser"}
        check_step(index, chooser_step, result, chooser, start, failures, refresh_hz=refresh_hz)
    if "chooser" in expect:
        # The file chooser's sheet (ADR 0029, decision 8): null, or where it is on the output, whether
        # it has the keyboard and whether the application's window is the active one.
        wanted, got = expect["chooser"], result.get("chooser")
        if wanted is None or got is None:
            if wanted != got:
                fail(f"the file chooser's sheet is {got}, expected {wanted}")
        else:
            for key, value in wanted.items():
                if got.get(key) != value:
                    fail(f"the file chooser's sheet has {key} {got.get(key)}, expected {value}")
    if "in_companion" in expect and screenshot is not None:
        # The companion application (ADR 0035), from its own report beside the director's
        # (`.companion`): its texts and React deliveries.
        path = pathlib.Path(screenshot).parent / "host-report.jsonl.companion"
        companion = ended_report(path)
        companion_step = {"name": f"{step.get('name', step['do'][0])}, in the companion",
                          "do": step["do"], "expect": expect["in_companion"], "within": "companion"}
        # Its window's menus (ADR 0021) are the companion's own, as the shell is told them.
        companion_result = dict(result, menus=result.get("companion_menus"))
        check_step(index, companion_step, companion_result, companion, start, failures, refresh_hz=refresh_hz)
    if "in_launched" in expect and screenshot is not None:
        # An application the session launched (a store's open, the shell's dock), from its own
        # report beside the director's (`.launched`): its texts and React deliveries. Of two running
        # at once (the store, and an application it opened), the first: the second's host writes
        # beside it (report.cpp).
        path = pathlib.Path(screenshot).parent / "host-report.jsonl.launched"
        launched = ended_report(path)
        launched_step = {"name": f"{step.get('name', step['do'][0])}, in the launched application",
                         "do": step["do"], "expect": expect["in_launched"], "within": "launched"}
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
    if "transactions" in expect:
        # The text-input transactions the application was given (ADR 0008), exactly and in order;
        # a transaction's parts left out of the expectation are what an empty part says.
        got = result.get("transactions", [])
        empty = {"delete": [0, 0], "commit": "", "preedit": "", "cursor": [0, 0], "segments": []}
        # An AutoFill fill (docs/adr/0050-keychain-autofill-and-passkeys.mdx) is said whole: the
        # field's text and its paired field's length.
        wanted = [want if "fill" in want else {**empty, **want} for want in expect["transactions"]]
        if got != wanted:
            fail(f"transactions given {got}, expected exactly {wanted}")
    if "delivered" in expect:
        delivered = dict(result.get("delivered", {}))
        leaked = delivered.pop("leaked", 0)
        # The session's shell takes its own input (its panels, the pointer leaving them).
        delivered.pop("shell", None)
        # Applications the shell launched (ADR 0013): checked, exactly, only when the step says.
        launched = delivered.pop("launched", {})
        # The file chooser (ADR 0029, decision 8): checked, exactly, only when the step says.
        chooser = delivered.pop("chooser", {})
        # The companion application, and the drag events the shell's surfaces received (ADR 0035):
        # checked, exactly, only when the step says.
        companion = delivered.pop("companion", {})
        shell_drags = delivered.pop("shell_drags", {})
        wanted = dict(expect["delivered"])
        if "companion" in wanted and companion != wanted.pop("companion"):
            fail(f"the companion was delivered {companion}, expected exactly {expect['delivered']['companion']}")
        if "shell_drags" in wanted and shell_drags != wanted.pop("shell_drags"):
            fail(f"the shell was delivered drag events {shell_drags}, expected exactly {expect['delivered']['shell_drags']}")
        # A list: any one of them, for a step whose launched application is told what it is in an
        # order of two clients' requests that is neither's to decide (Inbox quit from the dock's
        # menu: its window may close before or after the menu does, and has the keyboard back
        # for that moment only in the second).
        if "launched" in wanted:
            either = wanted.pop("launched")
            if launched not in (either if isinstance(either, list) else [either]):
                fail(f"the launched applications were delivered {launched}, expected exactly {expect['delivered']['launched']}")
        if "chooser" in wanted and chooser != wanted.pop("chooser"):
            fail(f"the file chooser was delivered {chooser}, expected exactly {expect['delivered']['chooser']}")
        if leaked:
            fail(f"{leaked} input events reached another client")
        if delivered != wanted:
            fail(f"compositor delivered {delivered}, expected exactly {wanted}")
    elif result.get("delivered", {}).get("leaked"):
        # Whatever a step says, no input reaches a client that is neither the application, its
        # shell, its companion, its file chooser nor one the shell launched.
        fail(f"{result['delivered']['leaked']} input events reached another client")
    if "popup_events" in expect:
        # Popup lifecycle (open, reposition, close, dismissed) in this step, exactly.
        got = {}
        for line in report:
            if line["kind"] == "popup" and start <= line["t"] <= end:
                key = f"{line['event']}@{line.get('view', line['surface'])}"
                got[key] = got.get(key, 0) + 1
        if got != expect["popup_events"]:
            fail(f"popup events {got}, expected exactly {expect['popup_events']}")
    if "menu_crossfade" in expect:
        # A menu bar's one popup changing from one menu into another (Bloom's viewport): the frames
        # it drew in the step, each with the texts it drew visibly (opacity above zero). `both`:
        # texts of the two menus drawn together (each menu's rows fading through the other's), in
        # at least `frames_min` frames and at most `frames_max`; `gone` absent from its last frame.
        wanted = expect["menu_crossfade"]
        menu_drawn = [line for line in report if line["kind"] == "frame" and start <= line["t"] <= end
                 and line.get("presented", True) and line["surface"] not in closed]
        menu_lines = [line for line in menu_drawn if any(text in t for t in line["texts"] for text in wanted["both"])]
        drawn_both = sum(1 for line in menu_lines if all(any(text in t for t in line["texts"]) for text in wanted["both"]))
        if drawn_both < frames_least(wanted.get("frames_min", 0)) or drawn_both > wanted.get("frames_max", drawn_both):
            fail(f"{drawn_both} frames drew both {wanted['both']}, expected between {wanted.get('frames_min', 0)} "
                 f"and {wanted.get('frames_max', 'any')}")
        # `kept`: the one popup throughout, never one closed and another opened.
        if wanted.get("kept"):
            lifecycle_events = [f"{line['event']}@{line.get('view', line['surface'])}" for line in report
                                if line["kind"] == "popup" and start <= line["t"] <= end
                                and line["event"] in ("open", "close", "dismissed")]
            if lifecycle_events:
                fail(f"popups opened or closed in the step: {lifecycle_events}, expected the one kept")
        last_drawn = menu_lines[-1]["texts"] if menu_lines else []
        for text in wanted.get("gone", []):
            if any(text in t for t in last_drawn):
                fail(f"{text!r} is still drawn in the menu's last frame of the step")
    if "menu_morph" in expect:
        # The menu open in each frame the compositor composed in the step (the popups of the grab
        # chain): ONE popup throughout, the same surface, whose place and size go from `from` to
        # `to` ([x, y, width, height] on the output), each coordinate monotonic between the two,
        # through at least `frames_min` places between them (`frames_max`: at most).
        wanted = expect["menu_morph"]
        composed_menus = result.get("menu_frames", [])
        surfaces = {(menu[0], menu[1]) for menus in composed_menus for menu in menus}
        if any(len(menus) != 1 for menus in composed_menus) or len(surfaces) != 1:
            fail(f"menus composed {composed_menus}: expected one popup in every frame, the same throughout")
        else:
            rects = []
            for menus in composed_menus:
                rect = menus[0][2:]
                if not rects or rects[-1] != rect:
                    rects.append(rect)
            start_rect, end_rect = wanted["from"], wanted["to"]
            if not rects or rects[0] != start_rect or rects[-1] != end_rect:
                fail(f"the menu went through {rects}, expected from {start_rect} to {end_rect}")
            for axis in range(4):
                low, high = sorted((start_rect[axis], end_rect[axis]))
                values = [rect[axis] for rect in rects]
                rising = end_rect[axis] >= start_rect[axis]
                ordered = all((b >= a) if rising else (b <= a) for a, b in zip(values, values[1:]))
                if not ordered or any(not low <= value <= high for value in values):
                    fail(f"the menu's {['x', 'y', 'width', 'height'][axis]} went {values}, not monotonic "
                         f"from {start_rect[axis]} to {end_rect[axis]}")
            between = max(0, len(rects) - 2)
            if between < frames_least(wanted.get("frames_min", 0)) or between > wanted.get("frames_max", between):
                fail(f"the menu showed {between} places between its two menus' ({rects}), expected between "
                     f"{wanted.get('frames_min', 0)} and {wanted.get('frames_max', 'any')}")
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
                offsets[scroll_view_name(line)] = [line["x"], line["y"]]
        # Exactly, but for a float's precision (an offset on a 1.25 display's pixels, 4.8, is reported
        # as the float nearest it).
        wanted = expect["scroll_offset"]
        if set(offsets) != set(wanted) or any(abs(a - b) > 1e-3 for k in wanted for a, b in zip(offsets[k], wanted[k])):
            fail(f"scroll offsets {offsets}, expected exactly {wanted}")
    if "scroll_input" in expect:
        # The scrolls the host received in this step (its input report: an application's, or in
        # `in_shell` the shell's, the dock's icons'), the way libinput turned them: every one that
        # moved that way, and that many.
        wanted = expect["scroll_input"]
        moved = [line["dy"] for line in report if line["kind"] == "input" and start <= line["t"] <= end
                 and line.get("event") == "scroll" and line.get("dy")]
        sign = 1 if wanted["way"] == "down" else -1
        least, most = wanted["events"]
        if not least <= len(moved) <= most or any(dy * sign <= 0 for dy in moved):
            fail(f"the host received scrolls of {moved}, expected {least} to {most}, every one {wanted['way']}")
    if "scroll_moved" in expect:
        # Each scroll view's offsets in this step, only ever the way given, and further than it began.
        for view, way in expect["scroll_moved"].items():
            ys = [line["y"] for line in report if line["kind"] == "deliver" and line["event"] == "scroll"
                  and start <= line["t"] <= end and scroll_view_name(line) == view]
            sign = 1 if way == "down" else -1
            before = offset_before(report, view, start)
            steps = [b - a for a, b in zip([before] + ys, ys)]
            if not ys or any(step * sign < 0 for step in steps) or (ys[-1] - before) * sign <= 0:
                fail(f"{view} scrolled from {before} through {ys}, expected only {way}")
    if "scrollbars" in expect or "scrollbar_thumbs" in expect:
        # The scroll views' overlay scrollbars (scrollbars.h) in this step, as the host reported them
        # changing: "scrollbars", each change counted exactly as "event@testID" (shown, wide-x and
        # wide-y as the pointer brings one wide, hidden once faded out); "scrollbar_thumbs", where
        # each view's thumbs were in the window as last reported ({testID: {axis: [x, y, w, h]}}).
        changes, thumbs = {}, {}
        for line in report:
            if line["kind"] == "scrollbar" and start <= line["t"] <= end:
                key = f"{line['event']}@{line['target']}"
                changes[key] = changes.get(key, 0) + 1
                thumbs[line["target"]] = line.get("thumbs", {})
        if "scrollbars" in expect and changes != expect["scrollbars"]:
            fail(f"the scrollbars changed {changes}, expected exactly {expect['scrollbars']}")
        for target, wanted in expect.get("scrollbar_thumbs", {}).items():
            got = thumbs.get(target, {})
            if any(got.get(axis) is None or any(abs(a - b) > 0.01 for a, b in zip(got[axis], rect))
                   for axis, rect in wanted.items()):
                fail(f"{target}'s scrollbar thumbs were last at {got}, expected {wanted}")
    if "host_input" in expect:
        # What the host reported handling in this step (Report::input), counted by name, exactly.
        counts = {}
        for line in report:
            if line["kind"] == "input" and start <= line["t"] <= end and line.get("event") in expect["host_input"]:
                counts[line["event"]] = counts.get(line["event"], 0) + 1
        wanted = {name: count for name, count in expect["host_input"].items() if count}
        if counts != wanted:
            fail(f"the host handled {counts}, expected exactly {wanted}")
    if "first_order" in expect:
        # The order in which things first reached the host in this step: each named one's first
        # occurrence, in this order (what a scenario's `host_order` forces). A name is a Report::input
        # ("focus.in", "test.keyboard.held") or an accessibility event from JavaScript as the host
        # took it, "a11y_event:focus@shell-topbar".
        firsts = {}
        for line in report:
            if not start <= line["t"] <= end:
                continue
            if line["kind"] == "input":
                key = line["event"]
            elif line["kind"] == "a11y_event":
                key = f"a11y_event:{line['event']}@{line.get('within') or line['target']}"
            else:
                continue
            firsts.setdefault(key, line["t"])
        wanted = expect["first_order"]
        got = sorted((name for name in wanted if name in firsts), key=lambda name: firsts[name])
        if got != wanted:
            fail(f"the host first had {got}, expected {wanted} in that order")
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
    # screen's among them), "launched" the surfaces of the applications the session launched
    # (Settings opened from the Oxy menu, their popups among them), anything else the
    # application's popups.
    def held_nodes(role):
        def place(surface):
            if surface == "1":
                return "window"
            if surface.startswith("shell-"):
                return "shell"
            return "launched" if "-" in surface else "popup"
        wanted = role if role in ("window", "shell", "launched") else "popup"
        return [n for s, tree in result.get("a11y", {}).items() if place(s) == wanted for n in tree]
    def matches(n, node):
        return all(n.get(k) == v for k, v in node.items())

    def inside(inner, outer):
        x, y, w, h = inner
        ox, oy, ow, oh = outer
        return ox <= x and oy <= y and x + w <= ox + ow and y + h <= oy + oh

    if "at_group" in expect:
        tree = result.get("a11y", {}).get("1", [])

        def nested_in(index, group):
            seen = set()
            parent = tree[index].get("parent")
            while parent is not None and parent not in seen:
                if parent == group:
                    return True
                seen.add(parent)
                parent = tree[parent].get("parent") if 0 <= parent < len(tree) else None
            return False

        for spec in expect["at_group"]:
            groups = [i for i, n in enumerate(tree) if matches(n, spec["group"])]
            if len(groups) != 1:
                fail(f"{len(groups)} window nodes match the group {spec['group']}, expected one")
                continue
            group = groups[0]
            found = []
            for item in spec["items"]:
                hits = [i for i, n in enumerate(tree) if matches(n, item)]
                if len(hits) != 1:
                    fail(f"{len(hits)} window nodes match {item}, expected one (in {spec['group']})")
                elif not nested_in(hits[0], group):
                    fail(f"{item} is not in the group {spec['group']}: parent "
                         f"{tree[hits[0]].get('parent')}, the group is node {group}")
                else:
                    found.append(hits[0])
            if len(found) != len(spec["items"]):
                continue
            rects = [tree[i]["rect"] for i in found]
            # In reading order, each after the last: to its right (a toolbar's), or below it (a
            # sidebar's rows).
            if found != sorted(found) or any(a[0] + a[2] > b[0] and a[1] + a[3] > b[1]
                                             for a, b in zip(rects, rects[1:])):
                fail(f"the group's items are not in order, each after the last: {rects}")
            outer = tree[group]["rect"]
            for item, rect in zip(spec["items"], rects):
                if not inside(rect, outer):
                    fail(f"{item} at {rect} is outside its group at {outer}")
            if spec.get("only"):
                held = [i for i, n in enumerate(tree) if n["role"] == "Button" and nested_in(i, group)]
                if held != found:
                    fail(f"the group holds the buttons {[tree[i]['name'] for i in held]}, "
                         f"expected exactly {[tree[i]['name'] for i in found]}")
    if "at_hit" in expect:
        tree = result.get("a11y", {}).get("1", [])
        wx, wy = (result.get("window") or [0, 0])[:2]
        for point, node in expect["at_hit"].items():
            px, py = (float(v) for v in point.split(","))
            # A press is in the window's coordinates; the compositor hands the tree on in the
            # output's.
            under = [n for n in tree if inside((px + wx, py + wy, 0, 0), n["rect"])]
            got = under[-1] if under else None
            if got is None or not matches(got, node):
                fail(f"a press at {point} lands on {got and {k: got[k] for k in ('role', 'name', 'rect')}}, "
                     f"expected {node}")
    if "at" in expect:
        for role, wanted in expect["at"].items():
            nodes = held_nodes(role)
            for node in wanted:
                if not any(all(same_text(n.get(k), v, end) for k, v in node.items()) for n in nodes):
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
        if len(said) != len(wanted) or not all(all(same_text(u.get(k), v, u["t"]) for k, v in w.items())
                                               for u, w in zip(said, wanted)):
            fail(f"the screen reader said {[(u['text'], u['interrupting'], u.get('voice')) for u in said]}, "
                 f"expected {expect['spoken']}")
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
        # What they hold, not the names of their fields: a password "mal" is not in a memory
        # line's "malloc_in_use".
        def values(item):
            if isinstance(item, dict):
                return [text for value in item.values() for text in values(value)]
            if isinstance(item, list):
                return [text for value in item for text in values(value)]
            return [item] if isinstance(item, str) else []
        held = values({k: v for k, v in result.items() if k not in STEP_KEYS})
        lines = [values(line) for line in report if line["t"] <= end]
        for text in expect["never"]:
            if any(text in value for value in held) or any(text in value for line in lines for value in line):
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
        if ran["frames"] < frames_least(wanted.get("frames_min", 0)):
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
    if "drag_icons" in expect:
        # The drag icon surfaces there are once the step settled, of any client (ADR 0035): a drag
        # shows exactly one, and none is left once its drag ended and its image went back.
        icons, wanted = result.get("drag_icons") or [], expect["drag_icons"]
        if "count" in wanted and len(icons) != wanted["count"]:
            fail(f"{len(icons)} drag icon surfaces, expected {wanted['count']}: {icons}")
        shown = [icon["rect"] for icon in icons if icon["rect"] is not None]
        if "shown" in wanted and shown != wanted["shown"]:
            fail(f"drag icons shown at {shown}, expected {wanted['shown']}")
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
        if drag.get("composed", 0) < frames_least(wanted.get("composed_min", 0)):
            fail(f"{drag.get('composed', 0)} frames were composed in the drag, expected at least {wanted['composed_min']}")
        if "composed_gap_ms" in wanted:
            low, high = wanted["composed_gap_ms"]
            median = (drag.get("composed_gap_ms") or {}).get("median")
            if SPEED_BUDGETS and (median is None or not low <= median <= high):
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
    if "resize_work" in expect:
        # A window resized (`drag_paced` on an edge, a zoom): the application's host's frames in the
        # step, the work each took, the JavaScript in it and React's commits.
        wanted = expect["resize_work"]
        # From the step's input on (a first step's window also holds the application's start).
        begun = result.get("t_inject") or start
        frames = [line for line in report if line["kind"] == "frame" and line.get("surface") == 1
                  and begun <= line["t"] <= end and line.get("presented", True)]
        if not frames:
            fail("the window showed no frame in the resize")
        commits = sum(line.get("react_commits", 0) for line in frames)
        if commits > wanted.get("react_commits_max", float("inf")):
            fail(f"React committed {commits} times in the resize (in {sum(1 for line in frames if line.get('react_commits'))} "
                 f"of its {len(frames)} frames), expected at most {wanted['react_commits_max']}")
        def p95(values):
            ordered = sorted(values)
            return ordered[min(len(ordered) - 1, round(0.95 * (len(ordered) - 1)))] if ordered else 0
        # JavaScript's work on the CPU (report.h, `js_cpu_ms`); the frame's work below is a deadline,
        # by the wall clock.
        js = p95([line.get("js_cpu_ms", 0) for line in frames])
        if SPEED_BUDGETS and js > wanted.get("js_ms_p95_max", float("inf")):
            fail(f"a frame of the resize ran {js:.1f} ms of JavaScript on the CPU at the 95th percentile "
                 f"({p95([line.get('js_ms', 0) for line in frames]):.1f} by the clock), expected at most "
                 f"{wanted['js_ms_p95_max']}")
        busy = p95([line.get("busy_ms", 0) for line in frames]) * refresh_hz / 1000
        if SPEED_BUDGETS and busy > wanted.get("busy_refreshes_p95_max", float("inf")):
            fail(f"a frame of the resize took {busy:.2f} refreshes of work at the 95th percentile, expected at most "
                 f"{wanted['busy_refreshes_p95_max']}")
        if "zoom_presented_ms_max" in wanted:
            zooms = [float(m.group(1)) for m in (re.search(r"zoom \(.*\): its configure .* its frame presented ([\d.]+) ms after",
                                                          line["text"])
                                                 for line in report if line["kind"] == "log" and begun <= line["t"] <= end)
                     if m]
            if not zooms:
                fail("the host logged no zoom in the step")
            elif not COLLECT and max(zooms) > wanted["zoom_presented_ms_max"]:
                fail(f"the zoom's frame was presented {max(zooms)} ms after it was asked, expected at most "
                     f"{wanted['zoom_presented_ms_max']}")
    if "glide" in expect:
        check_glide(expect["glide"], report, start, end, fail)
    if expect.get("scroll_times") == "dispatch":
        kinds = {"scroll", "scrollBeginDrag", "scrollEndDrag", "momentumScrollBegin", "momentumScrollEnd"}
        lines = [line for line in report if start <= line["t"] <= end and line["kind"] == "deliver"
                 and line.get("event") in kinds]
        if not lines:
            fail("no scroll event was delivered")
        for line in lines:
            stamp = line.get("timestamp")
            if stamp is None or abs(stamp - line["t"] / 1e6) > 2:
                fail(f"{line['event']}@{line.get('target')} carried timestamp {stamp}, dispatched at "
                     f"{line['t'] / 1e6:.1f} ms")
                break
    if "stretch" in expect:
        check_stretch(expect["stretch"], report, start, end, fail)
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
    if "network" in expect:
        # What the played network daemon holds once the step settled (ADR 0038): the session's
        # state, not only what was drawn.
        held = result.get("network")
        if held is None:
            fail("no network daemon's state (the scenario needs `network`)")
        else:
            held = json.loads(held) if isinstance(held, str) else held
            for problem in matches_within(expect["network"], held, "the network daemon's"):
                fail(problem)
    if "sound" in expect:
        # The played sound once the step settled (ADR 0056): what the session chose and set.
        held = result.get("sound")
        if held is None:
            fail("no played sound (the scenario needs `sound`)")
        else:
            for problem in matches_within(expect["sound"], held, "the sound's"):
                fail(problem)
    if "bluetooth" in expect:
        # What the played BlueZ holds once the step settled (ADR 0055): the session's calls, its
        # agent and discovery, not only what was drawn.
        held = result.get("bluetooth")
        if held is None:
            fail("no BlueZ's state (the scenario needs `bluetooth`)")
        else:
            held = json.loads(held) if isinstance(held, str) else held
            for problem in matches_within(expect["bluetooth"], held, "BlueZ's"):
                fail(problem)
    if "displays_off" in expect and result.get("displays_off") != expect["displays_off"]:
        # The displays turned off for want of input (ADR 0057, decision 6).
        fail(f"displays off: {result.get('displays_off')}, expected {expect['displays_off']}")
    if "power" in expect:
        # What the played power services hold once the step settled (ADR 0057): Low Power Mode's
        # hold, the charge limit in the battery's sysfs, the machine put to sleep.
        held = result.get("power")
        if held is None:
            fail("no power services' state (the scenario needs `power`)")
        else:
            held = json.loads(held) if isinstance(held, str) else held
            for problem in matches_within(expect["power"], held, "the power services'"):
                fail(problem)
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
    if "screenshots" in expect:
        check_screenshots(expect["screenshots"], result, fail)
    elif (result.get("screenshots") or {}).get("saved"):
        fail(f"screenshots saved, none expected: {result['screenshots']['saved']}")
    # The URLs the shell opened in the step (ADR 0034), exactly: each its URL and the application
    # started for it. A step that opens one and does not say so fails.
    # The system sounds played in the step (docs/adr/0044-system-sounds.mdx), exactly. A step that
    # plays one and does not say so fails.
    sounds = result.get("sounds") or []
    if not step.get("within") and sounds != expect.get("sounds", []):
        fail(f"sounds played: {sounds}, expected {expect.get('sounds', [])}")
    urls = result.get("urls_opened") or []
    # "{home}" in an expected URL is the run's home's file:// URI as a URL's parameter carries it
    # (encodeURIComponent's): a desktop folder opened in Dropsea (ADR 0047).
    home_uri = urllib.parse.quote(f"file://{home_root.root}", safe="-_.!~*'()") if home_root else "{home}"
    wanted_urls = [{**url, "url": url["url"].replace("{home}", home_uri)} if isinstance(url.get("url"), str) else url
                   for url in expect.get("urls_opened", [])]
    if urls != wanted_urls:
        fail(f"URLs opened: {urls}, expected {wanted_urls}")
    if bool(result.get("signed_out")) != bool(expect.get("signed_out", False)):
        fail(f"signed out: {bool(result.get('signed_out'))}, expected {bool(expect.get('signed_out', False))}")
    # Files the shell had opened with an application (ADR 0034, OpenFiles): exactly and in order,
    # each {"name": the file's name, "application"}; a step that opens one and does not say so fails.
    # The step's own, not each of its trees' (`in_shell`).
    files_opened = [{"name": pathlib.Path(o["path"]).name, "application": o["application"]}
                    for o in result.get("files_opened") or []]
    if not step.get("within") and files_opened != expect.get("files_opened", []):
        fail(f"files opened: {files_opened}, expected {expect.get('files_opened', [])}")
    if "window_shown" in expect and result.get("window_shown") != expect["window_shown"]:
        fail(f"window shown: {result.get('window_shown')}, expected {expect['window_shown']}")
    if "minimize_target" in expect and result.get("minimize_target") != expect["minimize_target"]:
        fail(f"minimize target: {result.get('minimize_target')}, expected {expect['minimize_target']}")
    if "window_states" in expect:
        # Every window as the shell is told of it once the step settled, by its number, exactly:
        # each given key ("app", "workspace", "minimized", "hidden", "focused") matching, and no
        # window more or fewer (the dock's press, ADR 0034's decision 9).
        got = result.get("window_states") or {}
        wanted = expect["window_states"]
        if set(got) != set(wanted) or any(
            got[number].get(key) != value for number, state in wanted.items() for key, value in state.items()
        ):
            fail(f"the windows are {got}, expected {wanted}")
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
        # The file as the step left it (the development compositor's report), else as the run did.
        if isinstance(result.get("preferences_file"), str):
            text = result["preferences_file"]
        else:
            text = preferences.read_text() if preferences and preferences.exists() else ""
        kept = {}
        for line in text.splitlines():
            if "=" in line and not line.lstrip().startswith("#"):
                key, value = line.split("=", 1)
                kept[key.strip()] = value.strip()
        for key, value in expect["preferences_file"].items():
            value = with_home(value, preferences)
            if kept.get(key) != value:
                fail(f"preferences file: {key} = {kept.get(key)!r}, expected {value!r}")
    for app, files in expect.get("app_data", {}).items():
        # What the application kept, as JSON, once the run ended.
        for name, content in files.items():
            run = preferences.parent
            data_home = run / "home" / ".local/share" if (run / "home").is_dir() else run / "xdg-data"
            path = data_home / "oxyos" / "apps" / app / name
            kept = json.loads(path.read_text()) if path.exists() else None
            content = with_home(content, preferences)
            if kept != content:
                fail(f"{app}'s {name} holds {kept!r}, expected {content!r}")
    if "window_rect" in expect and result.get("window") != expect["window_rect"]:
        fail(f"window at {result.get('window')}, expected {expect['window_rect']}")
    if "popup_rect" in expect and result.get("popups") != [expect["popup_rect"]]:
        fail(f"popups at {result.get('popups')}, expected [{expect['popup_rect']}]")
    if "popup_rects" in expect and result.get("popups", []) != expect["popup_rects"]:
        fail(f"popups at {result.get('popups', [])}, expected {expect['popup_rects']}")
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
    if "output_pixels_any" in expect:
        # One of several pictures, whichever it is (a shuffle's pick, which hashes what the person
        # chose: a photo folder's path in the run's own home): every point of one of them.
        misses = []
        for option in expect["output_pixels_any"]:
            missed = []
            check_pixels(name, {**expect, "pixels": option}, screenshot, [0, 0], missed.append)
            if not missed:
                break
            misses.append("; ".join(missed))
        else:
            fail("the output shows none of the pictures expected: " + " | ".join(misses))
    if "output_pixels_match" in expect:
        check_pixels_match(name, {**expect, "pixels_match": expect["output_pixels_match"]}, screenshot, [0, 0], fail)
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
    if "autofill" in expect:
        # AutoFill's list (docs/adr/0050-keychain-autofill-and-passkeys.mdx): what a shell draws
        # now (`shown`, null when none) and what the fixture keychain was asked during the step.
        held, wanted = result.get("autofill") or {}, expect["autofill"]
        differing = {k: held.get(k) for k, v in wanted.items() if held.get(k) != v}
        if differing:
            fail(f"AutoFill has {differing}, expected {wanted}")
    if "input_sources" in expect:
        held, wanted = result.get("input_sources") or {}, expect["input_sources"]
        differing = {k: held.get(k) for k, v in wanted.items() if held.get(k) != v}
        if differing:
            fail(f"the input sources have {differing}, expected {wanted}")
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
    if "pixels_match" in expect:
        check_pixels_match(name, expect, screenshot, result.get("window"), fail)
    if "latency_ms" in expect and result.get("latency_ms") is not None and not COLLECT:
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

def vulkan_driver_folders(environ):
    """The folders the Vulkan loader finds drivers' manifests in under `environ`, in its order
    (Linux), as oxyos-gpu-device's `manifest_dirs`: the user's and the system's configuration,
    then the user's and the system's data."""
    home = environ.get("HOME")
    config_home = environ.get("XDG_CONFIG_HOME") or (home and f"{home}/.config")
    data_home = environ.get("XDG_DATA_HOME") or (home and f"{home}/.local/share")
    bases = ([config_home] + (environ.get("XDG_CONFIG_DIRS") or "/etc/xdg").split(os.pathsep) + ["/etc", data_home]
             + (environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(os.pathsep))
    return [f"{base}/vulkan/icd.d" for base in bases if base and base.startswith("/")]


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

    def registry(self, into, offscreen):
        """The development registry (`applications/`) in `into`: each packaged application's entry the
        one oxyos-installd writes for it where its package is now (`oxyos-pack entry`, beside the
        offscreen compositor; the packages may have been built elsewhere and copied), with the
        development registry's place in the dock; any other entry as it is, its icon where its is."""
        source = pathlib.Path(__file__).resolve().parent / "applications" / "oxyos" / "applications"
        target = into / "oxyos" / "applications"
        target.mkdir(parents=True, exist_ok=True)
        pack = pathlib.Path(offscreen).resolve().parent / "oxyos-pack"
        for entry in sorted(source.glob("*.desktop")):
            text = entry.read_text().splitlines()
            if entry.stem in self.by_id:
                if not pack.is_file():
                    raise SystemExit(f"{pack} is not built: the registry's entries for the packages are its")
                lines = subprocess.run([str(pack), "entry", str(self.by_id[entry.stem]["dir"])], check=True,
                                       capture_output=True, text=True).stdout.splitlines()
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
    # Each package, or {"id", "listing"}: published as the store listing's (the slug its release
    # names, which `resolve` answers).
    for published in spec.get("publish", []):
        id, listing = (published, None) if isinstance(published, str) else (published["id"], published["listing"])
        run(pack, "publish", machine.by_id[id]["dir"], repository, keys, developer, *([listing] if listing else []))
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
        # What it opens goes with the capability that lets it (a manifest declares both or
        # neither): a version without `open-files` opens no types, one without `url-schemes` no
        # schemes.
        opens = {part: values for part, values in description.pop("opens", {}).items()
                 if {"types": "open-files", "schemes": "url-schemes"}.get(part) in later.get("capabilities", [])}
        if opens:
            description["opens"] = opens
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
# The run's store (`store_fixture`): its directory (the images it answered) and whether the machine
# went offline.
STORE = {}


def store_fixture(spec, out):
    """A store of the scenario's own (`store`, the module's docstring): its certificate authority and
    the server's certificate for api.oxy.so, made for the run with openssl; the server; the proxy that
    hands it api.oxy.so alone. The environment that makes the run reach it, and nothing else."""
    import ssl
    base = out / "store"
    shutil.rmtree(base, ignore_errors=True)
    base.mkdir(parents=True)
    run = lambda *args: subprocess.run(list(args), check=True, capture_output=True, cwd=base)
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca.key", "-out", "ca.pem",
        "-days", "2", "-subj", "/CN=OxyOS scenario store authority",
        "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign")
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", "server.key", "-out", "server.csr",
        "-subj", "/CN=api.oxy.so")
    hosts = ["api.oxy.so", "clients3.google.com", *spec.get("hosts", [])]
    (base / "server.ext").write_text("basicConstraints=critical,CA:FALSE\n"
                                     f"subjectAltName={','.join('DNS:' + host for host in hosts)}\n"
                                     "extendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature,keyEncipherment\n")
    run("openssl", "x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial",
        "-out", "server.pem", "-days", "2", "-extfile", "server.ext")
    failing = dict(spec.get("unavailable_first", {}))
    lock = threading.Lock()
    # The store's own files (`files`), where `{store_files}` in its answers names them.
    files = (base / "files").resolve()
    files.mkdir()
    for name, text in spec.get("files", {}).items():
        (files / name).write_text(text)
    # Whether the machine went offline (`offline_when_shown`).
    offline = threading.Event()
    # The tunnels open through the proxy, each its two sockets: cut when the machine goes offline,
    # as a network going away ends the connections over it (a client keeps them alive otherwise).
    tunnels = set()

    def go_offline():
        with lock:
            offline.set()
            for peers in list(tunnels):
                for peer in peers:
                    try:
                        peer.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass

    STORE.update(base=base, offline=offline, go_offline=go_offline)

    def image(handler, path):
        """An image of `store.images`: answered, its answer recorded when it ends."""
        wanted = spec["images"][path]
        if "png" in wanted:
            body, kind = png_bytes(*wanted["png"]), "image/png"
        elif "gif" in wanted:
            body, kind = gif_bytes(*wanted["gif"]), "image/gif"
        else:
            body, kind = wanted["svg"].encode(), "image/svg+xml"
        headers = dict(wanted.get("headers", {}))
        status = wanted.get("status", 200)
        lower = {name.lower(): value for name, value in headers.items()}
        if status == 200 and (
                ("etag" in lower and handler.headers.get("If-None-Match") == lower["etag"]) or
                ("last-modified" in lower and handler.headers.get("If-Modified-Since") == lower["last-modified"])):
            status, body = 304, b""
        handler.send_response(status)
        if status != 304:
            handler.send_header("Content-Type", kind)
        handler.send_header("Content-Length", str(len(body)))
        for name, value in headers.items():
            handler.send_header(name, value)
        handler.end_headers()
        cut = False
        try:
            if "delay_ms" in wanted:
                time.sleep(wanted["delay_ms"] / 1000)
            if "trickle_ms" in wanted:
                for at in range(0, len(body), 64):
                    time.sleep(wanted["trickle_ms"] / 1000)
                    handler.wfile.write(body[at:at + 64])
                    handler.wfile.flush()
            else:
                handler.wfile.write(body)
        except OSError:
            cut = True
            handler.close_connection = True
        with lock, open(base / "images.jsonl", "a") as log:
            log.write(json.dumps({"t": time.monotonic_ns(), "status": status, "path": path, "cut": cut}) + "\n")

    class Store(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def note(self, status):
            # Each request and its answer, for the run's record (`store/requests.log`).
            with lock, open(base / "requests.log", "a") as log:
                log.write(f"{time.monotonic():.3f} {status} {self.headers.get('Host', '')}{self.path}\n")

        def reply(self, status, body):
            self.note(status)
            data = json.dumps(body).replace("{store_files}", str(files)).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            from urllib.parse import parse_qs, urlsplit
            url = urlsplit(self.path)
            with lock:
                down = failing.get(url.path, 0) > 0
                if down:
                    failing[url.path] -= 1
            if down:
                return self.reply(503, {"message": "The store is down for the scenario"})
            if url.path in spec.get("images", {}):
                return image(self, url.path)
            if self.headers.get("Host", "").split(":")[0] == "clients3.google.com":
                if url.path != "/generate_204":
                    return self.reply(404, {"message": "Not found"})
                self.note(204)
                self.send_response(204)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            parts = [part for part in url.path.split("/") if part]
            page = lambda items: {"data": items, "pagination": {"total": len(items), "limit": 100,
                                                                "offset": 0, "hasMore": False}}
            if parts == ["store", "categories"]:
                return self.reply(200, {"data": spec.get("categories", [])})
            if parts == ["store", "apps"]:
                category = parse_qs(url.query).get("category", [None])[0]
                apps = [app for app in spec.get("apps", [])
                        if category is None or (app.get("category") or {}).get("slug") == category]
                return self.reply(200, page(apps))
            if len(parts) == 3 and parts[:2] == ["store", "apps"] and parts[2] in spec.get("listings", {}):
                return self.reply(200, {"data": spec["listings"][parts[2]]})
            if len(parts) == 4 and parts[:2] == ["store", "apps"] and parts[3] == "reviews":
                return self.reply(200, page([]))
            return self.reply(404, {"message": "Not found"})

        def do_HEAD(self):
            # NetInfo's check asks with HEAD.
            if self.headers.get("Host", "").split(":")[0] == "clients3.google.com" and \
                    self.path.split("?")[0] == "/generate_204":
                self.note(204)
                self.send_response(204)
                self.send_header("Content-Length", "0")
                self.end_headers()
            else:
                self.note(404)
                self.send_response(404)
                self.send_header("Content-Length", "0")
                self.end_headers()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Store)
    server.daemon_threads = True
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(base / "server.pem", base / "server.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    store_port = server.server_address[1]

    class Proxy(BaseHTTPRequestHandler):
        """api.oxy.so:443 (and the reachability check's host) tunnelled to the store; every other
        host refused."""

        def log_message(self, *args):
            pass

        def do_CONNECT(self):
            with lock, open(base / "requests.log", "a") as log:
                log.write(f"{time.monotonic():.3f} CONNECT {self.path}\n")
            if self.path not in [f"{host}:443" for host in hosts] or offline.is_set():
                self.send_error(403, "Offline: the scenario has no network but its store")
                return
            upstream = socket.create_connection(("127.0.0.1", store_port))
            self.send_response(200, "Connection established")
            self.end_headers()
            peers = (self.connection, upstream)
            with lock:
                tunnels.add(peers)

            def pump(source, sink):
                try:
                    while chunk := source.recv(65536):
                        sink.sendall(chunk)
                except OSError:
                    pass
                finally:
                    for peer in peers:
                        try:
                            peer.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            pass

            other = threading.Thread(target=pump, args=(upstream, self.connection), daemon=True)
            other.start()
            pump(self.connection, upstream)
            other.join()
            with lock:
                tunnels.discard(peers)
            upstream.close()
            self.close_connection = True

        def refuse(self):
            self.send_error(403, "Offline: the scenario has no network but its store")

        do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = do_PATCH = refuse

    proxy = ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
    proxy.daemon_threads = True
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    address = f"http://127.0.0.1:{proxy.server_address[1]}"
    return {"HTTPS_PROXY": address, "HTTP_PROXY": address, "ALL_PROXY": address,
            "NO_PROXY": "127.0.0.1,localhost", "SSL_CERT_FILE": str(base / "ca.pem")}


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


def matches_within(wanted, got, where):
    """How `got` differs from `wanted`: each key `wanted` gives (a dict's keys, a list's items, of
    the same length), compared exactly otherwise."""
    if isinstance(wanted, dict):
        if not isinstance(got, dict):
            return [f"{where} is {got!r}, expected {wanted!r}"]
        return [problem for key, value in wanted.items()
                for problem in matches_within(value, got.get(key), f"{where} {key}")]
    if isinstance(wanted, list):
        if not isinstance(got, list) or len(got) != len(wanted):
            return [f"{where} is {got!r}, expected {wanted!r}"]
        return [problem for index, (value, item) in enumerate(zip(wanted, got))
                for problem in matches_within(value, item, f"{where} [{index}]")]
    return [] if got == wanted else [f"{where} is {got!r}, expected {wanted!r}"]


def network_daemon(spec, out, offscreen):
    """A bus of the run's own and a played NetworkManager on it (docs/adr/0038-networks.mdx), the
    machine `spec` describes (oxyos-fake-networkmanager's: access points with their passwords,
    saved connections, Ethernet); its address and a function that stops both."""
    base = out / "network"
    shutil.rmtree(base, ignore_errors=True)
    base.mkdir(parents=True)
    # An abstract socket (`tmpdir`): a run's directory is too deep for a socket's path.
    bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address=1",
                            "--address=unix:tmpdir=/tmp"], stdout=subprocess.PIPE, text=True)
    address = bus.stdout.readline().strip()
    (base / "machine.json").write_text(json.dumps(spec))
    fake = pathlib.Path(offscreen).resolve().parent / "oxyos-fake-networkmanager"
    log = open(base / "daemon.log", "w")
    daemon = subprocess.Popen([str(fake), "--address", address, "--machine", str(base / "machine.json"),
                               "--state", str(base / "state.json")], stdout=log, stderr=log)
    wait_for(base / "state.json")

    def stop():
        for process in (daemon, bus):
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        log.close()
    return address, stop


def bluetooth_daemon(spec, out, offscreen):
    """A bus of the run's own and a played BlueZ on it (docs/adr/0055-bluetooth.mdx), the machine
    `spec` describes (oxyos-fake-bluez's: the adapter, the devices paired and nearby, how each
    pairs and connects); its address and a function that stops both."""
    base = out / "bluetooth"
    shutil.rmtree(base, ignore_errors=True)
    base.mkdir(parents=True)
    # An abstract socket (`tmpdir`): a run's directory is too deep for a socket's path.
    bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address=1",
                            "--address=unix:tmpdir=/tmp"], stdout=subprocess.PIPE, text=True)
    address = bus.stdout.readline().strip()
    (base / "machine.json").write_text(json.dumps(spec))
    fake = pathlib.Path(offscreen).resolve().parent / "oxyos-fake-bluez"
    log = open(base / "bluez.log", "w")
    daemon = subprocess.Popen([str(fake), "--address", address, "--machine", str(base / "machine.json"),
                               "--state", str(base / "state.json")], stdout=log, stderr=log)
    wait_for(base / "state.json")

    def stop():
        for process in (daemon, bus):
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        log.close()
    return address, stop


def power_services(spec, out, offscreen):
    """A bus of the run's own and UPower, power-profiles-daemon and logind played on it by the
    development compositor's own program (`--play-power`, docs/adr/0057-battery-and-energy.mdx),
    for the machine `spec` describes, its battery's sysfs a directory of the run's own (UPower's
    thresholds written there, 0 and 100 to begin with): the compositor's arguments and a function
    that stops both."""
    base = out / "power"
    shutil.rmtree(base, ignore_errors=True)
    sysfs = base / "BAT0"
    sysfs.mkdir(parents=True)
    (sysfs / "charge_control_start_threshold").write_text("0")
    (sysfs / "charge_control_end_threshold").write_text("100")
    machine = dict(spec)
    machine.setdefault("sysfs", str(sysfs))
    (base / "machine.json").write_text(json.dumps(machine))
    bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address=1",
                            "--address=unix:tmpdir=/tmp"], stdout=subprocess.PIPE, text=True)
    address = bus.stdout.readline().strip()
    log = open(base / "services.log", "w")
    services = subprocess.Popen([str(pathlib.Path(offscreen).resolve()), "--play-power", address,
                                 str(base / "machine.json"), str(base / "ready")], stdout=log, stderr=log)
    wait_for(base / "ready")

    def stop():
        for process in (services, bus):
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        log.close()
    return ["--power-bus", address], stop


def install_arguments(scenario, out, machine=None, offscreen=None):
    """System grants (`grants`: {ID: [GRANT, ...]}) as the image's policy gives them, and the
    install service the session asks (`installd`: null for a socket no service is at, or
    {"publish": [ID or {"id", "listing"}, ...]} for a real one, `install_service`),
    docs/adr/0030-installing-applications.mdx."""
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
    parser.add_argument("--input-methods", default=None,
                        help="the input-method engine a scenario with `input_methods` runs (ADR 0041), "
                             "else OXYOS_INPUT_METHODS")
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
    parser.add_argument("--renderer", choices=["cpu", "gpu"], default=None,
                        help="the host's painter (docs/adr/0031-gpu-buffers.mdx): the CPU's, or the GPU's (failing "
                             "when the compositor offers no GPU it can share buffers with); by default the GPU's "
                             "where one is offered")
    parser.add_argument("--check-render", action="store_true",
                        help="the host draws every frame on the CPU too; a GPU frame whose 4x4 blocks' means differ "
                             "from the CPU frame's by more than 32 levels fails")
    parser.add_argument("--gpu-only", action="store_true",
                        help="compose on the Vulkan device alone, with no CPU reference (measures it)")
    parser.add_argument("--no-render-counts", action="store_true",
                        help="never count what React re-renders, even for a scenario holding `renders` (whose "
                             "checks are then not made): perf.py's runs, which measure speed, which the count's "
                             "allocations would move")
    parser.add_argument("--app-cache", default=None,
                        help="the applications' cache directories in this directory (XDG_CACHE_HOME), kept across "
                             "runs, as they are after an application's first launch (perf.py's cold start: Ganesh's "
                             "compiled pipelines among them); afresh in the run's output by default")
    parser.add_argument("--collect", action="store_true",
                        help="enforce no budget on time (latency_ms, js_work's and resize_work's, the backstops): the "
                             "caller measures them over its runs and judges them at the median (perf.py)")
    parser.add_argument("--speed-budgets", action="store_true",
                        help="hold the steps' budgets on how fast the work runs too (`js_work`'s js_ms_max and "
                             "js_blocked_ms_max, `resize_work`'s js_ms_p95_max and busy_refreshes_p95_max): perf.py's "
                             "runs, on the hardware the budgets describe; never the load-sensitive deploy gate")
    options = parser.parse_args()
    global PACKAGES
    PACKAGES = pathlib.Path(options.packages) if options.packages else None
    global SPEED_BUDGETS, RENDER_COUNTS, COLLECT, RUN_TZ, RUN_LOCALE
    COLLECT = options.collect
    SPEED_BUDGETS = options.speed_budgets and not COLLECT

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
    # A native client's scenario runs no Bloom application but its file chooser's (ADR 0029,
    # decision 8) or a companion (ADR 0035), which need the runtimes and packages too.
    if "client" not in scenario or scenario.get("file_chooser") or scenario.get("companion"):
        if not options.runtimes or not options.packages:
            raise SystemExit("--runtimes and --packages are needed: the file chooser is a Bloom application")
        machine = Machine(pathlib.Path(options.runtimes), pathlib.Path(options.packages))
    width, height = scenario.get("window", [1024, 640])

    command = [options.offscreen, "--launch", "--out", str(out)] + (["--gpu"] if options.gpu else [])
    if options.gpu_only:
        command += ["--gpu-only"]
    # A scenario about what libinput does (natural scrolling) runs only through the devices.
    through_devices = options.uinput or scenario.get("uinput", False)
    if through_devices:
        command += ["--uinput"]
    if options.seat:
        command += ["--seat"]
    if "launch_settle_ms" in scenario:
        # How long the application's first frames must stay unchanged before the steps begin: a
        # browser shows its window before its interface draws in it (ADR 0029).
        command += ["--launch-settle", str(scenario["launch_settle_ms"])]
    if through_devices or "layout" in scenario:
        # The device expectations are the scenario's layout's: its dead keys, its Shift keys. It
        # is also the layout the session tells applications (ADR 0029).
        command += ["--layout", scenario.get("layout", "us")]
    if scenario.get("outputs"):
        # Several outputs (ADR 0018), a shell on each.
        command += ["--outputs", scenario["outputs"]]
    if scenario.get("screenshots"):
        # Screenshots written as the session writes them (ADR 0024, decision 10), in the run's own
        # Desktop and Documents.
        shutil.rmtree(out / "screenshots", ignore_errors=True)
        command += ["--screenshots", str(out / "screenshots")]
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
        # With a home of the run's own, the dock's Trash is its Trash, followed as the session
        # follows the user's (ADR 0034); never the developer's.
        if "home" in scenario:
            command += ["--trash"]
    if scenario.get("companion"):
        # A second application started once the first's window is shown (ADR 0035): the other side
        # of a drag between applications, its report the application's with ".companion".
        companion = scenario["companion"]
        companion_host, companion_bundle, _ = machine.run(companion["module"])
        companion_width, companion_height = companion.get("window", [320, 200])
        command += ["--companion", f"{companion_host},{companion_bundle},{companion['module']},--window,"
                    f"{companion_width},{companion_height}"]
        if "at" in companion:
            command += ["--companion-at", ",".join(map(str, companion["at"]))]
        if "app" in companion:
            # The application id it claims: the application's own makes the two windows one
            # application's (Super+`, docs/adr/0024-system-shortcuts.mdx).
            command += ["--companion-app", companion["app"]]
    if scenario.get("file_chooser"):
        # The file chooser the session starts for the application's requests (ADR 0029, decision
        # 8): the module run as the file manager is, in its picker mode, its report the
        # application's with ".chooser".
        chooser_host, chooser_bundle, _ = machine.run(scenario["file_chooser"])
        command += ["--file-chooser", f"{chooser_host},{chooser_bundle},{scenario['file_chooser']}"]
    # The user's preferences (ADR 0019), in a file of the run's own: light unless the scenario says
    # otherwise, so what a scenario sees never depends on the time of day it runs at (an automatic
    # appearance follows the sun). With `settings`, the application the session would grant it to.
    preferences = out / "preferences"
    # Carets stay lit unless the scenario turns blinking on (ADR 0037): a blinking caret never lets
    # a step's frames settle.
    given = {"appearance": "light", "caret_blink": "false", **scenario.get("preferences", {}),
             **scenario.get("settings", {}).get("preferences", {})}
    # "{repo}" in a value is the repository's root: a wallpaper of a file of its own (ADR 0048's
    # `photo:FIT:RRGGBB:PATH` takes an absolute path).
    repo = str(pathlib.Path(__file__).resolve().parents[2])
    preferences.write_text("".join(f"{key} = {str(value).replace('{repo}', repo)}\n"
                                   for key, value in given.items()))
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
    if "sound" in scenario:
        # The machine's outputs and inputs (ADR 0056), played in the development compositor as the
        # scenario describes them.
        sound = out / "sound.json"
        sound.write_text(json.dumps(scenario["sound"]))
        command += ["--sound", str(sound)]
    if scenario.get("installed"):
        command += ["--installed-fixture", str(installed_fixture(out / "installed", scenario["installed"],
                                                                  machine, options.offscreen))]
    stop_network = None
    if "network" in scenario:
        # The system's network daemon, played on a bus of the run's own (ADR 0038): the session's
        # networks and the played machine's Wi-Fi are its, never the developer's machine's.
        network_address, stop_network = network_daemon(scenario["network"], out, options.offscreen)
        command += ["--network-bus", network_address]
    stop_bluetooth = None
    if "bluetooth" in scenario:
        # The system's BlueZ, played on a bus of the run's own (ADR 0055): the session's Bluetooth,
        # its pairing agent and the played machine's Control Center module are its, never the
        # developer's machine's.
        bluetooth_address, stop_bluetooth = bluetooth_daemon(scenario["bluetooth"], out, options.offscreen)
        command += ["--bluetooth-bus", bluetooth_address]
    stop_power = None
    if "power" in scenario:
        # The machine's power services, played on a bus of the run's own (ADR 0057): the
        # session's battery, Low Power Mode, charge limit and sleep are theirs, never the
        # developer's machine's.
        power_arguments, stop_power = power_services(scenario["power"], out, options.offscreen)
        command += power_arguments
    if "input_sources" in scenario:
        # The input sources the session tells (docs/adr/0041-input-methods.mdx, decision 6),
        # played with an engine that has the input methods named (none: none installed), from the
        # run's preferences and its layout.
        command += ["--input-sources-available", ",".join(scenario["input_sources"]["available"])]
    if scenario.get("lock_password"):
        # The lock screen's password (ADR 0010): the development compositor's fixture unlocks with
        # it, never PAM.
        command += ["--lock-password", scenario["lock_password"]]
        if scenario.get("lock_pin"):
            # The account's PIN (ADR 0020 decision 2), which the fixture takes said as one.
            command += ["--lock-pin", scenario["lock_pin"]]
    if "reader" in scenario:
        # The session's screen reader (ADR 0014), saying into a record, in the scenario's locale.
        reader = options.reader or str(pathlib.Path(options.offscreen).resolve().parent / "oxyos-reader")
        if "locale" in scenario["reader"]:
            raise SystemExit(f"{options.scenario}: the reader speaks the scenario's `locale` (ADR 0040), not one of its own")
        command += ["--reader", reader, "--reader-locale", scenario.get("locale", FIXED_LOCALE)]
        if scenario["reader"].get("off"):
            command += ["--reader-off"]
    if scenario.get("input_method"):
        # The session plays an input method the scenario's steps drive (ADR 0008).
        command += ["--input-method"]
        if scenario.get("im_candidates"):
            # It hands the system candidates for what it composes, laid out "horizontal" or
            # "vertical", which the session's shell draws at the caret (ADR 0041).
            command += ["--im-candidates", scenario["im_candidates"]]
    if scenario.get("autofill"):
        # AutoFill (docs/adr/0050-keychain-autofill-and-passkeys.mdx) answered by the development
        # compositor's fixture keychain, its made-up items associated with the application named.
        command += ["--autofill-fixture", scenario["autofill"]["application"]]
    if scenario.get("input_methods"):
        # The session's input methods (ADR 0041): the real engine, fcitx5 and its engines, built
        # by tools/input-methods/build.sh, run as the session runs it.
        prefix = options.input_methods or os.environ.get("OXYOS_INPUT_METHODS")
        if not prefix:
            raise SystemExit(f"{options.scenario}: needs the input-method engine "
                             "(--input-methods PREFIX or OXYOS_INPUT_METHODS; tools/input-methods/build.sh)")
        command += ["--input-methods", prefix]
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
        command += director_args(step, out)
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
    # What React re-renders is counted (report.h, `react_render`) only for a scenario that holds it
    # and a run that does not measure speed: walking React's commits allocates, which moves where
    # Hermes' collections fall.
    RENDER_COUNTS = holds_renders(scenario["steps"]) and not options.no_render_counts
    env.pop("OXYOS_HOST_REPORT_RENDERS", None)
    if RENDER_COUNTS:
        env["OXYOS_HOST_REPORT_RENDERS"] = "1"
    # glibc fills what it frees (and what it hands out) with this byte, so a use of freed memory
    # crashes the run every time instead of only when the memory happened to be reused: a host
    # destroying JavaScript's pending timers after its runtime crashed one exit in tens, and with
    # this every exit that does it.
    env["MALLOC_PERTURB_"] = "165"
    if "client" not in scenario or scenario.get("companion"):
        # The runtimes the applications' entries name (ADR 0030), the scenario's in place of the image's.
        command[2:2] = ["--runtimes", str(machine.runtimes)]
        if login is None and "client" not in scenario:
            env["OXYOS_SCRIPT_URL"] = f"file://{app_script}"
    # The GPU driver's shader cache is the machine's, as a session's is: kept across launches, not
    # made afresh with the run's cache below, or every application would compile the GPU painter's
    # pipelines anew (seconds on lavapipe, where a warm cache takes a tenth of a second).
    if "MESA_SHADER_CACHE_DIR" not in env:
        machine_cache = env.get("XDG_CACHE_HOME") or str(pathlib.Path(env.get("HOME", "/tmp")) / ".cache")
        env["MESA_SHADER_CACHE_DIR"] = machine_cache  # Mesa keeps its cache in a folder of its own there
    # The GPU's drivers are the machine's, as a session's are, wherever the run's XDG directories
    # (its data, its registry, its home) put the folders the Vulkan loader looks in: those this
    # environment has are named (VK_ADD_DRIVER_FILES, which the loader and oxyos-gpu-device read
    # first), unless the developer named drivers alone (VK_DRIVER_FILES). Without them the shell's
    # and its applications' hosts found no driver and drew on the CPU, and a GPU composition none.
    if not env.get("VK_DRIVER_FILES") and not env.get("VK_ICD_FILENAMES"):
        env["VK_ADD_DRIVER_FILES"] = os.pathsep.join(
            [path for path in env.get("VK_ADD_DRIVER_FILES", "").split(os.pathsep) if path]
            + vulkan_driver_folders(env))
    # What applications keep (their data and cache directories, application.h) is the run's own,
    # never the user's; a scenario's `app_data` is there before the application starts.
    for kept in ("xdg-data", "xdg-cache"):
        shutil.rmtree(out / kept, ignore_errors=True)
    env["XDG_DATA_HOME"] = str(out / "xdg-data")
    env["XDG_CACHE_HOME"] = str(pathlib.Path(options.app_cache).resolve() if options.app_cache else out / "xdg-cache")
    if scenario.get("shell"):
        # The shell's applications are the development registry's alone (ADR 0013), not the
        # machine's, each packaged one from its package.
        env["XDG_DATA_DIRS"] = str(machine.registry(out / "registry", options.offscreen))
    # The application's title and id, as a session's launch records give them (ADR 0009).
    for key, name in (("title", "OXYOS_APP_TITLE"), ("id", "OXYOS_APP_ID")):
        if key in scenario.get("app", {}):
            env[name] = scenario["app"][key]
    # Its system grants, as a launch of the image's policy tells them (OXYOS_APP_GRANTS, ADR 0030):
    # what JavaScript offers with them (the store's installer with `store`). The applications the
    # shell starts are told theirs by the launcher, from the same `grants`.
    if scenario.get("app", {}).get("id") in scenario.get("grants", {}):
        env["OXYOS_APP_GRANTS"] = ",".join(scenario["grants"][scenario["app"]["id"]])
    # What the session tells an application it launches of where it came from and what its account
    # granted (ADR 0030, decision 8): an installed version is given only those capabilities.
    if "origin" in scenario.get("app", {}):
        env["OXYOS_APP_ORIGIN"] = scenario["app"]["origin"]
        env["OXYOS_APP_CAPABILITIES"] = ",".join(scenario["app"].get("capabilities", []))
    # An application the session did not start (ADR 0013): its id is only its claim.
    if scenario.get("app", {}).get("recorded") is False:
        env["OXYOS_APP_UNRECORDED"] = "1"
    # A local server for the scenario's probe, for as long as the run lasts, and the port after
    # it held bound without listening, so that connecting there is refused at once and nothing
    # else can answer (an unbound port may instead drop the connection, as WSL's firewall does).
    # The accounts' service's session socket (decision 6): the scenario's played one (`account`),
    # else a path where nothing listens, so no run ever asks the machine's.
    account_dir = pathlib.Path(tempfile.mkdtemp(prefix="oxyos-account-"))
    atexit.register(shutil.rmtree, account_dir, True)
    env["OXYOS_ACCOUNTD_SESSION_SOCKET"] = str(account_dir / "session")
    if "account" in scenario:
        account_log = out / "account-session.jsonl"
        account_log.unlink(missing_ok=True)
        account_listener = serve_account_session(account_dir / "session", scenario["account"], account_log)
        if scenario["account"].get("later"):
            account_later(scenario["account"]["later"], report_path)
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
    if options.renderer:
        env["OXYOS_HOST_RENDERER"] = options.renderer
    if options.check_render:
        env["OXYOS_HOST_RENDER_CHECK"] = "1"
    # The system's wallpapers, as the image installs them (docs/adr/0048-wallpapers.mdx), never the
    # developer's machine's.
    env["OXYOS_WALLPAPERS"] = str(pathlib.Path(__file__).resolve().parents[2] /
                                  "config/includes.chroot_after_packages/usr/share/oxyos/wallpapers")
    env.pop("OXYOS_WALLPAPER_NOW", None)
    if scenario.get("wallpaper"):
        # The moment the wallpaper follows and the time zone of its sun (ADR 0012), fixed.
        wallpaper = scenario["wallpaper"]
        env["TZ"] = wallpaper["tz"]
        env["OXYOS_WALLPAPER_NOW"] = str(wallpaper["now"])
    if login is not None:
        env.update(login[3])
    if "store" in scenario:
        env.update(store_fixture(scenario["store"], out))
        if scenario["store"].get("offline_when_shown"):
            offline_when_shown(scenario["store"]["offline_when_shown"], report_path)
    home_root = None
    if "home" in scenario:
        # A home of the run's own (docs/adr/0028-file-access.mdx), never the developer's.
        home_root = make_home(out, scenario["home"])
        env["HOME"] = str(home_root)
        env["XDG_DATA_HOME"] = str(home_root / ".local/share")
        env["XDG_CONFIG_HOME"] = str(home_root / ".config")
        env["XDG_CACHE_HOME"] = str(pathlib.Path(options.app_cache).resolve() if options.app_cache
                                    else home_root / ".cache")
    else:
        # Without one, a home of the run's own all the same, empty: never the developer's. A home
        # with Desktop, Documents and Downloads in it (a machine that had a desktop) made the
        # dock's zoom (dock-edges, window-zoom-scaled) leave the work area uncovered, and passed
        # for a user whose home had none. The machine's GPU cache and drivers are named above.
        empty = (out / "home").resolve()
        shutil.rmtree(empty, ignore_errors=True)
        (empty / ".config").mkdir(parents=True)
        env["HOME"] = str(empty)
        env["XDG_CONFIG_HOME"] = str(empty / ".config")
    # Its data directory is under the run's XDG_DATA_HOME, its home's in a scenario with one.
    for app, files in scenario.get("app_data", {}).items():
        directory = pathlib.Path(env["XDG_DATA_HOME"]) / "oxyos" / "apps" / app
        directory.mkdir(parents=True, exist_ok=True)
        for name, content in files.items():
            (directory / name).write_text(json.dumps(content))
    # One fixed language, never the machine's (docs/adr/0040-language-and-region.mdx, decision 8).
    for variable in ("LANGUAGE", "LC_MESSAGES", "LC_TIME", "LC_NUMERIC", "LC_CTYPE"):
        env.pop(variable, None)
    RUN_LOCALE = env["LANG"] = env["LC_ALL"] = scenario.get("locale", FIXED_LOCALE)
    # The session's keyboard composes with the image's compose tables (config/, as CI's tests do),
    # never the developer's machine's.
    env.setdefault("XLOCALEDIR", str(pathlib.Path(__file__).resolve().parents[2] /
                                     "config/includes.chroot_after_packages/usr/share/X11/locale"))
    # One fixed time zone, never the machine's: the scenario's `tz`, else the one the scenarios'
    # dates and times were written in (a machine in another zone showed every date it formats
    # hours apart, and a wallpaper that follows the sun at another hour).
    env["TZ"] = RUN_TZ = scenario.get("tz", scenario.get("wallpaper", {}).get("tz", FIXED_TZ))
    # The order the shell's host is to force between the keyboard reaching a region and JavaScript's
    # accessibility focus moving into it (bloom-host.cpp, OXYOS_HOST_TEST_ORDER): "focus-first",
    # "grant-first" or "popup-focus-first" (docs/adr/0039-keyboard-access.mdx, decision 26).
    env.pop("OXYOS_HOST_TEST_ORDER", None)
    if "host_order" in scenario:
        if scenario["host_order"] not in ("focus-first", "grant-first", "popup-focus-first"):
            raise ValueError(f"unknown host_order {scenario['host_order']!r}")
        env["OXYOS_HOST_TEST_ORDER"] = scenario["host_order"]
    recorder = None
    if home_root is not None:
        report_path.unlink(missing_ok=True)
        change_home_later(home_root, scenario["home"], report_path)
        recorder = HomeRecorder(home_root, wants_xattrs(scenario))
    try:
        run = subprocess.run(command, env=env, capture_output=True, text=True)
    finally:
        if stop_network is not None:
            stop_network()
        if stop_bluetooth is not None:
            stop_bluetooth()
        if stop_power is not None:
            stop_power()
    if recorder is not None:
        recorder.stop()
    (out / "director.log").write_text(run.stdout + run.stderr)
    recorded = []
    if login is not None:
        login[0].terminate()
        login[0].wait(timeout=20)
        recorded = login[2].read_text().splitlines() if login[2].exists() else []
        shutil.rmtree(login[4][1], ignore_errors=True)
    # A host that crashed (bloom-host.cpp's handler: "[host] fatal signal N" and its backtrace),
    # at any time, even as the run ended and whatever the steps said, fails the run, its backtrace
    # said: a crash is never a pass, and the run that saw it is the one that can say where.
    logged = run.stderr.splitlines()
    crashes = [at for at, line in enumerate(logged) if line.startswith("[host] fatal signal")]
    for at in crashes:
        print(f"FAIL: a host crashed; see {out / 'director.log'}")
        print("\n".join(logged[at:at + 24]))
    if run.returncode != 0:
        print(f"FAIL: the director failed ({run.returncode}); see {out / 'director.log'}")
        print("\n".join(run.stderr.strip().splitlines()[-5:]))
        return 1
    if crashes:
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
              if "reader" in scenario and speech_path.exists() else [] if "reader" in scenario else None)

    failures = []
    start = summary["t_launch"]
    refresh_hz = display_hz(scenario)
    for index, (step, result) in enumerate(zip(scenario["steps"], summary["steps"])):
        start = check_step(index, step, result, report, start, failures, through_devices,
                           out / f"launch-{index + 2}.png", shell_report, spoken,
                           out / "preferences", refresh_hz, recorder)
        check_views(index, step, result, summary["steps"], failures)
    # The person's input reached whom it should, once (every scenario, every step).
    check_input_delivery(summary, host_reports(out), failures)
    # Each host's frame diagnostics against its report, and the log they wrote to.
    check_diagnostics(report, run.stdout + run.stderr, failures)
    primary = (scenario.get("outputs") or "").split(",")[0]
    if ":" in primary and summary["steps"]:
        check_pacing(report, display_hz(scenario), summary["steps"][-1].get("t_settled", 0), failures)
    if shell_report:
        check_diagnostics(shell_report, run.stdout + run.stderr, failures, "the shell")
    # A frame the host drew from its damage that differs from the frame drawn whole.
    for line in report:
        if line["kind"] == "damage_mismatch":
            failures.append(f"the host's frame drawn from its damage differs in {line['pixels']} pixels")
    # A frame drawn on the GPU that does not look as the CPU draws it (ADR 0031, decision 13).
    launched_path = out / "host-report.jsonl.launched"
    launched_report = ended_report(launched_path)
    checked = [line for line in list(report) + list(shell_report or []) + launched_report
               if line["kind"] == "render_check"]
    # Skia's raster and Vulkan backends anti-alias edges, glyphs and blurs each their own way (up to 76
    # levels on an edge pixel, 1.2 % of a frame's pixels beyond 3 levels, in the scenarios measured
    # on lavapipe), which a 4 x 4 block's mean evens out (14 levels at most there); an edge a pixel
    # off, a glyph missing or a layer misplaced moves a block's mean by far more (64 levels for a
    # pixel-wide edge at full contrast).
    for line in checked:
        if line.get("blocks", 255) > 32:
            failures.append(f"surface {line['surface']}'s frame on the GPU differs from the CPU's: a 4x4 block's "
                            f"mean by {line.get('blocks')} levels ({line['beyond_three']} of {line['pixels']} pixels "
                            f"by more than 3 levels, {line['largest']} at most)")
    if options.check_render:
        if options.renderer != "cpu" and not checked:
            failures.append("--check-render, but no frame was drawn on the GPU")
        worst = max((line["largest"] for line in checked), default=0)
        print(f"render check: {len(checked)} GPU frames compared, largest difference {worst}", file=sys.stderr)
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
    # Secrets typed in the scenario (a Wi-Fi password, ADR 0038) reach the daemon alone: never
    # the user's preferences or configuration, nor what the application was told or reported.
    if scenario.get("secrets"):
        kept = [out / "preferences", report_path]
        config = pathlib.Path(env.get("XDG_CONFIG_HOME", ""))
        if config.is_dir() and str(config).startswith(str(out)):
            kept += [path for path in config.rglob("*") if path.is_file()]
        for secret in scenario["secrets"]:
            for path in kept:
                if path.exists() and secret.encode() in path.read_bytes():
                    failures.append(f"a secret typed is in {path}")
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
