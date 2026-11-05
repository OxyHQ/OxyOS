//! The system's sounds (docs/adr/0044-system-sounds.mdx): what each [`SystemSound`] is called in
//! the freedesktop.org sound theme naming (`file-trash`, `trash-empty`, `message-new-instant`), where
//! its file is (OxyOS's theme, then freedesktop's, in the data directories), its samples decoded once
//! from Ogg Vorbis ([`decode`]), and who plays them: PipeWire ([`PipeWire`], the session's) or a
//! record of what would have been played ([`Recorder`], the development compositor's).
//!
//! Whether a sound may play (the person's interface sound effects, Do Not Disturb, a muted output)
//! is the session's to decide before it asks a [`Player`]; a player plays what it is asked.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use oxyos_protocol::SystemSound;

mod pipewire_player;
pub use pipewire_player::PipeWire;

#[cfg(test)]
mod tests;

/// Every sound the system has.
pub const SOUNDS: [SystemSound; 3] = [
    SystemSound::MoveToTrash,
    SystemSound::EmptyTrash,
    SystemSound::Notification,
];

/// Its name in the freedesktop.org sound naming specification.
pub fn name(sound: SystemSound) -> &'static str {
    match sound {
        SystemSound::MoveToTrash => "file-trash",
        SystemSound::EmptyTrash => "trash-empty",
        SystemSound::Notification => "message-new-instant",
    }
}

/// Whether it is an interface sound effect, which a Mac's *Play user interface sound effects*
/// turns off (the Trash's), rather than an alert (a notification's), which it does not.
pub fn is_interface(sound: SystemSound) -> bool {
    !matches!(sound, SystemSound::Notification)
}

/// The data directories, as the XDG Base Directory specification orders them: `XDG_DATA_HOME`
/// (else `~/.local/share`), then `XDG_DATA_DIRS` (else `/usr/local/share:/usr/share`).
pub fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&dirs).filter(|path| path.is_absolute()))
        .collect()
}

/// The file of `sound`: `sounds/<theme>/stereo/<name>.oga` (or `.ogg`), OxyOS's theme first, then
/// freedesktop's, in each data directory in order.
pub fn find(data_dirs: &[PathBuf], sound: SystemSound) -> Option<PathBuf> {
    let name = name(sound);
    data_dirs.iter().find_map(|dir| {
        ["oxyos", "freedesktop"].iter().find_map(|theme| {
            ["oga", "ogg"].iter().find_map(|extension| {
                let file = dir
                    .join("sounds")
                    .join(theme)
                    .join("stereo")
                    .join(format!("{name}.{extension}"));
                file.is_file().then_some(file)
            })
        })
    })
}

/// A sound's samples, decoded: interleaved signed 16-bit, at its own rate (PipeWire converts it
/// to the device's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Arc<[i16]>,
}

impl Pcm {
    /// How many frames (a sample of each channel) it has.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// How long it plays.
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.frames() as f64 / f64::from(self.rate.max(1)))
    }
}

/// The longest sound decoded: a system sound is a moment, never a song.
pub const MAX_SECONDS: u32 = 10;

/// Decodes an Ogg Vorbis file whole (at most [`MAX_SECONDS`], one or two channels).
pub fn decode(path: &Path) -> io::Result<Pcm> {
    let file = std::fs::File::open(path)?;
    let invalid = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
    let mut reader = lewton::inside_ogg::OggStreamReader::new(io::BufReader::new(file))
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    let rate = reader.ident_hdr.audio_sample_rate;
    let channels = u16::from(reader.ident_hdr.audio_channels);
    if !(1..=2).contains(&channels) || !(8_000..=192_000).contains(&rate) {
        return Err(invalid(format!(
            "{}: {channels} channels at {rate} Hz",
            path.display()
        )));
    }
    let most = rate as usize * MAX_SECONDS as usize * usize::from(channels);
    let mut samples: Vec<i16> = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?
    {
        samples.extend_from_slice(&packet);
        if samples.len() > most {
            return Err(invalid(format!(
                "{}: longer than {MAX_SECONDS} s",
                path.display()
            )));
        }
    }
    // The last page's granule position is how many frames the stream has: its last packet decodes
    // whole, the padding past the end with it (Vorbis' end trimming, specification A.2).
    if let Some(end) = reader.get_last_absgp() {
        let end = usize::try_from(end)
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::from(channels));
        samples.truncate(end);
    }
    Ok(Pcm {
        rate,
        channels,
        samples: samples.into(),
    })
}

/// Plays the system's sounds.
pub trait Player: Send {
    /// Plays `sound` now, beside any still playing, without waiting for it.
    fn play(&mut self, sound: SystemSound);
}

/// A record of the sounds asked for, played by nothing: the development compositor's.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    played: Arc<Mutex<Vec<SystemSound>>>,
}

impl Recorder {
    /// What was asked for since the last call, in order.
    pub fn take(&self) -> Vec<SystemSound> {
        std::mem::take(&mut *self.played.lock().expect("not poisoned"))
    }
}

impl Player for Recorder {
    fn play(&mut self, sound: SystemSound) {
        self.played.lock().expect("not poisoned").push(sound);
    }
}
