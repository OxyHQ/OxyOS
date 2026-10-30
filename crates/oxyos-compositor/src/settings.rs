//! The user's preferences while a session runs (docs/adr/0019-settings.mdx), for the program
//! running the compositor: the session authority, and the preferences file's one writer. It
//! applies them (reduce motion to the transitions, `lock_after` to the idle lock), tells the scene what they are (every client
//! the ones it draws with, Settings every one) and takes the changes Settings asks for: each is
//! validated, written to the file whole, applied and told, or refused with nothing changed.
//!
//! An automatic appearance is dark while the sun is below the horizon where the system's time
//! zone is (`oxyos-solar`), looked at again only when the sun next crosses it; where the time zone
//! has no place (UTC) it is light.
//!
//! `lock_after` is the idle lock's (idle.rs): changed, its deadline is armed again from the
//! change. `screen_reader` is the program's to follow ([`Settings::serve`] says it changed): it alone
//! starts and stops its reader.

use crate::Compositor;
use oxyos_compositor_core::DrawingPreferences;
use oxyos_preferences::{Appearance, Preferences, KEYS};
use oxyos_protocol::PreeditText;
use oxyos_solar::Place;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The clock's time now, in unix seconds.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

// Every client is told the accent in one chunk.
const _: () = assert!(oxyos_preferences::MAX_ACCENT_BYTES <= oxyos_protocol::TEXT_CHUNK_BYTES);

/// Whether `appearance` is dark at `now` (unix seconds) from `place`, and when to look again:
/// when the sun next crosses the horizon, for an automatic appearance with a place; never
/// otherwise.
pub fn resolve(appearance: Appearance, place: Option<Place>, now: i64) -> (bool, Option<i64>) {
    match (appearance, place) {
        (Appearance::Light, _) => (false, None),
        (Appearance::Dark, _) => (true, None),
        (Appearance::Auto, Some(place)) => {
            let (night, crossing) = oxyos_solar::night_at(now, place);
            (night, Some(crossing))
        }
        (Appearance::Auto, None) => (false, None),
    }
}

pub struct Settings {
    preferences: Preferences,
    /// The file they are kept in; without one, changes last as long as the program.
    file: Option<PathBuf>,
    /// Where the system's time zone is: the sun an automatic appearance follows.
    place: Option<Place>,
    /// When the sun next crosses the horizon (unix seconds), while the appearance follows it.
    crossing: Option<i64>,
}

impl Settings {
    pub fn new(preferences: Preferences, file: Option<PathBuf>, place: Option<Place>) -> Self {
        Self {
            preferences,
            file,
            place,
            crossing: None,
        }
    }

    /// The preferences in `file` (the defaults where it says nothing usable, or without one).
    pub fn load(file: Option<PathBuf>, place: Option<Place>) -> Self {
        let preferences = file.as_deref().map(Preferences::load).unwrap_or_default();
        Self::new(preferences, file, place)
    }

    /// The session's: the user's file (`$XDG_CONFIG_HOME/oxyos/preferences`).
    pub fn session() -> Self {
        Self::from_file(oxyos_preferences::session_path())
    }

    /// The preferences in `file` (the defaults without one), following the sun where the
    /// system's time zone is.
    pub fn from_file(file: Option<PathBuf>) -> Self {
        Self::load(file, oxyos_solar::system_place())
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// Applies the preferences to the compositor and tells its clients, as they are at `now`
    /// (unix seconds).
    pub fn apply(&mut self, compositor: &mut Compositor, now: i64) {
        let preferences = &self.preferences;
        let (dark, crossing) = resolve(preferences.appearance, self.place, now);
        self.crossing = crossing;
        compositor
            .transitions
            .set_reduce_motion(preferences.reduce_motion);
        compositor.set_lock_after(preferences.lock_after);
        // Notifications (docs/adr/0026-notifications.mdx): Do Not Disturb and the lock screen's
        // previews.
        compositor.scene.set_notification_policy(
            oxyos_compositor_core::scene::NotificationPolicy {
                do_not_disturb: preferences.do_not_disturb,
                previews: preferences.notification_previews == oxyos_preferences::Previews::Always,
            },
        );
        let drawing = DrawingPreferences {
            dark,
            reduce_motion: preferences.reduce_motion,
            reduce_transparency: preferences.reduce_transparency,
            accent: PreeditText::new(&preferences.accent).expect("an accent is one chunk"),
        };
        let values: Vec<(String, String)> = KEYS
            .iter()
            .map(|key| {
                let value = preferences.get(key).expect("every key has a value");
                (key.to_string(), value)
            })
            .collect();
        compositor.scene.set_preferences(drawing, &values);
    }

    /// When the sun next crosses the horizon where the system is (unix seconds), with an automatic
    /// appearance: `serve` should run by then.
    pub fn next_crossing(&self) -> Option<i64> {
        self.crossing
    }

    /// One turn: the changes Settings asked for, each answered, and the sun if it crossed the
    /// horizon. The keys whose values changed, for the program to follow what is its own (the
    /// screen reader).
    pub fn serve(&mut self, compositor: &mut Compositor, now: i64) -> Vec<String> {
        let mut changed = Vec::new();
        for change in compositor.scene.take_preference_changes() {
            let accepted = match self.change(&change.key, &change.value) {
                Ok(true) => {
                    changed.push(change.key);
                    true
                }
                Ok(false) => true,
                Err(error) => {
                    eprintln!(
                        "[preferences] {}={:?} refused: {error}",
                        change.key, change.value
                    );
                    false
                }
            };
            compositor
                .scene
                .preference_set(change.client, change.serial, accepted);
        }
        if !changed.is_empty() || self.crossing.is_some_and(|crossing| now >= crossing) {
            self.apply(compositor, now);
        }
        changed
    }

    /// Keeps a change of what the session's displays keep (docs/adr/0018-several-displays.mdx):
    /// a display's mode and scale, an arrangement. Written whole first; refused, with nothing
    /// changed, if the file cannot be written, as any change is.
    pub fn keep_displays(&mut self, change: impl FnOnce(&mut Preferences)) -> Result<(), String> {
        let mut changed = self.preferences.clone();
        change(&mut changed);
        if changed == self.preferences {
            return Ok(());
        }
        if let Some(file) = &self.file {
            changed
                .save(file)
                .map_err(|error| format!("{}: {error}", file.display()))?;
        }
        self.preferences = changed;
        Ok(())
    }

    /// Sets one preference from its text and keeps it: whether it changed. Refused (and nothing
    /// changed) if it is not a value it takes or the file cannot be written: the file and the
    /// session never disagree.
    fn change(&mut self, key: &str, value: &str) -> Result<bool, String> {
        let mut changed = self.preferences.clone();
        changed.set(key, value).map_err(|error| error.to_string())?;
        if changed == self.preferences {
            return Ok(false);
        }
        if let Some(file) = &self.file {
            changed
                .save(file)
                .map_err(|error| format!("{}: {error}", file.display()))?;
        }
        self.preferences = changed;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_compositor_core::{ClientId, Grants, Output, Scene};
    use oxyos_protocol::{Event, Rect, Request, TextChunk};

    const APP: ClientId = 1;
    const SETTINGS: ClientId = 2;
    const MADRID: Place = Place {
        latitude: 40.4,
        longitude: -3.683_333,
    };
    /// 2026-06-21 at 01:00 and 12:00 UTC: night and day in Madrid.
    const NIGHT: i64 = 1_782_003_600;
    const DAY: i64 = NIGHT + 11 * 3600;

    fn compositor() -> Compositor {
        let output = Rect::new(0, 0, 64, 64);
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: output,
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        scene.connect(APP, Grants::default()).unwrap();
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
            "oxyos-compositor-settings-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn ask(compositor: &mut Compositor, serial: u32, text: &str) {
        let chunks = TextChunk::split(text);
        for (index, chunk) in chunks.iter().enumerate() {
            compositor
                .scene
                .handle(
                    SETTINGS,
                    Request::SetPreference {
                        serial,
                        index: index as u8,
                        last: index + 1 == chunks.len(),
                        text: PreeditText::new(chunk.as_str()).unwrap(),
                    },
                )
                .unwrap();
        }
    }

    /// What a client was told: (dark, reduce motion) of each Preferences, and (serial, accepted)
    /// of each PreferenceResult.
    type Told = (Vec<(bool, bool)>, Vec<(u32, bool)>);

    /// What `client` was told since the last call.
    fn told(compositor: &mut Compositor, client: ClientId) -> Told {
        let mut drawing = Vec::new();
        let mut results = Vec::new();
        for (to, event) in compositor.scene.drain_events() {
            match event {
                Event::Preferences {
                    dark,
                    reduce_motion,
                    ..
                } if to == client => drawing.push((dark, reduce_motion)),
                Event::PreferenceResult { serial, accepted } if to == client => {
                    results.push((serial, accepted))
                }
                _ => {}
            }
        }
        (drawing, results)
    }

    #[test]
    fn an_automatic_appearance_follows_the_sun_and_says_when_to_look_again() {
        let (dark, crossing) = resolve(Appearance::Auto, Some(MADRID), NIGHT);
        assert!(dark);
        let sunrise = crossing.unwrap();
        assert!(sunrise > NIGHT && sunrise < DAY);
        assert!(!resolve(Appearance::Auto, Some(MADRID), sunrise).0);
        assert_eq!(resolve(Appearance::Dark, Some(MADRID), DAY), (true, None));
        assert_eq!(
            resolve(Appearance::Light, Some(MADRID), NIGHT),
            (false, None)
        );
        // No place (UTC): no sun to follow.
        assert_eq!(resolve(Appearance::Auto, None, NIGHT), (false, None));
    }

    #[test]
    fn a_change_is_validated_kept_applied_and_told_and_a_refused_one_changes_nothing() {
        let directory = scratch("change");
        let file = directory.join("oxyos").join("preferences");
        let mut compositor = compositor();
        let mut settings = Settings::load(Some(file.clone()), Some(MADRID));
        settings.apply(&mut compositor, DAY);
        assert_eq!(told(&mut compositor, APP).0, [(false, false)]);
        ask(&mut compositor, 1, "reduce_motion=true");
        ask(&mut compositor, 2, "reduce_motion=sometimes");
        ask(&mut compositor, 3, "volume=11");
        assert_eq!(settings.serve(&mut compositor, DAY), ["reduce_motion"]);
        // Kept in the file, applied to the transitions, told to every client once.
        assert!(Preferences::load(&file).reduce_motion);
        assert!(compositor.transitions.reduce_motion());
        let (drawing, results) = told(&mut compositor, SETTINGS);
        assert_eq!(drawing, [(false, true)]);
        assert_eq!(results, [(1, true), (2, false), (3, false)]);
        // The same value again is taken, and changes nothing.
        ask(&mut compositor, 4, "reduce_motion=true");
        assert!(settings.serve(&mut compositor, DAY).is_empty());
        assert_eq!(told(&mut compositor, SETTINGS), (vec![], vec![(4, true)]));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_change_that_cannot_be_kept_is_refused() {
        let directory = scratch("unwritable");
        std::fs::create_dir_all(&directory).unwrap();
        // Its directory is a file: nothing can be written there.
        std::fs::write(directory.join("oxyos"), "").unwrap();
        let mut compositor = compositor();
        let mut settings = Settings::load(Some(directory.join("oxyos/preferences")), None);
        settings.apply(&mut compositor, DAY);
        told(&mut compositor, SETTINGS);
        ask(&mut compositor, 1, "appearance=dark");
        assert!(settings.serve(&mut compositor, DAY).is_empty());
        assert_eq!(settings.preferences().appearance, Appearance::Auto);
        assert_eq!(told(&mut compositor, SETTINGS), (vec![], vec![(1, false)]));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_client_draws_dark_when_the_sun_goes_down_and_light_when_it_comes_up() {
        let mut compositor = compositor();
        let mut settings = Settings::new(Preferences::default(), None, Some(MADRID));
        settings.apply(&mut compositor, NIGHT);
        assert_eq!(told(&mut compositor, APP).0, [(true, false)]);
        // Nothing until the sun crosses the horizon.
        settings.serve(&mut compositor, NIGHT + 60);
        assert!(told(&mut compositor, APP).0.is_empty());
        let (_, sunrise) = oxyos_solar::night_at(NIGHT, MADRID);
        settings.serve(&mut compositor, sunrise);
        assert_eq!(told(&mut compositor, APP).0, [(false, false)]);
        // A fixed appearance does not follow it.
        ask(&mut compositor, 1, "appearance=dark");
        settings.serve(&mut compositor, sunrise);
        assert_eq!(told(&mut compositor, APP).0, [(true, false)]);
        settings.serve(&mut compositor, sunrise + 86_400);
        assert!(told(&mut compositor, APP).0.is_empty());
    }

    #[test]
    fn lock_after_is_the_idle_locks_and_a_change_arms_it_again() {
        let minute = std::time::Duration::from_secs(60);
        let mut compositor = compositor();
        compositor.clock = Some(100 * minute);
        let mut settings = Settings::new(Preferences::default(), None, None);
        settings.apply(&mut compositor, DAY);
        assert_eq!(compositor.idle.deadline(), Some(105 * minute));
        // Four minutes on, Settings makes it one: a minute from the change.
        compositor.clock = Some(104 * minute);
        ask(&mut compositor, 1, "lock_after=1");
        assert_eq!(settings.serve(&mut compositor, DAY), ["lock_after"]);
        assert_eq!(compositor.idle.deadline(), Some(105 * minute));
        // Another preference changing does not arm it again.
        compositor.clock = Some(104 * minute + minute / 2);
        ask(&mut compositor, 2, "reduce_motion=true");
        assert_eq!(settings.serve(&mut compositor, DAY), ["reduce_motion"]);
        assert_eq!(compositor.idle.deadline(), Some(105 * minute));
        compositor.clock = Some(105 * minute - std::time::Duration::from_nanos(1));
        compositor.follow_idle();
        assert!(!compositor.scene.is_locked());
        compositor.clock = Some(105 * minute);
        compositor.follow_idle();
        assert!(compositor.scene.is_locked());
        // 0 takes it away.
        ask(&mut compositor, 3, "lock_after=0");
        settings.serve(&mut compositor, DAY);
        assert_eq!(compositor.idle.deadline(), None);
    }

    #[test]
    fn do_not_disturb_and_previews_are_the_shells_notification_policy() {
        const SHELL: ClientId = 7;
        let mut compositor = compositor();
        compositor
            .scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        let policies = |compositor: &mut Compositor| -> Vec<(bool, bool)> {
            compositor
                .scene
                .drain_events()
                .into_iter()
                .filter_map(|(to, event)| match event {
                    Event::NotificationPolicy {
                        do_not_disturb,
                        previews,
                    } if to == SHELL => Some((do_not_disturb, previews)),
                    _ => None,
                })
                .collect()
        };
        let mut settings = Settings::new(Preferences::default(), None, None);
        settings.apply(&mut compositor, DAY);
        assert_eq!(policies(&mut compositor), [(false, false)]);
        ask(&mut compositor, 1, "do_not_disturb=true");
        assert_eq!(settings.serve(&mut compositor, DAY), ["do_not_disturb"]);
        assert_eq!(policies(&mut compositor), [(true, false)]);
        ask(&mut compositor, 2, "notification_previews=always");
        settings.serve(&mut compositor, DAY);
        assert_eq!(policies(&mut compositor), [(true, true)]);
        // Another preference changing tells the shell nothing new.
        ask(&mut compositor, 3, "reduce_motion=true");
        settings.serve(&mut compositor, DAY);
        assert!(policies(&mut compositor).is_empty());
        // The shell's Control Center turns it off the same way: kept, applied and told.
        compositor
            .scene
            .handle(
                SHELL,
                Request::SetDoNotDisturb {
                    serial: 4,
                    enabled: false,
                },
            )
            .unwrap();
        assert_eq!(settings.serve(&mut compositor, DAY), ["do_not_disturb"]);
        assert!(!settings.preferences().do_not_disturb);
        assert_eq!(policies(&mut compositor), [(false, true)]);
    }

    #[test]
    fn every_preference_fits_what_a_client_is_told() {
        let mut preferences = Preferences::default();
        preferences
            .set("wallpaper", &"w".repeat(oxyos_preferences::MAX_VALUE_BYTES))
            .unwrap();
        for key in KEYS {
            let text = format!("{key}={}", preferences.get(key).unwrap());
            assert!(text.len() <= oxyos_protocol::MAX_PREFERENCE_BYTES);
        }
    }
}
