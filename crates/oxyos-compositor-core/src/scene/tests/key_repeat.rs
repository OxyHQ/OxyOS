//! Key repeat (ADR 0036): a key held is pressed again by the compositor, after the delay and at the
//! rate, where its press went, with its text; what ends it; and the system's keys that repeat
//! (ADR 0024, decision 8).

use super::*;
use crate::scene::controls::{KEY_MUTE, KEY_VOLUMEUP};
use crate::scene::{REPEAT_DELAY, REPEAT_INTERVAL};
use std::time::Duration;

const SHIFT: u32 = 42;
const CAPS_LOCK: u32 = 58;
const BACKSPACE: u32 = 14;
const TAB: u32 = 15;
const Q: u32 = 16;
const W: u32 = 17;
const GRAVE: u32 = 41;
/// When the tests begin, on the scene's clock, which only goes forward.
const START: Duration = Duration::from_secs(100);

/// A press the keymap repeats, through the keyboard's path, at `at`.
fn hold(h: &mut Harness, at: Duration, code: u32, text: Option<&str>) {
    h.scene.set_clock(at);
    h.scene.set_input_time(ns(at));
    h.scene.keyboard_repeating(code, true, text, true);
}

fn release(h: &mut Harness, code: u32) {
    h.scene.keyboard_repeating(code, false, None, false);
}

/// The clock at `at`, and a turn's repeat.
fn turn(h: &mut Harness, at: Duration) {
    h.scene.set_clock(at);
    h.scene.repeat_keys();
}

/// When the `n`th repeat of a key pressed at `at` is due.
fn nth(at: Duration, n: u32) -> Duration {
    at + REPEAT_DELAY + REPEAT_INTERVAL * (n - 1)
}

/// The presses `client` was sent, as (key, time).
fn presses_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<(u32, u64)> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::Key {
                key,
                pressed: true,
                time_ns,
                ..
            } => Some((*key, *time_ns)),
            _ => None,
        })
        .collect()
}

fn texts_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<String> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::Text { text, .. } | Event::TextInputCommitText { text, .. } => {
                Some(text.as_str().to_string())
            }
            _ => None,
        })
        .collect()
}

fn ns(time: Duration) -> u64 {
    time.as_nanos() as u64
}

fn focused_window(h: &mut Harness) -> SurfaceKey {
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(h);
    window
}

#[test]
fn a_held_key_repeats_after_the_delay_at_the_rate_with_its_text() {
    let mut h = Harness::new();
    focused_window(&mut h);
    hold(&mut h, START, KEY_A, Some("a"));
    assert_eq!(h.scene.next_repeat(), Some(START + REPEAT_DELAY));
    let events = logged(&mut h);
    assert_eq!(presses_to(&events, APP), [(KEY_A, ns(START))]);
    assert_eq!(texts_to(&events, APP), ["a"]);
    // Not before the delay.
    turn(&mut h, START + REPEAT_DELAY - Duration::from_millis(1));
    assert!(logged(&mut h).is_empty());
    // Then once a turn, at the rate, each a press with the text, at the time it was due.
    for n in 1..=3 {
        turn(&mut h, nth(START, n));
        let events = logged(&mut h);
        assert_eq!(presses_to(&events, APP), [(KEY_A, ns(nth(START, n)))]);
        assert_eq!(texts_to(&events, APP), ["a"]);
        assert!(keys_to(&events, APP).iter().all(|(_, pressed)| *pressed));
        assert_eq!(h.scene.next_repeat(), Some(nth(START, n + 1)));
    }
    // A turn three intervals late sends one, not the three it missed, and the next comes an
    // interval later.
    let late = nth(START, 7);
    turn(&mut h, late);
    assert_eq!(presses_to(&logged(&mut h), APP).len(), 1);
    assert_eq!(h.scene.next_repeat(), Some(late + REPEAT_INTERVAL));
    // Released: the release, once, and no more repeats.
    release(&mut h, KEY_A);
    assert_eq!(h.scene.next_repeat(), None);
    turn(&mut h, nth(START, 20));
    assert_eq!(keys_to(&logged(&mut h), APP), [(KEY_A, false)]);
}

#[test]
fn a_repeat_types_into_a_field_with_text_input_as_its_press_did() {
    let mut h = Harness::new();
    let window = focused_window(&mut h);
    enable_text_input(&mut h, window, "abc", 1);
    logged(&mut h);
    hold(&mut h, START, KEY_B, Some("b"));
    turn(&mut h, nth(START, 1));
    turn(&mut h, nth(START, 2));
    let events = logged(&mut h);
    assert_eq!(presses_to(&events, APP).len(), 3);
    assert_eq!(texts_to(&events, APP), ["b", "b", "b"]);
    assert!(!to(&events, APP)
        .iter()
        .any(|event| matches!(event, Event::Text { .. })));
    // Backspace types nothing: each repeat is its press alone.
    release(&mut h, KEY_B);
    let later = START + Duration::from_secs(5);
    hold(&mut h, later, BACKSPACE, None);
    turn(&mut h, nth(later, 1));
    let events = logged(&mut h);
    assert_eq!(
        presses_to(&events, APP),
        [(BACKSPACE, ns(later)), (BACKSPACE, ns(nth(later, 1)))]
    );
    assert!(texts_to(&events, APP).is_empty());
}

#[test]
fn a_client_with_key_text_gets_each_repeat_as_a_press_with_its_text() {
    // Astro (ADR 0029) takes each key's text with the key: a repeat is a KeyText press of the
    // key it holds, with the text, a new serial and the time it was due, and never a Text. It
    // has no repeat timer of its own and marks such a press a repeat (`EF_IS_REPEAT`).
    let mut h = Harness::new();
    focused_window(&mut h);
    h.request(APP, Request::SetKeyText { enabled: true })
        .unwrap();
    logged(&mut h);
    hold(&mut h, START, KEY_A, Some("é"));
    turn(&mut h, nth(START, 1));
    turn(&mut h, nth(START, 2));
    let up = nth(START, 2) + Duration::from_millis(10);
    h.scene.set_input_time(ns(up));
    release(&mut h, KEY_A);
    let events = logged(&mut h);
    let keys: Vec<(u64, bool, String, u64)> = to(&events, APP)
        .iter()
        .filter_map(|event| match event {
            Event::KeyText {
                serial,
                key: KEY_A,
                pressed,
                text,
                time_ns,
                ..
            } => Some((*serial, *pressed, text.as_str().to_string(), *time_ns)),
            _ => None,
        })
        .collect();
    let presses: Vec<(bool, &str, u64)> = keys
        .iter()
        .map(|(_, pressed, text, time)| (*pressed, text.as_str(), *time))
        .collect();
    assert_eq!(
        presses,
        [
            (true, "é", ns(START)),
            (true, "é", ns(nth(START, 1))),
            (true, "é", ns(nth(START, 2))),
            (false, "", ns(up)),
        ]
    );
    assert!(keys.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(!to(&events, APP)
        .iter()
        .any(|event| matches!(event, Event::Text { .. } | Event::Key { .. })));
}

#[test]
fn modifiers_locks_and_keys_the_keymap_does_not_repeat_never_do() {
    let mut h = Harness::new();
    focused_window(&mut h);
    // A modifier never, whatever the caller says.
    for modifier in [SHIFT, CTRL, ALT, SUPER] {
        hold(&mut h, START, modifier, None);
        assert_eq!(h.scene.repeating_key(), None, "{modifier}");
        release(&mut h, modifier);
    }
    // A key the keymap does not repeat (Caps Lock), and any key through the paths that never do.
    h.scene.keyboard_repeating(CAPS_LOCK, true, None, false);
    assert_eq!(h.scene.repeating_key(), None);
    h.scene.keyboard(KEY_A, true, Some("a"));
    h.scene.key(KEY_B, true);
    assert_eq!(h.scene.repeating_key(), None);
    logged(&mut h);
    turn(&mut h, nth(START, 3));
    assert!(logged(&mut h).is_empty());
}

#[test]
fn a_dead_key_or_the_keymaps_own_press_ends_it() {
    // The program stops the repeat for a press the scene never sees as a key; that key, the
    // compose table's, never starts one.
    let mut h = Harness::new();
    focused_window(&mut h);
    hold(&mut h, START, KEY_A, Some("a"));
    h.scene.stop_key_repeat();
    h.scene.preedit("´", 2);
    turn(&mut h, nth(START, 2));
    assert_eq!(presses_to(&logged(&mut h), APP), [(KEY_A, ns(START))]);
    assert_eq!(h.scene.next_repeat(), None);
}

#[test]
fn another_key_pressed_repeats_instead_and_a_modifier_ends_it() {
    let mut h = Harness::new();
    focused_window(&mut h);
    hold(&mut h, START, KEY_A, Some("a"));
    // B pressed while A is held: B repeats, from its own press, and A's release leaves it.
    let b = START + Duration::from_millis(300);
    hold(&mut h, b, KEY_B, Some("b"));
    release(&mut h, KEY_A);
    assert_eq!(h.scene.repeating_key(), Some(KEY_B));
    turn(&mut h, nth(START, 1));
    turn(&mut h, nth(b, 1));
    let events = logged(&mut h);
    assert_eq!(
        presses_to(&events, APP),
        [(KEY_A, ns(START)), (KEY_B, ns(b)), (KEY_B, ns(nth(b, 1)))]
    );
    assert_eq!(texts_to(&events, APP), ["a", "b", "b"]);
    // A modifier pressed ends it: the repeat never becomes another chord.
    h.scene.keyboard_repeating(SHIFT, true, None, false);
    assert_eq!(h.scene.repeating_key(), None);
    // Shift held, then an arrow: Shift+arrow repeats; Shift released ends it.
    hold(&mut h, nth(b, 2), RIGHT, None);
    assert_eq!(h.scene.repeating_key(), Some(RIGHT));
    release(&mut h, SHIFT);
    assert_eq!(h.scene.repeating_key(), None);
}

#[test]
fn keyboard_focus_moving_ends_it() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    hold(&mut h, START, KEY_A, Some("a"));
    assert_eq!(h.scene.repeating_key(), Some(KEY_A));
    h.activate(other);
    assert_eq!(h.scene.focus(), Some(other));
    assert_eq!(h.scene.repeating_key(), None);
    turn(&mut h, nth(START, 2));
    let events = logged(&mut h);
    assert!(presses_to(&events, OTHER).is_empty());
    assert!(texts_to(&events, OTHER).is_empty());
    assert_eq!(presses_to(&events, APP).len(), 1);
    // A key going to no surface does not repeat.
    let mut h = Harness::new();
    hold(&mut h, START, KEY_A, Some("a"));
    assert_eq!(h.scene.repeating_key(), None);
}

#[test]
fn the_lock_ends_it_and_the_lock_screen_repeats_its_own_keys() {
    let mut h = Harness::new();
    focused_window(&mut h);
    hold(&mut h, START, KEY_A, Some("a"));
    h.scene.lock();
    assert_eq!(h.scene.repeating_key(), None);
    let surface = h.lock_surface(1);
    assert_eq!(h.scene.focus(), Some(surface));
    logged(&mut h);
    // Backspace held in the password: it repeats there.
    let later = START + Duration::from_secs(1);
    hold(&mut h, later, BACKSPACE, None);
    turn(&mut h, nth(later, 1));
    turn(&mut h, nth(later, 2));
    let events = logged(&mut h);
    assert_eq!(presses_to(&events, LOCKER).len(), 3);
    assert!(presses_to(&events, APP).is_empty());
    release(&mut h, BACKSPACE);
    // Super+` while locked acts on nothing and never repeats; its key reaches nobody.
    h.scene.keyboard(SUPER, true, None);
    hold(&mut h, nth(later, 3), GRAVE, None);
    assert_eq!(h.scene.repeating_key(), None);
    h.scene.keyboard(SUPER, false, None);
}

#[test]
fn the_input_method_receives_each_repeat_as_a_press() {
    let (mut h, _window) = served("");
    hold(&mut h, START, KEY_A, Some("a"));
    turn(&mut h, nth(START, 1));
    let events = logged(&mut h);
    let keys: Vec<(u32, bool, u64)> = to(&events, IME)
        .iter()
        .filter_map(|event| match event {
            Event::InputMethodKey {
                key,
                pressed,
                time_ns,
                ..
            } => Some((*key, *pressed, *time_ns)),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        [(KEY_A, true, ns(START)), (KEY_A, true, ns(nth(START, 1)))]
    );
    // Given back, each reaches the field as a press with its text.
    let serials: Vec<u64> = to(&events, IME)
        .iter()
        .filter_map(|event| match event {
            Event::InputMethodKey { serial, .. } => Some(*serial),
            _ => None,
        })
        .collect();
    for serial in serials {
        h.request(
            IME,
            Request::InputMethodKeyHandled {
                serial,
                used: false,
            },
        )
        .unwrap();
    }
    let events = logged(&mut h);
    assert_eq!(presses_to(&events, APP).len(), 2);
    assert_eq!(texts_to(&events, APP), ["a", "a"]);
}

/// The shell's dock, so the shell shows output 1 and the switcher opens there.
fn dock(h: &mut Harness) {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
}

fn shortcuts_to_shell(events: &[(ClientId, Event)]) -> Vec<SystemShortcut> {
    to(events, SHELL)
        .iter()
        .filter_map(|event| match event {
            Event::SystemShortcut { shortcut, .. } => Some(*shortcut),
            _ => None,
        })
        .collect()
}

#[test]
fn the_switchers_moves_and_super_grave_repeat_and_the_other_shortcuts_never() {
    let mut h = Harness::new();
    dock(&mut h);
    let first = h.toplevel(APP, 1, 400, 300);
    let second = h.toplevel(APP, 2, 400, 300);
    let third = h.toplevel(APP, 3, 400, 300);
    h.activate(first);
    logged(&mut h);
    // Super+` held goes through the application's windows, as each press does, and keeps
    // repeating though each moves keyboard focus.
    h.scene.keyboard(SUPER, true, None);
    hold(&mut h, START, GRAVE, None);
    let after_press = h.scene.active_window();
    turn(&mut h, nth(START, 1));
    let after_one = h.scene.active_window();
    turn(&mut h, nth(START, 2));
    let after_two = h.scene.active_window();
    assert_eq!(h.scene.repeating_key(), Some(GRAVE));
    let visited = [after_press, after_one, after_two];
    for window in [first, second, third] {
        assert!(visited.contains(&Some(window)), "{window:?} in {visited:?}");
    }
    release(&mut h, GRAVE);
    // Super+W never repeats: one close request.
    let w = START + Duration::from_secs(2);
    hold(&mut h, w, W, None);
    assert_eq!(h.scene.repeating_key(), None);
    turn(&mut h, nth(w, 3));
    let closes = logged(&mut h)
        .iter()
        .filter(|(_, event)| matches!(event, Event::CloseRequested { .. }))
        .count();
    assert_eq!(closes, 1);
    release(&mut h, W);
    // Super+Tab held: the switcher opens and moves on while Tab is held.
    let tab = START + Duration::from_secs(4);
    hold(&mut h, tab, TAB, None);
    turn(&mut h, nth(tab, 1));
    turn(&mut h, nth(tab, 2));
    assert_eq!(
        shortcuts_to_shell(&logged(&mut h)),
        [SystemShortcut::SwitcherNext; 3]
    );
    release(&mut h, TAB);
    // Q in the switcher acts once.
    let q = START + Duration::from_secs(6);
    hold(&mut h, q, Q, None);
    assert_eq!(h.scene.repeating_key(), None);
    release(&mut h, Q);
    // Tab held again, then Super released: the choice is made, and Tab, held from Super's
    // chord, never repeats into the application.
    let again = START + Duration::from_secs(8);
    hold(&mut h, again, TAB, None);
    h.scene.keyboard(SUPER, false, None);
    assert_eq!(h.scene.repeating_key(), None);
    turn(&mut h, nth(again, 3));
    let events = logged(&mut h);
    assert_eq!(
        shortcuts_to_shell(&events),
        [
            SystemShortcut::SwitcherQuit,
            SystemShortcut::SwitcherNext,
            SystemShortcut::SwitcherDone
        ]
    );
    assert!(to(&events, APP)
        .iter()
        .all(|event| !matches!(event, Event::Key { .. })));
}

#[test]
fn a_repeat_that_would_go_elsewhere_ends_instead() {
    // Tab repeating in the switcher; the shell goes, and with it the switcher: Tab with Super
    // held would now open a switcher, which its press never did. It ends, sending nothing.
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.keyboard(SUPER, true, None);
    hold(&mut h, START, TAB, None);
    assert_eq!(h.scene.repeating_key(), Some(TAB));
    h.scene.disconnect(SHELL);
    logged(&mut h);
    turn(&mut h, nth(START, 1));
    assert_eq!(h.scene.repeating_key(), None);
    assert!(logged(&mut h).is_empty());
}

#[test]
fn the_readers_moves_and_the_hardware_keys_steps_repeat_and_their_switches_never() {
    let mut h = Harness::new();
    h.scene.connect(SCREEN_READER, screen_reader()).unwrap();
    focused_window(&mut h);
    h.scene.keyboard(CTRL, true, None);
    h.scene.keyboard(ALT, true, None);
    hold(&mut h, START, RIGHT, None);
    turn(&mut h, nth(START, 1));
    assert_eq!(
        commands(&logged(&mut h)),
        [ReaderCommand::Next, ReaderCommand::Next]
    );
    release(&mut h, RIGHT);
    hold(&mut h, START + Duration::from_secs(1), SPACE, Some(" "));
    assert_eq!(h.scene.repeating_key(), None);
    release(&mut h, SPACE);
    h.scene.keyboard(ALT, false, None);
    h.scene.keyboard(CTRL, false, None);
    // Volume up steps again while held; mute switches once.
    h.scene.take_control_requests();
    let volume = START + Duration::from_secs(2);
    hold(&mut h, volume, KEY_VOLUMEUP, None);
    turn(&mut h, nth(volume, 1));
    assert_eq!(h.scene.take_control_requests().len(), 2);
    release(&mut h, KEY_VOLUMEUP);
    let mute = START + Duration::from_secs(4);
    hold(&mut h, mute, KEY_MUTE, None);
    assert_eq!(h.scene.repeating_key(), None);
    turn(&mut h, nth(mute, 2));
    assert_eq!(h.scene.take_control_requests().len(), 1);
    assert!(keys_to(&logged(&mut h), APP)
        .iter()
        .all(|(key, _)| matches!(*key, CTRL | ALT)));
}
