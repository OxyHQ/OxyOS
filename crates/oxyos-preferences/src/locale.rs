//! The session's language and region (docs/adr/0040-language-and-region.mdx): the person's choice
//! (`language`, `region`, `hour_cycle`) over the system's (the session's environment, POSIX's
//! `LC_ALL`, `LC_MESSAGES`, `LANG`), as one BCP 47 tag every client is told.

use crate::{Preferences, Refused};

/// The languages OxyOS speaks: its shell's, login screen's and Settings' catalogues and the
/// reader's speech have them complete.
pub const LANGUAGES: &[&str] = &["en", "es"];

/// The language spoken when the system's is none OxyOS speaks.
const FALLBACK_LANGUAGE: &str = "en";

/// Twelve or 24 hours: the region's (`auto`), or the person's choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HourCycle {
    #[default]
    Auto,
    H12,
    H23,
}

impl HourCycle {
    pub fn name(self) -> &'static str {
        match self {
            HourCycle::Auto => "auto",
            HourCycle::H12 => "h12",
            HourCycle::H23 => "h23",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "auto" => HourCycle::Auto,
            "h12" => HourCycle::H12,
            "h23" => HourCycle::H23,
            _ => return None,
        })
    }
}

/// A `language` preference: empty is the system's, else one of [`LANGUAGES`].
pub(crate) fn language(value: &str) -> Result<Option<String>, Refused> {
    match value {
        "" => Ok(None),
        value if LANGUAGES.contains(&value) => Ok(Some(value.to_owned())),
        _ => Err(Refused::BadValue),
    }
}

/// A `region` preference: empty is the system's, else two ASCII capitals.
pub(crate) fn region(value: &str) -> Result<Option<String>, Refused> {
    match value {
        "" => Ok(None),
        value if value.len() == 2 && value.bytes().all(|b| b.is_ascii_uppercase()) => {
            Ok(Some(value.to_owned()))
        }
        _ => Err(Refused::BadValue),
    }
}

/// The region a language is formatted for when nothing names one: its likely region.
fn likely_region(language: &str) -> &'static str {
    match language {
        "es" => "ES",
        _ => "US",
    }
}

/// A POSIX locale's language and territory (`es_MX.UTF-8@euro` is `es` and `MX`): none for `C`
/// and `POSIX`, the codeset and the modifier ignored.
fn posix_parts(locale: &str) -> (Option<String>, Option<String>) {
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    if matches!(name, "" | "C" | "POSIX") {
        return (None, None);
    }
    let (language, territory) = match name.split_once('_') {
        Some((language, territory)) => (language, Some(territory)),
        None => (name, None),
    };
    let language = (!language.is_empty() && language.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| language.to_ascii_lowercase());
    let territory = territory
        .filter(|t| t.len() == 2 && t.bytes().all(|b| b.is_ascii_alphabetic()))
        .map(str::to_ascii_uppercase);
    (language, territory)
}

/// The session's locale as a BCP 47 tag (`es-ES`, `en-US`, `en-ES`, `es-MX-u-hc-h12`):
/// - its language is the person's, else the environment's when OxyOS speaks it, else English;
/// - its region the person's, else the environment's territory, else the language's likely one;
/// - and the hour cycle, only when the person chose one.
///
/// `environment` is the POSIX locale the session was started with ([`environment_locale`]); the
/// login screen, with no person's preferences, passes the defaults.
pub fn session_locale(preferences: &Preferences, environment: Option<&str>) -> String {
    let (environment_language, environment_region) =
        environment.map(posix_parts).unwrap_or_default();
    let language = preferences
        .language
        .clone()
        .or(environment_language.filter(|l| LANGUAGES.contains(&l.as_str())))
        .unwrap_or_else(|| FALLBACK_LANGUAGE.to_owned());
    let region = preferences
        .region
        .clone()
        .or(environment_region)
        .unwrap_or_else(|| likely_region(&language).to_owned());
    let mut tag = format!("{language}-{region}");
    if preferences.hour_cycle != HourCycle::Auto {
        tag.push_str("-u-hc-");
        tag.push_str(preferences.hour_cycle.name());
    }
    tag
}

/// The POSIX locale messages are in, from this process's environment: `LC_ALL`, else
/// `LC_MESSAGES`, else `LANG` (an empty one is unset, as POSIX says).
pub fn environment_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chosen(language: &str, region: &str, hour_cycle: &str) -> Preferences {
        let mut preferences = Preferences::default();
        preferences.set("language", language).unwrap();
        preferences.set("region", region).unwrap();
        preferences.set("hour_cycle", hour_cycle).unwrap();
        preferences
    }

    #[test]
    fn nothing_chosen_is_the_environment() {
        let none = Preferences::default();
        assert_eq!(session_locale(&none, Some("es_ES.UTF-8")), "es-ES");
        assert_eq!(session_locale(&none, Some("es_MX.UTF-8")), "es-MX");
        assert_eq!(session_locale(&none, Some("en_GB.UTF-8@euro")), "en-GB");
        assert_eq!(session_locale(&none, Some("es")), "es-ES");
        assert_eq!(session_locale(&none, Some("en")), "en-US");
    }

    #[test]
    fn a_language_oxyos_does_not_speak_is_english_formatted_for_its_territory() {
        let none = Preferences::default();
        assert_eq!(session_locale(&none, Some("fr_FR.UTF-8")), "en-FR");
        assert_eq!(session_locale(&none, Some("de")), "en-US");
    }

    #[test]
    fn c_posix_and_nothing_are_english_in_the_united_states() {
        let none = Preferences::default();
        for environment in [None, Some(""), Some("C"), Some("C.UTF-8"), Some("POSIX")] {
            assert_eq!(
                session_locale(&none, environment),
                "en-US",
                "{environment:?}"
            );
        }
    }

    #[test]
    fn the_persons_choice_is_over_the_environment() {
        assert_eq!(
            session_locale(&chosen("en", "", "auto"), Some("es_ES.UTF-8")),
            "en-ES"
        );
        assert_eq!(
            session_locale(&chosen("es", "MX", "h12"), Some("en_US.UTF-8")),
            "es-MX-u-hc-h12"
        );
        assert_eq!(
            session_locale(&chosen("es", "", "h23"), None),
            "es-ES-u-hc-h23"
        );
        assert_eq!(
            session_locale(&chosen("", "GB", "auto"), Some("C")),
            "en-GB"
        );
    }

    #[test]
    fn the_keys_take_only_what_they_name() {
        let mut preferences = Preferences::default();
        assert_eq!(preferences.set("language", "fr"), Err(Refused::BadValue));
        assert_eq!(preferences.set("language", "ES"), Err(Refused::BadValue));
        assert_eq!(preferences.set("region", "es"), Err(Refused::BadValue));
        assert_eq!(preferences.set("region", "ESP"), Err(Refused::BadValue));
        assert_eq!(preferences.set("region", "419"), Err(Refused::BadValue));
        assert_eq!(preferences.set("hour_cycle", "h24"), Err(Refused::BadValue));
        assert_eq!(preferences, Preferences::default());
        assert_eq!(preferences.set("language", "es"), Ok(()));
        assert_eq!(preferences.set("language", ""), Ok(()));
        assert_eq!(preferences.language, None);
    }

    #[test]
    fn the_environment_is_read_in_posix_order() {
        assert_eq!(
            posix_parts("es_MX.UTF-8"),
            (Some("es".into()), Some("MX".into()))
        );
        assert_eq!(
            posix_parts("ca_ES@valencia"),
            (Some("ca".into()), Some("ES".into()))
        );
        assert_eq!(posix_parts("C.UTF-8"), (None, None));
    }
}
