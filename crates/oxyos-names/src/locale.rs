//! The system's locale as POSIX names it (`language[_territory][.codeset][@modifier]`, as
//! `es_ES.UTF-8` or `sr_RS@latin`): which variable it is read from ([`messages_locale`]) and its
//! parts ([`PosixLocale`]). Read and split here, once; what a consumer makes of the parts (a BCP 47
//! tag, a Desktop Entry's `Name[ll_CC]` keys, the reader's voice, the session's language and
//! region) is the consumer's.

/// The variables the locale of messages is read from, in POSIX's order: `LC_ALL` over
/// `LC_MESSAGES` over `LANG`.
pub const MESSAGES_VARIABLES: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// The locale messages are in, as `variable` gives the environment: the first of
/// [`MESSAGES_VARIABLES`] set and not empty (an empty one is unset, as POSIX says).
pub fn messages_locale(variable: impl Fn(&str) -> Option<String>) -> Option<String> {
    MESSAGES_VARIABLES
        .into_iter()
        .find_map(|name| variable(name).filter(|value| !value.is_empty()))
}

/// [`messages_locale`] from this process's environment (a value that is not UTF-8 is unset).
pub fn environment_messages_locale() -> Option<String> {
    messages_locale(|name| std::env::var(name).ok())
}

/// A POSIX locale's parts, as written (neither validated nor case-folded: a consumer decides
/// what it accepts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PosixLocale<'a> {
    /// `language[_territory]`: what comes before the codeset and the modifier.
    pub name: &'a str,
    /// The name up to its first `_`.
    pub language: &'a str,
    /// The name after its first `_`, if it has one.
    pub territory: Option<&'a str>,
    /// After the `.`, as `UTF-8`.
    pub codeset: Option<&'a str>,
    /// After the `@`, as `latin` or `euro`.
    pub modifier: Option<&'a str>,
}

impl<'a> PosixLocale<'a> {
    /// `value`'s parts; None for no locale: empty, `C` or `POSIX` (with any codeset or modifier,
    /// as `C.UTF-8`), whose messages are the untranslated ones.
    pub fn parse(value: &'a str) -> Option<Self> {
        let (rest, modifier) = match value.split_once('@') {
            Some((rest, modifier)) => (rest, Some(modifier)),
            None => (value, None),
        };
        let (name, codeset) = match rest.split_once('.') {
            Some((name, codeset)) => (name, Some(codeset)),
            None => (rest, None),
        };
        if matches!(name, "" | "C" | "POSIX") {
            return None;
        }
        let (language, territory) = match name.split_once('_') {
            Some((language, territory)) => (language, Some(territory)),
            None => (name, None),
        };
        Some(Self {
            name,
            language,
            territory,
            codeset,
            modifier,
        })
    }
}

/// Locales as systems set them, odd ones too, for consumers' tests to compare their readings
/// over (not part of the rule).
#[doc(hidden)]
pub const EXAMPLES: &[&str] = &[
    "",
    "C",
    "POSIX",
    "C.UTF-8",
    "C.utf8",
    "POSIX@euro",
    "c",
    "posix",
    "en",
    "es",
    "fi",
    "de",
    "en_US",
    "en_US.UTF-8",
    "es_ES.UTF-8",
    "es_MX.UTF-8",
    "es_ES@euro",
    "en_GB.UTF-8@euro",
    "en_US@euro.UTF-8",
    "sr_RS@latin",
    "sr_RS.UTF-8@latin",
    "sr_RS@cyrillic",
    "uz_UZ@cyrillic",
    "ca_ES@valencia",
    "pt_BR",
    "pt_BR.UTF-8",
    "zh_CN.GB18030",
    "fr_FR@euro",
    "de_DE",
    "xx_YY",
    "ES_es.UTF-8",
    "Es_mx",
    "es-MX",
    "es-ES-u-hc-h23",
    "en-US",
    "sr-Latn-RS",
    "_ES",
    "es_",
    "es__ES",
    "en_US_POSIX",
    "en_USA",
    "e1_ES",
    "12",
    "@latin",
    ".UTF-8",
    "es.",
    "es@",
    "es_ES.",
    "es_ES@",
    " es_ES",
    "es_ES ",
    "es ES",
    "es_ES.UTF-8@euro@x",
    "es_ES..UTF-8",
    "a_b_c.d@e",
    "español",
    "zh_Hant_TW",
    "ast_ES",
    "C_ES",
    "POSIX_US",
];

#[cfg(test)]
mod tests;
