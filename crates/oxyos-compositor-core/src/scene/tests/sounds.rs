//! System sounds (docs/adr/0044-system-sounds.mdx): an interface sound asked for by any client
//! with an input it received lately, and by the program; kept in order for the program to play.

use super::*;
use oxyos_protocol::SystemSound;

#[test]
fn an_interface_sound_is_asked_for_only_with_the_clients_own_input() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let at = h.rect(window);
    let press = h.press(at.x + 10, at.y + 10);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    // Citing no input, or another client's: nothing.
    for (client, input) in [(APP, 0), (OTHER, press), (APP, press + 1000)] {
        h.request(
            client,
            Request::PlaySystemSound {
                input,
                sound: SystemSound::MoveToTrash,
            },
        )
        .unwrap();
    }
    assert!(h.scene.take_system_sounds().is_empty());
    // Its own press: asked for, once, in order with the program's own.
    h.request(
        APP,
        Request::PlaySystemSound {
            input: press,
            sound: SystemSound::MoveToTrash,
        },
    )
    .unwrap();
    h.scene.request_system_sound(SystemSound::EmptyTrash);
    assert_eq!(
        h.scene.take_system_sounds(),
        [SystemSound::MoveToTrash, SystemSound::EmptyTrash]
    );
    assert!(h.scene.take_system_sounds().is_empty());
    // Sounds are of now: beyond what is waiting, more are dropped.
    for _ in 0..MAX_PENDING_SOUNDS + 3 {
        h.scene.request_system_sound(SystemSound::MoveToTrash);
    }
    assert_eq!(h.scene.take_system_sounds().len(), MAX_PENDING_SOUNDS);
}
