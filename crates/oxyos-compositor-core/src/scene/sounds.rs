//! System sounds (docs/adr/0044-system-sounds.mdx): the sounds the session plays for what the
//! person did, asked of the scene by its clients (`PlaySystemSound`, citing an input they received
//! lately) and by the program running the compositor (the Trash's, when it puts files in it or
//! empties it), and the notification sound for a notification posted with one (notifications.rs).
//! The scene only keeps what was asked, in order: the program takes it (`take_system_sounds`) and
//! plays each as the person's settings and the output allow.

use super::*;
use oxyos_protocol::SystemSound;

/// Sounds asked for and not yet taken by the program; more are not kept.
pub const MAX_PENDING_SOUNDS: usize = 8;
/// A notification sound within this long of the one before is not asked for again: a burst of
/// notifications sounds once, as a Mac's does.
pub const NOTIFICATION_SOUND_COALESCE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Default)]
pub(super) struct Sounds {
    pending: Vec<SystemSound>,
    /// When the notification sound was last asked for, on the scene's clock.
    notification_at: Option<std::time::Duration>,
}

impl Scene {
    /// A client asks for an interface sound for what the person just did: only with an input it
    /// received lately, never the notification sound (which notifications alone ask for).
    pub(super) fn play_system_sound_request(
        &mut self,
        client: ClientId,
        input: u64,
        sound: SystemSound,
    ) -> Result<(), ErrorCode> {
        if sound == SystemSound::Notification {
            return Err(ErrorCode::InvalidMessage);
        }
        if input == 0 || !self.recent_input(client, input) {
            return Ok(());
        }
        self.request_system_sound(sound);
        Ok(())
    }

    /// The program asks for a system sound itself (the Trash's, docs/adr/0044-system-sounds.mdx).
    /// Beyond [`MAX_PENDING_SOUNDS`] not taken, it is dropped: sounds are of now.
    pub fn request_system_sound(&mut self, sound: SystemSound) {
        if self.sounds.pending.len() < MAX_PENDING_SOUNDS {
            self.sounds.pending.push(sound);
        }
    }

    /// The notification sound, unless one was asked for within [`NOTIFICATION_SOUND_COALESCE`].
    pub(super) fn request_notification_sound(&mut self) {
        let now = self.clock;
        if self
            .sounds
            .notification_at
            .is_some_and(|at| now.saturating_sub(at) < NOTIFICATION_SOUND_COALESCE)
        {
            return;
        }
        self.sounds.notification_at = Some(now);
        self.request_system_sound(SystemSound::Notification);
    }

    /// The sounds asked for since the last call, in order: the program plays each as the person's
    /// settings (interface sound effects) and the output (muted) allow.
    pub fn take_system_sounds(&mut self) -> Vec<SystemSound> {
        std::mem::take(&mut self.sounds.pending)
    }
}
