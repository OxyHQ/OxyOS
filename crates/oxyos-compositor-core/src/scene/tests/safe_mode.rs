//! Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10): the shell and the login screen
//! are told the session started in it, once, as they connect; nothing else is.

use super::*;
use crate::scene::SAFE_MODE_VERSION;

fn told(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::SafeMode {}))
        .count()
}

#[test]
fn the_shell_and_the_lock_screen_are_told_safe_mode_once_and_nothing_else_is() {
    let mut h = Harness::new();
    // A shell of an older version is never told it.
    h.scene
        .connect(
            20,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.scene.set_version(20, SAFE_MODE_VERSION - 1);
    // A normal start tells nothing.
    assert!(!h.scene.safe_mode());
    assert_eq!(told(&to(&logged(&mut h), SHELL)), 0);
    h.scene.set_safe_mode();
    assert!(h.scene.safe_mode());
    let events = logged(&mut h);
    assert_eq!(told(&to(&events, SHELL)), 1);
    assert_eq!(told(&to(&events, LOCKER)), 1);
    assert_eq!(told(&to(&events, 20)), 0);
    assert_eq!(told(&to(&events, APP)), 0);
    assert_eq!(told(&to(&events, OTHER)), 0);
    // Once: saying it again tells nothing more.
    h.scene.set_safe_mode();
    assert_eq!(told(&to(&logged(&mut h), SHELL)), 0);
    // A shell and a lock screen that connect later are told it; an application is not.
    for (client, grants) in [
        (
            21,
            Grants {
                shell: true,
                ..Grants::default()
            },
        ),
        (
            22,
            Grants {
                lock: true,
                ..Grants::default()
            },
        ),
        (23, Grants::default()),
    ] {
        h.scene.connect(client, grants).unwrap();
    }
    let events = logged(&mut h);
    assert_eq!(told(&to(&events, 21)), 1);
    assert_eq!(told(&to(&events, 22)), 1);
    assert_eq!(told(&to(&events, 23)), 0);
}
