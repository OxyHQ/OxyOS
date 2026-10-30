//! The session's one set of motions (docs/adr/0011-transitions.mdx): how every surface the
//! compositor animates arrives, goes, zooms and slides. Each is a spring, as a Mac's window
//! server and SwiftUI move things: a size and a place go on a spring released from rest, an
//! opacity on a shorter one; none is linear, none is a fixed curve cut at a duration. A spring
//! is told by its response (the period it would swing at undamped, how quick it feels) and its
//! damping ratio (1 is critically damped: it arrives without overshooting); a motion lasts until
//! its springs are at rest.

use std::time::Duration;

/// Within this share of its way from its end, a spring is at rest: a thousandth (under half a
/// pixel of a 400 px move, a quarter of an opacity level).
const AT_REST: f64 = 0.001;
/// No spring of the set runs longer than this.
const LONGEST: Duration = Duration::from_secs(2);

/// A spring released from rest at 0 toward 1: its response in milliseconds and damping ratio in
/// thousandths (at most 1000).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spring {
    pub response_ms: u32,
    pub damping_permille: u32,
}

impl Spring {
    pub const fn new(response_ms: u32, damping_permille: u32) -> Self {
        assert!(response_ms > 0 && damping_permille > 0 && damping_permille <= 1000);
        Self {
            response_ms,
            damping_permille,
        }
    }

    /// Critically damped: it arrives as soon as it can without going past.
    pub const fn critical(response_ms: u32) -> Self {
        Self::new(response_ms, 1000)
    }

    fn omega(&self) -> f64 {
        std::f64::consts::TAU * 1000.0 / f64::from(self.response_ms)
    }

    fn zeta(&self) -> f64 {
        f64::from(self.damping_permille) / 1000.0
    }

    /// Where it is `elapsed` after its release: 0 at first, 1 at rest; past 1 while an
    /// underdamped one overshoots.
    pub fn at(&self, elapsed: Duration) -> f64 {
        let (omega, zeta, t) = (self.omega(), self.zeta(), elapsed.as_secs_f64());
        if self.damping_permille == 1000 {
            1.0 - (1.0 + omega * t) * (-omega * t).exp()
        } else {
            let damped = omega * (1.0 - zeta * zeta).sqrt();
            1.0 - (-zeta * omega * t).exp()
                * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
        }
    }

    /// How far from rest it can still be at `elapsed`: its displacement's envelope.
    fn envelope(&self, elapsed: Duration) -> f64 {
        let (omega, zeta, t) = (self.omega(), self.zeta(), elapsed.as_secs_f64());
        if self.damping_permille == 1000 {
            (1.0 + omega * t) * (-omega * t).exp()
        } else {
            (-zeta * omega * t).exp() / (1.0 - zeta * zeta).sqrt()
        }
    }

    /// When it comes to rest, to the millisecond: from then on it is within `AT_REST` of 1.
    pub fn settles(&self) -> Duration {
        let mut ms = 0;
        while ms < LONGEST.as_millis() as u64 && self.envelope(Duration::from_millis(ms)) > AT_REST
        {
            ms += 1;
        }
        Duration::from_millis(ms)
    }
}

/// How a surface's opacity goes: on its own spring, beginning `delay` after the motion does
/// (a window going into the dock stays whole until it is nearly there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fade {
    pub spring: Spring,
    pub delay: Duration,
}

impl Fade {
    pub const fn new(spring: Spring) -> Self {
        Self {
            spring,
            delay: Duration::ZERO,
        }
    }

    pub const fn after(spring: Spring, delay_ms: u64) -> Self {
        Self {
            spring,
            delay: Duration::from_millis(delay_ms),
        }
    }

    /// How far it is at `elapsed` into its motion: 0 until its delay, then its spring, never
    /// past 1 (an opacity does not overshoot).
    pub fn at(&self, elapsed: Duration) -> f64 {
        match elapsed.checked_sub(self.delay) {
            Some(elapsed) => self.spring.at(elapsed).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    pub fn settles(&self) -> Duration {
        self.delay + self.spring.settles()
    }
}

/// The motions of the set, by what they move. Windows: a new one grows into place from 94 % as
/// it fades in; one closing shrinks back to 94 %, quicker, as it fades out (a Mac's dismissals
/// are quicker than its arrivals).
pub const WINDOW_ARRIVES: Spring = Spring::critical(300);
pub const WINDOW_FADES_IN: Spring = Spring::critical(180);
pub const WINDOW_GOES: Spring = Spring::critical(220);
pub const WINDOW_FADES_OUT: Spring = Spring::critical(150);
pub const WINDOW_SCALE_PERMILLE: u32 = 940;
/// A menu appears at once, and fades out when dismissed.
pub const MENU_FADES_OUT: Spring = Spring::critical(150);
/// The shell's overlays (Launchpad's icons, the overview) settle from 104 % as they fade in, and
/// grow back to it as they fade out, as Launchpad's icons do; their backdrops only fade.
pub const OVERLAY_ARRIVES: Spring = Spring::critical(320);
pub const OVERLAY_FADES_IN: Spring = Spring::critical(200);
pub const OVERLAY_GOES: Spring = Spring::critical(240);
pub const OVERLAY_FADES_OUT: Spring = Spring::critical(160);
pub const OVERLAY_SCALE_PERMILLE: u32 = 1040;
pub const BACKDROP_FADES_IN: Spring = Spring::critical(300);
pub const BACKDROP_FADES_OUT: Spring = Spring::critical(240);
/// A window minimized goes into the dock, whole until it is nearly there; restored, it comes
/// back out, whole at once.
pub const DOCK: Spring = Spring::critical(380);
pub const INTO_DOCK_FADES: Fade = Fade::after(Spring::critical(160), 180);
pub const OUT_OF_DOCK_FADES: Fade = Fade::new(Spring::critical(120));
/// A switch of workspace slides the windows by the output's width, as a Mac's spaces.
pub const WORKSPACE: Spring = Spring::critical(380);
/// A window's frame zooming to its new rectangle (and its corners squaring or rounding).
pub const ZOOM: Spring = Spring::critical(340);
/// A notification slides in from beside itself with the slightest bounce, as a Mac's banner,
/// and fades out.
pub const NOTIFICATION_ARRIVES: Spring = Spring::new(400, 820);
pub const NOTIFICATION_FADES_IN: Spring = Spring::critical(200);
pub const NOTIFICATION_FADES_OUT: Spring = Spring::critical(200);
/// The lock screen fades in over the lock's covers and out on unlock; the shell's panels fade in
/// as the session starts; the splash fades out over the shell.
pub const LOCK_FADES: Spring = Spring::critical(300);
pub const PANEL_FADES_IN: Spring = Spring::critical(300);
pub const SPLASH_FADES_OUT: Spring = Spring::critical(450);
/// With reduce motion every motion is this crossfade: nothing grows, shrinks or slides.
pub const CROSSFADE: Spring = Spring::critical(200);

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn a_critically_damped_spring_eases_out_without_overshooting() {
        let spring = Spring::critical(300);
        // ω = 2π / 0.3 s ≈ 20.94 rad/s: 1 - (1 + ωt)e^(-ωt), worked by hand.
        assert_eq!(spring.at(Duration::ZERO), 0.0);
        assert!(
            (spring.at(ms(50)) - 0.2814).abs() < 1e-3,
            "{}",
            spring.at(ms(50))
        );
        assert!((spring.at(ms(150)) - 0.8210).abs() < 1e-3);
        assert!((spring.at(ms(300)) - 0.9864).abs() < 1e-3);
        // At rest (within a thousandth) at ωt ≈ 9.23: 441 ms.
        assert_eq!(spring.settles(), ms(441));
        // Never past its end, never back: each millisecond further than the last.
        let path: Vec<f64> = (0..=600).map(|t| spring.at(ms(t))).collect();
        assert!(path.windows(2).all(|w| w[1] > w[0] && w[1] <= 1.0));
        // Ahead of a linear motion of the same length from its first frame on (16 ms): it eases
        // out, quick at first and slow to arrive.
        let settles = spring.settles().as_secs_f64();
        assert!((16..441).all(|t| spring.at(ms(t)) >= t as f64 / 1000.0 / settles));
    }

    #[test]
    fn an_underdamped_spring_overshoots_once_by_its_damping() {
        let spring = NOTIFICATION_ARRIVES;
        let path: Vec<f64> = (0..=1000).map(|t| spring.at(ms(t))).collect();
        let peak = path.iter().cloned().fold(0.0, f64::max);
        // ζ = 0.82: an overshoot of e^(-πζ/√(1-ζ²)) ≈ 1.1 %.
        assert!((peak - 1.011).abs() < 1e-3, "{peak}");
        let settles = spring.settles();
        assert!(path[settles.as_millis() as usize..]
            .iter()
            .all(|x| (x - 1.0).abs() <= AT_REST));
    }

    #[test]
    fn a_fade_waits_for_its_delay_and_never_overshoots() {
        let fade = INTO_DOCK_FADES;
        assert_eq!(fade.at(ms(179)), 0.0);
        assert!(fade.at(ms(200)) > 0.0);
        assert_eq!(fade.settles(), ms(180) + Spring::critical(160).settles());
        let bouncy = Fade::new(NOTIFICATION_ARRIVES);
        assert!((0..1000).all(|t| bouncy.at(ms(t)) <= 1.0));
    }

    #[test]
    fn every_motion_is_most_of_the_way_in_a_quarter_second_and_at_rest_in_six_tenths() {
        for spring in [
            WINDOW_ARRIVES,
            WINDOW_GOES,
            OVERLAY_ARRIVES,
            OVERLAY_GOES,
            BACKDROP_FADES_IN,
            DOCK,
            WORKSPACE,
            ZOOM,
            NOTIFICATION_ARRIVES,
            LOCK_FADES,
            CROSSFADE,
        ] {
            let settles = spring.settles();
            assert!(settles <= ms(600), "{spring:?} at rest at {settles:?}");
            // Most of the way (90 %) within a Mac's 200 to 350 ms.
            let most = (0..).find(|&t| spring.at(ms(t)) >= 0.9).unwrap();
            assert!((60..=260).contains(&most), "{spring:?} 90 % at {most} ms");
        }
    }
}
