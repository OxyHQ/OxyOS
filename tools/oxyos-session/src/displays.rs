//! The session's displays (docs/adr/0018-several-displays.mdx): every connected connector of its
//! card is an output, arranged left to right, top-aligned, the built-in panel first (the
//! primary), each at its preferred mode and at a scale from its density. When the connectors
//! change (a monitor plugged in or out), `plan` says which outputs go, where those that stay
//! move to close the gaps, and which come, at the right end.

use oxyos_compositor::arrangement::{self, Monitor};
use oxyos_compositor_core::{Output, OutputId};
use oxyos_protocol::Rect;

/// A connector as the card reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connector {
    /// Its object id (`oxyos_kms::Output::id`).
    pub id: u32,
    pub name: String,
    pub internal: bool,
    /// Its preferred mode, if it is connected and has one: pixels and refresh in Hz.
    pub mode: Option<(u32, u32, u32)>,
    pub size_mm: (u32, u32),
}

impl Connector {
    pub fn of(output: &oxyos_kms::Output) -> Self {
        Self {
            id: output.id,
            name: output.name.clone(),
            internal: output.internal,
            mode: output
                .connected
                .then(|| output.preferred_mode())
                .flatten()
                .map(|mode| {
                    (
                        u32::from(mode.width),
                        u32::from(mode.height),
                        mode.refresh_hz,
                    )
                }),
            size_mm: output.size_mm,
        }
    }

    fn monitor(&self, (width, height, _): (u32, u32, u32)) -> Monitor {
        Monitor {
            internal: self.internal,
            pixels: (width, height),
            size_mm: self.size_mm,
        }
    }
}

/// An output the session shows: the connector it is on (none once it went dark) and the mode it
/// was set to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub output: Output,
    pub connector: Option<u32>,
    pub mode: (u32, u32, u32),
}

/// What reading the connectors again asks for, applied in this order: outputs removed, those
/// left moved, outputs added (each with its connector and mode). `dark`: no connector is
/// connected; the outputs stay, undisplayed and on no connector, until one is, and an output for
/// it takes their windows (the scene keeps its windows on some output).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub remove: Vec<OutputId>,
    pub dark: bool,
    pub moves: Vec<(OutputId, (i32, i32))>,
    pub add: Vec<Shown>,
}

/// From what is shown (in the arrangement's order) and the connectors now (in the card's
/// order): an output goes when its connector is no longer connected or now prefers another mode
/// (another monitor on the same port); those that stay keep their order, closing up to the
/// left; new connectors come at the right, built-in panels first, numbered from `next_id`.
pub fn plan(shown: &[Shown], connectors: &[Connector], next_id: OutputId) -> Plan {
    let connected: Vec<(&Connector, (u32, u32, u32))> = connectors
        .iter()
        .filter_map(|connector| connector.mode.map(|mode| (connector, mode)))
        .collect();
    if connected.is_empty() {
        // Gone dark now, or already: nothing more to do.
        return Plan {
            dark: shown.iter().any(|shown| shown.connector.is_some()),
            ..Plan::default()
        };
    }
    let stays = |shown: &Shown| {
        connected
            .iter()
            .any(|(connector, mode)| Some(connector.id) == shown.connector && *mode == shown.mode)
    };
    let remove: Vec<OutputId> = shown
        .iter()
        .filter(|shown| !stays(shown))
        .map(|shown| shown.output.id)
        .collect();
    let kept: Vec<&Shown> = shown.iter().filter(|shown| stays(shown)).collect();
    let new: Vec<(&Connector, (u32, u32, u32))> = connected
        .iter()
        .filter(|(connector, _)| {
            !kept
                .iter()
                .any(|shown| shown.connector == Some(connector.id))
        })
        .copied()
        .collect();
    let monitors: Vec<Monitor> = new
        .iter()
        .map(|(connector, mode)| connector.monitor(*mode))
        .collect();
    let order = arrangement::order(&monitors);
    let scales: Vec<u32> = order
        .iter()
        .map(|&index| arrangement::scale_120(&monitors[index]))
        .collect();
    let sizes: Vec<(u32, u32)> = kept
        .iter()
        .map(|shown| (shown.output.rect.width, shown.output.rect.height))
        .chain(
            order
                .iter()
                .zip(&scales)
                .map(|(&index, &scale)| arrangement::logical(monitors[index].pixels, scale)),
        )
        .collect();
    let places = arrangement::arrange(&sizes);
    let moves = kept
        .iter()
        .zip(&places)
        .filter(|(shown, &at)| (shown.output.rect.x, shown.output.rect.y) != at)
        .map(|(shown, &at)| (shown.output.id, at))
        .collect();
    let add = order
        .iter()
        .zip(&scales)
        .zip(sizes.iter().zip(&places).skip(kept.len()))
        .enumerate()
        .map(
            |(number, ((&index, &scale_120), (&(width, height), &at)))| {
                let (connector, mode) = new[index];
                Shown {
                    output: Output {
                        id: next_id + number as OutputId,
                        rect: Rect::new(at.0, at.1, width, height),
                        scale_120,
                        refresh_mhz: mode.2 * 1000,
                    },
                    connector: Some(connector.id),
                    mode,
                }
            },
        )
        .collect();
    Plan {
        remove,
        dark: false,
        moves,
        add,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connector(id: u32, name: &str, internal: bool, mode: Option<(u32, u32, u32)>) -> Connector {
        Connector {
            id,
            name: name.into(),
            internal,
            mode,
            size_mm: match (internal, mode) {
                (true, _) => (301, 188),
                (false, Some((3840, _, _))) => (597, 336),
                _ => (531, 299),
            },
        }
    }

    /// The ThinkPad P14s Gen 3: its panel (eDP-1) and two DisplayPort connectors (USB-C) and an
    /// HDMI one, in the card's order.
    fn thinkpad(dp1: Option<(u32, u32, u32)>, hdmi: Option<(u32, u32, u32)>) -> Vec<Connector> {
        vec![
            connector(236, "DP-1", false, dp1),
            connector(247, "eDP-1", true, Some((1920, 1200, 60))),
            connector(255, "DP-2", false, None),
            connector(262, "HDMI-A-1", false, hdmi),
        ]
    }

    fn apply(shown: &mut Vec<Shown>, plan: &Plan) {
        shown.retain(|shown| !plan.remove.contains(&shown.output.id));
        for (id, at) in &plan.moves {
            let output = &mut shown
                .iter_mut()
                .find(|s| s.output.id == *id)
                .unwrap()
                .output;
            (output.rect.x, output.rect.y) = *at;
        }
        shown.extend(plan.add.iter().cloned());
    }

    #[test]
    fn at_the_start_the_panel_is_first_and_the_monitors_follow_in_the_cards_order() {
        let plan = plan(
            &[],
            &thinkpad(Some((3840, 2160, 60)), Some((1920, 1080, 60))),
            1,
        );
        let placed: Vec<(OutputId, Option<u32>, Rect, u32)> = plan
            .add
            .iter()
            .map(|shown| {
                (
                    shown.output.id,
                    shown.connector,
                    shown.output.rect,
                    shown.output.scale_120,
                )
            })
            .collect();
        assert_eq!(
            placed,
            [
                // The panel at 1.25: 1536x960, the primary.
                (1, Some(247), Rect::new(0, 0, 1536, 960), 150),
                // The 27-inch 4K monitor at 1.5, then the 1080p one at 1.
                (2, Some(236), Rect::new(1536, 0, 2560, 1440), 180),
                (3, Some(262), Rect::new(4096, 0, 1920, 1080), 120),
            ]
        );
        assert_eq!(plan.add[0].output.refresh_mhz, 60_000);
        assert!(plan.remove.is_empty() && plan.moves.is_empty() && !plan.dark);
    }

    #[test]
    fn a_monitor_plugged_in_comes_at_the_right_and_unplugged_closes_the_gap() {
        let mut shown = Vec::new();
        apply(&mut shown, &plan(&[], &thinkpad(None, None), 1));
        // A 1080p monitor on HDMI: right of the panel.
        let plugged = plan(&shown, &thinkpad(None, Some((1920, 1080, 60))), 2);
        assert!(plugged.remove.is_empty() && plugged.moves.is_empty());
        assert_eq!(plugged.add.len(), 1);
        assert_eq!(plugged.add[0].output.rect, Rect::new(1536, 0, 1920, 1080));
        apply(&mut shown, &plugged);
        // Then a 4K one on DP-1: right of both, whatever the card's order.
        let second = plan(
            &shown,
            &thinkpad(Some((3840, 2160, 60)), Some((1920, 1080, 60))),
            3,
        );
        assert_eq!(second.add[0].output.rect, Rect::new(3456, 0, 2560, 1440));
        apply(&mut shown, &second);
        // Nothing changed: nothing to do.
        assert_eq!(
            plan(
                &shown,
                &thinkpad(Some((3840, 2160, 60)), Some((1920, 1080, 60))),
                4
            ),
            Plan::default()
        );
        // The HDMI one unplugged: it goes, and the 4K one moves left to close the gap.
        let unplugged = plan(&shown, &thinkpad(Some((3840, 2160, 60)), None), 4);
        assert_eq!(unplugged.remove, [2]);
        assert_eq!(unplugged.moves, [(3, (1536, 0))]);
        assert!(unplugged.add.is_empty());
    }

    #[test]
    fn another_monitor_on_the_same_port_is_a_new_output() {
        let mut shown = Vec::new();
        apply(
            &mut shown,
            &plan(&[], &thinkpad(None, Some((1920, 1080, 60))), 1),
        );
        let swapped = plan(&shown, &thinkpad(None, Some((3840, 2160, 30))), 3);
        assert_eq!(swapped.remove, [2]);
        assert_eq!(swapped.add.len(), 1);
        assert_eq!(swapped.add[0].output.id, 3);
        assert_eq!(swapped.add[0].output.rect, Rect::new(1536, 0, 2560, 1440));
        assert_eq!(swapped.add[0].output.refresh_mhz, 30_000);
    }

    #[test]
    fn with_nothing_connected_the_outputs_stay_dark_until_a_display_comes() {
        let monitor_only = |mode| vec![connector(262, "HDMI-A-1", false, mode)];
        let mut shown = Vec::new();
        apply(
            &mut shown,
            &plan(&[], &monitor_only(Some((1920, 1080, 60))), 1),
        );
        let gone = plan(&shown, &monitor_only(None), 2);
        assert_eq!(
            gone,
            Plan {
                dark: true,
                ..Plan::default()
            }
        );
        // Dark, it is on no connector, and stays so; plugged in again, an output for it takes
        // the dark one's place (and its windows).
        shown[0].connector = None;
        assert_eq!(plan(&shown, &monitor_only(None), 2), Plan::default());
        let back = plan(&shown, &monitor_only(Some((1920, 1080, 60))), 2);
        assert_eq!(back.remove, [1]);
        assert_eq!(back.add.len(), 1);
        assert_eq!(
            (back.add[0].output.id, back.add[0].output.rect),
            (2, Rect::new(0, 0, 1920, 1080))
        );
        // Nothing at all, from the start: nothing to show.
        assert_eq!(plan(&[], &monitor_only(None), 1), Plan::default());
    }
}
