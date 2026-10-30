//! The session's displays (docs/adr/0018-several-displays.mdx), for the program running the
//! compositor: every connected connector of its card is an output, shown on a display of its own.
//!
//! - **Who a display is.** A display is known by its identity (its EDID's maker, product and
//!   serial, `oxyos_kms::Output::identity`), so what is kept for it follows it to any port.
//! - **How it is shown.** At the mode the person chose for it, else its own: its native resolution
//!   (its preferred mode's pixels) at the fastest refresh it offers there that the card drives (an
//!   atomic test, `Heads::prepare`), the next fastest when the card refuses one (a link without
//!   the bandwidth); at the scale kept for it, else the one its density recommends
//!   (`arrangement`).
//! - **Where.** As the arrangement kept for the set of displays connected says, the primary first;
//!   else left to right, top-aligned, the built-in panel first (the primary). A monitor plugged in
//!   comes at the right end; one unplugged goes, and those that touched only it come back beside
//!   the others (`arrangement::reflow`). With nothing connected the outputs stay, undisplayed,
//!   until a display comes, and an output for it takes their windows.
//! - **Settings changes them** (its `settings` grant): a scale, an origin (placed beside the
//!   others, `arrangement::snap`) and the primary are applied and kept at once; a mode is shown on
//!   trial and kept only if Settings keeps it within [`TRIAL`], as a Mac asks "Keep these display
//!   settings?": otherwise, or if anything plugged or unplugged meanwhile, every display goes back
//!   to what it showed. What is kept is written whole first, so a change that cannot be kept is
//!   refused with nothing changed.
//!
//! What shows an output on a connector is the program's ([`Heads`]): the session's card, or the
//! development compositor's virtual displays.

use crate::arrangement::{self, Monitor};
use crate::settings::Settings;
use crate::{Compositor, Result};
use oxyos_compositor_core::scene::MAX_DISPLAY_MODES;
use oxyos_compositor_core::{DisplayInfo, DisplayRequest, ModeInfo, Output, OutputId};
use oxyos_preferences::{Arrangement, DisplayMode, DisplaySettings, Preferences};
use oxyos_protocol::Rect;
use std::time::{Duration, Instant};

/// How long a mode is shown on trial before the display goes back to what it showed, unless
/// Settings keeps it: a Mac's 15 seconds.
pub const TRIAL: Duration = Duration::from_secs(15);

/// A connector as the card reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connector {
    /// Its object id (`oxyos_kms::Output::id`).
    pub id: u32,
    pub name: String,
    /// A panel built into the machine (eDP, LVDS, DSI).
    pub internal: bool,
    /// Who the display on it is (`oxyos_kms::Output::identity`).
    pub identity: String,
    /// The monitor's own name, from its EDID.
    pub product: Option<String>,
    pub size_mm: (u32, u32),
    /// Its modes while a display is connected (none otherwise), as the card lists them.
    pub modes: Vec<ModeInfo>,
    /// Whether the display and the card can refresh it at a variable rate (`vrr_capable`), and the
    /// vertical refresh it takes, least and most, in hertz (its EDID's range limits).
    pub vrr_capable: bool,
    pub refresh_range_hz: Option<(u32, u32)>,
}

impl Connector {
    pub fn of(output: &oxyos_kms::Output) -> Self {
        Self {
            id: output.id,
            name: output.name.clone(),
            internal: output.internal,
            identity: output.identity(),
            product: output.edid.as_ref().and_then(|edid| edid.name.clone()),
            size_mm: output.size_mm,
            modes: if output.connected {
                output
                    .modes
                    .iter()
                    .map(|mode| ModeInfo {
                        width: u32::from(mode.width),
                        height: u32::from(mode.height),
                        refresh_mhz: mode.refresh_mhz,
                        preferred: mode.preferred,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            vrr_capable: output.vrr_capable,
            refresh_range_hz: output.edid.as_ref().and_then(|edid| edid.refresh_range_hz),
        }
    }

    pub fn connected(&self) -> bool {
        !self.modes.is_empty()
    }

    /// The mode it prefers (its native one), else its first.
    pub fn preferred(&self) -> Option<ModeInfo> {
        self.modes
            .iter()
            .find(|mode| mode.preferred)
            .or(self.modes.first())
            .copied()
    }

    /// Its own modes, as it is shown unless the person chose another: its native resolution (its
    /// preferred mode's pixels) at each refresh it offers there, the fastest first. A Mac drives a
    /// ProMotion panel or a 144 Hz monitor at its fastest by default; so does the session.
    pub fn own_modes(&self) -> Vec<ModeInfo> {
        let Some(preferred) = self.preferred() else {
            return Vec::new();
        };
        let mut modes: Vec<ModeInfo> = self
            .modes
            .iter()
            .filter(|mode| (mode.width, mode.height) == (preferred.width, preferred.height))
            .copied()
            .collect();
        modes.sort_by_key(|mode| std::cmp::Reverse(mode.refresh_mhz));
        modes
    }

    /// Its mode of these pixels and refresh, if it has one.
    pub fn mode(&self, width: u32, height: u32, refresh_mhz: u32) -> Option<ModeInfo> {
        self.modes
            .iter()
            .find(|mode| {
                (mode.width, mode.height, mode.refresh_mhz) == (width, height, refresh_mhz)
            })
            .copied()
    }

    /// The display as the arrangement sees it, at `mode`.
    pub fn monitor(&self, mode: &ModeInfo) -> Monitor {
        Monitor {
            internal: self.internal,
            pixels: (mode.width, mode.height),
            size_mm: self.size_mm,
        }
    }
}

/// A mode as the preferences keep it.
fn display_mode(mode: &ModeInfo) -> DisplayMode {
    DisplayMode {
        width: mode.width,
        height: mode.height,
        refresh_mhz: mode.refresh_mhz,
    }
}

/// Whether two modes are the same pixels and refresh.
fn same(a: &ModeInfo, b: &ModeInfo) -> bool {
    (a.width, a.height, a.refresh_mhz) == (b.width, b.height, b.refresh_mhz)
}

/// An output the session shows: the connector it is on (none once it went dark), the display on
/// it as it was read, and the mode it is driven at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub output: Output,
    pub connector: Option<u32>,
    pub display: Connector,
    pub mode: ModeInfo,
    /// The mode the person chose for it, as kept (Settings); none: its own
    /// (`Connector::own_modes`). Shown unless the card refuses it now, and kept either way.
    pub chosen: Option<DisplayMode>,
}

impl Shown {
    fn pixels(&self) -> (u32, u32) {
        (self.mode.width, self.mode.height)
    }

    /// What is kept for it: the mode the person chose, if any, and its scale.
    fn settings(&self) -> DisplaySettings {
        DisplaySettings {
            mode: self.chosen,
            scale_120: self.output.scale_120,
        }
    }

    /// Whether it is shown at the mode the person chose.
    fn at_chosen(&self) -> bool {
        self.chosen
            .is_some_and(|chosen| chosen == display_mode(&self.mode))
    }
}

/// What shows an output on a connector: the session's card (a display on a CRTC of its own, its
/// scanout buffers and pointer images), or the development compositor's virtual displays.
pub trait Heads {
    /// A connector made ready to show an output.
    type Head;
    /// The connectors now, in the card's order.
    fn connectors(&mut self) -> Result<Vec<Connector>>;
    /// Makes `connector` ready to show an output at `mode` and `scale_120` (its pointer's images),
    /// or says why it cannot: nothing free to drive it, or the card refusing the mode.
    fn prepare(
        &mut self,
        compositor: &mut Compositor,
        connector: &Connector,
        mode: &ModeInfo,
        scale_120: u32,
    ) -> std::result::Result<Self::Head, String>;
    /// Gives `output`'s screen what was made ready for it.
    fn attach(&mut self, compositor: &mut Compositor, output: OutputId, head: Self::Head);
    /// `output` is shown at another scale on the same display (its pointer's images).
    fn rescale(&mut self, _compositor: &mut Compositor, _output: OutputId, _scale_120: u32) {}
}

/// Where each output was, at which scale and mode, and which was the primary: what a mode on
/// trial goes back to.
#[derive(Clone, Debug)]
struct Layout {
    outputs: Vec<(OutputId, Rect, u32, ModeInfo)>,
}

/// A mode shown on trial.
#[derive(Clone, Debug)]
struct Trial {
    output: OutputId,
    deadline: Instant,
    before: Layout,
}

/// The displays the session shows, in the arrangement's order (the scene's: the first is the
/// primary), and a mode on trial.
#[derive(Debug)]
pub struct Displays {
    shown: Vec<Shown>,
    next_id: OutputId,
    trial: Option<Trial>,
}

impl Displays {
    /// No display shown yet; outputs are numbered from `first`.
    pub fn new(first: OutputId) -> Self {
        Self {
            shown: Vec::new(),
            next_id: first,
            trial: None,
        }
    }

    pub fn shown(&self) -> &[Shown] {
        &self.shown
    }

    /// Whether a mode is on trial, and the output it is shown on.
    pub fn on_trial(&self) -> Option<OutputId> {
        self.trial.as_ref().map(|trial| trial.output)
    }

    /// Reads the connectors and follows them (at the start, on a hotplug, coming back from
    /// another session): outputs whose display went are removed (their windows go to the
    /// primary), new displays are shown as `preferences` keep them, and the arrangement is the
    /// one kept for the set now connected, else the others' kept with the new at the right. A
    /// mode on trial goes back first. Returns the outputs added and removed.
    pub fn follow<H: Heads>(
        &mut self,
        heads: &mut H,
        compositor: &mut Compositor,
        preferences: &Preferences,
    ) -> Result<(Vec<OutputId>, Vec<OutputId>)> {
        self.revert(heads, compositor);
        let connectors = heads.connectors()?;
        let connected: Vec<&Connector> = connectors.iter().filter(|c| c.connected()).collect();
        if connected.is_empty() {
            if self.shown.iter().any(|shown| shown.connector.is_some()) {
                eprintln!("[displays] no display is connected: nothing is shown until one is");
                for shown in &mut self.shown {
                    compositor.detach_display(shown.output.id);
                    shown.connector = None;
                }
            }
            self.tell(compositor);
            return Ok((Vec::new(), Vec::new()));
        }
        // A display stays while the same one is on its connector, as it was (another monitor on
        // the port, or the same one listing other modes, is a new display).
        let stays = |shown: &Shown| {
            connected.iter().any(|connector| {
                Some(connector.id) == shown.connector
                    && connector.identity == shown.display.identity
                    && connector.modes == shown.display.modes
            })
        };
        let removed: Vec<OutputId> = self
            .shown
            .iter()
            .filter(|shown| !stays(shown))
            .map(|shown| shown.output.id)
            .collect();
        for &id in &removed {
            if let Some(shown) = self.shown.iter().find(|shown| shown.output.id == id) {
                eprintln!("[displays] output {id} ({}) is gone", shown.display.name);
            }
            compositor.remove_output(id);
            self.shown.retain(|shown| shown.output.id != id);
        }
        // New displays: built-in panels first, then the card's order.
        let mut new: Vec<&Connector> = connected
            .iter()
            .copied()
            .filter(|connector| {
                !self
                    .shown
                    .iter()
                    .any(|shown| shown.connector == Some(connector.id))
            })
            .collect();
        new.sort_by_key(|connector| !connector.internal);
        let mut added = Vec::new();
        for connector in new {
            // The first the card drives of the modes it may be shown at, best first.
            let mut shown_at = None;
            for (mode, scale_120) in choices(connector, preferences) {
                match heads.prepare(compositor, connector, &mode, scale_120) {
                    Ok(head) => {
                        shown_at = Some((mode, scale_120, head));
                        break;
                    }
                    Err(error) => eprintln!(
                        "[displays] {} at {}x{}@{:.3}: {error}",
                        connector.name,
                        mode.width,
                        mode.height,
                        f64::from(mode.refresh_mhz) / 1000.0
                    ),
                }
            }
            let Some((mode, scale_120, head)) = shown_at else {
                eprintln!(
                    "[displays] {} is not shown: none of the modes tried could be",
                    connector.name
                );
                continue;
            };
            // At the right end for now, top-aligned; arranged with the others below.
            let (width, height) = arrangement::logical((mode.width, mode.height), scale_120);
            let right = self
                .shown
                .iter()
                .map(|shown| shown.output.rect.right())
                .max()
                .unwrap_or(0);
            let top = self
                .shown
                .iter()
                .map(|shown| shown.output.rect.y)
                .min()
                .unwrap_or(0);
            let output = Output {
                id: self.next_id,
                rect: Rect::new(right as i32, top, width, height),
                scale_120,
                refresh_mhz: mode.refresh_mhz,
            };
            self.next_id += 1;
            compositor.add_output(output, Some((mode.width, mode.height)))?;
            heads.attach(compositor, output.id, head);
            self.shown.push(Shown {
                output,
                connector: Some(connector.id),
                display: connector.clone(),
                mode,
                chosen: preferences
                    .display(&connector.identity)
                    .and_then(|kept| kept.mode),
            });
            added.push(output.id);
        }
        self.arrange(compositor, preferences)?;
        for shown in &self.shown {
            if added.contains(&shown.output.id) {
                self.log(shown);
            }
        }
        self.tell(compositor);
        Ok((added, removed))
    }

    /// Arranges the outputs as `preferences` keep the set of displays shown, else where they are,
    /// those that touch none placed beside the others.
    fn arrange(&mut self, compositor: &mut Compositor, preferences: &Preferences) -> Result<()> {
        if self.shown.is_empty() {
            return Ok(());
        }
        let kept = (self.shown.len() > 1)
            .then(|| {
                preferences.arrangement(self.shown.iter().map(|s| s.display.identity.as_str()))
            })
            .flatten();
        let before: Vec<Rect> = match kept {
            Some(kept) => {
                // The kept order: the primary first.
                let order: Vec<Shown> = kept
                    .displays
                    .iter()
                    .filter_map(|(identity, _)| {
                        self.shown
                            .iter()
                            .find(|shown| shown.display.identity == *identity)
                            .cloned()
                    })
                    .collect();
                self.shown = order;
                compositor.set_primary_output(self.shown[0].output.id)?;
                self.shown
                    .iter()
                    .zip(&kept.displays)
                    .map(|(shown, (_, (x, y)))| {
                        Rect::new(*x, *y, shown.output.rect.width, shown.output.rect.height)
                    })
                    .collect()
            }
            None => self.shown.iter().map(|shown| shown.output.rect).collect(),
        };
        let sizes: Vec<(u32, u32)> = before.iter().map(|r| (r.width, r.height)).collect();
        let origins = arrangement::normalize(&arrangement::reflow(&before, &sizes));
        let layout: Vec<(Rect, u32)> = self
            .shown
            .iter()
            .zip(origins)
            .map(|(shown, (x, y))| {
                let rect = Rect::new(x, y, shown.output.rect.width, shown.output.rect.height);
                (rect, shown.output.scale_120)
            })
            .collect();
        self.apply(compositor, &layout)
    }

    /// Gives every output, in order, its rectangle and scale.
    fn apply(&mut self, compositor: &mut Compositor, layout: &[(Rect, u32)]) -> Result<()> {
        for (shown, &(rect, scale_120)) in self.shown.iter_mut().zip(layout) {
            if (shown.output.rect, shown.output.scale_120) != (rect, scale_120)
                || compositor
                    .screen(shown.output.id)
                    .is_some_and(|screen| (screen.width, screen.height) != shown.pixels())
            {
                compositor.set_output_geometry(
                    shown.output.id,
                    rect,
                    scale_120,
                    Some(shown.pixels()),
                )?;
                shown.output.rect = rect;
                shown.output.scale_120 = scale_120;
            }
        }
        Ok(())
    }

    /// The layout once output `index` is `size` (logical): the others keep their places against
    /// it, the whole normalized.
    fn reflowed(&self, index: usize, size: (u32, u32), scale_120: u32) -> Vec<(Rect, u32)> {
        let before: Vec<Rect> = self.shown.iter().map(|shown| shown.output.rect).collect();
        let sizes: Vec<(u32, u32)> = before
            .iter()
            .enumerate()
            .map(|(i, r)| {
                if i == index {
                    size
                } else {
                    (r.width, r.height)
                }
            })
            .collect();
        let origins = arrangement::normalize(&arrangement::reflow(&before, &sizes));
        self.shown
            .iter()
            .zip(origins.iter().zip(&sizes))
            .enumerate()
            .map(|(i, (shown, (&(x, y), &(width, height))))| {
                let scale = if i == index {
                    scale_120
                } else {
                    shown.output.scale_120
                };
                (Rect::new(x, y, width, height), scale)
            })
            .collect()
    }

    /// The arrangement of the displays shown, at these rectangles, the primary first: what is
    /// kept for their set (none for one display alone).
    fn arrangement_of(&self, layout: &[(Rect, u32)]) -> Option<Arrangement> {
        (self.shown.len() > 1).then(|| Arrangement {
            displays: self
                .shown
                .iter()
                .zip(layout)
                .map(|(shown, (rect, _))| (shown.display.identity.clone(), (rect.x, rect.y)))
                .collect(),
        })
    }

    fn layout(&self) -> Layout {
        Layout {
            outputs: self
                .shown
                .iter()
                .map(|shown| {
                    (
                        shown.output.id,
                        shown.output.rect,
                        shown.output.scale_120,
                        shown.mode,
                    )
                })
                .collect(),
        }
    }

    fn index(&self, output: OutputId) -> std::result::Result<usize, String> {
        self.shown
            .iter()
            .position(|shown| shown.output.id == output && shown.connector.is_some())
            .ok_or_else(|| format!("output {output} shows no display"))
    }

    /// When the mode on trial goes back unless it is kept, if one is: `serve` should run by then.
    pub fn trial_deadline(&self) -> Option<Instant> {
        self.trial.as_ref().map(|trial| trial.deadline)
    }

    /// One turn: the changes Settings asked for, each judged, applied, kept and answered
    /// (`PreferenceResult`), and a mode on trial gone back once its time is up.
    pub fn serve<H: Heads>(
        &mut self,
        heads: &mut H,
        compositor: &mut Compositor,
        settings: &mut Settings,
        now: Instant,
    ) {
        let changes = compositor.scene.take_display_changes();
        let mut changed = !changes.is_empty();
        for change in changes {
            let accepted = match self.change(heads, compositor, settings, change.request, now) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("[displays] {:?} refused: {error}", change.request);
                    false
                }
            };
            compositor
                .scene
                .preference_set(change.client, change.serial, accepted);
        }
        if self
            .trial
            .as_ref()
            .is_some_and(|trial| now >= trial.deadline)
        {
            eprintln!("[displays] the mode on trial was not kept: the display goes back");
            self.revert(heads, compositor);
            changed = true;
        }
        // Settings is told what the displays are now, only once something may have changed.
        if changed {
            self.tell(compositor);
        }
    }

    fn change<H: Heads>(
        &mut self,
        heads: &mut H,
        compositor: &mut Compositor,
        settings: &mut Settings,
        request: DisplayRequest,
        now: Instant,
    ) -> std::result::Result<(), String> {
        // While a mode is on trial, it alone is answered: kept or gone back.
        if self.trial.is_some() && !matches!(request, DisplayRequest::Confirm { .. }) {
            return Err("a mode is on trial".into());
        }
        let error = |error: Box<dyn std::error::Error + Send + Sync>| error.to_string();
        match request {
            DisplayRequest::Mode {
                output,
                width,
                height,
                refresh_mhz,
            } => {
                let index = self.index(output)?;
                let shown = &self.shown[index];
                let mode = shown
                    .display
                    .mode(width, height, refresh_mhz)
                    .ok_or("not one of the display's modes")?;
                if same(&mode, &shown.mode) {
                    return Ok(());
                }
                // At the scale its density recommends at that mode.
                let scale_120 = arrangement::scale_120(&shown.display.monitor(&mode));
                let before = self.layout();
                self.show_mode(heads, compositor, index, mode, scale_120)?;
                eprintln!(
                    "[displays] output {output} at {width}x{height}@{:.3} on trial",
                    f64::from(refresh_mhz) / 1000.0
                );
                self.trial = Some(Trial {
                    output,
                    deadline: now + TRIAL,
                    before,
                });
                compositor
                    .scene
                    .display_confirmation(output, TRIAL.as_secs() as u32);
                Ok(())
            }
            DisplayRequest::Scale { output, scale_120 } => {
                let index = self.index(output)?;
                let shown = &self.shown[index];
                if !arrangement::scale_choices(&shown.display.monitor(&shown.mode))
                    .contains(&scale_120)
                {
                    return Err("not one of the display's scales".into());
                }
                if scale_120 == shown.output.scale_120 {
                    return Ok(());
                }
                let size = arrangement::logical(shown.pixels(), scale_120);
                let layout = self.reflowed(index, size, scale_120);
                let identity = shown.display.identity.clone();
                let kept = DisplaySettings {
                    scale_120,
                    ..shown.settings()
                };
                let arrangement = self.arrangement_of(&layout);
                settings.keep_displays(|preferences| {
                    preferences.keep_display(&identity, kept);
                    if let Some(arrangement) = arrangement {
                        preferences.keep_arrangement(arrangement);
                    }
                })?;
                self.apply(compositor, &layout).map_err(error)?;
                heads.rescale(compositor, output, scale_120);
                eprintln!(
                    "[displays] output {output} at {}x scale",
                    f64::from(scale_120) / 120.0
                );
                Ok(())
            }
            DisplayRequest::Origin { output, x, y } => {
                let index = self.index(output)?;
                if self.shown.len() < 2 {
                    return Ok(());
                }
                let others: Vec<Rect> = self
                    .shown
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != index)
                    .map(|(_, shown)| shown.output.rect)
                    .collect();
                let rect = self.shown[index].output.rect;
                let at = arrangement::snap(&others, (rect.width, rect.height), (x, y));
                let origins: Vec<(i32, i32)> = self
                    .shown
                    .iter()
                    .enumerate()
                    .map(|(i, shown)| {
                        if i == index {
                            at
                        } else {
                            (shown.output.rect.x, shown.output.rect.y)
                        }
                    })
                    .collect();
                let layout: Vec<(Rect, u32)> = self
                    .shown
                    .iter()
                    .zip(arrangement::normalize(&origins))
                    .map(|(shown, (x, y))| {
                        let rect = shown.output.rect;
                        (
                            Rect::new(x, y, rect.width, rect.height),
                            shown.output.scale_120,
                        )
                    })
                    .collect();
                let arrangement = self.arrangement_of(&layout);
                settings.keep_displays(|preferences| {
                    if let Some(arrangement) = arrangement {
                        preferences.keep_arrangement(arrangement);
                    }
                })?;
                self.apply(compositor, &layout).map_err(error)
            }
            DisplayRequest::Primary { output } => {
                let index = self.index(output)?;
                if index == 0 {
                    return Ok(());
                }
                let mut order = self.shown.clone();
                let primary = order.remove(index);
                order.insert(0, primary);
                let arrangement = Arrangement {
                    displays: order
                        .iter()
                        .map(|shown| {
                            let rect = shown.output.rect;
                            (shown.display.identity.clone(), (rect.x, rect.y))
                        })
                        .collect(),
                };
                settings.keep_displays(|preferences| preferences.keep_arrangement(arrangement))?;
                compositor.set_primary_output(output).map_err(error)?;
                self.shown = order;
                eprintln!("[displays] output {output} is the primary");
                Ok(())
            }
            DisplayRequest::Confirm { keep } => {
                let Some(trial) = self.trial.take() else {
                    return Err("no mode is on trial".into());
                };
                if !keep {
                    self.trial = Some(trial);
                    self.revert(heads, compositor);
                    return Ok(());
                }
                let index = self.index(trial.output)?;
                // The mode on trial is the person's choice now, kept as chosen.
                let before = self.shown[index].chosen;
                self.shown[index].chosen = Some(display_mode(&self.shown[index].mode));
                let shown = &self.shown[index];
                let (identity, kept) = (shown.display.identity.clone(), shown.settings());
                let layout: Vec<(Rect, u32)> = self
                    .shown
                    .iter()
                    .map(|shown| (shown.output.rect, shown.output.scale_120))
                    .collect();
                let arrangement = self.arrangement_of(&layout);
                let output = trial.output;
                if let Err(error) = settings.keep_displays(|preferences| {
                    preferences.keep_display(&identity, kept);
                    if let Some(arrangement) = arrangement {
                        preferences.keep_arrangement(arrangement);
                    }
                }) {
                    // Not kept, so not shown: the file and the session never disagree.
                    self.shown[index].chosen = before;
                    self.trial = Some(trial);
                    self.revert(heads, compositor);
                    return Err(error);
                }
                eprintln!("[displays] output {output}'s mode is kept");
                compositor.scene.display_confirmation(output, 0);
                Ok(())
            }
        }
    }

    /// Shows output `index` at `mode` and `scale_120`: its display made ready again at that mode
    /// (if the card refuses it, the display shows what it did and the change is refused), and the
    /// others kept where they are against it.
    fn show_mode<H: Heads>(
        &mut self,
        heads: &mut H,
        compositor: &mut Compositor,
        index: usize,
        mode: ModeInfo,
        scale_120: u32,
    ) -> std::result::Result<(), String> {
        let shown = self.shown[index].clone();
        let id = shown.output.id;
        compositor.detach_display(id);
        let head = match heads.prepare(compositor, &shown.display, &mode, scale_120) {
            Ok(head) => head,
            Err(error) => {
                match heads.prepare(
                    compositor,
                    &shown.display,
                    &shown.mode,
                    shown.output.scale_120,
                ) {
                    Ok(head) => heads.attach(compositor, id, head),
                    Err(again) => eprintln!(
                        "[displays] output {id} cannot be shown again at its mode: {again}"
                    ),
                }
                return Err(error);
            }
        };
        let size = arrangement::logical((mode.width, mode.height), scale_120);
        let layout = self.reflowed(index, size, scale_120);
        self.shown[index].mode = mode;
        self.shown[index].output.refresh_mhz = mode.refresh_mhz;
        // Its frames are paced by its new refresh, and every surface on it is told it.
        let applied = self
            .apply(compositor, &layout)
            .and_then(|()| compositor.set_output_refresh(id, mode.refresh_mhz));
        heads.attach(compositor, id, head);
        applied.map_err(|error| error.to_string())
    }

    /// A mode on trial goes back: its display at the mode it showed, and every output where it
    /// was, at its scale, the primary as it was.
    fn revert<H: Heads>(&mut self, heads: &mut H, compositor: &mut Compositor) {
        let Some(trial) = self.trial.take() else {
            return;
        };
        compositor.scene.display_confirmation(trial.output, 0);
        let Ok(index) = self.index(trial.output) else {
            return;
        };
        let Some(&(_, _, scale_120, mode)) = trial
            .before
            .outputs
            .iter()
            .find(|(id, ..)| *id == trial.output)
        else {
            return;
        };
        if !same(&mode, &self.shown[index].mode) {
            if let Err(error) = self.show_mode(heads, compositor, index, mode, scale_120) {
                eprintln!("[displays] output {} going back: {error}", trial.output);
            }
        }
        let layout: Vec<(Rect, u32)> = self
            .shown
            .iter()
            .map(|shown| {
                trial
                    .before
                    .outputs
                    .iter()
                    .find(|(id, ..)| *id == shown.output.id)
                    .map_or((shown.output.rect, shown.output.scale_120), |b| (b.1, b.2))
            })
            .collect();
        if let Err(error) = self.apply(compositor, &layout) {
            eprintln!("[displays] going back: {error}");
        }
        eprintln!("[displays] output {} went back", trial.output);
    }

    /// Tells the scene what each display is, for Settings.
    fn tell(&self, compositor: &mut Compositor) {
        let infos = self
            .shown
            .iter()
            .filter(|shown| shown.connector.is_some())
            .map(info)
            .collect();
        compositor.scene.set_displays(infos);
    }

    fn log(&self, shown: &Shown) {
        let output = shown.output;
        let variable = match (shown.display.vrr_capable, shown.display.refresh_range_hz) {
            (true, Some((least, most))) => format!(", variable refresh {least} to {most} Hz"),
            (true, None) => ", variable refresh".to_string(),
            (false, _) => String::new(),
        };
        eprintln!(
            "[displays] output {} on {} ({}): {}x{}@{:.3} ({}){variable} at {}x scale, {}x{} at ({}, {}){}",
            output.id,
            shown.display.name,
            shown.display.identity,
            shown.mode.width,
            shown.mode.height,
            f64::from(shown.mode.refresh_mhz) / 1000.0,
            if shown.at_chosen() { "chosen" } else { "its own" },
            f64::from(output.scale_120) / 120.0,
            output.rect.width,
            output.rect.height,
            output.rect.x,
            output.rect.y,
            if self
                .shown
                .first()
                .is_some_and(|first| first.output.id == output.id)
            {
                ", the primary"
            } else {
                ""
            }
        );
    }
}

/// The modes a display may be shown at, best first, each with its scale: the mode the person chose
/// for it, if it still has that mode; then its own (`own_modes`: its native resolution, the fastest
/// refresh first), for the card to take the first it drives. The scale kept with a mode, at that
/// mode (the chosen one, or its own when none was chosen), if it is still one of its choices there;
/// otherwise the one its density recommends.
fn choices(connector: &Connector, preferences: &Preferences) -> Vec<(ModeInfo, u32)> {
    let kept = preferences.display(&connector.identity);
    let scale = |mode: &ModeInfo, kept_with_it: bool| {
        let monitor = connector.monitor(mode);
        match kept {
            Some(kept)
                if kept_with_it
                    && arrangement::scale_choices(&monitor).contains(&kept.scale_120) =>
            {
                kept.scale_120
            }
            _ => arrangement::scale_120(&monitor),
        }
    };
    let chosen = kept.and_then(|kept| kept.mode);
    let shown_chosen =
        chosen.and_then(|mode| connector.mode(mode.width, mode.height, mode.refresh_mhz));
    let mut choices: Vec<(ModeInfo, u32)> = shown_chosen
        .iter()
        .map(|mode| (*mode, scale(mode, true)))
        .collect();
    for mode in connector.own_modes() {
        if shown_chosen.is_none_or(|chosen| !same(&chosen, &mode)) {
            choices.push((mode, scale(&mode, chosen.is_none())));
        }
    }
    choices
}

/// What Settings is told of a display: its name, its modes (the largest first, at most
/// MAX_DISPLAY_MODES, its own among them) and its scale choices at its mode.
fn info(shown: &Shown) -> DisplayInfo {
    let display = &shown.display;
    let mut modes = display.modes.clone();
    modes.sort_by(|a, b| {
        (b.width, b.height, b.refresh_mhz).cmp(&(a.width, a.height, a.refresh_mhz))
    });
    modes.truncate(MAX_DISPLAY_MODES);
    if !modes.iter().any(|mode| same(mode, &shown.mode)) {
        modes.pop();
        modes.push(shown.mode);
    }
    let mode = modes
        .iter()
        .position(|mode| same(mode, &shown.mode))
        .unwrap_or(0) as u32;
    let monitor = display.monitor(&shown.mode);
    DisplayInfo {
        output: shown.output.id,
        name: display
            .product
            .clone()
            .unwrap_or_else(|| display.name.clone()),
        builtin: display.internal,
        size_mm: display.size_mm,
        modes,
        mode,
        scales: arrangement::scale_choices(&monitor),
        recommended: arrangement::scale_120(&monitor),
    }
}

#[cfg(test)]
mod tests;
