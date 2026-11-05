//! The system's sounds for the program running the compositor (docs/adr/0044-system-sounds.mdx):
//! the sounds the scene was asked for (a client's interface sound citing its input, the Trash's,
//! a notification's) and the compositor's own (a screenshot's shutter), each played by the
//! session's player unless the person turned interface sound effects off (`ui_sounds`: the Trash's
//! and the shutter; a notification's is an alert, which it does not silence, as on a Mac) or the
//! output is muted. What may play is decided here; the player plays.

use oxyos_compositor_core::Scene;
use oxyos_sound::{is_interface, Player, Sound};

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
            self.play(sound.into(), silent);
        }
    }

    /// Plays `sound` now, unless `silent` or it is an interface sound effect and they are off.
    pub fn play(&mut self, sound: Sound, silent: bool) {
        if silent || (is_interface(sound) && !self.interface) {
            return;
        }
        self.player.play(sound);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_sound::{Recorder, SystemSound};

    #[test]
    fn a_sound_plays_unless_it_is_an_interface_sound_and_they_are_off_or_the_output_is_muted() {
        let recorder = Recorder::default();
        let mut sounds = Sounds::new(Box::new(recorder.clone()));
        let mut scene = Scene::new();
        let ask = |scene: &mut Scene| {
            scene.request_system_sound(SystemSound::MoveToTrash);
            scene.request_system_sound(SystemSound::Notification);
        };
        ask(&mut scene);
        sounds.serve(&mut scene, false);
        sounds.play(Sound::ScreenCapture, false);
        assert_eq!(
            recorder.take(),
            [
                Sound::MoveToTrash,
                Sound::Notification,
                Sound::ScreenCapture
            ]
        );
        // Interface sound effects off: the Trash's and the shutter are silent, a notification's
        // is not.
        sounds.interface = false;
        ask(&mut scene);
        sounds.serve(&mut scene, false);
        sounds.play(Sound::ScreenCapture, false);
        assert_eq!(recorder.take(), [Sound::Notification]);
        // The output muted: nothing, and nothing kept for later.
        sounds.interface = true;
        ask(&mut scene);
        sounds.serve(&mut scene, true);
        sounds.play(Sound::ScreenCapture, true);
        sounds.serve(&mut scene, false);
        assert!(recorder.take().is_empty());
    }
}
