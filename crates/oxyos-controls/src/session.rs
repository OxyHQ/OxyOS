//! A session's [`System`]: the machine the session runs on, through what it already runs.

use crate::backlight::{self, Setter};
use crate::{audio_level, bluetooth, network, power, set_audio_level, volume_direction};
use crate::{Battery, Bluetooth, Control, Level, Network, System};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use zbus::blocking::Connection;

/// The machine: PipeWire's volumes through the session's client (`oxyos_audio`), backlights in `sysfs` set through
/// `setter`, and UPower, NetworkManager and BlueZ on the system bus. The bus and logind are
/// reached when first needed and again after a failure, so a session started before them
/// follows them once they are up.
pub struct Session {
    audio: Arc<dyn oxyos_audio::Audio>,
    sysfs: PathBuf,
    setter: Option<Box<dyn Setter>>,
    /// Makes the setter: logind's `SetBrightness` on this process's session.
    make_setter: Box<dyn Fn() -> io::Result<Box<dyn Setter>> + Send>,
    bus: Option<Connection>,
}

impl Session {
    /// The session's own: its sound (`audio`), `/sys`, logind.
    pub fn new(audio: Arc<dyn oxyos_audio::Audio>) -> Self {
        Self::with(
            audio,
            PathBuf::from("/sys"),
            Box::new(|| {
                oxyos_seat::Brightness::of_session().map(|logind| Box::new(logind) as Box<_>)
            }),
        )
    }

    pub fn with(
        audio: Arc<dyn oxyos_audio::Audio>,
        sysfs: PathBuf,
        make_setter: Box<dyn Fn() -> io::Result<Box<dyn Setter>> + Send>,
    ) -> Self {
        Self {
            audio,
            sysfs,
            setter: None,
            make_setter,
            bus: None,
        }
    }

    fn backlight(&self, control: Control) -> Option<backlight::Device> {
        match control {
            Control::DisplayBrightness => backlight::display(&self.sysfs),
            Control::KeyboardBrightness => backlight::keyboard(&self.sysfs),
            _ => None,
        }
    }

    fn bus(&mut self) -> io::Result<&Connection> {
        if self.bus.is_none() {
            // Kept, its executor waiting without waking (`oxyos_seat::system_bus`).
            self.bus = Some(oxyos_seat::system_bus()?);
        }
        Ok(self.bus.as_ref().expect("connected"))
    }

    /// A call on the bus; failed, the bus is reached anew next time.
    fn on_bus<T>(&mut self, call: impl Fn(&Connection) -> io::Result<T>) -> io::Result<T> {
        let result = call(self.bus()?);
        if result.is_err() {
            self.bus = None;
        }
        result
    }
}

impl System for Session {
    fn level(&mut self, control: Control) -> io::Result<Option<Level>> {
        match control {
            Control::OutputVolume | Control::InputVolume => {
                Ok(audio_level(&self.audio.state(), control))
            }
            _ => self
                .backlight(control)
                .map(|device| device.level())
                .transpose(),
        }
    }

    fn set_level(&mut self, control: Control, value: u32) -> io::Result<()> {
        match control {
            Control::OutputVolume | Control::InputVolume => {
                set_audio_level(self.audio.as_ref(), control, value)
            }
            _ => {
                let device = self
                    .backlight(control)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no backlight"))?;
                if self.setter.is_none() {
                    self.setter = Some((self.make_setter)()?);
                }
                let result = device.set(self.setter.as_deref().expect("made"), value);
                if result.is_err() {
                    self.setter = None;
                }
                result
            }
        }
    }

    fn set_muted(&mut self, control: Control, muted: bool) -> io::Result<()> {
        let direction =
            volume_direction(control).ok_or_else(|| io::Error::other("not a volume"))?;
        self.audio.set_muted(direction, muted);
        Ok(())
    }

    fn keyboard_changed_by_firmware(&mut self) -> bool {
        backlight::keyboard(&self.sysfs).is_some_and(|device| device.changed_by_firmware())
    }

    fn battery(&mut self) -> io::Result<Battery> {
        self.on_bus(power::read)
    }

    fn network(&mut self) -> io::Result<Network> {
        self.on_bus(network::read)
    }

    fn set_wifi(&mut self, enabled: bool) -> io::Result<()> {
        self.on_bus(|bus| network::set_wifi(bus, enabled))
    }

    fn bluetooth(&mut self) -> io::Result<Bluetooth> {
        self.on_bus(bluetooth::read)
    }
}
