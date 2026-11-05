//! The system's sounds for the program running the compositor (docs/adr/0044-system-sounds.mdx):
//! the sounds the scene was asked for (a client's interface sound citing its input, the Trash's,
//! a notification's), each played by the session's player unless the person turned interface sound
//! effects off (`ui_sounds`, a Trash's sound; a notification's is an alert, which it does not
//! silence, as on a Mac) or the output is muted. What may play is decided here; the player plays.

use oxyos_compositor_core::Scene;
use oxyos_sound::{is_interface, Player};

pub struct Sounds {
    player: Box<dyn Player>,
    /// *Play user interface sound effects* (the `ui_sounds` preference): on by default.
    pub interface: bool,
}

impl Sounds {
    pub fn new(player: Box<dyn Player>) -> Self {
        Self {
            player,
            interface: true,
        }
    }

    /// Plays what the scene was asked for since the last turn; nothing while `silent` (the output
    /// muted).
    pub fn serve(&mut self, scene: &mut Scene, silent: bool) {
        for sound in scene.take_system_sounds() {
            if silent || (is_interface(sound) && !self.interface) {
                continue;
            }
            self.player.play(sound);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_sound::{Recorder, SystemSound};

    #[test]
    fn a_sound_plays_unless_interface_sounds_are_off_or_the_output_is_muted() {
        let recorder = Recorder::default();
        let mut sounds = Sounds::new(Box::new(recorder.clone()));
        let mut scene = Scene::new();
        let ask = |scene: &mut Scene| {
            scene.request_system_sound(SystemSound::MoveToTrash);
            scene.request_system_sound(SystemSound::Notification);
        };
        ask(&mut scene);
        sounds.serve(&mut scene, false);
        assert_eq!(
            recorder.take(),
            [SystemSound::MoveToTrash, SystemSound::Notification]
        );
        // Interface sound effects off: the Trash's is silent, a notification's is not.
        sounds.interface = false;
        ask(&mut scene);
        sounds.serve(&mut scene, false);
        assert_eq!(recorder.take(), [SystemSound::Notification]);
        // The output muted: nothing, and nothing kept for later.
        sounds.interface = true;
        ask(&mut scene);
        sounds.serve(&mut scene, true);
        sounds.serve(&mut scene, false);
        assert!(recorder.take().is_empty());
    }
}
