//! The session's side of the sound's outputs and inputs over a played machine
//! ([`oxyos_audio::Played`]): who may ask, what is told to whom, a device chosen, a volume through
//! the hardware controls, the input metered only while asked, and headphones that take over.

use super::*;
use oxyos_audio::Played;
use oxyos_compositor_core::Grants;
use oxyos_controls::{Fake, Worker};
use oxyos_protocol::{Control, Event};
use std::collections::BTreeMap;

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
const APP: ClientId = 3;

const MACHINE: &str = r#"{
    "outputs": [
        {"id": "speakers", "name": "Speakers", "kind": "speakers", "transport": "builtIn", "volume": 0.5},
        {"id": "dell", "name": "DELL U2720Q", "kind": "hdmi", "transport": "hdmi", "volume": 0.75}
    ],
    "inputs": [
        {"id": "mic", "name": "Digital Microphone", "kind": "microphone", "transport": "builtIn", "volume": 0.8, "balance": null, "level": 0.25}
    ]
}"#;

struct Session {
    scene: Scene,
    sound: SoundDevices,
    played: Arc<Played>,
    controls: Controls,
    serial: u32,
    /// What each client was told, whole: `serial:text`.
    told: BTreeMap<ClientId, Vec<String>>,
    /// The shell's control levels told: (control, level, muted).
    levels: Vec<(Control, u32, bool)>,
}

impl Session {
    fn new() -> Self {
        let mut scene = Scene::new();
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
            // An application that asks the session for something else (the store).
            (
                APP,
                Grants {
                    store: true,
                    ..Grants::default()
                },
            ),
        ] {
            scene.connect(client, grants).unwrap();
        }
        let played = Arc::new(Played::parse(MACHINE).unwrap());
        let fake = Fake::parse("display_brightness = 0.5\n")
            .unwrap()
            .with_sound(played.clone());
        let controls = Controls::new(Worker::inline(Box::new(fake)));
        let mut session = Self {
            scene,
            sound: SoundDevices::new(played.clone()),
            played,
            controls,
            serial: 0,
            told: BTreeMap::new(),
            levels: Vec::new(),
        };
        session.turn();
        session.told.clear();
        session.levels.clear();
        session
    }

    fn turn(&mut self) {
        self.sound.serve(&mut self.scene, Some(&mut self.controls));
        self.controls.serve(&mut self.scene);
        let mut partial: BTreeMap<(ClientId, u32), String> = BTreeMap::new();
        for (client, event) in self.scene.drain_events() {
            match event {
                Event::SystemText {
                    serial, text, last, ..
                } => {
                    let whole = partial.entry((client, serial)).or_default();
                    whole.push_str(text.as_str());
                    if last {
                        let whole = partial.remove(&(client, serial)).unwrap();
                        self.told
                            .entry(client)
                            .or_default()
                            .push(format!("{serial}:{whole}"));
                    }
                }
                Event::ControlLevel {
                    control,
                    level_65536,
                    muted,
                    ..
                } if client == SHELL => self.levels.push((control, level_65536, muted)),
                _ => {}
            }
        }
    }

    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> u32 {
        self.serial += 1;
        let request = SystemRequest {
            client,
            serial: self.serial,
            with_input,
            text: text.into(),
        };
        // What the scene hands this part: requests of its vocabulary.
        assert!(ours(&request.text), "{text}");
        self.sound
            .handle(&mut self.scene, Some(&mut self.controls), request);
        self.turn();
        self.serial
    }

    /// The answer `client` was given to its request `serial`, as JSON.
    fn answer(&self, client: ClientId, serial: u32) -> serde_json::Value {
        let prefix = format!("{serial}:");
        let text = self.told.get(&client).and_then(|told| {
            told.iter()
                .find_map(|t| t.strip_prefix(&prefix).map(str::to_string))
        });
        serde_json::from_str(&text.unwrap_or_else(|| panic!("no answer {serial}: {:?}", self.told)))
            .unwrap()
    }

    /// What `client` was told unasked (serial 0) since the last call, as JSON.
    fn unasked(&mut self, client: ClientId) -> Vec<serde_json::Value> {
        let told = self.told.remove(&client).unwrap_or_default();
        told.iter()
            .filter_map(|t| t.strip_prefix("0:"))
            .map(|t| serde_json::from_str(t).unwrap())
            .collect()
    }

    fn selected(&self, direction: &str) -> serde_json::Value {
        self.played.machine()[format!("default_{direction}")].clone()
    }
}

fn selected_in(state: &serde_json::Value, direction: &str) -> Option<String> {
    state["sound"][direction]["devices"]
        .as_array()?
        .iter()
        .find(|device| device["selected"] == true)
        .map(|device| device["id"].as_str().unwrap().to_string())
}

#[test]
fn the_shell_and_settings_are_told_the_sound_and_no_one_else() {
    let mut session = Session::new();
    let asked = session.ask(SETTINGS, r#"{"sound":{}}"#, false);
    let state = session.answer(SETTINGS, asked);
    assert_eq!(state["sound"]["available"], true);
    assert_eq!(selected_in(&state, "output").as_deref(), Some("speakers"));
    assert_eq!(state["sound"]["output"]["volume"], 0.5);
    assert_eq!(state["sound"]["output"]["balance"], 0.0);
    assert_eq!(
        state["sound"]["output"]["devices"][1],
        serde_json::json!({"id": "dell", "name": "DELL U2720Q", "kind": "hdmi", "transport": "hdmi", "selected": false})
    );
    assert_eq!(state["sound"]["input"]["balance"], serde_json::Value::Null);
    // Another application's grant is not the sound's.
    let asked = session.ask(APP, r#"{"sound":{}}"#, true);
    assert_eq!(
        session.answer(APP, asked),
        serde_json::json!({"refused": {"reason": "notPermitted"}})
    );
    // Read strictly.
    let asked = session.ask(SETTINGS, r#"{"sound":{"all":true}}"#, true);
    assert_eq!(
        session.answer(SETTINGS, asked),
        serde_json::json!({"refused": {"reason": "invalidRequest"}})
    );
    // Not the sound's vocabulary: left for whoever takes it.
    assert!(!ours(r#"{"networks":{}}"#));
    assert!(!ours(r#"{"sound":{},"scan":{}}"#));
    assert!(ours(r#"{"soundMeter":{"on":true}}"#));
}

#[test]
fn a_device_chosen_is_wireplumbers_default_and_every_follower_is_told() {
    let mut session = Session::new();
    session.ask(SHELL, r#"{"sound":{}}"#, false);
    session.ask(SETTINGS, r#"{"sound":{}}"#, false);
    session.unasked(SHELL);
    // Without an input it changes nothing.
    let asked = session.ask(
        SHELL,
        r#"{"soundDevice":{"direction":"output","id":"dell"}}"#,
        false,
    );
    assert_eq!(
        session.answer(SHELL, asked),
        serde_json::json!({"refused": {"reason": "needsInput"}})
    );
    assert_eq!(session.selected("output"), "speakers");
    // An unknown device fails.
    let asked = session.ask(
        SHELL,
        r#"{"soundDevice":{"direction":"output","id":"mic"}}"#,
        true,
    );
    assert_eq!(
        session.answer(SHELL, asked),
        serde_json::json!({"failed": {"reason": "notFound"}})
    );
    session.levels.clear();
    let asked = session.ask(
        SHELL,
        r#"{"soundDevice":{"direction":"output","id":"dell"}}"#,
        true,
    );
    let state = session.answer(SHELL, asked);
    assert_eq!(selected_in(&state, "output").as_deref(), Some("dell"));
    assert_eq!(session.selected("output"), "dell");
    assert_eq!(
        session.played.machine()["calls"],
        serde_json::json!(["default output dell"])
    );
    // Settings follows it, once; the shell's slider shows the display's own volume.
    let told = session.unasked(SETTINGS);
    assert_eq!(told.len(), 1);
    assert_eq!(selected_in(&told[0], "output").as_deref(), Some("dell"));
    assert_eq!(told[0]["sound"]["output"]["volume"], 0.75);
    assert_eq!(
        session.levels,
        [(Control::OutputVolume, FULL_LEVEL * 3 / 4, false)]
    );
    // Nothing more while nothing changes.
    session.turn();
    assert!(session.unasked(SETTINGS).is_empty());
}

#[test]
fn settings_volume_goes_through_the_controls_and_the_balance_to_the_sound() {
    let mut session = Session::new();
    session.ask(SHELL, r#"{"sound":{}}"#, false);
    session.unasked(SHELL);
    let asked = session.ask(
        SETTINGS,
        r#"{"soundVolume":{"direction":"output","volume":0.25}}"#,
        true,
    );
    assert_eq!(
        session.answer(SETTINGS, asked)["sound"]["output"]["volume"],
        0.25
    );
    // The shell's slider and the bezel's level follow it, as a key's.
    assert_eq!(
        session.levels,
        [(Control::OutputVolume, FULL_LEVEL / 4, false)]
    );
    assert_eq!(session.unasked(SHELL)[0]["sound"]["output"]["volume"], 0.25);
    let asked = session.ask(
        SETTINGS,
        r#"{"soundMuted":{"direction":"input","muted":true}}"#,
        true,
    );
    assert_eq!(
        session.answer(SETTINGS, asked)["sound"]["input"]["muted"],
        true
    );
    let asked = session.ask(SETTINGS, r#"{"soundBalance":{"balance":-0.5}}"#, true);
    assert_eq!(
        session.answer(SETTINGS, asked)["sound"]["output"]["balance"],
        -0.5
    );
    assert_eq!(
        session.played.machine()["calls"],
        serde_json::json!(["volume output 0.25", "muted input true", "balance -0.5"])
    );
    // Out of range or not a number: refused, nothing done.
    for text in [
        r#"{"soundVolume":{"direction":"output","volume":1.5}}"#,
        r#"{"soundVolume":{"direction":"output","volume":-0.1}}"#,
        r#"{"soundBalance":{"balance":2}}"#,
    ] {
        let asked = session.ask(SETTINGS, text, true);
        assert_eq!(
            session.answer(SETTINGS, asked),
            serde_json::json!({"refused": {"reason": "invalidRequest"}}),
            "{text}"
        );
    }
    assert_eq!(
        session.played.machine()["calls"].as_array().unwrap().len(),
        3
    );
}

#[test]
fn nothing_changes_while_locked() {
    let mut session = Session::new();
    session.scene.lock();
    for text in [
        r#"{"soundDevice":{"direction":"output","id":"dell"}}"#,
        r#"{"soundVolume":{"direction":"output","volume":0.1}}"#,
        r#"{"soundBalance":{"balance":0.1}}"#,
        r#"{"soundMeter":{"on":true}}"#,
    ] {
        let asked = session.ask(SHELL, text, true);
        assert_eq!(
            session.answer(SHELL, asked),
            serde_json::json!({"refused": {"reason": "locked"}}),
            "{text}"
        );
    }
    assert_eq!(session.played.machine()["calls"], serde_json::json!([]));
    // Reading is not changing.
    let asked = session.ask(SHELL, r#"{"sound":{}}"#, false);
    assert_eq!(session.answer(SHELL, asked)["sound"]["available"], true);
}

#[test]
fn the_input_is_metered_while_someone_asks_and_never_after() {
    let mut session = Session::new();
    let asked = session.ask(SETTINGS, r#"{"soundMeter":{"on":true}}"#, false);
    assert_eq!(
        session.answer(SETTINGS, asked),
        serde_json::json!({"soundMeter": {"on": true}})
    );
    assert_eq!(session.played.machine()["metering"], true);
    session.turn();
    assert_eq!(
        session.unasked(SETTINGS),
        [serde_json::json!({"inputLevel": 0.25})]
    );
    // The shell, which did not ask, is not told.
    assert!(session.unasked(SHELL).is_empty());
    session
        .played
        .act(r#"{"change": {"id": "mic", "level": 0.5}}"#)
        .unwrap();
    session.turn();
    assert_eq!(
        session.unasked(SETTINGS),
        [serde_json::json!({"inputLevel": 0.5})]
    );
    // Settings gone: nothing listens to the input any more.
    session.scene.disconnect(SETTINGS);
    session.turn();
    assert_eq!(session.played.machine()["metering"], false);
    // Asked and stopped by the shell: on, then off.
    session.ask(SHELL, r#"{"soundMeter":{"on":true}}"#, false);
    session.ask(SHELL, r#"{"soundMeter":{"on":false}}"#, false);
    assert_eq!(
        session.played.machine()["calls"],
        serde_json::json!(["meter true", "meter false", "meter true", "meter false"])
    );
}

#[test]
fn headphones_connected_take_over_and_a_display_does_not() {
    let mut session = Session::new();
    session.ask(SHELL, r#"{"sound":{}}"#, false);
    session.unasked(SHELL);
    session
        .played
        .act(r#"{"plug": {"direction": "output", "device": {"id": "lg", "name": "LG HDR 4K", "kind": "hdmi", "transport": "hdmi"}}}"#)
        .unwrap();
    session.turn();
    assert_eq!(session.selected("output"), "speakers");
    let told = session.unasked(SHELL);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0]["sound"]["output"]["devices"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    session.levels.clear();
    session
        .played
        .act(r#"{"plug": {"direction": "output", "device": {"id": "airpods", "name": "AirPods Pro", "kind": "headphones", "transport": "bluetooth", "volume": 0.4}}}"#)
        .unwrap();
    session.turn();
    assert_eq!(session.selected("output"), "airpods");
    // Told once, chosen; the slider shows the headphones' volume.
    let told = session.unasked(SHELL);
    assert_eq!(told.len(), 1);
    assert_eq!(selected_in(&told[0], "output").as_deref(), Some("airpods"));
    assert_eq!(
        session.levels,
        [(
            Control::OutputVolume,
            (0.4 * f64::from(FULL_LEVEL)).round() as u32,
            false
        )]
    );
    // Disconnected: back to the speakers, as WirePlumber remembers.
    session
        .played
        .act(r#"{"unplug": {"id": "airpods"}}"#)
        .unwrap();
    session.turn();
    assert_eq!(session.selected("output"), "speakers");
    assert_eq!(
        session.played.machine()["calls"],
        serde_json::json!(["default output airpods"])
    );
}
