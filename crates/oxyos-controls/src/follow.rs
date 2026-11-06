//! What changes the battery, the network, Bluetooth and the levels, followed as it changes instead
//! of looked at on a clock (docs/adr/0025-hardware-keys-and-control-center.mdx): a session doing
//! nothing wakes for nothing (docs/performance.mdx, *Idle*). Each follower waits on its source and
//! on `stop` (readable once its caller's order to end is given) at once, never on a timer, calls
//! `changed` with what changed until it returns false, and returns when its source goes (the
//! caller follows it again once it is back):
//!
//! - [`bus`]: the system bus's signals from UPower (the battery), BlueZ (Bluetooth) and, where
//!   nothing else follows it, NetworkManager, and their names' owners (a service started or gone);
//! - [`backlights`]: the display's backlight and a keyboard's the firmware changes itself, by the
//!   kernel's notification of their sysfs attributes (`actual_brightness`, which the backlight
//!   class notifies whenever the brightness is set, by a key or by a program, and
//!   `brightness_hw_changed`);
//! - [`audio`]: PipeWire's default output and input (WirePlumber's `default` metadata), and the
//!   volumes of its sinks and sources and the routes of its devices (their `Props` and `Route`
//!   parameters).

use crate::backlight;
use crate::Control;
use futures_lite::StreamExt;
use pipewire as pw;
use rustix::event::{poll, PollFd, PollFlags};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read, Seek};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// What changed: the status (the battery, the network, Bluetooth), or a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Status,
    Level(Control),
}

const UPOWER: &str = "org.freedesktop.UPower";
const BLUEZ: &str = "org.bluez";
const NM: &str = "org.freedesktop.NetworkManager";
/// Signals that come together are one change, told this long after the first.
const SETTLE: Duration = Duration::from_millis(100);

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// Follows the system bus (the module's doc), with NetworkManager's signals where `network`.
/// Without the bus, returns the error; returns when the bus closes or `stop` is readable.
pub fn bus(
    network: bool,
    stop: BorrowedFd<'_>,
    mut changed: impl FnMut(Change) -> bool,
) -> io::Result<()> {
    let stop = async_io::Async::new(stop.try_clone_to_owned()?)?;
    // Its executor waits on a thread of its own without waking for nothing, and ends with it
    // (`oxyos_seat::system_bus`); its signals are waited for with futures-lite's block_on.
    let bus = oxyos_seat::system_bus()?;
    let inner = bus.inner();
    let mut services = vec![UPOWER, BLUEZ];
    if network {
        services.push(NM);
    }
    let mut streams = Vec::new();
    for service in &services {
        let signals = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(*service)
            .map_err(failed)?
            .build();
        let owner = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .map_err(failed)?
            .member("NameOwnerChanged")
            .map_err(failed)?
            .arg(0, *service)
            .map_err(failed)?
            .build();
        for rule in [signals, owner] {
            streams.push(
                futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
                    rule,
                    inner,
                    Some(64),
                ))
                .map_err(failed)?,
            );
        }
    }
    loop {
        let signal = futures_lite::future::block_on(futures_lite::future::or(
            async { next(&mut streams).await.map(|()| true) },
            async {
                let _ = stop.readable().await;
                Some(false)
            },
        ));
        if signal != Some(true) {
            // Stopped, or the bus closed.
            return Ok(());
        }
        // What follows at once is the same change.
        let until = Instant::now() + SETTLE;
        let ended = futures_lite::future::block_on(async {
            loop {
                let more = futures_lite::future::or(
                    async { Some(next(&mut streams).await.is_some()) },
                    async {
                        async_io::Timer::at(until).await;
                        None
                    },
                )
                .await;
                match more {
                    Some(true) => {}
                    Some(false) => return true,
                    None => return false,
                }
            }
        });
        if !changed(Change::Status) || ended {
            return Ok(());
        }
    }
}

/// The next signal of any of `streams`; `None` once one ends (the bus closed).
async fn next(streams: &mut [zbus::MessageStream]) -> Option<()> {
    futures_lite::future::poll_fn(|cx| {
        for stream in streams.iter_mut() {
            if let std::task::Poll::Ready(signal) = stream.poll_next(cx) {
                return std::task::Poll::Ready(signal.map(|_| ()));
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// An attribute the kernel notifies, opened, and the level it is.
struct Notified {
    file: std::fs::File,
    control: Control,
}

impl Notified {
    fn open(path: PathBuf, control: Control) -> Option<Self> {
        let mut notified = Self {
            file: std::fs::File::open(path).ok()?,
            control,
        };
        // Read once: a notification is what comes after the last read.
        notified.read();
        Some(notified)
    }

    fn read(&mut self) {
        let mut text = String::new();
        let _ = self.file.rewind();
        let _ = self.file.read_to_string(&mut text);
    }
}

/// Follows the backlights under `sysfs` (`/sys`; the module's doc) until `stop` is readable or
/// `changed` returns false. A machine with none waits for `stop` alone.
pub fn backlights(
    sysfs: &Path,
    stop: BorrowedFd<'_>,
    mut changed: impl FnMut(Change) -> bool,
) -> io::Result<()> {
    let mut watched: Vec<Notified> = Vec::new();
    if let Some(display) = backlight::display(sysfs) {
        watched.extend(Notified::open(
            display.attribute("actual_brightness"),
            Control::DisplayBrightness,
        ));
    }
    if let Some(keyboard) =
        backlight::keyboard(sysfs).filter(backlight::Device::changed_by_firmware)
    {
        watched.extend(Notified::open(
            keyboard.attribute("brightness_hw_changed"),
            Control::KeyboardBrightness,
        ));
    }
    loop {
        let mut fds = vec![PollFd::from_borrowed_fd(stop, PollFlags::IN)];
        // A sysfs attribute's notification is POLLPRI and POLLERR; a plain file never has one.
        fds.extend(
            watched
                .iter()
                .map(|notified| PollFd::from_borrowed_fd(notified.file.as_fd(), PollFlags::PRI)),
        );
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        if !fds[0].revents().is_empty() {
            return Ok(());
        }
        let notified: Vec<usize> = fds[1..]
            .iter()
            .enumerate()
            .filter(|(_, fd)| !fd.revents().is_empty())
            .map(|(index, _)| index)
            .collect();
        for index in notified {
            watched[index].read();
            if !changed(Change::Level(watched[index].control)) {
                return Ok(());
            }
        }
    }
}

/// PipeWire's socket: what [`audio`] waits to be made when PipeWire is not there
/// (`PIPEWIRE_REMOTE` in `PIPEWIRE_RUNTIME_DIR`, else `XDG_RUNTIME_DIR`; `pipewire-0` by default).
pub fn pipewire_socket() -> Option<PathBuf> {
    let remote = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    if Path::new(&remote).is_absolute() {
        return Some(PathBuf::from(remote));
    }
    let dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))?;
    Some(PathBuf::from(dir).join(remote))
}

/// Follows PipeWire (the module's doc) on this thread's own PipeWire loop, until `stop` is
/// readable or `changed` returns false. Without PipeWire, or once it goes, returns the error.
pub fn audio(
    stop: BorrowedFd<'_>,
    changed: impl FnMut(Change) -> bool + 'static,
) -> io::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(failed)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(failed)?;
    let core = context.connect_rc(None).map_err(failed)?;
    let registry = core.get_registry_rc().map_err(failed)?;
    let gone: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let changed = Rc::new(RefCell::new(changed));
    let tell = {
        let (changed, mainloop) = (changed.clone(), mainloop.clone());
        Rc::new(move |change: Change| {
            if !(changed.borrow_mut())(change) {
                mainloop.quit();
            }
        })
    };
    let _core_listener = core
        .add_listener_local()
        .error({
            let (gone, mainloop) = (gone.clone(), mainloop.clone());
            move |id, _seq, _res, message| {
                if id == pw::core::PW_ID_CORE {
                    *gone.borrow_mut() = Some(message.to_string());
                    mainloop.quit();
                }
            }
        })
        .register();
    // The order to stop, read by the loop with PipeWire's own descriptors.
    let stopping: OwnedFd = stop.try_clone_to_owned()?;
    let _stop = mainloop
        .loop_()
        .add_io(stopping, pw::spa::support::system::IoFlags::IN, {
            let mainloop = mainloop.clone();
            move |_| mainloop.quit()
        });
    type Bound = (Box<dyn pw::proxy::ProxyT>, Box<dyn pw::proxy::Listener>);
    let bound: Rc<RefCell<HashMap<u32, Bound>>> = Rc::new(RefCell::new(HashMap::new()));
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let (registry, bound, tell) = (registry.downgrade(), bound.clone(), tell.clone());
            move |global| {
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                let props = global.props;
                let class = props.and_then(|props| props.get("media.class"));
                let entry: Option<Bound> = match global.type_ {
                    pw::types::ObjectType::Metadata
                        if props.and_then(|props| props.get("metadata.name"))
                            == Some("default") =>
                    {
                        let Ok(metadata) = registry.bind::<pw::metadata::Metadata, _>(global)
                        else {
                            return;
                        };
                        let tell = tell.clone();
                        let listener = metadata
                            .add_listener_local()
                            .property(move |_subject, key, _type, _value| {
                                match key {
                                    Some("default.audio.sink") => {
                                        tell(Change::Level(Control::OutputVolume))
                                    }
                                    Some("default.audio.source") => {
                                        tell(Change::Level(Control::InputVolume))
                                    }
                                    _ => {}
                                }
                                0
                            })
                            .register();
                        Some((Box::new(metadata), Box::new(listener)))
                    }
                    pw::types::ObjectType::Node => {
                        let control = match class {
                            Some("Audio/Sink") => Control::OutputVolume,
                            Some("Audio/Source") => Control::InputVolume,
                            _ => return,
                        };
                        let Ok(node) = registry.bind::<pw::node::Node, _>(global) else {
                            return;
                        };
                        node.subscribe_params(&[pw::spa::param::ParamType::Props]);
                        let tell = tell.clone();
                        let listener = node
                            .add_listener_local()
                            .param(move |_, _, _, _, _| tell(Change::Level(control)))
                            .register();
                        Some((Box::new(node), Box::new(listener)))
                    }
                    pw::types::ObjectType::Device if class == Some("Audio/Device") => {
                        let Ok(device) = registry.bind::<pw::device::Device, _>(global) else {
                            return;
                        };
                        device.subscribe_params(&[pw::spa::param::ParamType::Route]);
                        let tell = tell.clone();
                        let listener = device
                            .add_listener_local()
                            .param(move |_, _, _, _, _| {
                                tell(Change::Level(Control::OutputVolume));
                                tell(Change::Level(Control::InputVolume));
                            })
                            .register();
                        Some((Box::new(device), Box::new(listener)))
                    }
                    _ => None,
                };
                if let Some(entry) = entry {
                    bound.borrow_mut().insert(global.id, entry);
                }
            }
        })
        .global_remove({
            let bound = bound.clone();
            move |id| {
                bound.borrow_mut().remove(&id);
            }
        })
        .register();
    mainloop.run();
    // The proxies and their listeners before the connection.
    bound.borrow_mut().clear();
    let gone = gone.borrow_mut().take();
    match gone {
        Some(message) => Err(failed(format!("PipeWire went away: {message}"))),
        None => Ok(()),
    }
}
