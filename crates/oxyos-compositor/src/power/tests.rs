//! The session's side of battery and energy, over UPower, power-profiles-daemon and logind played
//! on a bus of the test's own (`oxyos_power::played`): who may ask, what is told to whom, Low Power
//! Mode held and let go with the power source, the charge limit reaching the battery's sysfs, the
//! low battery's warnings, the idle timers of the power source, the machine put to sleep, the
//! history recorded and each application's energy.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_controls::{Fake, Worker as ControlsWorker};
use oxyos_power::played::{Machine, Played};
use oxyos_protocol::Event as Told;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
const APP: ClientId = 3;
const MINUTE: Duration = Duration::from_secs(60);

/// The history's clock: noon of 6 November 2026, UTC, on a quarter of an hour's edge, where the
/// charts' last bin ends.
fn noon() -> u64 {
    1_793_966_400
}

struct Bus(Child);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "oxyos-compositor-power-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn private_bus(directory: &Path) -> (String, Bus) {
    let mut bus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .arg(format!(
            "--address=unix:path={}",
            directory.join("bus").display()
        ))
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon (the dbus-daemon package)");
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    (address.trim().to_string(), Bus(bus))
}

/// The reference ThinkPad's battery: 64 %, discharging, a charge threshold of 75–80 %.
fn thinkpad(sysfs: &Path) -> Machine {
    serde_json::from_value(json!({
        "battery": {
            "percent": 64.0, "state": "discharging", "time_to_empty": 12000,
            "energy_full": 49.6, "energy_full_design": 57.0, "cycles": 412,
            "threshold": {"start": 75, "end": 80}
        },
        "on_ac": false,
        "sysfs": sysfs,
    }))
    .unwrap()
}

struct Session {
    compositor: Compositor,
    power: Power,
    played: Played,
    preferences: Preferences,
    directory: PathBuf,
    /// Every whole text each client was told, by serial.
    told: BTreeMap<ClientId, Vec<(u32, String)>>,
    partial: BTreeMap<(ClientId, u32), String>,
    serial: u32,
    _bus: Bus,
}

impl Session {
    fn new(name: &str, machine: impl FnOnce(&Path) -> Machine) -> Self {
        let directory = scratch(name);
        let sysfs = directory.join("BAT0");
        std::fs::create_dir_all(&sysfs).unwrap();
        let (address, bus) = private_bus(&directory);
        let played = Played::serve(&address, machine(&sysfs)).unwrap();
        let mut compositor = Compositor::new(Scene::new(), 1000);
        compositor.clock = Some(Duration::from_secs(1000));
        for (client, grants) in [
            (
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            ),
            (
                SETTINGS,
                Grants {
                    settings: true,
                    ..Grants::default()
                },
            ),
            (APP, Grants::default()),
        ] {
            compositor.scene.connect(client, grants).unwrap();
        }
        let fake = Fake::parse("display_brightness = 0.5\n")
            .unwrap()
            .with_power_bus(&address)
            .unwrap();
        compositor.controls = Some(crate::controls::Controls::new(ControlsWorker::inline(
            Box::new(fake),
        )));
        let power = Power::with_clock(Some(address), Some(directory.join("battery-history")), noon)
            .unwrap();
        let mut session = Self {
            compositor,
            power,
            played,
            preferences: Preferences::default(),
            directory,
            told: BTreeMap::new(),
            partial: BTreeMap::new(),
            serial: 0,
            _bus: bus,
        };
        session.turn();
        session
    }

    /// A turn of the session: the controls, then the power's part, and what clients were told.
    fn turn(&mut self) {
        if let Some(controls) = self.compositor.controls.as_mut() {
            controls.serve(&mut self.compositor.scene);
        }
        let running = vec![Running {
            pid: std::process::id(),
            id: "so.oxy.allo".into(),
            name: "Allo".into(),
        }];
        self.power
            .serve(&mut self.compositor, &self.preferences, &running);
        for (client, event) in self.compositor.scene.drain_events() {
            if let Told::SystemText {
                serial, text, last, ..
            } = event
            {
                let whole = self.partial.entry((client, serial)).or_default();
                whole.push_str(text.as_str());
                if last {
                    let whole = self.partial.remove(&(client, serial)).unwrap();
                    self.told.entry(client).or_default().push((serial, whole));
                }
            }
        }
    }

    /// Turns until `done` holds of the session, at most five seconds.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.turn();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "not in five seconds: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> u32 {
        self.serial += 1;
        let running = vec![Running {
            pid: std::process::id(),
            id: "so.oxy.allo".into(),
            name: "Allo".into(),
        }];
        self.power.handle(
            &mut self.compositor.scene,
            SystemRequest {
                client,
                serial: self.serial,
                with_input,
                text: text.into(),
            },
            &running,
        );
        self.serial
    }

    fn answer(&self, client: ClientId, serial: u32) -> Option<Value> {
        self.told
            .get(&client)?
            .iter()
            .find(|(told, _)| *told == serial)
            .map(|(_, text)| serde_json::from_str(text).unwrap())
    }

    /// The state last told `client` unasked.
    fn state(&self, client: ClientId) -> Option<Value> {
        self.told
            .get(&client)?
            .iter()
            .rev()
            .find(|(serial, _)| *serial == 0)
            .map(|(_, text)| serde_json::from_str::<Value>(text).unwrap()["power"].clone())
    }

    /// The played machine changed, as the controls' follower would have read it.
    fn act(&mut self, change: Value) {
        self.played.act(&change).unwrap();
        self.compositor.controls.as_mut().unwrap().read_battery();
    }

    fn set(&mut self, key: &str, value: &str) {
        self.preferences.set(key, value).unwrap();
    }
}

#[test]
fn the_shell_and_settings_are_told_the_battery_and_nobody_else() {
    let mut session = Session::new("told", thinkpad);
    session.until("told the battery and its health", |s| {
        s.state(SHELL)
            .is_some_and(|state| state["health"]["maxCapacity"] == 87)
    });
    let state = session.state(SETTINGS).unwrap();
    assert_eq!(
        state["battery"],
        json!({"percent": 64, "state": "discharging", "onAc": false, "minutes": 200, "low": false})
    );
    assert_eq!(
        state["health"],
        json!({"maxCapacity": 87, "cycles": 412, "worn": false})
    );
    assert_eq!(
        state["chargeLimit"],
        json!({"enabled": false, "end": 80, "start": 75})
    );
    assert_eq!(state["lowPower"]["mode"], "never");
    assert_eq!(state["lowPower"]["on"], false);
    assert_eq!(state["showPercentage"], false);
    assert!(session.state(APP).is_none());
    // An application asks: refused.
    let asked = session.ask(APP, r#"{"power":{}}"#, false);
    assert_eq!(
        session.answer(APP, asked),
        None,
        "told nothing: the scene refuses a client without a grant itself"
    );
    // The shell asks: the state.
    let asked = session.ask(SHELL, r#"{"power":{}}"#, false);
    session.turn();
    assert_eq!(
        session.answer(SHELL, asked).unwrap()["power"]["battery"]["percent"],
        64
    );
    // Not of this vocabulary, read strictly.
    let asked = session.ask(SHELL, r#"{"power":{"now":true}}"#, false);
    session.turn();
    assert_eq!(
        session.answer(SHELL, asked).unwrap(),
        json!({"refused": {"reason": "invalidRequest"}})
    );
    // The percentage shown in the menu bar, as the preference says.
    session.set("battery_percentage", "true");
    session.turn();
    assert_eq!(session.state(SHELL).unwrap()["showPercentage"], true);
}

#[test]
fn the_charge_limit_is_settings_alone_with_an_input_and_reaches_the_batterys_sysfs() {
    let mut session = Session::new("limit", thinkpad);
    let sysfs = session.directory.join("BAT0");
    let asked = session.ask(SHELL, r#"{"chargeLimit":{"enabled":true}}"#, true);
    session.turn();
    assert_eq!(
        session.answer(SHELL, asked).unwrap(),
        json!({"refused": {"reason": "notPermitted"}})
    );
    let asked = session.ask(SETTINGS, r#"{"chargeLimit":{"enabled":true}}"#, false);
    session.turn();
    assert_eq!(
        session.answer(SETTINGS, asked).unwrap(),
        json!({"refused": {"reason": "needsInput"}})
    );
    assert!(!sysfs.join("charge_control_end_threshold").exists());
    let asked = session.ask(SETTINGS, r#"{"chargeLimit":{"enabled":true}}"#, true);
    session.until("the limit set", |s| s.answer(SETTINGS, asked).is_some());
    assert_eq!(
        session.answer(SETTINGS, asked).unwrap(),
        json!({"chargeLimit": {}})
    );
    let read = |name: &str| std::fs::read_to_string(sysfs.join(name)).unwrap();
    assert_eq!(read("charge_control_start_threshold"), "75");
    assert_eq!(read("charge_control_end_threshold"), "80");
    session.until("told the limit", |s| {
        s.state(SETTINGS)
            .is_some_and(|state| state["chargeLimit"]["enabled"] == true)
    });
    // Let it charge full again.
    let asked = session.ask(SETTINGS, r#"{"chargeLimit":{"enabled":false}}"#, true);
    session.until("the limit off", |s| s.answer(SETTINGS, asked).is_some());
    assert_eq!(read("charge_control_end_threshold"), "100");
}

#[test]
fn a_charge_limit_polkit_refuses_says_so() {
    let mut session = Session::new("limit-refused", |sysfs| {
        let mut machine = thinkpad(sysfs);
        machine.deny = vec!["charge_limit".into()];
        machine
    });
    let asked = session.ask(SETTINGS, r#"{"chargeLimit":{"enabled":true}}"#, true);
    session.until("refused", |s| s.answer(SETTINGS, asked).is_some());
    assert_eq!(
        session.answer(SETTINGS, asked).unwrap(),
        json!({"failed": {"reason": "notPermitted"}})
    );
}

/// The ThinkPad's machine with UPower's state kept in `upower` beside its battery's directory,
/// saying the limit is on, and the battery as UPower 1.90.9 leaves it once it started (the
/// thresholds written again from that state).
fn limited_thinkpad(sysfs: &Path) -> Machine {
    let state = sysfs.parent().unwrap().join("upower");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("charging-threshold-status"), "1").unwrap();
    forget(sysfs);
    let mut machine = thinkpad(sysfs);
    machine.state_dir = Some(state);
    machine
}

/// The battery's thresholds as firmware that forgot them leaves them: charging full.
fn forget(sysfs: &Path) {
    std::fs::write(sysfs.join("charge_control_start_threshold"), "0").unwrap();
    std::fs::write(sysfs.join("charge_control_end_threshold"), "100").unwrap();
}

fn limit_told(session: &Session) -> Option<Value> {
    session
        .state(SETTINGS)
        .map(|state| state["chargeLimit"]["enabled"].clone())
}

#[test]
fn a_charge_limit_the_battery_forgot_while_asleep_is_written_again_by_upower_on_waking() {
    let mut session = Session::new("limit-wake", limited_thinkpad);
    let sysfs = session.directory.join("BAT0");
    let read = |name: &str| std::fs::read_to_string(sysfs.join(name)).unwrap();
    // UPower started and wrote them: the session, starting, has nothing to ask.
    session.until("told the limit", |s| limit_told(s) == Some(json!(true)));
    assert_eq!(read("charge_control_end_threshold"), "80");
    assert_eq!(session.played.describe()["calls"], json!([]));
    // Asleep, the firmware forgets; UPower writes nothing as the machine wakes.
    session.compositor.sleeping = true;
    session.turn();
    forget(&sysfs);
    session.compositor.sleeping = false;
    session.until("written again", |s| {
        s.played.describe()["calls"] == json!(["EnableChargeThreshold true"])
    });
    session.until("in effect", |_| {
        read("charge_control_start_threshold") == "75"
            && read("charge_control_end_threshold") == "80"
    });
    session.until("told", |s| limit_told(s) == Some(json!(true)));
    // Once: nothing more asked while nothing changes.
    for _ in 0..20 {
        session.turn();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        session.played.describe()["calls"],
        json!(["EnableChargeThreshold true"])
    );
}

#[test]
fn a_charge_limit_the_battery_forgot_and_polkit_will_not_have_written_again_is_not_claimed() {
    let mut session = Session::new("limit-wake-refused", |sysfs| {
        let mut machine = limited_thinkpad(sysfs);
        machine.deny = vec!["charge_limit".into()];
        machine
    });
    let sysfs = session.directory.join("BAT0");
    session.until("told the limit", |s| limit_told(s) == Some(json!(true)));
    session.compositor.sleeping = true;
    session.turn();
    forget(&sysfs);
    session.compositor.sleeping = false;
    // UPower still says on; Settings is told what the battery holds: off.
    session.until("told it is off", |s| limit_told(s) == Some(json!(false)));
    assert_eq!(
        std::fs::read_to_string(sysfs.join("charge_control_end_threshold")).unwrap(),
        "100"
    );
}

#[test]
fn low_power_mode_on_battery_holds_power_saver_until_plugged_in() {
    let mut session = Session::new("low-power", thinkpad);
    session.until("the battery read", |s| s.state(SHELL).is_some());
    let low_power = session.power.low_power();
    let changes = low_power.changes().unwrap();
    session.set("low_power_mode", "battery");
    session.until("power-saver held", |s| {
        s.played.describe()["active_profile"] == "power-saver"
    });
    assert!(low_power.on() && changes.take());
    assert_eq!(
        session.played.describe()["holds"][0]["reason"],
        "Low Power Mode"
    );
    session.until("told", |s| {
        s.state(SETTINGS).is_some_and(|state| {
            state["lowPower"]
                == json!({"mode": "battery", "on": true, "profiles": true, "held": true})
        })
    });
    // Plugged in: released, the machine as the person set it.
    session.act(json!({"on_ac": true, "state": "charging"}));
    session.until("released", |s| {
        s.played.describe()["active_profile"] == "balanced"
    });
    assert!(!low_power.on() && changes.take());
    // Always: held on the adapter too.
    session.set("low_power_mode", "always");
    session.until("held again", |s| {
        s.played.describe()["active_profile"] == "power-saver"
    });
    // The session goes: the daemon lets go of its hold.
    let Session {
        power,
        played,
        _bus,
        ..
    } = session;
    drop(power);
    let deadline = Instant::now() + Duration::from_secs(5);
    while played.describe()["active_profile"] != "balanced" {
        assert!(Instant::now() < deadline, "the hold outlived the session");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_idle_timers_are_the_power_sources_and_the_machine_sleeps_through_logind() {
    let mut session = Session::new("sleep", thinkpad);
    session.until("the battery read", |s| s.state(SHELL).is_some());
    // On battery: the display off after two minutes, asleep after ten.
    assert_eq!(
        session.compositor.idle.energy(),
        (Some(2 * MINUTE), Some(10 * MINUTE))
    );
    session.act(json!({"on_ac": true, "state": "charging"}));
    session.until("on the adapter", |s| {
        s.compositor.idle.energy() == (Some(10 * MINUTE), None)
    });
    session.set("sleep_adapter", "30");
    session.turn();
    assert_eq!(
        session.compositor.idle.energy(),
        (Some(10 * MINUTE), Some(30 * MINUTE))
    );
    // Thirty minutes with no input: the displays off at ten, the machine asleep at thirty.
    session.compositor.idle.pass(10 * MINUTE);
    session.compositor.follow_idle();
    assert!(session.compositor.displays_off());
    session.compositor.idle.pass(20 * MINUTE);
    session.compositor.follow_idle();
    session.until("asleep", |s| s.played.describe()["suspends"] == 1);
    // Recorded: the display off.
    let history = std::fs::read_to_string(session.directory.join("battery-history")).unwrap();
    assert!(
        history.lines().any(|line| line.ends_with(" s 0")),
        "{history}"
    );
}

#[test]
fn the_battery_running_low_warns_at_ten_and_five_percent_and_says_upowers_action() {
    let mut session = Session::new("low", thinkpad);
    session.until("the battery read", |s| s.state(SHELL).is_some());
    let notifications = |session: &mut Session| -> Vec<String> {
        // The shell is told each notification's texts.
        let mut texts = Vec::new();
        for (client, event) in session.compositor.scene.drain_events() {
            if let (SHELL, Told::NotificationText { text, .. }) = (client, event) {
                texts.push(text.as_str().to_string());
            }
        }
        texts
    };
    session.act(json!({"percent": 11.0}));
    session.until("11 %", |s| {
        s.state(SHELL)
            .is_some_and(|state| state["battery"]["percent"] == 11)
    });
    session.act(json!({"percent": 10.0}));
    session
        .compositor
        .controls
        .as_mut()
        .unwrap()
        .serve(&mut session.compositor.scene);
    session
        .power
        .serve(&mut session.compositor, &session.preferences, &[]);
    let told = notifications(&mut session);
    assert!(told.contains(&"Low Battery".to_string()), "{told:?}");
    assert!(
        told.iter().any(|text| text.starts_with("10% battery")),
        "{told:?}"
    );
    // 9 %: no warning again.
    session.act(json!({"percent": 9.0}));
    session
        .compositor
        .controls
        .as_mut()
        .unwrap()
        .serve(&mut session.compositor.scene);
    session
        .power
        .serve(&mut session.compositor, &session.preferences, &[]);
    assert!(notifications(&mut session).is_empty());
    session.act(json!({"percent": 5.0}));
    session
        .compositor
        .controls
        .as_mut()
        .unwrap()
        .serve(&mut session.compositor.scene);
    session
        .power
        .serve(&mut session.compositor, &session.preferences, &[]);
    assert!(notifications(&mut session)
        .iter()
        .any(|text| text.starts_with("5% battery")));
    // UPower's action level: what it is about to do (HybridSleep: hibernate).
    session.act(json!({"percent": 2.0, "warning": "action"}));
    session
        .compositor
        .controls
        .as_mut()
        .unwrap()
        .serve(&mut session.compositor.scene);
    session
        .power
        .serve(&mut session.compositor, &session.preferences, &[]);
    assert!(notifications(&mut session)
        .iter()
        .any(|text| text.contains("will hibernate now")));
    // The session never put it to sleep itself.
    assert_eq!(session.played.describe()["suspends"], 0);
}

#[test]
fn each_applications_energy_is_sampled_when_asked_and_the_history_charted() {
    let mut session = Session::new("energy", thinkpad);
    // Some work of this process's own.
    let mut sum = 0u64;
    for i in 0..5_000_000u64 {
        sum = sum.wrapping_add(i * i);
    }
    assert!(sum > 0);
    let asked = session.ask(SETTINGS, r#"{"energy":{}}"#, false);
    session.until("sampled", |s| s.answer(SETTINGS, asked).is_some());
    let apps = session.answer(SETTINGS, asked).unwrap()["energy"]["apps"].clone();
    assert_eq!(apps[0]["id"], "so.oxy.allo");
    assert_eq!(apps[0]["name"], "Allo");
    assert_eq!(apps[0]["running"], true);
    assert!(apps[0]["cpu"].as_f64().unwrap() > 0.0);
    // The history: this session's start, its display and its battery, all at noon and charted at
    // noon, in the day's last quarter.
    let asked = session.ask(SHELL, r#"{"powerHistory":{}}"#, false);
    session.turn();
    let charts = session.answer(SHELL, asked).unwrap()["powerHistory"].clone();
    assert_eq!(charts["day"]["levels"].as_array().unwrap().len(), 96);
    assert_eq!(charts["day"]["levels"][95], 64);
    assert_eq!(charts["days"]["levels"].as_array().unwrap().len(), 240);
}

fn descriptors() -> Vec<String> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .map(|target| target.display().to_string())
        .collect()
}

#[test]
#[ignore = "counts the process's descriptors: run alone"]
fn the_power_part_leaves_no_descriptor_once_it_goes() {
    let directory = scratch("descriptors");
    let (address, _bus) = private_bus(&directory);
    drop(oxyos_power::connect(Some(&address)).unwrap());
    let played = Played::serve(&address, thinkpad(&directory)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let before = descriptors();
    {
        let mut compositor = Compositor::new(Scene::new(), 1000);
        let mut power = Power::new(
            Some(address.clone()),
            Some(directory.join("battery-history")),
        )
        .unwrap();
        let mut preferences = Preferences::default();
        preferences.set("low_power_mode", "always").unwrap();
        for _ in 0..50 {
            power.serve(&mut compositor, &preferences, &[]);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(played.describe()["active_profile"], "power-saver");
        power.end();
    }
    std::thread::sleep(Duration::from_millis(1000));
    let after = descriptors();
    assert!(after.len() <= before.len(), "{before:?} → {after:?}");
}
