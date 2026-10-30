use super::*;
use oxyos_compositor_core::{ClientId, Grants, Scene};
use oxyos_protocol::{Event, Request};
use std::path::PathBuf;

const SETTINGS: ClientId = 2;
const PANEL: &str = "BOE-0A4B-00000001";
const MONITOR: &str = "DEL-A0B4-7Y9K2N3";

fn mode(width: u32, height: u32, refresh_mhz: u32, preferred: bool) -> ModeInfo {
    ModeInfo {
        width,
        height,
        refresh_mhz,
        preferred,
    }
}

fn connector(
    id: u32,
    name: &str,
    identity: &str,
    internal: bool,
    modes: Vec<ModeInfo>,
) -> Connector {
    Connector {
        id,
        name: name.into(),
        internal,
        identity: identity.into(),
        product: (!internal).then(|| format!("{identity} monitor")),
        size_mm: match (internal, modes.first().map(|m| m.width)) {
            (true, _) => (301, 188),
            (false, Some(3840)) => (597, 336),
            _ => (531, 299),
        },
        modes,
        vrr_capable: false,
        refresh_range_hz: None,
    }
}

fn panel() -> Connector {
    connector(
        247,
        "eDP-1",
        PANEL,
        true,
        vec![
            mode(1920, 1200, 60_000, true),
            mode(1280, 800, 60_000, false),
        ],
    )
}

fn monitor() -> Connector {
    connector(
        236,
        "DP-1",
        MONITOR,
        false,
        vec![
            mode(3840, 2160, 60_000, true),
            mode(2560, 1440, 59_951, false),
            mode(1920, 1080, 60_000, false),
        ],
    )
}

fn hdmi(identity: &str) -> Connector {
    connector(
        262,
        "HDMI-A-1",
        identity,
        false,
        vec![mode(1920, 1080, 60_000, true)],
    )
}

/// The connectors of a ThinkPad P14s: DP-1, the panel, DP-2 and HDMI, in the card's order,
/// those not given disconnected.
fn thinkpad(dp1: Option<Connector>, hdmi: Option<Connector>) -> Vec<Connector> {
    let dark = |id: u32, name: &str| connector(id, name, name, false, Vec::new());
    vec![
        dp1.unwrap_or_else(|| dark(236, "DP-1")),
        panel(),
        dark(255, "DP-2"),
        hdmi.unwrap_or_else(|| dark(262, "HDMI-A-1")),
    ]
}

/// The program's side, played: connectors as given; a mode the card refuses on a connector.
#[derive(Default)]
struct Fake {
    connectors: Vec<Connector>,
    refused: Vec<(u32, ModeInfo)>,
    prepared: Vec<(u32, (u32, u32), u32)>,
}

impl Heads for Fake {
    type Head = ();

    fn connectors(&mut self) -> Result<Vec<Connector>> {
        Ok(self.connectors.clone())
    }

    fn prepare(
        &mut self,
        _compositor: &mut Compositor,
        connector: &Connector,
        mode: &ModeInfo,
        scale_120: u32,
    ) -> std::result::Result<(), String> {
        self.prepared
            .push((connector.id, (mode.width, mode.height), scale_120));
        if self
            .refused
            .iter()
            .any(|(id, refused)| *id == connector.id && same(refused, mode))
        {
            return Err("the card refuses the mode".into());
        }
        Ok(())
    }

    fn attach(&mut self, _compositor: &mut Compositor, _output: OutputId, _head: ()) {}
}

fn compositor() -> Compositor {
    let mut scene = Scene::new();
    scene
        .connect(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    Compositor::new(scene, 0)
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "oxyos-compositor-displays-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

/// Each output in the scene's order: its id, rectangle and scale.
fn outputs(compositor: &Compositor) -> Vec<(OutputId, Rect, u32)> {
    compositor
        .scene
        .outputs()
        .iter()
        .map(|output| (output.id, output.rect, output.scale_120))
        .collect()
}

fn ask(compositor: &mut Compositor, request: Request) {
    compositor.scene.handle(SETTINGS, request).unwrap();
}

/// What Settings was told: (serial, accepted) of each answer, and (output, seconds) of each mode on
/// trial.
type Answers = (Vec<(u32, bool)>, Vec<(OutputId, u32)>);

/// What Settings was told since the last call.
fn answers(compositor: &mut Compositor) -> Answers {
    let (mut results, mut trials) = (Vec::new(), Vec::new());
    for (to, event) in compositor.scene.drain_events() {
        match event {
            Event::PreferenceResult { serial, accepted } if to == SETTINGS => {
                results.push((serial, accepted))
            }
            Event::DisplayConfirmation { output, seconds } if to == SETTINGS => {
                trials.push((output, seconds))
            }
            _ => {}
        }
    }
    (results, trials)
}

#[test]
fn at_the_start_the_panel_is_first_and_the_monitors_follow_in_the_cards_order() {
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), Some(hdmi("GSM-5B7F-HDMI-A-1"))),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    let (added, removed) = displays
        .follow(&mut heads, &mut compositor, &Preferences::default())
        .unwrap();
    assert_eq!((added, removed), (vec![1, 2, 3], vec![]));
    assert_eq!(
        outputs(&compositor),
        [
            // The panel at 1.25: 1536x960, the primary.
            (1, Rect::new(0, 0, 1536, 960), 150),
            // The 27-inch 4K monitor at 1.5, then the 1080p one at 1.
            (2, Rect::new(1536, 0, 2560, 1440), 180),
            (3, Rect::new(4096, 0, 1920, 1080), 120),
        ]
    );
    // Each screen composes its display's pixels.
    assert_eq!(
        compositor
            .screens
            .iter()
            .map(|screen| (screen.width, screen.height))
            .collect::<Vec<_>>(),
        [(1920, 1200), (3840, 2160), (1920, 1080)]
    );
}

#[test]
fn a_monitor_plugged_in_comes_at_the_right_and_one_unplugged_closes_the_gap() {
    let mut compositor = compositor();
    let none = Preferences::default();
    let mut heads = Fake {
        connectors: thinkpad(None, None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays.follow(&mut heads, &mut compositor, &none).unwrap();
    // A 1080p monitor on HDMI: right of the panel.
    heads.connectors = thinkpad(None, Some(hdmi("GSM-5B7F-HDMI-A-1")));
    assert_eq!(
        displays.follow(&mut heads, &mut compositor, &none).unwrap(),
        (vec![2], vec![])
    );
    assert_eq!(
        outputs(&compositor)[1],
        (2, Rect::new(1536, 0, 1920, 1080), 120)
    );
    // Then a 4K one on DP-1: right of both, whatever the card's order.
    heads.connectors = thinkpad(Some(monitor()), Some(hdmi("GSM-5B7F-HDMI-A-1")));
    displays.follow(&mut heads, &mut compositor, &none).unwrap();
    assert_eq!(
        outputs(&compositor)[2],
        (3, Rect::new(3456, 0, 2560, 1440), 180)
    );
    // Nothing changed: nothing to do.
    assert_eq!(
        displays.follow(&mut heads, &mut compositor, &none).unwrap(),
        (vec![], vec![])
    );
    // The HDMI one unplugged: it goes, and the 4K one comes back beside the panel.
    heads.connectors = thinkpad(Some(monitor()), None);
    assert_eq!(
        displays.follow(&mut heads, &mut compositor, &none).unwrap(),
        (vec![], vec![2])
    );
    assert_eq!(
        outputs(&compositor),
        [
            (1, Rect::new(0, 0, 1536, 960), 150),
            (3, Rect::new(1536, 0, 2560, 1440), 180)
        ]
    );
}

#[test]
fn another_monitor_on_the_same_port_is_a_new_display() {
    let mut compositor = compositor();
    let none = Preferences::default();
    let mut heads = Fake {
        connectors: thinkpad(None, Some(hdmi("GSM-5B7F-HDMI-A-1"))),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays.follow(&mut heads, &mut compositor, &none).unwrap();
    heads.connectors = thinkpad(None, Some(hdmi("ACR-0001-12345678")));
    assert_eq!(
        displays.follow(&mut heads, &mut compositor, &none).unwrap(),
        (vec![3], vec![2])
    );
    assert_eq!(displays.shown()[1].display.identity, "ACR-0001-12345678");
}

#[test]
fn with_nothing_connected_the_outputs_stay_dark_until_a_display_comes() {
    let mut compositor = compositor();
    let none = Preferences::default();
    let only = |connector: Option<Connector>| {
        vec![connector.unwrap_or_else(|| self::connector(262, "HDMI-A-1", "x", false, vec![]))]
    };
    let mut heads = Fake {
        connectors: only(Some(hdmi("GSM-5B7F-HDMI-A-1"))),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays.follow(&mut heads, &mut compositor, &none).unwrap();
    heads.connectors = only(None);
    assert_eq!(
        displays.follow(&mut heads, &mut compositor, &none).unwrap(),
        (vec![], vec![])
    );
    // Dark, it is on no connector, and stays so; Settings is told of no display.
    assert_eq!(displays.shown()[0].connector, None);
    assert_eq!(outputs(&compositor).len(), 1);
    // Plugged in again, an output for it takes the dark one's place (and its windows).
    heads.connectors = only(Some(hdmi("GSM-5B7F-HDMI-A-1")));
    assert_eq!(
        displays.follow(&mut heads, &mut compositor, &none).unwrap(),
        (vec![2], vec![1])
    );
    assert_eq!(
        outputs(&compositor),
        [(2, Rect::new(0, 0, 1920, 1080), 120)]
    );
}

#[test]
fn what_is_kept_for_the_displays_is_how_they_are_shown_at_the_next_start() {
    let mut preferences = Preferences::default();
    preferences.keep_display(
        MONITOR,
        DisplaySettings {
            mode: Some(DisplayMode {
                width: 2560,
                height: 1440,
                refresh_mhz: 59_951,
            }),
            scale_120: 120,
        },
    );
    // The monitor the primary, the panel right of it, bottoms aligned.
    preferences.keep_arrangement(Arrangement {
        displays: vec![(MONITOR.into(), (0, 0)), (PANEL.into(), (2560, 480))],
    });
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, &preferences)
        .unwrap();
    assert_eq!(
        outputs(&compositor),
        [
            (2, Rect::new(0, 0, 2560, 1440), 120),
            (1, Rect::new(2560, 480, 1536, 960), 150),
        ]
    );
    assert_eq!(compositor.screens[0].id, 2);
    assert_eq!(
        (compositor.screens[0].width, compositor.screens[0].height),
        (2560, 1440)
    );
    // A kept mode the display no longer has: its own, at the scale its density recommends (the
    // one kept went with the mode).
    let mut gone = preferences.clone();
    gone.keep_display(
        MONITOR,
        DisplaySettings {
            mode: Some(DisplayMode {
                width: 5120,
                height: 2880,
                refresh_mhz: 60_000,
            }),
            scale_120: 240,
        },
    );
    assert_eq!(
        choices(&monitor(), &gone),
        [(mode(3840, 2160, 60_000, true), 180)]
    );
}

/// A 27-inch 1440p monitor of 60, 120 and 144 Hz (its preferred mode the 60 Hz one, as many
/// say), and 1080p at 60 and 144.
fn fast_monitor() -> Connector {
    connector(
        236,
        "DP-1",
        MONITOR,
        false,
        vec![
            mode(2560, 1440, 59_951, true),
            mode(2560, 1440, 119_998, false),
            mode(2560, 1440, 143_998, false),
            mode(1920, 1080, 60_000, false),
            mode(1920, 1080, 144_001, false),
        ],
    )
}

/// The refresh each output is driven at, in the scene's order: what its frames are paced by and
/// what its surfaces are told.
fn refreshes(compositor: &Compositor) -> Vec<(OutputId, u32)> {
    compositor
        .scene
        .outputs()
        .iter()
        .map(|output| (output.id, output.refresh_mhz))
        .collect()
}

#[test]
fn a_display_is_driven_at_its_native_resolution_and_fastest_refresh() {
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(fast_monitor()), None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, &Preferences::default())
        .unwrap();
    // The panel at its only 60 Hz; the monitor at 2560x1440, its preferred pixels, at 144 Hz, not
    // its preferred 60 nor 1080p's faster 144.001.
    assert_eq!(refreshes(&compositor), [(1, 60_000), (2, 143_998)]);
    assert_eq!(displays.shown()[1].mode, mode(2560, 1440, 143_998, false));
    assert_eq!(displays.shown()[1].chosen, None);
    let screen = compositor.screen(2).unwrap();
    assert_eq!(
        (screen.width, screen.height, screen.refresh_mhz),
        (2560, 1440, 143_998)
    );
    // Its frames are 6.94 ms apart; the fastest display's is what a frame's work must fit in.
    assert_eq!(screen.refresh_period(), Duration::from_nanos(6_944_540));
    assert_eq!(
        compositor.fastest_refresh(),
        Some(Duration::from_nanos(6_944_540))
    );
}

#[test]
fn a_rate_the_card_refuses_falls_back_to_the_next_fastest() {
    // A dock's link without the bandwidth for 1440p at 144 Hz: the card's atomic test refuses it.
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(fast_monitor()), None),
        refused: vec![(236, mode(2560, 1440, 143_998, false))],
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, &Preferences::default())
        .unwrap();
    assert_eq!(refreshes(&compositor), [(1, 60_000), (2, 119_998)]);
    // Tried at 144 first, then shown at 120, at the same pixels and scale.
    let tried: Vec<_> = heads
        .prepared
        .iter()
        .filter(|(id, ..)| *id == 236)
        .collect();
    assert_eq!(
        tried,
        [&(236, (2560, 1440), 120), &(236, (2560, 1440), 120)]
    );
    // Refusing every rate of its native resolution: not shown.
    let mut compositor = self::compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(fast_monitor()), None),
        refused: [59_951, 119_998, 143_998]
            .iter()
            .map(|&mhz| (236, mode(2560, 1440, mhz, false)))
            .collect(),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, &Preferences::default())
        .unwrap();
    assert_eq!(refreshes(&compositor), [(1, 60_000)]);
}

#[test]
fn a_rate_chosen_is_kept_and_wins_and_a_scale_alone_leaves_the_rate_the_displays_own() {
    let directory = scratch("rate");
    let file = directory.join("preferences");
    let mut settings = Settings::load(Some(file.clone()), None);
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(fast_monitor()), None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    // A scale chosen on the monitor driven at its fastest: kept with no mode.
    ask(
        &mut compositor,
        Request::SetDisplayScale {
            serial: 1,
            output: 2,
            scale_120: 150,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, Instant::now());
    assert_eq!(answers(&mut compositor).0, [(1, true)]);
    let kept = oxyos_preferences::Preferences::load(&file);
    assert_eq!(kept.display(MONITOR).unwrap().text(), "default 1.25");
    // 60 Hz chosen in Settings (the same pixels: nothing moves) and kept.
    let start = Instant::now();
    ask(
        &mut compositor,
        Request::SetDisplayMode {
            serial: 2,
            output: 2,
            width: 2560,
            height: 1440,
            refresh_mhz: 59_951,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, start);
    let rect = outputs(&compositor)[1].1;
    // Shown at once, paced at 60 Hz: the scene's output, its screen and what its surfaces are told.
    assert_eq!(refreshes(&compositor), [(1, 60_000), (2, 59_951)]);
    assert_eq!(compositor.screen(2).unwrap().refresh_mhz, 59_951);
    ask(
        &mut compositor,
        Request::ConfirmDisplays {
            serial: 3,
            keep: true,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, start);
    assert_eq!(
        answers(&mut compositor),
        (vec![(2, true), (3, true)], vec![(2, 15), (2, 0)])
    );
    let kept = oxyos_preferences::Preferences::load(&file);
    assert_eq!(kept.display(MONITOR).unwrap().text(), "2560x1440@59951 1");
    // At the next start the 60 Hz chosen wins over its fastest; then a scale keeps it chosen.
    let mut compositor = self::compositor();
    let mut displays = Displays::new(1);
    let mut settings = Settings::load(Some(file.clone()), None);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    assert_eq!(refreshes(&compositor), [(1, 60_000), (2, 59_951)]);
    assert_eq!(outputs(&compositor)[1].1, rect);
    ask(
        &mut compositor,
        Request::SetDisplayScale {
            serial: 4,
            output: 2,
            scale_120: 150,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, Instant::now());
    let kept = oxyos_preferences::Preferences::load(&file);
    assert_eq!(
        kept.display(MONITOR).unwrap().text(),
        "2560x1440@59951 1.25"
    );
    // On a link that cannot take the rate chosen, its own fastest the link takes, at the scale its
    // density recommends; the choice stays kept for a link that can.
    let mut heads = Fake {
        connectors: thinkpad(Some(fast_monitor()), None),
        refused: vec![(236, mode(2560, 1440, 59_951, true))],
        ..Fake::default()
    };
    let mut compositor = self::compositor();
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    assert_eq!(refreshes(&compositor), [(1, 60_000), (2, 143_998)]);
    assert_eq!(outputs(&compositor)[1].2, 120);
    assert_eq!(
        displays.shown()[1].chosen,
        Some(DisplayMode {
            width: 2560,
            height: 1440,
            refresh_mhz: 59_951
        })
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_scale_is_applied_kept_and_the_others_stay_against_it() {
    let directory = scratch("scale");
    let file = directory.join("preferences");
    let mut settings = Settings::load(Some(file.clone()), None);
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    let now = Instant::now();
    // More space on the panel: 1 (1920x1200); the monitor moves right with its edge.
    ask(
        &mut compositor,
        Request::SetDisplayScale {
            serial: 1,
            output: 1,
            scale_120: 120,
        },
    );
    // Not one of its choices: refused.
    ask(
        &mut compositor,
        Request::SetDisplayScale {
            serial: 2,
            output: 1,
            scale_120: 270,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, now);
    assert_eq!(answers(&mut compositor).0, [(1, true), (2, false)]);
    assert_eq!(
        outputs(&compositor),
        [
            (1, Rect::new(0, 0, 1920, 1200), 120),
            (2, Rect::new(1920, 0, 2560, 1440), 180),
        ]
    );
    let kept = oxyos_preferences::Preferences::load(&file);
    // Its scale alone: its mode is still its own, not one chosen.
    assert_eq!(kept.display(PANEL).unwrap().text(), "default 1");
    assert_eq!(
        kept.arrangement([PANEL, MONITOR]).unwrap().displays,
        [
            (PANEL.to_string(), (0, 0)),
            (MONITOR.to_string(), (1920, 0))
        ]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_mode_is_on_trial_and_goes_back_unless_it_is_kept() {
    let directory = scratch("trial");
    let file = directory.join("preferences");
    let mut settings = Settings::load(Some(file.clone()), None);
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    let start = Instant::now();
    let before = outputs(&compositor);
    let to_1080 = |serial| Request::SetDisplayMode {
        serial,
        output: 2,
        width: 1920,
        height: 1080,
        refresh_mhz: 60_000,
    };
    ask(&mut compositor, to_1080(1));
    displays.serve(&mut heads, &mut compositor, &mut settings, start);
    // Shown at once, at the scale it recommends there (1: a 27-inch 1080p display), on trial for
    // 15 seconds.
    assert_eq!(answers(&mut compositor), (vec![(1, true)], vec![(2, 15)]));
    assert_eq!(
        outputs(&compositor)[1],
        (2, Rect::new(1536, 0, 1920, 1080), 120)
    );
    assert_eq!(compositor.screens[1].width, 1920);
    // Nothing else is taken meanwhile.
    ask(
        &mut compositor,
        Request::SetPrimaryDisplay {
            serial: 2,
            output: 2,
        },
    );
    displays.serve(
        &mut heads,
        &mut compositor,
        &mut settings,
        start + TRIAL / 2,
    );
    assert_eq!(answers(&mut compositor), (vec![(2, false)], vec![]));
    // Not kept in time: it goes back, as everything was.
    displays.serve(&mut heads, &mut compositor, &mut settings, start + TRIAL);
    assert_eq!(answers(&mut compositor).1, [(2, 0)]);
    assert_eq!(outputs(&compositor), before);
    assert_eq!(compositor.screens[1].width, 3840);
    assert!(!file.exists(), "nothing was kept");
    // Again, and kept: the file holds it.
    ask(&mut compositor, to_1080(3));
    displays.serve(
        &mut heads,
        &mut compositor,
        &mut settings,
        start + TRIAL * 2,
    );
    ask(
        &mut compositor,
        Request::ConfirmDisplays {
            serial: 4,
            keep: true,
        },
    );
    displays.serve(
        &mut heads,
        &mut compositor,
        &mut settings,
        start + TRIAL * 2,
    );
    assert_eq!(
        answers(&mut compositor),
        (vec![(3, true), (4, true)], vec![(2, 15), (2, 0)])
    );
    let kept = oxyos_preferences::Preferences::load(&file);
    assert_eq!(kept.display(MONITOR).unwrap().text(), "1920x1080@60000 1");
    // Long after, it stays.
    displays.serve(
        &mut heads,
        &mut compositor,
        &mut settings,
        start + TRIAL * 4,
    );
    assert_eq!(outputs(&compositor)[1].1.width, 1920);
    // Keeping with nothing on trial is refused; going back at once is Revert's.
    ask(&mut compositor, to_1080(5));
    ask(
        &mut compositor,
        Request::ConfirmDisplays {
            serial: 6,
            keep: true,
        },
    );
    displays.serve(
        &mut heads,
        &mut compositor,
        &mut settings,
        start + TRIAL * 4,
    );
    assert_eq!(answers(&mut compositor).0, [(5, true), (6, false)]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_mode_the_card_refuses_is_refused_and_the_display_shows_what_it_did() {
    let mut settings = Settings::new(Preferences::default(), None, None);
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), None),
        refused: vec![(236, mode(2560, 1440, 59_951, false))],
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    let before = outputs(&compositor);
    heads.prepared.clear();
    ask(
        &mut compositor,
        Request::SetDisplayMode {
            serial: 1,
            output: 2,
            width: 2560,
            height: 1440,
            refresh_mhz: 59_951,
        },
    );
    // A mode it does not list at all.
    ask(
        &mut compositor,
        Request::SetDisplayMode {
            serial: 2,
            output: 2,
            width: 1024,
            height: 768,
            refresh_mhz: 60_000,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, Instant::now());
    assert_eq!(
        answers(&mut compositor),
        (vec![(1, false), (2, false)], vec![])
    );
    assert_eq!(outputs(&compositor), before);
    // Tried, then made ready again at its own mode.
    assert_eq!(
        heads.prepared,
        [(236, (2560, 1440), 120), (236, (3840, 2160), 180)]
    );
}

#[test]
fn a_display_dragged_goes_beside_the_other_and_the_primary_is_kept() {
    let directory = scratch("arrangement");
    let file = directory.join("preferences");
    let mut settings = Settings::load(Some(file.clone()), None);
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    // The monitor dropped above the panel, centred on it: the arrangement's corner is the
    // monitor's now.
    ask(
        &mut compositor,
        Request::SetDisplayOrigin {
            serial: 1,
            output: 2,
            x: -500,
            y: -1400,
        },
    );
    ask(
        &mut compositor,
        Request::SetPrimaryDisplay {
            serial: 2,
            output: 2,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, Instant::now());
    assert_eq!(answers(&mut compositor).0, [(1, true), (2, true)]);
    assert_eq!(
        outputs(&compositor),
        [
            (2, Rect::new(0, 0, 2560, 1440), 180),
            (1, Rect::new(512, 1440, 1536, 960), 150),
        ]
    );
    let kept = oxyos_preferences::Preferences::load(&file);
    assert_eq!(
        kept.arrangement([PANEL, MONITOR]).unwrap().displays,
        [
            (MONITOR.to_string(), (0, 0)),
            (PANEL.to_string(), (512, 1440))
        ]
    );
    // Started again with them: the same arrangement.
    let mut again = super::super::Compositor::new(Scene::new(), 0);
    Displays::new(1)
        .follow(&mut heads, &mut again, &kept)
        .unwrap();
    assert_eq!(outputs(&again), outputs(&compositor));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_change_that_cannot_be_kept_changes_nothing() {
    let directory = scratch("unwritable");
    std::fs::create_dir_all(&directory).unwrap();
    // Its directory is a file: nothing can be written there.
    std::fs::write(directory.join("oxyos"), "").unwrap();
    let mut settings = Settings::load(Some(directory.join("oxyos/preferences")), None);
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(None, None),
        ..Fake::default()
    };
    let mut displays = Displays::new(1);
    displays
        .follow(&mut heads, &mut compositor, settings.preferences())
        .unwrap();
    ask(
        &mut compositor,
        Request::SetDisplayScale {
            serial: 1,
            output: 1,
            scale_120: 180,
        },
    );
    displays.serve(&mut heads, &mut compositor, &mut settings, Instant::now());
    assert_eq!(answers(&mut compositor).0, [(1, false)]);
    assert_eq!(outputs(&compositor), [(1, Rect::new(0, 0, 1536, 960), 150)]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn settings_is_told_each_display_with_its_modes_and_scales() {
    let mut compositor = compositor();
    let mut heads = Fake {
        connectors: thinkpad(Some(monitor()), None),
        ..Fake::default()
    };
    Displays::new(1)
        .follow(&mut heads, &mut compositor, &Preferences::default())
        .unwrap();
    let told: Vec<Event> = compositor
        .scene
        .drain_events()
        .into_iter()
        .filter(|(to, _)| *to == SETTINGS)
        .map(|(_, event)| event)
        .collect();
    assert!(told.contains(&Event::Display {
        output: 2,
        rect: Rect::new(1536, 0, 2560, 1440),
        scale_120: 180,
        mode: 0,
        width_mm: 597,
        height_mm: 336,
        primary: false,
        builtin: false,
    }));
    let scales: Vec<(u32, bool)> = told
        .iter()
        .filter_map(|event| match event {
            Event::DisplayScale {
                output: 1,
                scale_120,
                recommended,
                ..
            } => Some((*scale_120, *recommended)),
            _ => None,
        })
        .collect();
    assert_eq!(
        scales,
        [(120, false), (150, true), (180, false), (210, false)]
    );
}
