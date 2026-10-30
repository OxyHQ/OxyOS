//! The development compositor's displays (`--displays FILE`, docs/adr/0018-several-displays.mdx):
//! virtual connectors, as a card would report them, which the session's own displays
//! (`oxyos_compositor::displays`) show, arrange and change from Settings as a session does: what
//! the user's preferences keep, a mode on trial and going back, the arrangement and the primary.
//! Nothing is lit: each output composes its mode's pixels into its frame, paced as a display at
//! its mode's refresh (each frame shown at its next vblank).
//!
//! FILE has a line for each connector, in the card's order:
//!
//! ```text
//! # identity  kind  size (mm)  modes (* the preferred, ! one the card refuses)  name
//! BOE-0A4B-00000001 internal 301x188 1920x1200@60000*,1280x800@60000 eDP-1
//! DEL-A0B4-7Y9K2N3 external 597x336 3840x2160@60000*,2560x1440@59951 DP-1 DELL U2723QE
//! ```
//!
//! after the connector's name, its monitor's (EDID) name, if it has one. A mode the card refuses is
//! one its atomic test would refuse (a link without the bandwidth for it): listed, never shown.

use oxyos_compositor::displays::{Connector, Heads};
use oxyos_compositor::{Compositor, Result};
use oxyos_compositor_core::{ModeInfo, OutputId};

/// Virtual connectors, numbered in the file's order, and the modes the card refuses on each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Virtual {
    pub connectors: Vec<Connector>,
    pub refused: Vec<(u32, ModeInfo)>,
}

impl Virtual {
    pub fn parse(text: &str) -> Result<Self> {
        let mut connectors = Vec::new();
        let mut refused = Vec::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let words: Vec<&str> = line.splitn(6, ' ').collect();
            let [identity, kind, size, modes, name, rest @ ..] = words.as_slice() else {
                return Err(format!("{line:?}: identity, kind, size, modes and name").into());
            };
            let internal = match *kind {
                "internal" => true,
                "external" => false,
                _ => return Err(format!("{kind:?}: internal or external").into()),
            };
            let (width_mm, height_mm) = size.split_once('x').ok_or("a size WxH in mm")?;
            let id = connectors.len() as u32 + 1;
            let modes = modes
                .split(',')
                .map(|mode| {
                    let (mode, refuses) = match mode.strip_suffix('!') {
                        Some(mode) => (mode, true),
                        None => (mode, false),
                    };
                    let (mode, preferred) = match mode.strip_suffix('*') {
                        Some(mode) => (mode, true),
                        None => (mode, false),
                    };
                    let (pixels, refresh) = mode.split_once('@').ok_or("a mode WxH@MHZ")?;
                    let (width, height) = pixels.split_once('x').ok_or("a mode WxH@MHZ")?;
                    let mode = ModeInfo {
                        width: width.parse()?,
                        height: height.parse()?,
                        refresh_mhz: refresh.parse()?,
                        preferred,
                    };
                    if !(1_000..=1_000_000).contains(&mode.refresh_mhz) {
                        return Err(format!("{mode:?}: a refresh from 1 to 1000 Hz").into());
                    }
                    if refuses {
                        refused.push((id, mode));
                    }
                    Ok(mode)
                })
                .collect::<Result<Vec<ModeInfo>>>()?;
            let identified = !identity.is_empty()
                && identity
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-');
            if !identified || modes.is_empty() {
                return Err(format!("{line:?}: an identity and a mode at least").into());
            }
            connectors.push(Connector {
                id,
                name: name.to_string(),
                internal,
                identity: identity.to_string(),
                product: rest.first().map(|name| name.to_string()),
                size_mm: (width_mm.parse()?, height_mm.parse()?),
                modes,
                vrr_capable: false,
                refresh_range_hz: None,
            });
        }
        if connectors.is_empty() {
            return Err("--displays: no display".into());
        }
        Ok(Self {
            connectors,
            refused,
        })
    }
}

impl Heads for Virtual {
    /// Nothing to light: each output composes its mode's pixels.
    type Head = ();

    fn connectors(&mut self) -> Result<Vec<Connector>> {
        Ok(self.connectors.clone())
    }

    /// The card's atomic test, played: it refuses the modes the file marks.
    fn prepare(
        &mut self,
        _compositor: &mut Compositor,
        connector: &Connector,
        mode: &ModeInfo,
        _scale_120: u32,
    ) -> std::result::Result<(), String> {
        let refuses = self.refused.iter().any(|(id, refused)| {
            *id == connector.id
                && (refused.width, refused.height, refused.refresh_mhz)
                    == (mode.width, mode.height, mode.refresh_mhz)
        });
        if refuses {
            return Err("the card refuses the mode (a played atomic test)".into());
        }
        Ok(())
    }

    /// Its frames are paced as a display's, at its mode's refresh.
    fn attach(&mut self, compositor: &mut Compositor, output: OutputId, _head: ()) {
        if let Some(screen) = compositor.screen_mut(output) {
            screen.paced = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_a_connector_and_its_display() {
        let displays = Virtual::parse(
            "# identity kind size modes name\n\
             BOE-0A4B-00000001 internal 301x188 1920x1200@60000*,1280x800@60000 eDP-1\n\
             DEL-A0B4-7Y9K2N3 external 597x336 3840x2160@60000*,2560x1440@59951 DP-1 DELL U2723QE\n",
        )
        .unwrap();
        let [panel, monitor] = displays.connectors.as_slice() else {
            panic!("two connectors");
        };
        assert_eq!(
            (panel.id, panel.internal, panel.product.as_deref()),
            (1, true, None)
        );
        assert_eq!(panel.preferred().unwrap().width, 1920);
        assert_eq!(monitor.product.as_deref(), Some("DELL U2723QE"));
        assert_eq!(monitor.size_mm, (597, 336));
        assert_eq!(monitor.modes[1].refresh_mhz, 59_951);
        assert!(Virtual::parse("").is_err());
        assert!(Virtual::parse("X bad 1x1 1x1@1000 n").is_err());
        assert!(Virtual::parse("../x internal 1x1 1x1@1000 n").is_err());
        assert!(Virtual::parse("X internal 1x1 1x1@0 n").is_err());
        // A 144 Hz monitor whose link takes 120 at most.
        let fast = Virtual::parse(
            "GSM-5B7F-1 external 597x336 2560x1440@59951*,2560x1440@119998,2560x1440@143998! DP-1\n",
        )
        .unwrap();
        assert_eq!(fast.connectors[0].own_modes()[0].refresh_mhz, 143_998);
        assert_eq!(fast.refused, [(1, fast.connectors[0].own_modes()[0])]);
    }
}
