//! Screenshots as files (docs/adr/0024-system-shortcuts.mdx, decision 10). The compositor composes
//! a capture from the frame it shows (`Compositor::frame`, at each display's own pixels, without
//! the pointer, the lock screen, what the shell leaves out or anything in a secure region); the
//! session writes it, as the user it runs as, with a [`Writer`]: a PNG in the folder the person
//! chose (the Desktop by default, as xdg-user-dirs names it, made if it is missing), named as a
//! Mac names it in the session's language ("Screenshot 2026-10-31 at 10.04.05.png", "Captura de
//! pantalla 2026-10-31 a las 10.04.05.png"), each display of a capture of every display its own
//! file (the second "… (2).png"). A file is written whole under a hidden name and then linked to
//! its own, so nothing ever sees half of one, and never over another: a name taken goes on to the
//! next number. Encoding and writing happen on a thread of their own, never in the compositor's
//! turn; the shutter sounds as the capture is taken, when the system has its sound.

use oxyos_compositor_core::OutputId;
use oxyos_protocol::{CaptureDestination, CaptureKind};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

/// One display's (or one window's) capture, at its display's own pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The display it is of; a window's, the display it is on.
    pub output: OutputId,
    pub width: u32,
    pub height: u32,
    /// That display's scale, in 120ths: its pixels per logical pixel, written as the image's
    /// density.
    pub scale_120: u32,
    /// 0x00RRGGBB, rows top to bottom; with `alpha`, premultiplied by it (composed over black).
    pub pixels: Vec<u32>,
    /// Each pixel's opacity, for a window's capture, whose shadow is over nothing; none: opaque.
    pub alpha: Option<Vec<u8>>,
}

/// What one capture made: every display's, an area's or a window's, and where it is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub kind: CaptureKind,
    pub destination: CaptureDestination,
    pub captures: Vec<Capture>,
}

/// A screenshot written, or why it could not be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub screenshot: u32,
    pub output: OutputId,
    pub width: u32,
    pub height: u32,
    pub path: Result<PathBuf, String>,
}

/// The languages screenshots are named in, as a Mac names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    Spanish,
}

impl Language {
    /// The first of the session's languages (`es_ES`, `es`, `en`...) screenshots have a name in;
    /// English without one.
    pub fn of(languages: &[String]) -> Self {
        languages
            .iter()
            .find_map(|language| match language.split(['_', '-']).next() {
                Some("es") => Some(Language::Spanish),
                Some("en") => Some(Language::English),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The `n`th name of a screenshot taken at `at` (local time): the first has no number.
    pub fn file_name(self, at: jiff::civil::DateTime, n: usize) -> String {
        let date = format!("{:04}-{:02}-{:02}", at.year(), at.month(), at.day());
        let time = format!("{:02}.{:02}.{:02}", at.hour(), at.minute(), at.second());
        let base = match self {
            Language::English => format!("Screenshot {date} at {time}"),
            Language::Spanish => format!("Captura de pantalla {date} a las {time}"),
        };
        if n <= 1 {
            format!("{base}.png")
        } else {
            format!("{base} ({n}).png")
        }
    }
}

/// The shutter's sound, if the system has one: `screen-capture` of the freedesktop.org sound
/// theme naming, OxyOS's theme first, then `freedesktop`'s, in each of the data directories.
pub fn shutter_sound(data_dirs: &[PathBuf]) -> Option<PathBuf> {
    data_dirs.iter().find_map(|dir| {
        ["oxyos", "freedesktop"].iter().find_map(|theme| {
            ["oga", "ogg", "wav"].iter().find_map(|extension| {
                let sound = dir
                    .join("sounds")
                    .join(theme)
                    .join("stereo")
                    .join(format!("screen-capture.{extension}"));
                sound.is_file().then_some(sound)
            })
        })
    })
}

/// A capture's pixels as a PNG: RGB when opaque; RGBA, its colours no longer premultiplied, with
/// alpha. sRGB, and the display's density (72 dots an inch at scale 1, 144 at 2, as a Mac writes
/// a Retina display's).
pub fn encode_png(capture: &Capture) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, capture.width, capture.height);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_color(if capture.alpha.is_some() {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_compression(png::Compression::Fast);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        // Pixels a metre: 72 an inch at scale 1.
        let density = (u64::from(capture.scale_120) * 72 * 10_000 / (120 * 254)) as u32;
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: density,
            yppu: density,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        let channels = if capture.alpha.is_some() { 4 } else { 3 };
        let mut data = Vec::with_capacity(capture.pixels.len() * channels);
        for (index, &pixel) in capture.pixels.iter().enumerate() {
            let [b, g, r] = [pixel as u8, (pixel >> 8) as u8, (pixel >> 16) as u8];
            match &capture.alpha {
                None => data.extend([r, g, b]),
                Some(alpha) => {
                    let a = alpha[index];
                    let straight = |c: u8| match a {
                        0 => 0,
                        255 => c,
                        a => ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8,
                    };
                    data.extend([straight(r), straight(g), straight(b), a]);
                }
            }
        }
        writer.write_image_data(&data).map_err(io::Error::other)?;
    }
    Ok(out)
}

/// Writes `bytes` into `folder` (made if missing) under the first of `names` not taken, never
/// over a file: whole under a hidden name first, synced, then linked to its name (`link` fails on
/// a name taken) and the hidden one removed. The path written and the index of its name.
pub fn write_new(
    folder: &Path,
    names: impl IntoIterator<Item = String>,
    bytes: &[u8],
) -> io::Result<(PathBuf, usize)> {
    std::fs::create_dir_all(folder)?;
    let hidden = folder.join(format!(
        ".oxyos-screenshot-{}-{}.png",
        std::process::id(),
        HIDDEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&hidden)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&hidden);
        return Err(error);
    }
    let mut result = Err(io::Error::other("no name left"));
    for (index, name) in names.into_iter().enumerate() {
        let path = folder.join(&name);
        match std::fs::hard_link(&hidden, &path) {
            Ok(()) => {
                result = Ok((path, index));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                result = Err(error);
                break;
            }
        }
    }
    let _ = std::fs::remove_file(&hidden);
    // The name is kept across a crash: the folder's entry is synced too.
    if result.is_ok() {
        if let Ok(directory) = std::fs::File::open(folder) {
            let _ = directory.sync_all();
        }
    }
    result
}

/// Hidden names are unique within the process.
static HIDDEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The most names tried for one screenshot before giving up.
const MAX_NAMES: usize = 1000;

/// Where each destination is: the program's rule (the session's, xdg-user-dirs').
pub type Folders = Box<dyn Fn(CaptureDestination) -> PathBuf>;

/// One capture's files to write: the folder, when it was taken, its screenshots.
struct Job {
    folder: PathBuf,
    at: jiff::civil::DateTime,
    language: Language,
    sound: Option<PathBuf>,
    captures: Vec<(u32, Capture)>,
}

/// Writes screenshots on a thread of its own (the module's doc).
pub struct Writer {
    jobs: mpsc::Sender<Job>,
    finished: Arc<Mutex<VecDeque<Saved>>>,
    wake: UnixStream,
    folders: Folders,
    pub language: Language,
    /// The shutter's sound, played as each capture is taken ([`shutter_sound`]).
    pub sound: Option<PathBuf>,
    next: u32,
    paths: BTreeMap<u32, PathBuf>,
}

impl Writer {
    /// A writer saving to `folders`' folders, naming in `language`.
    pub fn spawn(folders: Folders, language: Language, sound: Option<PathBuf>) -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (jobs, received) = mpsc::channel::<Job>();
        let finished = Arc::new(Mutex::new(VecDeque::new()));
        let queue = finished.clone();
        std::thread::Builder::new()
            .name("oxyos-screenshots".into())
            .spawn(move || {
                for job in received {
                    let shutter = job.sound.as_ref().and_then(|sound| {
                        std::process::Command::new("pw-play")
                            .arg(sound)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .ok()
                    });
                    let mut first = 1;
                    for (screenshot, capture) in job.captures {
                        let names = (first..first + MAX_NAMES).map(|n| job.language.file_name(job.at, n));
                        let path = encode_png(&capture)
                            .and_then(|bytes| write_new(&job.folder, names, &bytes))
                            .map(|(path, index)| {
                                first += index + 1;
                                path
                            })
                            .map_err(|error| error.to_string());
                        queue.lock().expect("not poisoned").push_back(Saved {
                            screenshot,
                            output: capture.output,
                            width: capture.width,
                            height: capture.height,
                            path,
                        });
                        let _ = (&woken).write(&[1]);
                    }
                    if let Some(mut shutter) = shutter {
                        let _ = shutter.wait();
                    }
                }
            })?;
        Ok(Self {
            jobs,
            finished,
            wake,
            folders,
            language,
            sound,
            next: 1,
            paths: BTreeMap::new(),
        })
    }

    /// Saves a capture's screenshots, taken now (local time); their numbers, in order.
    pub fn save(&mut self, shot: Shot) -> Vec<u32> {
        let now = jiff::Zoned::now().datetime();
        self.save_at(shot, now)
    }

    /// The same, taken at `at`.
    pub fn save_at(&mut self, shot: Shot, at: jiff::civil::DateTime) -> Vec<u32> {
        let folder = (self.folders)(shot.destination);
        let captures: Vec<(u32, Capture)> = shot
            .captures
            .into_iter()
            .map(|capture| {
                let screenshot = self.next;
                self.next = self.next.wrapping_add(1).max(1);
                (screenshot, capture)
            })
            .collect();
        let numbers = captures.iter().map(|(screenshot, _)| *screenshot).collect();
        if !captures.is_empty() {
            let _ = self.jobs.send(Job {
                folder,
                at,
                language: self.language,
                sound: self.sound.clone(),
                captures,
            });
        }
        numbers
    }

    /// Readable while screenshots written wait to be taken ([`Writer::finished`]).
    pub fn wake_fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    /// The screenshots written (or not) since the last call.
    pub fn finished(&mut self) -> Vec<Saved> {
        let mut buffer = [0u8; 64];
        while matches!((&self.wake).read(&mut buffer), Ok(read) if read > 0) {}
        let saved: Vec<Saved> = self
            .finished
            .lock()
            .expect("not poisoned")
            .drain(..)
            .collect();
        for saved in &saved {
            if let Ok(path) = &saved.path {
                self.paths.insert(saved.screenshot, path.clone());
            }
        }
        saved
    }

    /// Where a screenshot it wrote is.
    pub fn path(&self, screenshot: u32) -> Option<&Path> {
        self.paths.get(&screenshot).map(PathBuf::as_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> jiff::civil::DateTime {
        jiff::civil::date(2026, 10, 31).at(9, 4, 5, 0)
    }

    #[test]
    fn names_are_a_macs_in_the_sessions_language() {
        assert_eq!(
            Language::English.file_name(at(), 1),
            "Screenshot 2026-10-31 at 09.04.05.png"
        );
        assert_eq!(
            Language::English.file_name(at(), 2),
            "Screenshot 2026-10-31 at 09.04.05 (2).png"
        );
        assert_eq!(
            Language::Spanish.file_name(at(), 1),
            "Captura de pantalla 2026-10-31 a las 09.04.05.png"
        );
        assert_eq!(
            Language::of(&["es_ES".into(), "es".into()]),
            Language::Spanish
        );
        assert_eq!(Language::of(&["fr_FR".into(), "en".into()]), Language::English);
        assert_eq!(Language::of(&[]), Language::English);
        assert_eq!(Language::of(&["de_DE".into()]), Language::English);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oxyos-screenshots-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_file_is_written_whole_under_a_name_not_taken() {
        let dir = scratch("names");
        let names = || (1..4).map(|n| Language::English.file_name(at(), n));
        let (first, index) = write_new(&dir.join("Desktop"), names(), b"one").unwrap();
        assert_eq!(index, 0);
        assert_eq!(first, dir.join("Desktop/Screenshot 2026-10-31 at 09.04.05.png"));
        // The same second again: the next number, the first left as it was.
        let (second, index) = write_new(&dir.join("Desktop"), names(), b"two").unwrap();
        assert_eq!(index, 1);
        assert!(second.ends_with("Screenshot 2026-10-31 at 09.04.05 (2).png"));
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        // Nothing hidden is left behind.
        let entries: Vec<_> = std::fs::read_dir(dir.join("Desktop"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_png_holds_the_pixels_and_a_windows_alpha() {
        let opaque = Capture {
            output: 1,
            width: 2,
            height: 1,
            scale_120: 150,
            pixels: vec![0x0011_2233, 0x00ff_8000],
            alpha: None,
        };
        let decoded = |bytes: Vec<u8>| {
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let mut reader = decoder.read_info().unwrap();
            let mut data = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut data).unwrap();
            let dims = reader.info().pixel_dims.unwrap();
            data.truncate(info.buffer_size());
            (info.color_type, data, dims.xppu)
        };
        let (color, data, density) = decoded(encode_png(&opaque).unwrap());
        assert_eq!(color, png::ColorType::Rgb);
        assert_eq!(data, [0x11, 0x22, 0x33, 0xff, 0x80, 0x00]);
        // 90 dots an inch at 1.25.
        assert_eq!(density, 3543);
        let window = Capture {
            alpha: Some(vec![255, 128]),
            pixels: vec![0x0011_2233, 0x0040_2000],
            ..opaque
        };
        let (color, data, _) = decoded(encode_png(&window).unwrap());
        assert_eq!(color, png::ColorType::Rgba);
        assert_eq!(data, [0x11, 0x22, 0x33, 255, 0x80, 0x40, 0x00, 128]);
    }

    #[test]
    fn a_capture_of_every_display_is_one_file_each_numbered_in_order() {
        let dir = scratch("writer");
        let desktop = dir.join("Escritorio");
        let documents = dir.join("Documentos");
        let folders: Folders = {
            let (desktop, documents) = (desktop.clone(), documents.clone());
            Box::new(move |destination| match destination {
                CaptureDestination::Desktop => desktop.clone(),
                CaptureDestination::Documents => documents.clone(),
            })
        };
        let mut writer = Writer::spawn(folders, Language::Spanish, None).unwrap();
        let capture = |output| Capture {
            output,
            width: 4,
            height: 2,
            scale_120: 120,
            pixels: vec![0x0010_2030; 8],
            alpha: None,
        };
        let numbers = writer.save_at(
            Shot {
                kind: CaptureKind::Screens,
                destination: CaptureDestination::Desktop,
                captures: vec![capture(1), capture(2)],
            },
            at(),
        );
        assert_eq!(numbers, [1, 2]);
        let mut saved = Vec::new();
        for _ in 0..500 {
            saved.extend(writer.finished());
            if saved.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let paths: Vec<PathBuf> = saved.iter().map(|s| s.path.clone().unwrap()).collect();
        assert_eq!(
            paths,
            [
                desktop.join("Captura de pantalla 2026-10-31 a las 09.04.05.png"),
                desktop.join("Captura de pantalla 2026-10-31 a las 09.04.05 (2).png"),
            ]
        );
        assert_eq!((saved[1].output, saved[1].width, saved[1].height), (2, 4, 2));
        assert_eq!(writer.path(2), Some(paths[1].as_path()));
        assert!(!documents.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_shutter_is_the_sound_themes_screen_capture() {
        let dir = scratch("sound");
        assert_eq!(shutter_sound(std::slice::from_ref(&dir)), None);
        let theme = dir.join("sounds/freedesktop/stereo");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("screen-capture.oga"), b"").unwrap();
        assert_eq!(
            shutter_sound(&[dir.join("missing"), dir.clone()]),
            Some(theme.join("screen-capture.oga"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
