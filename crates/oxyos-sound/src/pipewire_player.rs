//! Playing the system's sounds through PipeWire, the session's one audio stack: a thread of its
//! own runs a PipeWire main loop, every sound decoded once as it starts and kept; each sound asked
//! for is a playback stream of its own (an event's: `media.role` Notification, a short quantum),
//! which PipeWire mixes with whatever plays and routes to the default output at its volume, and
//! which is disconnected once it has drained, so nothing keeps the device awake between sounds.
//! The connection is made at the first sound and made again after PipeWire went away.

use crate::{decode, find, name, Pcm, Player, Sound, SOUNDS};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use std::cell::{Cell, RefCell};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Sounds playing at once; one more stops the oldest.
const MAX_PLAYING: usize = 4;
/// The quantum asked for: 256 frames at 48 kHz, about 5 ms, so a sound starts at once.
const LATENCY: &str = "256/48000";

enum Command {
    Play(Sound),
    Quit,
}

/// The session's player (the module's doc).
pub struct PipeWire {
    sender: pw::channel::Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PipeWire {
    /// Starts the player, its sounds found in `data_dirs` ([`crate::find`]) and decoded once there.
    pub fn spawn(data_dirs: Vec<PathBuf>) -> io::Result<Self> {
        let (sender, receiver) = pw::channel::channel();
        let thread = std::thread::Builder::new()
            .name("oxyos-sounds".into())
            .spawn(move || {
                if let Err(error) = run(receiver, &data_dirs) {
                    eprintln!("[sounds] PipeWire: {error}; no sound plays");
                }
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }
}

impl Player for PipeWire {
    fn play(&mut self, sound: Sound) {
        let _ = self.sender.send(Command::Play(sound));
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The connection to PipeWire, and whether it broke (PipeWire went away).
struct Connection {
    core: pw::core::CoreRc,
    _listener: pw::core::Listener,
    broken: Rc<Cell<bool>>,
}

/// A sound playing: its stream, and whether it is done (drained, or failed).
struct Playing {
    _listener: pw::stream::StreamListener<Position>,
    stream: pw::stream::StreamRc,
    done: Rc<Cell<bool>>,
}

/// Where a stream is in its sound, and whether it was asked to drain.
#[derive(Default)]
struct Position {
    frame: usize,
    draining: bool,
}

#[derive(Default)]
struct State {
    connection: Option<Connection>,
    playing: Vec<Playing>,
}

fn run(receiver: pw::channel::Receiver<Command>, data_dirs: &[PathBuf]) -> Result<(), pw::Error> {
    pw::init();
    // Each sound decoded once, before any is asked for.
    let mut decoded: Vec<(Sound, Arc<Pcm>)> = Vec::new();
    for sound in SOUNDS {
        match find(data_dirs, sound) {
            Some(path) => match decode(&path) {
                Ok(pcm) => decoded.push((sound, Arc::new(pcm))),
                Err(error) => eprintln!("[sounds] {}: {error}", name(sound)),
            },
            None => eprintln!("[sounds] {}: no such sound in the theme", name(sound)),
        }
    }
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let state = Rc::new(RefCell::new(State::default()));
    let _attached = receiver.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        let state = state.clone();
        move |command| match command {
            Command::Quit => mainloop.quit(),
            Command::Play(sound) => {
                let Some((_, pcm)) = decoded.iter().find(|(s, _)| *s == sound) else {
                    return;
                };
                let mut state = state.borrow_mut();
                if let Err(error) = play(&context, &mut state, sound, pcm.clone()) {
                    eprintln!("[sounds] {} not played: {error}", name(sound));
                    // Connected anew for the next one.
                    state.connection = None;
                }
            }
        }
    });
    mainloop.run();
    // The streams before their connection.
    state.borrow_mut().playing.clear();
    Ok(())
}

fn connect(context: &pw::context::ContextRc) -> Result<Connection, pw::Error> {
    let core = context.connect_rc(Some(properties! {
        *pw::keys::APP_NAME => "OxyOS",
    }))?;
    let broken = Rc::new(Cell::new(false));
    let listener = core
        .add_listener_local()
        .error({
            let broken = broken.clone();
            move |id, _seq, _res, message| {
                if id == pw::core::PW_ID_CORE {
                    eprintln!("[sounds] PipeWire: {message}");
                    broken.set(true);
                }
            }
        })
        .register();
    Ok(Connection {
        core,
        _listener: listener,
        broken,
    })
}

fn play(
    context: &pw::context::ContextRc,
    state: &mut State,
    sound: Sound,
    pcm: Arc<Pcm>,
) -> Result<(), pw::Error> {
    state.playing.retain(|playing| !playing.done.get());
    while state.playing.len() >= MAX_PLAYING {
        let oldest = state.playing.remove(0);
        let _ = oldest.stream.disconnect();
    }
    if state
        .connection
        .as_ref()
        .is_none_or(|connection| connection.broken.get())
    {
        state.playing.clear();
        state.connection = None;
        state.connection = Some(connect(context)?);
    }
    let core = state.connection.as_ref().expect("connected").core.clone();
    let stream = pw::stream::StreamRc::new(
        core,
        "oxyos-system-sound",
        properties! {
            *pw::keys::NODE_NAME => "oxyos-system-sound",
            *pw::keys::NODE_DESCRIPTION => "OxyOS system sounds",
            *pw::keys::APP_NAME => "OxyOS",
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Notification",
            *pw::keys::MEDIA_NAME => name(sound),
            *pw::keys::NODE_LATENCY => LATENCY,
            // The sound theme's event, as libcanberra names what it plays.
            "event.id" => name(sound),
        },
    )?;
    let done = Rc::new(Cell::new(false));
    let channels = usize::from(pcm.channels);
    let listener = stream
        .add_local_listener_with_user_data(Position::default())
        .process({
            let pcm = pcm.clone();
            move |stream, position| fill(stream, &pcm, channels, position)
        })
        .drained({
            let done = done.clone();
            move |stream, _| {
                done.set(true);
                let _ = stream.disconnect();
            }
        })
        .state_changed({
            let done = done.clone();
            move |_, _, _, now| {
                if let pw::stream::StreamState::Error(error) = now {
                    eprintln!("[sounds] {}: {error}", name(sound));
                    done.set(true);
                }
            }
        })
        .register()?;
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::S16LE);
    info.set_rate(pcm.rate);
    info.set_channels(u32::from(pcm.channels));
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    if pcm.channels == 1 {
        position[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    } else {
        position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
        position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    }
    info.set_position(position);
    let format: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .map_err(|_| pw::Error::CreationFailed)?
    .0
    .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&format).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    state.playing.push(Playing {
        _listener: listener,
        stream,
        done,
    });
    Ok(())
}

/// One buffer of the sound from where the stream is, as much as PipeWire asked for; past its
/// end, the stream is drained.
fn fill(stream: &pw::stream::Stream, pcm: &Pcm, channels: usize, position: &mut Position) {
    if position.draining {
        return;
    }
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let requested = usize::try_from(buffer.requested()).unwrap_or(usize::MAX);
    let stride = channels * std::mem::size_of::<i16>();
    let data = &mut buffer.datas_mut()[0];
    let frames = match data.data() {
        Some(bytes) => {
            let room = bytes.len() / stride;
            let wanted = if requested > 0 {
                requested.min(room)
            } else {
                room
            };
            let frames = wanted.min(pcm.frames() - position.frame);
            let from = position.frame * channels;
            for (out, sample) in bytes
                .chunks_exact_mut(2)
                .zip(&pcm.samples[from..from + frames * channels])
            {
                out.copy_from_slice(&sample.to_le_bytes());
            }
            position.frame += frames;
            frames
        }
        None => 0,
    };
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = stride as i32;
    *chunk.size_mut() = (frames * stride) as u32;
    drop(buffer);
    if position.frame >= pcm.frames() {
        position.draining = true;
        let _ = stream.flush(true);
    }
}
