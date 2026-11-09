//! Battery and energy for the program running the compositor (docs/adr/0057-battery-and-
//! energy.mdx), the session authority: what the battery's menu and Settings › Battery are told and
//! may ask, Low Power Mode, the charge history, each application's energy, the display's and the
//! machine's sleep timers for the power source, and the low battery's warnings.
//!
//! - **What it follows.** The battery as the hardware controls read it (UPower's signals,
//!   `oxyos_controls::follow`), handed over each turn ([`Controls::take_batteries`]); the user's
//!   preferences; the displays turning off and on and the machine sleeping and waking, as the
//!   compositor does them. Each is recorded in the history as it changes, and an application's use
//!   is sampled with it. Nothing here wakes on a clock.
//! - **What it asks of the system,** on a thread of its own with a connection of its own (every
//!   call may wait): UPower's details and its charge threshold (kept in effect as the session
//!   starts and each time the machine wakes: where UPower says it is on and the battery has
//!   forgotten it, UPower is asked to write its own choice again, `upower::keep_charge_limit`), power-profiles-daemon's hold of
//!   `power-saver`, logind's sleep, and the applications' processes in `/proc`. A second thread
//!   follows power-profiles-daemon, so a hold lost with it is taken again.
//! - **Who may ask** (`SystemRequest`, the session's vocabulary): the shell (`shell`) and Settings
//!   (`settings`) the state, the history and the energy; Settings alone the charge limit, citing
//!   an input. Both are told the state, unasked, as it changes.
//!
//! [`Controls::take_batteries`]: crate::controls::Controls::take_batteries

use crate::stop::Stop;
use crate::Compositor;
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use oxyos_controls::{Battery, Warning};
use oxyos_power::energy::{self, Meter, Running};
use oxyos_power::history::{Charge, Event, History};
use oxyos_power::upower::Details;
use oxyos_power::{follow, profiles, sleep, upower, Failure, LowPower};
use oxyos_preferences::{Energy, Preferences};
use oxyos_protocol::BatteryState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Low Power Mode's pace: what is drawn goes at most 60 times a second.
pub const LOW_POWER_REFRESH_MHZ: u32 = 60_000;
/// The charges at which the person is warned the battery is low, as a Mac warns.
pub const WARN_AT: [u8; 2] = [10, 5];
/// On battery, the display at three quarters of its brightness (`dim_on_battery`).
const DIM_ON_BATTERY: (u32, u32) = (3, 4);
/// The system application the session's notifications are posted as: Settings, whose Battery page
/// activating one opens.
pub const NOTIFIER: &str = "so.oxy.settings";

/// What a client asks, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// The state (the battery's health read again).
    Power {},
    /// The charge and the display's use over 24 hours and ten days.
    PowerHistory {},
    /// Each application's energy, sampled now.
    Energy {},
    /// Settings alone, with an input: the battery's charge limited or not.
    ChargeLimit { enabled: bool },
}

const ASKS: [&str; 4] = ["power", "powerHistory", "energy", "chargeLimit"];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

/// What the thread is asked.
enum Job {
    /// UPower's details, its critical action and whether power-profiles-daemon is there.
    Details,
    /// The same, UPower's charge limit kept in effect first (the session starting, the machine
    /// awake again).
    KeepAndDetails,
    ChargeLimit {
        client: ClientId,
        serial: u32,
        enabled: bool,
    },
    /// Hold power-saver (`true`) or not.
    Hold(bool),
    /// power-profiles-daemon changed.
    Followed(follow::Change),
    Sleep,
    Sample {
        at: Duration,
        running: Vec<Running>,
    },
}

/// What the thread did.
enum Done {
    Details {
        details: Option<Details>,
        critical_action: Option<String>,
        profiles: bool,
    },
    ChargeLimit {
        client: ClientId,
        serial: u32,
        result: Result<(), Failure>,
    },
    /// Whether power-saver is held now.
    Held(bool),
    Sampled {
        at: Duration,
        running: Vec<Running>,
        reads: Vec<energy::Read>,
    },
}

/// The thread's side: its answers and the socket that wakes the compositor's turn.
#[derive(Clone)]
struct Back {
    done: mpsc::Sender<Done>,
    wake: Arc<UnixStream>,
}

impl Back {
    fn send(&self, done: Done) {
        if self.done.send(done).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

/// The calls, on one connection kept while it works: a hold is the connection's.
struct Worker {
    address: Option<String>,
    bus: Option<oxyos_power::Bus>,
    hold: Option<profiles::Hold>,
    want_hold: bool,
}

impl Worker {
    fn bus(&mut self) -> Option<&oxyos_power::Bus> {
        if self.bus.is_none() {
            match oxyos_power::connect(self.address.as_deref()) {
                Ok(bus) => self.bus = Some(bus),
                Err(error) => eprintln!("[power] the system bus: {error}"),
            }
        }
        self.bus.as_ref()
    }

    fn details(&mut self) -> Done {
        let Some(bus) = self.bus() else {
            return Done::Details {
                details: None,
                critical_action: None,
                profiles: false,
            };
        };
        let details = upower::details(bus).unwrap_or_else(|error| {
            eprintln!("[power] the battery's details: {error}");
            None
        });
        Done::Details {
            details,
            critical_action: upower::critical_action(bus).ok().flatten(),
            profiles: profiles::available(bus).unwrap_or(false),
        }
    }

    fn hold(&mut self, hold: bool) -> Done {
        self.want_hold = hold;
        if !hold {
            if let (Some(taken), Some(bus)) = (self.hold.take(), self.bus.as_ref()) {
                taken.release(bus);
            }
            return Done::Held(false);
        }
        if self.hold.is_none() {
            let Some(bus) = self.bus() else {
                return Done::Held(false);
            };
            match profiles::Hold::take(bus) {
                Ok(taken) => self.hold = Some(taken),
                // A machine without the daemon still has Low Power Mode's own savings.
                Err(Failure::Unsupported | Failure::Unavailable) => {}
                Err(failure) => {
                    eprintln!("[power] holding power-saver: {}", failure.name())
                }
            }
        }
        Done::Held(self.hold.is_some())
    }

    fn run(&mut self, job: Job, back: &Back) {
        match job {
            Job::Details => back.send(self.details()),
            Job::KeepAndDetails => {
                if let Some(bus) = self.bus() {
                    match upower::keep_charge_limit(bus) {
                        Ok(true) => eprintln!(
                            "[power] the battery had forgotten the charge limit: UPower wrote it again"
                        ),
                        Ok(false) => {}
                        // Said as it is: the limit is not in effect, and the state says so.
                        Err(failure) => eprintln!(
                            "[power] the battery forgot the charge limit and UPower could not write it again: {}",
                            failure.name()
                        ),
                    }
                }
                back.send(self.details());
            }
            Job::ChargeLimit {
                client,
                serial,
                enabled,
            } => {
                let result = match self.bus() {
                    Some(bus) => upower::set_charge_limit(bus, enabled),
                    None => Err(Failure::Unavailable),
                };
                back.send(Done::ChargeLimit {
                    client,
                    serial,
                    result,
                });
                back.send(self.details());
            }
            Job::Hold(hold) => back.send(self.hold(hold)),
            Job::Followed(change) => {
                let ours = match change {
                    // Started again: the hold before it is gone.
                    follow::Change::Owner => true,
                    follow::Change::Released(cookie) => {
                        self.hold.as_ref().map(profiles::Hold::cookie) == Some(cookie)
                    }
                };
                if !ours {
                    return;
                }
                let lost = self.hold.take().is_some();
                // Another program chose a profile: the person's choice stands until Low Power Mode
                // changes. The daemon restarted: held again.
                if self.want_hold && change == follow::Change::Owner {
                    back.send(self.hold(true));
                } else if lost {
                    back.send(Done::Held(false));
                }
            }
            Job::Sleep => {
                let result = match self.bus() {
                    Some(bus) => sleep::suspend(bus),
                    None => Err(Failure::Unavailable),
                };
                if let Err(failure) = result {
                    eprintln!("[power] the machine does not sleep: {}", failure.name());
                }
            }
            Job::Sample { at, running } => {
                let reads = energy::read(&energy::Proc::system(), &running);
                back.send(Done::Sampled { at, running, reads });
            }
        }
    }
}

/// What the person is told when the battery runs low, in the session's language.
struct Words {
    low_title: &'static str,
    low_body: fn(u8) -> String,
    action_body: fn(&str) -> String,
}

fn words(spanish: bool) -> Words {
    if spanish {
        Words {
            low_title: "Batería baja",
            low_body: |percent| {
                format!("Queda un {percent} % de batería. Conecta el ordenador a la corriente.")
            },
            action_body: |action| match action {
                "PowerOff" => "La batería está agotada: el ordenador se apagará ahora.".into(),
                "Ignore" => "La batería está agotada. Conecta el ordenador a la corriente.".into(),
                "Suspend" => {
                    "La batería está agotada: el ordenador entrará en reposo ahora.".into()
                }
                _ => "La batería está agotada: el ordenador hibernará ahora.".into(),
            },
        }
    } else {
        Words {
            low_title: "Low Battery",
            low_body: |percent| {
                format!("{percent}% battery remaining. Plug your computer into a power outlet.")
            },
            action_body: |action| match action {
                "PowerOff" => "The battery is empty: your computer will shut down now.".into(),
                "Ignore" => "The battery is empty. Plug your computer into a power outlet.".into(),
                "Suspend" => "The battery is empty: your computer will sleep now.".into(),
                _ => "The battery is empty: your computer will hibernate now.".into(),
            },
        }
    }
}

fn charge(battery: &Battery) -> Option<Charge> {
    match battery.state {
        BatteryState::Absent => None,
        BatteryState::Charging => Some(Charge::Charging),
        BatteryState::Discharging => Some(Charge::Discharging),
        BatteryState::Full | BatteryState::NotCharging => Some(Charge::Plugged),
    }
}

fn state_name(state: BatteryState) -> &'static str {
    match state {
        BatteryState::Absent => "absent",
        BatteryState::Charging => "charging",
        BatteryState::Discharging => "discharging",
        BatteryState::Full => "full",
        BatteryState::NotCharging => "notCharging",
    }
}

fn wall_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn refused(reason: &str) -> String {
    json!({ "refused": { "reason": reason } }).to_string()
}

pub struct Power {
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
    woken: UnixStream,
    stop: Arc<Stop>,
    threads: Vec<std::thread::JoinHandle<()>>,
    history: History,
    /// The wall clock the history is recorded and charted on, in seconds since the epoch: the
    /// system's, or a test's.
    clock: fn() -> u64,
    meter: Meter,
    low_power: LowPower,
    /// When it began, for the samples' clock.
    began: Instant,
    started: bool,
    battery: Option<Battery>,
    details: Option<Details>,
    critical_action: Option<String>,
    profiles: bool,
    held: bool,
    /// The preferences as last applied, and the power source they were applied for.
    energy: Option<Energy>,
    on_ac: Option<bool>,
    show_percentage: bool,
    spanish: bool,
    /// The lowest charge warned of in this discharge (100: none), and whether UPower's action was.
    warned: u8,
    warned_action: bool,
    displays_off: bool,
    sleeping: bool,
    /// Who waits for a sample's energy.
    waiting_energy: Vec<(ClientId, u32)>,
    told: Option<String>,
}

impl Power {
    /// The power's part, asking the system's services on the system bus, or on the bus at
    /// `address` (the development compositor's played ones), and keeping its history at
    /// `history` (none: for this session only).
    pub fn new(
        address: Option<String>,
        history: Option<std::path::PathBuf>,
    ) -> std::io::Result<Self> {
        Self::with_clock(address, history, wall_now)
    }

    /// The same, its history on `clock`.
    fn with_clock(
        address: Option<String>,
        history: Option<std::path::PathBuf>,
        clock: fn() -> u64,
    ) -> std::io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        let back = Back {
            done,
            wake: Arc::new(wake),
        };
        let (jobs, taken) = mpsc::channel::<Job>();
        let stop = Arc::new(Stop::new()?);
        let mut threads = Vec::new();
        {
            let (address, back) = (address.clone(), back.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("power".into())
                    .spawn(move || {
                        let mut worker = Worker {
                            address,
                            bus: None,
                            hold: None,
                            want_hold: false,
                        };
                        // Waits on its jobs alone; it ends when the part goes.
                        while let Ok(job) = taken.recv() {
                            worker.run(job, &back);
                        }
                        if let (Some(hold), Some(bus)) = (worker.hold.take(), worker.bus.as_ref()) {
                            hold.release(bus);
                        }
                    })?,
            );
        }
        {
            let (stop, jobs) = (stop.clone(), jobs.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("power-follow".into())
                    .spawn(move || {
                        let followed = oxyos_power::connect(address.as_deref()).and_then(|bus| {
                            follow::profiles(&bus, stop.fd(), |change| {
                                jobs.send(Job::Followed(change)).is_ok()
                            })
                        });
                        if let Err(error) = followed {
                            eprintln!("[power] following power-profiles-daemon: {error}");
                        }
                        // Gone or failed: nothing more to follow until the part goes.
                        stop.wait();
                    })?,
            );
        }
        Ok(Self {
            jobs,
            done: received,
            woken,
            stop,
            threads,
            history: History::open(history, clock()),
            clock,
            meter: Meter::default(),
            low_power: LowPower::default(),
            began: Instant::now(),
            started: false,
            battery: None,
            details: None,
            critical_action: None,
            profiles: false,
            held: false,
            energy: None,
            on_ac: None,
            show_percentage: false,
            spanish: false,
            warned: 100,
            warned_action: false,
            displays_off: false,
            sleeping: false,
            waiting_energy: Vec::new(),
            told: None,
        })
    }

    /// What wakes the compositor's turn when the thread is done (`Compositor::wakers`).
    pub fn wake_fd(&self) -> std::io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// Low Power Mode as the session's parts obey it (the content indexer, the updates).
    pub fn low_power(&self) -> LowPower {
        self.low_power.clone()
    }

    fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    fn now(&self) -> Duration {
        self.began.elapsed()
    }

    /// Whether the machine runs on its adapter, as the battery last said (a machine without a
    /// battery does).
    fn on_ac(&self) -> bool {
        self.battery.is_none_or(|battery| battery.on_ac)
    }

    /// A turn: the battery's changes, the preferences, the displays and sleep, the requests
    /// that are this part's, and what the thread did. `running` are the applications the session
    /// started that run now.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        preferences: &Preferences,
        running: &[Running],
    ) {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        let now = (self.clock)();
        if !self.started {
            self.started = true;
            self.history.record(now, Event::Session(true));
            self.history.record(now, Event::Display(true));
            self.send(Job::KeepAndDetails);
        }
        self.spanish = oxyos_preferences::session_locale(
            preferences,
            oxyos_preferences::environment_locale().as_deref(),
        )
        .starts_with("es");
        let batteries = compositor
            .controls
            .as_mut()
            .map(|controls| controls.take_batteries())
            .unwrap_or_default();
        for battery in batteries {
            self.battery_is(compositor, battery, running, now);
        }
        self.apply(compositor, preferences);
        // The displays and the machine, as the compositor took them.
        if compositor.displays_off() != self.displays_off {
            self.displays_off = compositor.displays_off();
            self.history.record(now, Event::Display(!self.displays_off));
        }
        if compositor.sleeping() != self.sleeping {
            self.sleeping = compositor.sleeping();
            self.history.record(now, Event::Asleep(self.sleeping));
            if !self.sleeping && !self.displays_off {
                self.history.record(now, Event::Display(true));
            }
            // Awake: UPower writes nothing as the machine wakes, and firmware may have forgotten
            // the thresholds while it slept.
            if !self.sleeping {
                self.send(Job::KeepAndDetails);
            }
        }
        if compositor.take_sleep() {
            self.send(Job::Sleep);
        }
        for request in compositor
            .scene
            .take_system_requests_where(|request| ours(&request.text))
        {
            self.handle(&mut compositor.scene, request, running);
        }
        for done in self.done.try_iter().collect::<Vec<_>>() {
            match done {
                Done::Details {
                    details,
                    critical_action,
                    profiles,
                } => {
                    self.details = details;
                    self.critical_action = critical_action;
                    self.profiles = profiles;
                }
                Done::ChargeLimit {
                    client,
                    serial,
                    result,
                } => {
                    let text = match result {
                        Ok(()) => json!({ "chargeLimit": {} }).to_string(),
                        Err(failure) => {
                            json!({ "failed": { "reason": failure.name() } }).to_string()
                        }
                    };
                    compositor.scene.system_text(client, serial, &text);
                }
                Done::Held(held) => self.held = held,
                Done::Sampled { at, running, reads } => {
                    self.meter.sample(at, &running, &reads);
                    if !self.waiting_energy.is_empty() {
                        let text =
                            json!({ "energy": { "apps": self.meter.report(at) } }).to_string();
                        for (client, serial) in std::mem::take(&mut self.waiting_energy) {
                            compositor.scene.system_text(client, serial, &text);
                        }
                    }
                }
            }
        }
        self.tell(&mut compositor.scene);
    }

    /// Activity Monitor's Energy (docs/adr/0063-activity-monitor.mdx): the applications' use as
    /// its sampler read it, sampled into this meter so the two count one way, and each one's
    /// report, with its twelve-hour average.
    pub fn sample_energy(
        &mut self,
        running: &[Running],
        reads: &[energy::Read],
    ) -> Vec<energy::Energy> {
        let at = self.now();
        self.meter.sample(at, running, reads);
        self.meter.report(at)
    }

    /// The session ends: recorded, and power-saver released (the thread releases it as it ends).
    pub fn end(&mut self) {
        self.history.record((self.clock)(), Event::Session(false));
    }

    fn battery_is(
        &mut self,
        compositor: &mut Compositor,
        battery: Battery,
        running: &[Running],
        now: u64,
    ) {
        let before = self.battery.replace(battery);
        if let Some(charge) = charge(&battery) {
            self.history
                .record(now, Event::Battery(battery.percent, charge));
        }
        // A change of the battery is when each application's use is sampled too.
        if !running.is_empty()
            && before.map(|b| (b.percent, b.state)) != Some((battery.percent, battery.state))
        {
            self.send(Job::Sample {
                at: self.now(),
                running: running.to_vec(),
            });
        }
        self.warn(&mut compositor.scene, battery);
    }

    /// The low battery's warnings: at 10 % and 5 % as it discharges, once each, and at UPower's
    /// action level what UPower is about to do (it does it itself: the session never does it
    /// twice). Charging again, the next discharge warns anew.
    fn warn(&mut self, scene: &mut Scene, battery: Battery) {
        if battery.state != BatteryState::Discharging {
            self.warned = 100;
            self.warned_action = false;
            return;
        }
        let words = words(self.spanish);
        if battery.warning == Warning::Action && !self.warned_action {
            self.warned_action = true;
            let action = self.critical_action.as_deref().unwrap_or("HybridSleep");
            let body = (words.action_body)(action);
            let _ = scene.post_system_notification(
                NOTIFIER,
                "low-battery",
                words.low_title,
                &body,
                true,
            );
            return;
        }
        let Some(level) = WARN_AT
            .iter()
            .copied()
            .filter(|level| battery.percent <= *level && *level < self.warned)
            .min()
        else {
            return;
        };
        self.warned = level;
        let body = (words.low_body)(battery.percent);
        let _ =
            scene.post_system_notification(NOTIFIER, "low-battery", words.low_title, &body, true);
    }

    /// The preferences and the power source applied: the idle timers, Low Power Mode, the
    /// display dimmer on battery.
    fn apply(&mut self, compositor: &mut Compositor, preferences: &Preferences) {
        let energy = preferences.energy;
        let on_ac = self.on_ac();
        self.show_percentage = preferences.battery_percentage;
        if self.energy == Some(energy) && self.on_ac == Some(on_ac) {
            return;
        }
        let was_on_ac = self.on_ac.replace(on_ac);
        self.energy = Some(energy);
        let now = compositor.now();
        compositor
            .idle
            .set_energy(energy.display_off(on_ac), energy.sleep(on_ac), now);
        let low = energy.low_power_mode.on(on_ac);
        if low != self.low_power.on() || was_on_ac.is_none() {
            eprintln!("[power] Low Power Mode {}", if low { "on" } else { "off" });
            self.low_power.set(low);
            compositor
                .scene
                .set_refresh_cap(low.then_some(LOW_POWER_REFRESH_MHZ));
            self.send(Job::Hold(low));
        }
        // Unplugged or plugged in: the display a little dimmer on battery, as it was on the
        // adapter (ADR 0057, decision 9).
        if let (Some(was), true) = (was_on_ac, energy.dim_on_battery) {
            if was != on_ac {
                if let Some(controls) = compositor.controls.as_mut() {
                    let (n, d) = DIM_ON_BATTERY;
                    if on_ac {
                        controls.scale_display(&mut compositor.scene, d, n);
                    } else {
                        controls.scale_display(&mut compositor.scene, n, d);
                    }
                }
            }
        }
    }

    fn handle(&mut self, scene: &mut Scene, request: SystemRequest, running: &[Running]) {
        let (client, serial) = (request.client, request.serial);
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            scene.system_text(client, serial, &refused("invalidRequest"));
            return;
        };
        let Some(grants) = scene.grants_of(client) else {
            return;
        };
        let allowed = match ask {
            Ask::ChargeLimit { .. } => grants.settings,
            _ => grants.settings || grants.shell,
        };
        if !allowed {
            scene.system_text(client, serial, &refused("notPermitted"));
            return;
        }
        match ask {
            Ask::Power {} => {
                // The health as UPower has it now; the state told again once read.
                self.send(Job::Details);
                scene.system_text(client, serial, &self.state().to_string());
            }
            Ask::PowerHistory {} => {
                let charts = self.history.charts((self.clock)());
                scene.system_text(
                    client,
                    serial,
                    &json!({ "powerHistory": charts }).to_string(),
                );
            }
            Ask::Energy {} => {
                self.waiting_energy.push((client, serial));
                self.send(Job::Sample {
                    at: self.now(),
                    running: running.to_vec(),
                });
            }
            Ask::ChargeLimit { enabled } => {
                if !request.with_input {
                    scene.system_text(client, serial, &refused("needsInput"));
                    return;
                }
                self.send(Job::ChargeLimit {
                    client,
                    serial,
                    enabled,
                });
            }
        }
    }

    /// The state the shell and Settings are told.
    fn state(&self) -> Value {
        let battery = self
            .battery
            .filter(|battery| battery.state != BatteryState::Absent)
            .map(|battery| {
                json!({
                    "percent": battery.percent,
                    "state": state_name(battery.state),
                    "onAc": battery.on_ac,
                    "minutes": battery.minutes,
                    "low": battery.state == BatteryState::Discharging
                        && (battery.percent <= WARN_AT[0] || battery.warning != Warning::None),
                })
            });
        let health = self.details.as_ref().map(|details| {
            json!({
                "maxCapacity": details.max_capacity(),
                "cycles": details.cycles,
                "worn": details.worn(),
            })
        });
        let charge_limit = self
            .details
            .as_ref()
            .and_then(|details| details.charge_limit);
        let mode = self
            .energy
            .map_or(oxyos_preferences::LowPowerMode::Never, |energy| {
                energy.low_power_mode
            });
        json!({ "power": {
            "battery": battery,
            "health": health,
            "chargeLimit": charge_limit,
            "lowPower": {
                "mode": mode.name(),
                "on": self.low_power.on(),
                "profiles": self.profiles,
                "held": self.held,
            },
            "showPercentage": self.show_percentage,
            "criticalAction": self.critical_action,
        }})
    }

    /// Tells the shell and Settings the state, if it changed.
    fn tell(&mut self, scene: &mut Scene) {
        let text = self.state().to_string();
        if self.told.as_deref() == Some(text.as_str()) {
            return;
        }
        for client in scene.power_watchers() {
            scene.system_text(client, 0, &text);
        }
        self.told = Some(text);
    }
}

impl Drop for Power {
    fn drop(&mut self) {
        self.stop.give();
        // The worker ends once its jobs' channel closes: a new one replaces it here.
        let (closed, _) = mpsc::channel();
        self.jobs = closed;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
