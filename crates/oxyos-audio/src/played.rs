//! A machine's sound played for tests and the development compositor: its outputs and inputs are
//! what it is told (JSON), it chooses its defaults as WirePlumber does (the device chosen last
//! that is there, else the first), and everything the session does to it is kept, so a scenario
//! checks what the session did ([`Played::machine`]). A step changes it as the world would
//! ([`Played::act`]): headphones plugged in, a display unplugged, another program's volume.
//! Everything happens in the caller's turn, so a scenario is the same every run.

use crate::{Audio, Device, Direction, Event, Kind, Side, State, Transport};
use serde::{Deserialize, Serialize};
use std::os::fd::BorrowedFd;
use std::sync::Mutex;

/// One played output or input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayedDevice {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub transport: Transport,
    #[serde(default = "full")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    /// `None`: mono (an output with no balance).
    #[serde(default = "centred")]
    pub balance: Option<f64>,
    /// An input's level as a meter would read it now.
    #[serde(default)]
    pub level: f64,
}

fn full() -> f64 {
    1.0
}

fn centred() -> Option<f64> {
    Some(0.0)
}

/// The machine, as a scenario describes it and reads it back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    #[serde(default)]
    pub outputs: Vec<PlayedDevice>,
    #[serde(default)]
    pub inputs: Vec<PlayedDevice>,
    /// The devices chosen, most recent first, as WirePlumber keeps them
    /// (`default.configured.audio.sink` and the ones before it).
    #[serde(default)]
    pub configured_outputs: Vec<String>,
    #[serde(default)]
    pub configured_inputs: Vec<String>,
    /// What the machine has chosen now (read back; never told).
    #[serde(default, skip_deserializing)]
    pub default_output: Option<String>,
    #[serde(default, skip_deserializing)]
    pub default_input: Option<String>,
    /// Whether the input is metered now (read back).
    #[serde(default, skip_deserializing)]
    pub metering: bool,
    /// Every change the session made, in order: `default output headphones`, `volume output 0.5`.
    #[serde(default, skip_deserializing)]
    pub calls: Vec<String>,
}

impl Machine {
    fn devices(&self, direction: Direction) -> &Vec<PlayedDevice> {
        match direction {
            Direction::Output => &self.outputs,
            Direction::Input => &self.inputs,
        }
    }

    fn devices_mut(&mut self, direction: Direction) -> &mut Vec<PlayedDevice> {
        match direction {
            Direction::Output => &mut self.outputs,
            Direction::Input => &mut self.inputs,
        }
    }

    fn configured_mut(&mut self, direction: Direction) -> &mut Vec<String> {
        match direction {
            Direction::Output => &mut self.configured_outputs,
            Direction::Input => &mut self.configured_inputs,
        }
    }

    /// WirePlumber's choice: the most recently chosen device that is there, else the first.
    fn chosen(&self, direction: Direction) -> Option<&PlayedDevice> {
        let devices = self.devices(direction);
        let configured = match direction {
            Direction::Output => &self.configured_outputs,
            Direction::Input => &self.configured_inputs,
        };
        configured
            .iter()
            .find_map(|id| devices.iter().find(|device| &device.id == id))
            .or_else(|| devices.first())
    }

    fn chosen_mut(&mut self, direction: Direction) -> Option<&mut PlayedDevice> {
        let id = self.chosen(direction)?.id.clone();
        self.devices_mut(direction)
            .iter_mut()
            .find(|device| device.id == id)
    }

    fn settle(&mut self) {
        self.default_output = self.chosen(Direction::Output).map(|d| d.id.clone());
        self.default_input = self.chosen(Direction::Input).map(|d| d.id.clone());
    }

    fn state(&self) -> State {
        let side = |direction: Direction| {
            let chosen = self.chosen(direction).map(|device| device.id.clone());
            let mut devices: Vec<Device> = self
                .devices(direction)
                .iter()
                .map(|device| Device {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    kind: device.kind,
                    transport: device.transport,
                    selected: Some(&device.id) == chosen.as_ref(),
                })
                .collect();
            devices.sort_by(|a, b| {
                (a.transport, a.name.to_lowercase(), &a.id).cmp(&(
                    b.transport,
                    b.name.to_lowercase(),
                    &b.id,
                ))
            });
            let selected = self.chosen(direction);
            Side {
                devices,
                volume: selected.map(|device| device.volume),
                muted: selected.is_some_and(|device| device.muted),
                balance: selected
                    .and_then(|device| device.balance)
                    .filter(|_| direction == Direction::Output),
            }
        };
        State {
            available: true,
            output: side(Direction::Output),
            input: side(Direction::Input),
            generation: 1,
            audible: Default::default(),
        }
    }
}

/// What a step does to the machine.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Act {
    /// A device plugged in or connected.
    Plug {
        direction: Direction,
        device: PlayedDevice,
    },
    /// A device unplugged or disconnected, by id.
    Unplug { id: String },
    /// A device changed by the world: its route (headphones in the jack: `kind`, `name`), its
    /// volume set by another program, an input's level.
    Change {
        id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        kind: Option<Kind>,
        #[serde(default)]
        volume: Option<f64>,
        #[serde(default)]
        muted: Option<bool>,
        #[serde(default)]
        level: Option<f64>,
    },
}

/// The played machine (the module's doc).
pub struct Played {
    machine: Mutex<Machine>,
    events: Mutex<Vec<Event>>,
    told: Mutex<State>,
    /// The meter's level as last told; `None` while it is off.
    level_told: Mutex<Option<f64>>,
}

fn rounded(value: f64) -> f64 {
    (value.clamp(0.0, 1.0) * 65536.0).round() / 65536.0
}

impl Played {
    /// A machine described by `json` ([`Machine`]); its first state is told.
    pub fn parse(json: &str) -> Result<Self, String> {
        let mut machine: Machine =
            serde_json::from_str(json).map_err(|error| format!("the played sound: {error}"))?;
        machine.settle();
        let played = Self {
            machine: Mutex::new(machine),
            events: Mutex::new(Vec::new()),
            told: Mutex::new(State::unavailable()),
            level_told: Mutex::new(None),
        };
        played.changed();
        Ok(played)
    }

    /// The machine as it is now, as JSON.
    pub fn machine(&self) -> serde_json::Value {
        serde_json::to_value(&*self.lock()).unwrap_or_default()
    }

    /// A step's change of the machine (`{"plug": …}`, `{"unplug": …}`, `{"change": …}`).
    pub fn act(&self, json: &str) -> Result<(), String> {
        let act: Act = serde_json::from_str(json).map_err(|error| format!("{json}: {error}"))?;
        {
            let mut machine = self.lock();
            match act {
                Act::Plug { direction, device } => {
                    machine.devices_mut(direction).push(device);
                }
                Act::Unplug { id } => {
                    machine.outputs.retain(|device| device.id != id);
                    machine.inputs.retain(|device| device.id != id);
                }
                Act::Change {
                    id,
                    name,
                    kind,
                    volume,
                    muted,
                    level,
                } => {
                    let Machine {
                        outputs, inputs, ..
                    } = &mut *machine;
                    let device = outputs
                        .iter_mut()
                        .chain(inputs.iter_mut())
                        .find(|device| device.id == id)
                        .ok_or_else(|| format!("no played device {id:?}"))?;
                    if let Some(name) = name {
                        device.name = name;
                    }
                    if let Some(kind) = kind {
                        device.kind = kind;
                    }
                    if let Some(volume) = volume {
                        device.volume = rounded(volume);
                    }
                    if let Some(muted) = muted {
                        device.muted = muted;
                    }
                    if let Some(level) = level {
                        device.level = level.clamp(0.0, 1.0);
                    }
                }
            }
            machine.settle();
        }
        self.changed();
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Machine> {
        self.machine.lock().expect("the played sound")
    }

    /// Tells the state if it changed, and the meter's level while it is on.
    fn changed(&self) {
        let (state, level) = {
            let machine = self.lock();
            let level = machine
                .metering
                .then(|| machine.chosen(Direction::Input).map(|device| device.level))
                .flatten();
            (machine.state(), level)
        };
        let mut told = self.told.lock().expect("the played sound");
        let mut events = self.events.lock().expect("the played sound");
        if *told != state {
            *told = state.clone();
            events.push(Event::State(state));
        }
        let mut level_told = self.level_told.lock().expect("the played sound");
        if level.is_some() && *level_told != level {
            events.extend(level.map(Event::InputLevel));
        }
        *level_told = level;
    }

    fn call(&self, call: String, change: impl FnOnce(&mut Machine)) {
        {
            let mut machine = self.lock();
            change(&mut machine);
            machine.calls.push(call);
            machine.settle();
        }
        self.changed();
    }
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Output => "output",
        Direction::Input => "input",
    }
}

impl Audio for Played {
    fn state(&self) -> State {
        self.lock().state()
    }

    fn set_default(&self, direction: Direction, id: &str) {
        if !self
            .lock()
            .devices(direction)
            .iter()
            .any(|device| device.id == id)
        {
            return;
        }
        let id = id.to_string();
        self.call(
            format!("default {} {id}", direction_name(direction)),
            |machine| {
                let configured = machine.configured_mut(direction);
                configured.retain(|old| *old != id);
                configured.insert(0, id);
            },
        );
    }

    fn set_volume(&self, direction: Direction, volume: f64) {
        let volume = rounded(volume);
        self.call(
            format!("volume {} {volume}", direction_name(direction)),
            |machine| {
                if let Some(device) = machine.chosen_mut(direction) {
                    device.volume = volume;
                }
            },
        );
    }

    fn set_muted(&self, direction: Direction, muted: bool) {
        self.call(
            format!("muted {} {muted}", direction_name(direction)),
            |machine| {
                if let Some(device) = machine.chosen_mut(direction) {
                    device.muted = muted;
                }
            },
        );
    }

    fn set_balance(&self, balance: f64) {
        let balance = (balance.clamp(-1.0, 1.0) * 1000.0).round() / 1000.0;
        self.call(format!("balance {balance}"), |machine| {
            if let Some(device) = machine.chosen_mut(Direction::Output) {
                if device.balance.is_some() {
                    device.balance = Some(balance);
                }
            }
        });
    }

    fn meter(&self, on: bool) {
        if self.lock().metering == on {
            return;
        }
        self.call(format!("meter {on}"), |machine| machine.metering = on);
    }

    fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().expect("the played sound"))
    }

    fn wake_fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }
}
