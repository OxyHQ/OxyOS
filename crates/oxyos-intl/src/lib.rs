//! Internationalization for applications on OxyOS: what ECMA-402's `Intl` needs underneath, for the
//! Bloom host, which installs `Intl` into Hermes (Hermes on Linux has none) and reaches this crate
//! through the C ABI (`oxyos-client-ffi`). The host's JavaScript does ECMA-402's option processing
//! that needs JavaScript values; this crate takes the options as the application gave them, as
//! strings, and does the rest: locale canonicalization and lookup, default digit options, and
//! formatting into ECMA-402's parts.
//!
//! The locale data is CLDR's, compiled into the library by ICU4X (for dates, currencies and
//! percentages only what OxyOS formats; see `data.rs`). Time zones come from the system's time
//! zone database (`/usr/share/zoneinfo`, through `jiff`): the local zone from `TZ` or
//! `/etc/localtime`, and the named IANA zones the database has.
//!
//! Only the Gregorian calendar and Latin digits are formatted. An option this crate does not
//! implement is refused with an error saying so, never ignored; see [`date`] and [`number`] for
//! what each supports and where its output departs from what a browser prints.

// The compiled data names `alloc`'s paths (src/data.rs).
extern crate alloc;

use icu_locale::LocaleCanonicalizer;
use icu_locale_core::extensions::unicode::Key;
use icu_locale_core::{LanguageIdentifier, Locale};
use icu_provider::prelude::*;
use std::fmt;
use writeable::{Part as Annotation, PartsWrite};

mod data;
pub mod date;
pub mod number;
#[cfg(test)]
mod tests;

/// What ECMA-402 throws as a RangeError: an invalid tag or value, or an option OxyOS does not
/// support yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn unsupported(what: impl fmt::Display) -> Error {
    Error(format!("{what} is not supported on OxyOS yet"))
}

/// One part of a formatted value, as `formatToParts` gives it: its ECMA-402 type (`"month"`,
/// `"literal"`, `"integer"`, ...) and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub kind: &'static str,
    pub value: String,
}

/// The parts' text, joined: what `format` gives.
pub fn text(parts: &[Part]) -> String {
    parts.iter().map(|part| part.value.as_str()).collect()
}

/// An option bag as the C ABI carries it: `name=value` pairs joined by `;`.
pub fn options(text: &str) -> Result<Vec<(&str, &str)>, Error> {
    text.split(';')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            pair.split_once('=')
                .ok_or_else(|| Error(format!("option {pair:?} has no value")))
        })
        .collect()
}

/// The locale the system is set to: `LC_ALL`, else `LC_MESSAGES`, else `LANG` (POSIX's order),
/// as a BCP 47 tag OxyOS has data for; `en-US` for `C`, `POSIX`, none or one without data.
pub fn default_locale() -> String {
    system_locale(|name| std::env::var(name).ok())
}

/// [`default_locale`] from the environment `variable` gives.
fn system_locale(variable: impl Fn(&str) -> Option<String>) -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| variable(name).filter(|value| !value.is_empty()))
        .and_then(|value| from_posix(&value))
        .and_then(|locale| lookup(&locale))
        .map_or_else(|| "en-US".into(), |id| id.to_string())
}

/// `es_ES.UTF-8` as `es-ES`; `sr_RS@latin` as `sr-Latn-RS`. `None` for `C` and `POSIX`.
fn from_posix(value: &str) -> Option<Locale> {
    let (name, modifier) = value.split_once('@').unwrap_or((value, ""));
    let name = name.split_once('.').map_or(name, |(name, _codeset)| name);
    if matches!(name, "C" | "POSIX") {
        return None;
    }
    let mut locale = Locale::try_from_str(&name.replace('_', "-")).ok()?;
    let script = match modifier {
        "latin" => Some(icu_locale_core::subtags::script!("Latn")),
        "cyrillic" => Some(icu_locale_core::subtags::script!("Cyrl")),
        _ => None,
    };
    locale.id.script = script.or(locale.id.script);
    LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    Some(locale)
}

/// ECMA-402's CanonicalizeUnicodeLocaleId: `EN-us` as `en-US`, `iw` as `he`. An invalid tag is an
/// error.
pub fn canonicalize(tag: &str) -> Result<String, Error> {
    parse(tag).map(|locale| locale.to_string())
}

fn parse(tag: &str) -> Result<Locale, Error> {
    let invalid = || Error(format!("Incorrect locale information provided: {tag:?}"));
    // UTS 35 also accepts `_` between subtags; BCP 47, and so ECMA-402, does not.
    if tag.contains('_') || !structurally_valid(tag) {
        return Err(invalid());
    }
    let mut locale = Locale::try_from_str(tag).map_err(|_| invalid())?;
    LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    Ok(locale)
}

/// ECMA-402's checks beyond the grammar ICU4X parses: no variant twice, no singleton twice.
fn structurally_valid(tag: &str) -> bool {
    let subtags: Vec<String> = tag.split('-').map(str::to_ascii_lowercase).collect();
    let first_singleton = subtags.iter().position(|subtag| subtag.len() == 1);
    let variants = &subtags[1..first_singleton.unwrap_or(subtags.len()).max(1)];
    let variants: Vec<&String> = variants
        .iter()
        .filter(|subtag| {
            subtag.len() >= 5
                || (subtag.len() == 4 && subtag.starts_with(|c: char| c.is_ascii_digit()))
        })
        .collect();
    let mut singletons: Vec<&String> = Vec::new();
    for subtag in &subtags[first_singleton.unwrap_or(subtags.len())..] {
        if subtag == "x" {
            break;
        }
        if subtag.len() == 1 {
            singletons.push(subtag);
        }
    }
    let unique = |list: &[&String]| {
        let mut sorted = list.to_vec();
        sorted.sort();
        sorted.windows(2).all(|pair| pair[0] != pair[1])
    };
    unique(&variants) && unique(&singletons)
}

/// ECMA-402's LookupMatcher over `requested` (tags joined by `,`): the first that OxyOS has data
/// for, trying each tag and then shorter ones (`de-DE-1996`, `de-DE`, `de`), with its Unicode
/// extension keywords (`-u-hc-h23`; each format keeps those relevant to it) and without its other
/// extensions; else [`default_locale`]. An invalid tag is an error, as in CanonicalizeLocaleList.
pub fn resolve(requested: &str) -> Result<String, Error> {
    let requested = requested
        .split(',')
        .filter(|tag| !tag.is_empty())
        .map(parse)
        .collect::<Result<Vec<_>, _>>()?;
    let found = requested.iter().find_map(|locale| {
        let mut found = Locale::from(lookup(locale)?);
        found.extensions.unicode.keywords = locale.extensions.unicode.keywords.clone();
        Some(found.to_string())
    });
    Ok(found.unwrap_or_else(default_locale))
}

/// The longest prefix of `locale` OxyOS has data for (see [`available`]).
fn lookup(locale: &Locale) -> Option<LanguageIdentifier> {
    let mut id = locale.id.clone();
    loop {
        if available(&id) {
            return Some(id);
        }
        if !id.variants.is_empty() {
            let mut variants = id.variants.to_vec();
            variants.pop();
            id.variants = icu_locale_core::subtags::Variants::from_vec_unchecked(variants);
        } else if id.region.take().is_none() && id.script.take().is_none() {
            return None;
        }
    }
}

/// Whether OxyOS has data for `id`: its language, in its script, has month names of its own
/// (every language with date data has; the root locale's are `M01`...). ICU4X's fallback takes
/// `zh-TW` to `zh-Hant`, and `en-Cyrl` (not English's script) to the root.
///
/// CLDR's compiled data leaves out a locale whose data equals its parent's (`es-ES`'s is `es`'s),
/// so any region is taken (`es-XX` too, where a browser gives `es`); a variant only when the data
/// has it exactly.
fn available(id: &LanguageIdentifier) -> bool {
    use icu_datetime::provider::neo::DatetimeNamesMonthGregorianV1;
    if id.language.is_unknown() {
        return false;
    }
    let locale = DataLocale::from(id);
    // Abbreviated, wide and narrow names: the data leaves out a width equal to another.
    let found = ["3", "4", "5"].iter().find_map(|attributes| {
        let request = DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(
                DataMarkerAttributes::from_str_or_panic(attributes),
                &locale,
            ),
            ..Default::default()
        };
        let response =
            DataProvider::<DatetimeNamesMonthGregorianV1>::load(&data::Baked, request).ok()?;
        // Found as asked (no locale), or through fallback for the locale it names.
        match response.metadata.locale {
            Some(found) if found.is_unknown() => None,
            found => Some(found),
        }
    });
    match found {
        Some(None) => true,
        Some(Some(_)) => id.variants.is_empty(),
        None => false,
    }
}

/// A locale [`resolve`] gave, for one format: its Unicode extension keywords cut to those
/// `relevant` to it (ECMA-402's relevantExtensionKeys), refusing those this crate cannot honour:
/// `-u-ca` but `gregory`, `-u-nu` but `latn`, `-u-hc-h24` (ICU4X has no such clock).
fn formatting_locale(tag: &str, relevant: &[Key]) -> Result<Locale, Error> {
    let parsed = parse(tag)?;
    let mut locale = Locale::from(parsed.id);
    for (key, value) in parsed.extensions.unicode.keywords.iter() {
        if !relevant.contains(key) {
            continue;
        }
        let text = value.to_string();
        let refused = match key.as_str() {
            "ca" => text != "gregory",
            "nu" => text != "latn",
            "hc" => text == "h24",
            _ => false,
        };
        if refused {
            return Err(unsupported(format!("The locale's -u-{key}-{text}")));
        }
        locale.extensions.unicode.keywords.set(*key, value.clone());
    }
    Ok(locale)
}

/// The numbering system to ask ICU4X for so that digits are Latin, whatever the locale's default
/// (OxyOS formats only those yet): none when they are its default already, since ICU4X keeps a
/// locale's default symbols without the numbering system's name.
fn latin_digits(
    locale: &Locale,
) -> Option<icu_locale_core::preferences::extensions::unicode::keywords::NumberingSystem> {
    use icu_decimal::provider::{Baked, DecimalSymbolsV1};
    use icu_locale_core::extensions::unicode::value;
    let locale = DataLocale::from(locale);
    let request = DataRequest {
        id: DataIdentifierBorrowed::for_locale(&locale),
        ..Default::default()
    };
    let default = DataProvider::<DecimalSymbolsV1>::load(&Baked, request);
    if default.is_ok_and(|symbols| symbols.payload.get().numsys() == "latn") {
        return None;
    }
    value!("latn").try_into().ok()
}

/// Text written through `writeable`, in runs: each with the parts it is inside, outermost first.
#[derive(Default)]
struct Runs {
    runs: Vec<(Vec<Annotation>, String)>,
    inside: Vec<Annotation>,
}

impl Runs {
    fn of(value: &impl writeable::Writeable) -> Vec<(Vec<Annotation>, String)> {
        let mut runs = Runs::default();
        // Writing into memory cannot fail.
        let _ = value.write_to_parts(&mut runs);
        runs.runs
    }
}

impl fmt::Write for Runs {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        match self.runs.last_mut() {
            _ if text.is_empty() => {}
            Some((inside, run)) if *inside == self.inside => run.push_str(text),
            _ => self.runs.push((self.inside.clone(), text.into())),
        }
        Ok(())
    }
}

impl PartsWrite for Runs {
    type SubPartsWrite = Self;

    fn with_part(
        &mut self,
        part: Annotation,
        mut f: impl FnMut(&mut Self) -> fmt::Result,
    ) -> fmt::Result {
        self.inside.push(part);
        let result = f(self);
        self.inside.pop();
        result
    }
}

/// Parts, with neighbouring literals merged as ECMA-402's are.
fn push(parts: &mut Vec<Part>, kind: &'static str, value: &str) {
    match parts.last_mut() {
        _ if value.is_empty() => {}
        Some(last) if kind == "literal" && last.kind == "literal" => last.value.push_str(value),
        _ => parts.push(Part {
            kind,
            value: value.into(),
        }),
    }
}

/// A number option (ECMA-402's GetNumberOption): an integer from `minimum` to `maximum`, floored.
fn number(service: &str, name: &str, value: &str, minimum: u8, maximum: u8) -> Result<u8, Error> {
    let range = || {
        Error(format!(
            "{name} value {value:?} is out of range for Intl.{service} (from {minimum} to {maximum})"
        ))
    };
    let number: f64 = value.trim().parse().map_err(|_| range())?;
    if !(f64::from(minimum)..=f64::from(maximum)).contains(&number) {
        return Err(range());
    }
    // In range, so it fits.
    Ok(number.floor() as u8)
}

/// A string option (ECMA-402's GetOption with values): one of `allowed`.
fn choice(
    service: &str,
    name: &str,
    value: &str,
    allowed: &[&'static str],
) -> Result<&'static str, Error> {
    allowed
        .iter()
        .find(|allowed| **allowed == value)
        .copied()
        .ok_or_else(|| {
            Error(format!(
                "Value {value} out of range for Intl.{service} options property {name}"
            ))
        })
}

/// A boolean option.
fn boolean(service: &str, name: &str, value: &str) -> Result<bool, Error> {
    choice(service, name, value, &["true", "false"]).map(|value| value == "true")
}
