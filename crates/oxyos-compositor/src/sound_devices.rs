//! The sound's outputs and inputs for the program running the compositor
//! (docs/adr/0051-sound-devices.mdx): the session's side of the shell's Control Center and
//! Settings' Sound page, over the session's sound ([`oxyos_audio::Audio`]: PipeWire, or a played
//! machine).
//!
//! The shell (its `shell` grant) and Settings (its `settings` grant) ask in the session's
//! vocabulary (`SystemRequest`, one JSON object read strictly, [`Ask`]); what changes the machine
//! must cite an input the client received and is refused while the session is locked. A client
//! that asked for the sound's state is told it again as it changes; one that asked for the input's
//! level is told it while it asks (`{"inputLevel": n}`), and nothing listens to the input once no
//! one does. A volume or mute goes through the hardware controls, so the keys, the bezel and
//! Control Center's slider follow it; the controls are told every volume the sound says
//! ([`crate::controls::Controls::audio_is`]). A device plugged in or connected that takes over, as
//! a Mac's headphones do ([`oxyos_audio::takeover`]), is chosen here.

use crate::controls::Controls;
use crate::stop::Stop;
use oxyos_audio::{Audio, Direction, Event, State};
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, ControlRequest, Scene};
use oxyos_controls::{volume_control, FULL_LEVEL};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::sync::Arc;

/// What a client asks, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// The sound's state, and to be told it as it changes.
    Sound {},
    /// With an input: a device chosen for a direction.
    SoundDevice { direction: Direction, id: String },
    /// With an input: the chosen device's volume, 0 to 1.
    SoundVolume { direction: Direction, volume: f64 },
    /// With an input: the chosen device muted or not.
    SoundMuted { direction: Direction, muted: bool },
    /// With an input: the chosen output's balance, -1 (left) to 1 (right).
    SoundBalance { balance: f64 },
    /// The input's level told while on.
    SoundMeter { on: bool },
}

/// The vocabulary's names: a request naming one of them is this part's.
const ASKS: [&str; 6] = [
    "sound",
    "soundDevice",
    "soundVolume",
    "soundMuted",
    "soundBalance",
    "soundMeter",
];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

fn refused_text(reason: &str) -> String {
    json!({"refused": {"reason": reason}}).to_string()
}

fn state_text(state: &State) -> String {
    json!({ "sound": state }).to_string()
}

pub struct SoundDevices {
    audio: Arc<dyn Audio>,
    /// The state as last told.
    state: State,
    told: String,
    /// Who asked for the state: told it as it changes.
    followers: BTreeSet<ClientId>,
    /// Who asked for the input's level.
    metering: BTreeSet<ClientId>,
    /// The thread following PipeWire (a session's), stopped when this goes.
    following: Option<(Arc<Stop>, std::thread::JoinHandle<()>)>,
}

impl SoundDevices {
    pub fn new(audio: Arc<dyn Audio>) -> Self {
        let state = audio.state();
        Self {
            told: state_text(&state),
            state,
            audio,
            followers: BTreeSet::new(),
            metering: BTreeSet::new(),
            following: None,
        }
    }

    /// The session's: PipeWire, followed on a thread of its own (again whenever its socket is
    /// made anew, never on a clock), and the client the hardware controls set volumes through.
    pub fn pipewire() -> std::io::Result<(Self, Arc<oxyos_audio::PipeWire>)> {
        let pipewire = Arc::new(oxyos_audio::PipeWire::new()?);
        let mut devices = Self::new(pipewire.clone());
        let stop = Arc::new(Stop::new()?);
        let thread = {
            let (stop, pipewire) = (stop.clone(), pipewire.clone());
            std::thread::Builder::new()
                .name("oxyos-audio".into())
                .spawn(move || {
                    crate::controls::again(
                        "PipeWire",
                        &stop,
                        oxyos_audio::pipewire_socket(),
                        |stop| pipewire.follow(stop.fd()),
                    );
                })?
        };
        devices.following = Some((stop, thread));
        Ok((devices, pipewire))
    }

    /// What wakes the compositor's turn when the sound has changed (`Compositor::wakers`); none
    /// for a played machine, which changes only in the turn.
    pub fn wake_fd(&self) -> Option<std::io::Result<OwnedFd>> {
        self.audio
            .wake_fd()
            .map(|fd: BorrowedFd<'_>| fd.try_clone_to_owned())
    }

    /// The state as last told.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// A turn: the scene's requests that are this part's, and what the sound said.
    pub fn serve(&mut self, scene: &mut Scene, mut controls: Option<&mut Controls>) {
        let requests = scene.take_system_requests_where(|request| ours(&request.text));
        let mut changed = !requests.is_empty();
        for request in requests {
            self.handle(scene, controls.as_deref_mut(), request);
        }
        // Those gone are no longer told, nor metered for.
        let before = self.metering.len();
        self.followers
            .retain(|client| scene.grants_of(*client).is_some());
        self.metering
            .retain(|client| scene.grants_of(*client).is_some());
        if before > 0 && self.metering.is_empty() {
            self.audio.meter(false);
        }
        for event in self.audio.take_events() {
            match event {
                Event::State(_) => changed = true,
                Event::InputLevel(level) => {
                    let text = json!({ "inputLevel": level }).to_string();
                    for client in &self.metering {
                        scene.system_text(*client, 0, &text);
                    }
                }
            }
        }
        if changed {
            self.changed(scene, controls);
        }
    }

    /// Tells what changed: the followers the state, the controls the volumes, and a device that
    /// takes over chosen.
    fn changed(&mut self, scene: &mut Scene, controls: Option<&mut Controls>) {
        let state = self.audio.state();
        if state == self.state {
            return;
        }
        for (direction, id) in oxyos_audio::takeover(&self.state, &state) {
            eprintln!("[sound] {id} takes over the {direction:?}");
            self.audio.set_default(direction, &id);
        }
        // What choosing it said at once.
        let state = self.audio.state();
        if let Some(controls) = controls {
            controls.audio_is(scene, &state);
        }
        let text = state_text(&state);
        if text != self.told {
            for client in &self.followers {
                scene.system_text(*client, 0, &text);
            }
            self.told = text;
        }
        self.state = state;
    }

    /// One request of this part's vocabulary: read strictly, checked against the grant it needs
    /// and, if it changes anything, an input the client received and an unlocked session.
    pub fn handle(
        &mut self,
        scene: &mut Scene,
        controls: Option<&mut Controls>,
        request: SystemRequest,
    ) {
        let (client, serial) = (request.client, request.serial);
        let Some(grants) = scene.grants_of(client) else {
            return;
        };
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            scene.system_text(client, serial, &refused_text("invalidRequest"));
            return;
        };
        if !(grants.shell || grants.settings) {
            scene.system_text(client, serial, &refused_text("notPermitted"));
            return;
        }
        let changes = !matches!(ask, Ask::Sound {} | Ask::SoundMeter { .. });
        if changes && !request.with_input {
            scene.system_text(client, serial, &refused_text("needsInput"));
            return;
        }
        let locked = scene.is_locked();
        if locked && (changes || matches!(ask, Ask::SoundMeter { on: true })) {
            scene.system_text(client, serial, &refused_text("locked"));
            return;
        }
        let valid = |value: f64, low: f64| value.is_finite() && (low..=1.0).contains(&value);
        match ask {
            Ask::Sound {} => {
                self.followers.insert(client);
                let text = state_text(&self.audio.state());
                scene.system_text(client, serial, &text);
                return;
            }
            Ask::SoundMeter { on } => {
                let was = !self.metering.is_empty();
                if on {
                    self.metering.insert(client);
                } else {
                    self.metering.remove(&client);
                }
                let now = !self.metering.is_empty();
                if was != now {
                    self.audio.meter(now);
                }
                scene.system_text(
                    client,
                    serial,
                    &json!({"soundMeter": {"on": on}}).to_string(),
                );
                return;
            }
            Ask::SoundDevice { direction, id } => {
                let known = self
                    .audio
                    .state()
                    .side(direction)
                    .devices
                    .iter()
                    .any(|device| device.id == id);
                if !known {
                    scene.system_text(
                        client,
                        serial,
                        &json!({"failed": {"reason": "notFound"}}).to_string(),
                    );
                    return;
                }
                self.audio.set_default(direction, &id);
            }
            Ask::SoundVolume { direction, volume } => {
                if !valid(volume, 0.0) {
                    scene.system_text(client, serial, &refused_text("invalidRequest"));
                    return;
                }
                let level = (volume * f64::from(FULL_LEVEL)).round() as u32;
                let control = volume_control(direction);
                match controls {
                    // The keys' and Control Center's way, so all of them follow it.
                    Some(controls) => {
                        controls.ask(scene, ControlRequest::SetLevel { control, level })
                    }
                    None => self.audio.set_volume(direction, volume),
                }
                // The controls' worker may answer in a later turn: the state then says it.
                let mut state = self.audio.state();
                if let Some(side) = Some(state.side_mut(direction)).filter(|s| s.volume.is_some()) {
                    side.volume = Some(f64::from(level) / f64::from(FULL_LEVEL));
                }
                scene.system_text(client, serial, &state_text(&state));
                return;
            }
            Ask::SoundMuted { direction, muted } => {
                let control = volume_control(direction);
                match controls {
                    Some(controls) => {
                        controls.ask(scene, ControlRequest::SetMuted { control, muted })
                    }
                    None => self.audio.set_muted(direction, muted),
                }
            }
            Ask::SoundBalance { balance } => {
                if !(balance.is_finite() && (-1.0..=1.0).contains(&balance)) {
                    scene.system_text(client, serial, &refused_text("invalidRequest"));
                    return;
                }
                self.audio.set_balance(balance);
            }
        }
        scene.system_text(client, serial, &state_text(&self.audio.state()));
    }
}

impl Drop for SoundDevices {
    fn drop(&mut self) {
        if !self.metering.is_empty() {
            self.audio.meter(false);
        }
        if let Some((stop, thread)) = self.following.take() {
            stop.give();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
