//! The displays as Settings sees and changes them (docs/adr/0018-several-displays.mdx). The program
//! running the compositor says what each output's display is ([`DisplayInfo`]: its name, modes and
//! scale choices); a client with the `settings` grant is told each, with its place, scale and
//! whether it is the primary as the scene has them, when the grant is given and as any of it
//! changes. Its requests (a mode, a scale, an origin, the primary, keeping a mode) are handed to
//! the program, which judges, applies and answers each (`PreferenceResult`, as a preference's).
//! The scene knows no mode and keeps no file.

use super::{ClientId, OutputId, Scene};
use oxyos_protocol::{ErrorCode, Event, PreeditText, Rect, Request, TextChunk};
use std::collections::{BTreeMap, BTreeSet};

/// Changes asked for and not yet taken.
const MAX_PENDING_CHANGES: usize = 16;
/// Most modes and scale choices one display is told with.
pub const MAX_DISPLAY_MODES: usize = 64;
pub const MAX_SCALE_CHOICES: usize = 8;
/// The longest display name told, in bytes (a monitor's EDID name is at most 13).
pub const MAX_DISPLAY_NAME_BYTES: usize = 64;

/// One mode a display can be driven at: its pixels and refresh in millihertz, and whether the
/// display prefers it (its native mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeInfo {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    pub preferred: bool,
}

/// What the program running the compositor says of an output's display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub output: OutputId,
    /// The monitor's own name (its EDID's), else its connector's.
    pub name: String,
    /// A panel built into the machine.
    pub builtin: bool,
    /// Its physical size in millimetres, 0 when unknown.
    pub size_mm: (u32, u32),
    /// Its modes, at most [`MAX_DISPLAY_MODES`], and the one it is driven at (an index of them).
    pub modes: Vec<ModeInfo>,
    pub mode: u32,
    /// The scales it may be shown at, from the most space to the largest text (at most
    /// [`MAX_SCALE_CHOICES`]), and the one its density recommends.
    pub scales: Vec<u32>,
    pub recommended: u32,
}

/// A change Settings asked for, for the program to judge and answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayRequest {
    Mode {
        output: OutputId,
        width: u32,
        height: u32,
        refresh_mhz: u32,
    },
    Scale {
        output: OutputId,
        scale_120: u32,
    },
    Origin {
        output: OutputId,
        x: i32,
        y: i32,
    },
    Primary {
        output: OutputId,
    },
    /// Keep the mode shown on trial, or go back now.
    Confirm {
        keep: bool,
    },
}

/// Whose request (`serial`) and what it asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayChange {
    pub client: ClientId,
    pub serial: u32,
    pub request: DisplayRequest,
}

/// A display as a client was told it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Told {
    info: DisplayInfo,
    rect: Rect,
    scale_120: u32,
    primary: bool,
}

#[derive(Debug, Default)]
pub(super) struct Displays {
    /// What the program said of each output's display.
    info: Vec<DisplayInfo>,
    asked: Vec<DisplayChange>,
    /// What each client with the grant was told.
    told: BTreeMap<ClientId, BTreeMap<OutputId, Told>>,
    /// The display whose mode is on trial and the seconds it was given, as last said, and the
    /// clients told it.
    confirming: Option<(OutputId, u32)>,
    confirmed_to: BTreeSet<ClientId>,
}

impl Scene {
    pub(super) fn display_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].settings {
            return Err(ErrorCode::Unauthorized);
        }
        let (serial, request) = match request {
            Request::SetDisplayMode {
                serial,
                output,
                width,
                height,
                refresh_mhz,
            } => (
                serial,
                DisplayRequest::Mode {
                    output,
                    width,
                    height,
                    refresh_mhz,
                },
            ),
            Request::SetDisplayScale {
                serial,
                output,
                scale_120,
            } => (serial, DisplayRequest::Scale { output, scale_120 }),
            Request::SetDisplayOrigin {
                serial,
                output,
                x,
                y,
            } => (serial, DisplayRequest::Origin { output, x, y }),
            Request::SetPrimaryDisplay { serial, output } => {
                (serial, DisplayRequest::Primary { output })
            }
            Request::ConfirmDisplays { serial, keep } => (serial, DisplayRequest::Confirm { keep }),
            _ => unreachable!("only the displays' requests are passed here"),
        };
        let displays = &mut self.displays;
        if displays.asked.len() >= MAX_PENDING_CHANGES {
            return Err(ErrorCode::QuotaExceeded);
        }
        displays.asked.push(DisplayChange {
            client,
            serial,
            request,
        });
        Ok(())
    }

    /// The changes Settings asked for since the last call, in order: the program judges and
    /// applies each and answers it with [`Scene::preference_set`].
    pub fn take_display_changes(&mut self) -> Vec<DisplayChange> {
        std::mem::take(&mut self.displays.asked)
    }

    /// What each output's display is, from the program running the compositor; a display not on
    /// one of the scene's outputs is not told.
    pub fn set_displays(&mut self, displays: Vec<DisplayInfo>) {
        self.displays.info = displays;
    }

    /// A display's mode is on trial for `seconds` (0: it was kept, or went back): told to the
    /// clients with the `settings` grant.
    pub fn display_confirmation(&mut self, output: OutputId, seconds: u32) {
        self.displays.confirming = (seconds > 0).then_some((output, seconds));
        let settings: Vec<ClientId> = self.settings_clients();
        for &client in &settings {
            self.emit(client, Event::DisplayConfirmation { output, seconds });
        }
        self.displays.confirmed_to = settings.into_iter().collect();
    }

    fn settings_clients(&self) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|(_, grants)| grants.settings)
            .map(|(client, _)| *client)
            .collect()
    }

    /// Tells each client with the grant what changed of the displays since it was last told
    /// (before events are sent); one given the grant is told every display and a mode on trial.
    pub(super) fn tell_displays(&mut self) {
        let settings = self.settings_clients();
        if settings.is_empty() {
            return;
        }
        let now: BTreeMap<OutputId, Told> = self
            .displays
            .info
            .iter()
            .filter_map(|info| {
                let index = self.outputs.iter().position(|o| o.id == info.output)?;
                let output = self.outputs[index];
                Some((
                    info.output,
                    Told {
                        info: info.clone(),
                        rect: output.rect,
                        scale_120: output.scale_120,
                        primary: index == 0,
                    },
                ))
            })
            .collect();
        for client in settings {
            let told = self.displays.told.remove(&client).unwrap_or_default();
            for output in told.keys().filter(|output| !now.contains_key(output)) {
                self.emit(client, Event::DisplayRemoved { output: *output });
            }
            for (&output, display) in &now {
                let before = told.get(&output);
                if before == Some(display) {
                    continue;
                }
                let info = &display.info;
                if before.is_none_or(|b| b.info.name != info.name) {
                    self.emit_display_name(client, output, &info.name);
                }
                if before.is_none_or(|b| b.info.modes != info.modes) {
                    for (index, mode) in info.modes.iter().take(MAX_DISPLAY_MODES).enumerate() {
                        self.emit(
                            client,
                            Event::DisplayMode {
                                output,
                                index: index as u32,
                                width: mode.width,
                                height: mode.height,
                                refresh_mhz: mode.refresh_mhz,
                                preferred: mode.preferred,
                            },
                        );
                    }
                }
                if before.is_none_or(|b| {
                    (&b.info.scales, b.info.recommended) != (&info.scales, info.recommended)
                }) {
                    for (index, &scale_120) in
                        info.scales.iter().take(MAX_SCALE_CHOICES).enumerate()
                    {
                        self.emit(
                            client,
                            Event::DisplayScale {
                                output,
                                index: index as u32,
                                scale_120,
                                recommended: scale_120 == info.recommended,
                            },
                        );
                    }
                }
                self.emit(
                    client,
                    Event::Display {
                        output,
                        rect: display.rect,
                        scale_120: display.scale_120,
                        mode: info.mode,
                        width_mm: info.size_mm.0,
                        height_mm: info.size_mm.1,
                        primary: display.primary,
                        builtin: info.builtin,
                    },
                );
            }
            if let Some((output, seconds)) = self.displays.confirming {
                if self.displays.confirmed_to.insert(client) {
                    self.emit(client, Event::DisplayConfirmation { output, seconds });
                }
            }
            self.displays.told.insert(client, now.clone());
        }
    }

    /// A display's name in chunks, cut to whole characters within its limit; one (empty) for
    /// none.
    fn emit_display_name(&mut self, client: ClientId, output: OutputId, name: &str) {
        let mut end = name.len().min(MAX_DISPLAY_NAME_BYTES);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        let mut chunks: Vec<PreeditText> = TextChunk::split(&name[..end])
            .iter()
            .map(|chunk| PreeditText::new(chunk.as_str()).expect("a chunk fits"))
            .collect();
        if chunks.is_empty() {
            chunks.push(PreeditText::new("").expect("empty"));
        }
        let count = chunks.len();
        for (index, text) in chunks.into_iter().enumerate() {
            self.emit(
                client,
                Event::DisplayName {
                    output,
                    index: index as u8,
                    last: index + 1 == count,
                    text,
                },
            );
        }
    }

    /// A client gone or no longer granted: its changes not taken are dropped, and what it was
    /// told forgotten (granted again, it is told everything).
    pub(super) fn forget_display_client(&mut self, client: ClientId) {
        self.displays.asked.retain(|change| change.client != client);
        self.displays.told.remove(&client);
        self.displays.confirmed_to.remove(&client);
    }
}
