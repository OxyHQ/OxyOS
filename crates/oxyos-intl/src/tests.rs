//! Expected strings are CLDR's (the data ICU4X 2.1 compiles in, CLDR 48) as Chrome and Android
//! print them for the same locale and options; where OxyOS prints something else, the test says
//! so and why.

use super::*;

/// 2026-10-29T14:05:09.123Z, a Thursday.
const THURSDAY: f64 = 1_793_282_709_123.0;
/// 2026-10-29T09:05:00Z.
const MORNING: f64 = 1_793_264_700_000.0;
/// Noon UTC the day before and the day after Europe's clocks went back (2026-10-25).
const BEFORE_DST_ENDS: f64 = 1_792_843_200_000.0;
const AFTER_DST_ENDS: f64 = 1_793_016_000_000.0;

fn date(locale: &str, options: &[(&str, &str)], epoch_ms: f64) -> String {
    text(&date::format(locale, options, epoch_ms).unwrap())
}

fn number(locale: &str, options: &[(&str, &str)], value: f64) -> String {
    text(&number::format(locale, options, value).unwrap())
}

fn kinds(parts: &[Part]) -> Vec<(&str, &str)> {
    parts
        .iter()
        .map(|part| (part.kind, part.value.as_str()))
        .collect()
}

fn resolved<'a>(pairs: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn tags_are_canonicalized_and_invalid_ones_refused() {
    assert_eq!(canonicalize("EN-us").unwrap(), "en-US");
    // CLDR's language aliases.
    assert_eq!(canonicalize("iw").unwrap(), "he");
    assert_eq!(canonicalize("zh-hant-tw").unwrap(), "zh-Hant-TW");
    assert_eq!(canonicalize("en-u-hc-h23").unwrap(), "en-u-hc-h23");
    // ECMA-402's RangeErrors: `_` (UTS 35 allows it, BCP 47 does not), a variant or singleton
    // twice, no language.
    for invalid in [
        "en_US",
        "en-GB-oed-oed",
        "en-u-ca-gregory-u-nu-latn",
        "x-private",
        "",
        "e",
    ] {
        let error = canonicalize(invalid).unwrap_err();
        assert_eq!(
            error.0,
            format!("Incorrect locale information provided: {invalid:?}")
        );
    }
}

#[test]
fn the_lookup_matcher_takes_the_first_tag_with_data() {
    assert_eq!(resolve("xx,es-ES,fr").unwrap(), "es-ES");
    // Shorter tags in turn; a variant only with data of its own.
    assert_eq!(resolve("de-DE-1996").unwrap(), "de-DE");
    // ICU4X's fallback takes zh-TW to zh-Hant's data.
    assert_eq!(resolve("zh-TW").unwrap(), "zh-TW");
    assert_eq!(resolve("sr-Latn-RS").unwrap(), "sr-Latn-RS");
    // Unicode extension keywords stay (each format keeps those relevant to it); other
    // extensions go.
    assert_eq!(resolve("en-US-u-hc-h23").unwrap(), "en-US-u-hc-h23");
    assert_eq!(resolve("en-x-private").unwrap(), "en");
    // Deviation: a browser gives `es` (ICU has no es-XX); OxyOS cannot tell a region without data
    // of its own (CLDR's es-ES equals es) from an unknown one.
    assert_eq!(resolve("es-XX").unwrap(), "es-XX");
    // Nothing with data, or nothing asked for: the system's locale.
    assert_eq!(resolve("xx").unwrap(), default_locale());
    assert_eq!(resolve("").unwrap(), default_locale());
    assert!(resolve("en,en_US").is_err());
}

/// An environment with only these variables.
fn environment<'a>(pairs: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn the_default_locale_follows_posix() {
    assert_eq!(
        system_locale(environment(&[("LANG", "es_ES.UTF-8")])),
        "es-ES"
    );
    assert_eq!(
        system_locale(environment(&[
            ("LANG", "es_ES.UTF-8"),
            ("LC_MESSAGES", "de_DE")
        ])),
        "de-DE"
    );
    assert_eq!(
        system_locale(environment(&[
            ("LC_ALL", "fr_FR@euro"),
            ("LC_MESSAGES", "de_DE")
        ])),
        "fr-FR"
    );
    // An empty variable is as good as none.
    assert_eq!(
        system_locale(environment(&[("LC_ALL", ""), ("LANG", "pt_BR.UTF-8")])),
        "pt-BR"
    );
    assert_eq!(
        system_locale(environment(&[("LANG", "sr_RS@latin")])),
        "sr-Latn-RS"
    );
    for none in ["C", "C.UTF-8", "POSIX", "xx_YY"] {
        assert_eq!(
            system_locale(environment(&[("LANG", none)])),
            "en-US",
            "{none}"
        );
    }
    assert_eq!(system_locale(environment(&[])), "en-US");
}

#[test]
fn a_long_month_date_in_spanish() {
    // CLDR es: `d 'de' MMMM 'de' y`.
    let options = [
        ("day", "numeric"),
        ("month", "long"),
        ("year", "numeric"),
        ("timeZone", "UTC"),
    ];
    let parts = date::format("es-ES", &options, THURSDAY).unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("day", "29"),
            ("literal", " de "),
            ("month", "octubre"),
            ("literal", " de "),
            ("year", "2026")
        ]
    );
    let pairs = date::resolve("es-ES", &options).unwrap();
    assert_eq!(
        pairs,
        [
            ("locale", "es-ES".to_string()),
            ("calendar", "gregory".into()),
            ("numberingSystem", "latn".into()),
            ("timeZone", "UTC".into()),
            ("year", "numeric".into()),
            ("month", "long".into()),
            ("day", "numeric".into()),
        ]
    );
}

#[test]
fn styles_in_english_and_german() {
    let utc = ("timeZone", "UTC");
    // CLDR 42 put U+202F (narrow no-break space) before AM and PM; Chrome 110 and Android 14
    // print it.
    let parts = date::format("en-US", &[("timeStyle", "short"), utc], THURSDAY).unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("hour", "2"),
            ("literal", ":"),
            ("minute", "05"),
            ("literal", "\u{202f}"),
            ("dayPeriod", "PM")
        ]
    );
    assert_eq!(
        date("en-US", &[("dateStyle", "full"), utc], THURSDAY),
        "Thursday, October 29, 2026"
    );
    assert_eq!(
        date("en-US", &[("dateStyle", "short"), utc], THURSDAY),
        "10/29/26"
    );
    // CLDR's `{1} 'at' {0}` joins a long date and a time.
    assert_eq!(
        date(
            "en-US",
            &[("dateStyle", "long"), ("timeStyle", "short"), utc],
            THURSDAY
        ),
        "October 29, 2026 at 2:05\u{202f}PM"
    );
    assert_eq!(
        date(
            "en-US",
            &[("dateStyle", "medium"), ("timeStyle", "medium"), utc],
            THURSDAY
        ),
        "Oct 29, 2026, 2:05:09\u{202f}PM"
    );
    assert_eq!(
        date("de-DE", &[("dateStyle", "full"), utc], THURSDAY),
        "Donnerstag, 29. Oktober 2026"
    );
    let pairs = date::resolve("en-US", &[("dateStyle", "full"), ("timeStyle", "short")]).unwrap();
    assert_eq!(resolved(&pairs, "dateStyle"), Some("full"));
    assert_eq!(resolved(&pairs, "timeStyle"), Some("short"));
    assert_eq!(resolved(&pairs, "hourCycle"), Some("h12"));
    assert_eq!(resolved(&pairs, "weekday"), None);
}

#[test]
fn fields_and_their_widths() {
    let utc = ("timeZone", "UTC");
    // ToDateTimeOptions' default: year, month and day, numeric.
    assert_eq!(date("en-US", &[utc], THURSDAY), "10/29/2026");
    assert_eq!(
        date(
            "en-US",
            &[
                ("weekday", "long"),
                ("year", "numeric"),
                ("month", "long"),
                ("day", "numeric"),
                utc
            ],
            THURSDAY
        ),
        "Thursday, October 29, 2026"
    );
    assert_eq!(
        date(
            "en-US",
            &[
                ("weekday", "short"),
                ("month", "short"),
                ("day", "numeric"),
                utc
            ],
            THURSDAY
        ),
        "Thu, Oct 29"
    );
    assert_eq!(
        date(
            "en-US",
            &[("month", "long"), ("year", "numeric"), utc],
            THURSDAY
        ),
        "October 2026"
    );
    assert_eq!(
        date(
            "en-US",
            &[
                ("year", "2-digit"),
                ("month", "2-digit"),
                ("day", "2-digit"),
                utc
            ],
            THURSDAY
        ),
        "10/29/26"
    );
    assert_eq!(
        date(
            "en-US",
            &[("era", "short"), ("year", "numeric"), utc],
            THURSDAY
        ),
        "2026 AD"
    );
    let parts = date::format(
        "en-US",
        &[
            ("hour", "numeric"),
            ("minute", "numeric"),
            ("second", "numeric"),
            ("fractionalSecondDigits", "3"),
            utc,
        ],
        THURSDAY,
    )
    .unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("hour", "2"),
            ("literal", ":"),
            ("minute", "05"),
            ("literal", ":"),
            ("second", "09"),
            ("literal", "."),
            ("fractionalSecond", "123"),
            ("literal", "\u{202f}"),
            ("dayPeriod", "PM")
        ]
    );
    // Deviation: ICU4X's lengths. A narrow weekday comes out abbreviated (a browser: `T`), and a
    // long weekday beside a numeric month as wide as the month (a browser: `Thursday, 10/29/2026`).
    assert_eq!(
        date("en-US", &[("weekday", "narrow"), utc], THURSDAY),
        "Thu"
    );
    assert_eq!(
        date(
            "en-US",
            &[
                ("weekday", "long"),
                ("year", "numeric"),
                ("month", "numeric"),
                ("day", "numeric"),
                utc
            ],
            THURSDAY
        ),
        "Thu, 10/29/2026"
    );
}

#[test]
fn hour_cycles() {
    let utc = ("timeZone", "UTC");
    let two_digits = [("hour", "2-digit"), ("minute", "2-digit"), utc];
    // A 2-digit hour on the 24-hour clock; en-US's own clock is 12-hour, padded (`hh:mm a`).
    assert_eq!(
        date(
            "en-US",
            &[two_digits.as_slice(), &[("hourCycle", "h23")]].concat(),
            MORNING
        ),
        "09:05"
    );
    assert_eq!(date("en-US", &two_digits, MORNING), "09:05\u{202f}AM");
    assert_eq!(
        date(
            "en-US",
            &[("hour", "numeric"), ("hour12", "false"), utc],
            THURSDAY
        ),
        "14"
    );
    // en-GB and de-DE are on the 24-hour clock (CLDR's `HH:mm`).
    assert_eq!(
        date(
            "en-GB",
            &[("hour", "numeric"), ("minute", "numeric"), utc],
            MORNING
        ),
        "09:05"
    );
    let pairs = date::resolve("de-DE", &[("hour", "numeric"), ("minute", "numeric")]).unwrap();
    assert_eq!(resolved(&pairs, "hourCycle"), Some("h23"));
    assert_eq!(resolved(&pairs, "hour12"), Some("false"));
    // The pattern's `HH`, as a browser reports it.
    assert_eq!(resolved(&pairs, "hour"), Some("2-digit"));
    // The locale's -u-hc, unless an option overrides it (and then the locale loses it).
    assert_eq!(
        date("en-US-u-hc-h23", &[("hour", "numeric"), utc], THURSDAY),
        "14"
    );
    let pairs =
        date::resolve("en-US-u-hc-h23", &[("hour", "numeric"), ("hour12", "true")]).unwrap();
    assert_eq!(resolved(&pairs, "locale"), Some("en-US"));
    assert_eq!(resolved(&pairs, "hourCycle"), Some("h12"));
    let pairs = date::resolve("en-US-u-hc-h23-co-phonebk", &[("hour", "numeric")]).unwrap();
    assert_eq!(resolved(&pairs, "locale"), Some("en-US-u-hc-h23"));
    // No hour, no hour cycle.
    let pairs = date::resolve("en-US", &[("hour12", "true")]).unwrap();
    assert_eq!(resolved(&pairs, "hourCycle"), None);
}

#[test]
fn a_named_zone_across_the_end_of_summer_time() {
    let madrid = [
        ("hour", "numeric"),
        ("minute", "numeric"),
        ("timeZone", "Europe/Madrid"),
    ];
    let utc = [
        ("hour", "numeric"),
        ("minute", "numeric"),
        ("timeZone", "UTC"),
    ];
    // Noon UTC is 14:00 in Madrid on summer time (UTC+2), 13:00 after (UTC+1).
    assert_eq!(date("es-ES", &madrid, BEFORE_DST_ENDS), "14:00");
    assert_eq!(date("es-ES", &madrid, AFTER_DST_ENDS), "13:00");
    assert_eq!(date("es-ES", &utc, BEFORE_DST_ENDS), "12:00");
    assert_eq!(date("es-ES", &utc, AFTER_DST_ENDS), "12:00");
    // New York's clocks go back a week later (2026-11-01): still UTC-4.
    let new_york = [("hour", "numeric"), ("timeZone", "America/New_York")];
    assert_eq!(date("en-US", &new_york, AFTER_DST_ENDS), "8\u{202f}AM");
}

#[test]
fn time_zones_are_canonicalized() {
    let zone = |name| {
        let pairs = date::resolve("en-US", &[("timeZone", name)]).unwrap();
        resolved(&pairs, "timeZone").unwrap().to_string()
    };
    assert_eq!(zone("europe/madrid"), "Europe/Madrid");
    assert_eq!(zone("etc/gmt"), "UTC");
    assert_eq!(zone("Etc/UTC"), "UTC");
    assert_eq!(zone("+0530"), "+05:30");
    assert_eq!(zone("-00:00"), "+00:00");
    assert_eq!(
        date(
            "en-US",
            &[("hour", "numeric"), ("timeZone", "-05")],
            MORNING
        ),
        "4\u{202f}AM"
    );
    for invalid in ["Mars/Olympus", "+5", "+05:30:00", "+24:00"] {
        let error = date::format("en-US", &[("timeZone", invalid)], MORNING).unwrap_err();
        assert_eq!(error.0, format!("Invalid time zone specified: {invalid}"));
    }
    // The system's zone has a name, or is UTC.
    let pairs = date::resolve("en-US", &[]).unwrap();
    assert!(resolved(&pairs, "timeZone").is_some_and(|name| !name.is_empty()));
}

#[test]
fn date_options_oxyos_does_not_support_are_refused() {
    let refused = |locale: &str, options: &[(&str, &str)]| {
        let error = date::format(locale, options, THURSDAY).unwrap_err();
        assert!(
            error.0.ends_with("is not supported on OxyOS yet"),
            "{error}"
        );
    };
    refused("en-US", &[("timeZoneName", "short")]);
    refused("en-US", &[("dayPeriod", "long"), ("hour", "numeric")]);
    refused("en-US", &[("timeStyle", "long")]);
    refused("en-US", &[("timeStyle", "full")]);
    refused("en-US", &[("hour", "numeric"), ("hourCycle", "h24")]);
    refused("en-US", &[("calendar", "buddhist")]);
    refused("en-US", &[("numberingSystem", "arab")]);
    refused("th-TH-u-ca-buddhist", &[]);
    refused("en-US-u-hc-h24", &[]);
    refused("en-US", &[("minute", "2-digit"), ("second", "2-digit")]);
    refused("en-US", &[("year", "numeric"), ("weekday", "long")]);
    refused("en-US", &[("month", "long"), ("hour", "numeric")]);
    refused(
        "en-US",
        &[("era", "long"), ("month", "long"), ("day", "numeric")],
    );
    // What ECMA-402 throws for anyway.
    let error = |options: &[(&str, &str)], epoch_ms| {
        date::format("en-US", options, epoch_ms).unwrap_err().0
    };
    assert_eq!(
        error(&[("month", "foo")], THURSDAY),
        "Value foo out of range for Intl.DateTimeFormat options property month"
    );
    assert_eq!(
        error(&[("dateStyle", "long"), ("year", "numeric")], THURSDAY),
        "Can't set option year when dateStyle or timeStyle is used"
    );
    assert_eq!(error(&[], f64::NAN), "Invalid time value");
    assert_eq!(error(&[], 8.64e15 + 1.0), "Invalid time value");
    assert_eq!(
        error(&[("weekday", "long"), ("colour", "red")], THURSDAY),
        "Intl.DateTimeFormat has no option colour"
    );
    // The gregorian calendar and Latin digits, asked for, are what OxyOS formats.
    assert!(date::format(
        "en-US",
        &[("calendar", "gregory"), ("numberingSystem", "latn")],
        0.0
    )
    .is_ok());
}

#[test]
fn decimals_group_as_each_locale_does() {
    let parts = number::format("de-DE", &[], 1234.5).unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("integer", "1"),
            ("group", "."),
            ("integer", "234"),
            ("decimal", ","),
            ("fraction", "5")
        ]
    );
    assert_eq!(number("en-US", &[], 1234567.891), "1,234,567.891");
    // CLDR es: a group separator only from five digits (minimumGroupingDigits 2).
    assert_eq!(number("es-ES", &[], 1234.5), "1234,5");
    assert_eq!(number("es-ES", &[], 12345.5), "12.345,5");
    // CLDR fr: U+202F between groups.
    assert_eq!(number("fr-FR", &[], 1234567.5), "1\u{202f}234\u{202f}567,5");
    assert_eq!(
        number("en-US", &[("useGrouping", "false")], 1234567.0),
        "1234567"
    );
    assert_eq!(number("es-ES", &[("useGrouping", "min2")], 1234.0), "1234");
    // Deviation: ICU4X groups `always` as `auto`, so es keeps its minimum of two digits before
    // a separator (a browser: `1.234`).
    assert_eq!(
        number("es-ES", &[("useGrouping", "always")], 1234.0),
        "1234"
    );
    assert_eq!(number("en-US", &[("useGrouping", "min2")], 1234.0), "1234");
    // Deviation: OxyOS formats Latin digits only (a browser: `١٬٢٣٤٫٥`).
    assert_eq!(number("ar-EG", &[], 1234.5), "1,234.5");
}

#[test]
fn digit_options_and_their_defaults() {
    // A decimal: at most 3 fraction digits, rounded half away from zero, from the shortest
    // decimal that reads back as the double (1.005 is 1.00499999999999989...).
    assert_eq!(number("en-US", &[], 1.23456), "1.235");
    assert_eq!(
        number("en-US", &[("maximumFractionDigits", "2")], 1.005),
        "1.01"
    );
    assert_eq!(
        number("en-US", &[("minimumFractionDigits", "2")], 1.5),
        "1.50"
    );
    assert_eq!(
        number("en-US", &[("minimumIntegerDigits", "3")], 7.0),
        "007"
    );
    assert_eq!(
        number("en-US", &[("maximumSignificantDigits", "3")], 123456.0),
        "123,000"
    );
    assert_eq!(
        number("en-US", &[("maximumSignificantDigits", "2")], 9.99),
        "10"
    );
    assert_eq!(
        number("en-US", &[("minimumSignificantDigits", "3")], 0.0),
        "0.00"
    );
    assert_eq!(
        number(
            "en-US",
            &[("maximumFractionDigits", "0"), ("roundingMode", "floor")],
            -1.5
        ),
        "-2"
    );
    assert_eq!(
        number(
            "en-US",
            &[("maximumFractionDigits", "0"), ("roundingMode", "halfEven")],
            2.5
        ),
        "2"
    );
    assert_eq!(
        number(
            "en-US",
            &[
                ("minimumFractionDigits", "2"),
                ("trailingZeroDisplay", "stripIfInteger")
            ],
            3.0
        ),
        "3"
    );
    let pairs = number::resolve("en-US", &[("maximumFractionDigits", "1")]).unwrap();
    assert_eq!(resolved(&pairs, "minimumFractionDigits"), Some("0"));
    assert_eq!(resolved(&pairs, "maximumFractionDigits"), Some("1"));
    // A minimum above the default maximum raises the maximum.
    let pairs = number::resolve("en-US", &[("minimumFractionDigits", "5")]).unwrap();
    assert_eq!(resolved(&pairs, "maximumFractionDigits"), Some("5"));
    let pairs = number::resolve("en-US", &[("maximumSignificantDigits", "4")]).unwrap();
    assert_eq!(resolved(&pairs, "minimumSignificantDigits"), Some("1"));
    assert_eq!(resolved(&pairs, "minimumFractionDigits"), None);
    let pairs = number::resolve("en-US-u-nu-latn-hc-h23", &[]).unwrap();
    assert_eq!(
        pairs,
        [
            ("locale", "en-US-u-nu-latn".to_string()),
            ("numberingSystem", "latn".into()),
            ("style", "decimal".into()),
            ("minimumIntegerDigits", "1".into()),
            ("minimumFractionDigits", "0".into()),
            ("maximumFractionDigits", "3".into()),
            ("useGrouping", "auto".into()),
            ("notation", "standard".into()),
            ("signDisplay", "auto".into()),
            ("roundingIncrement", "1".into()),
            ("roundingMode", "halfExpand".into()),
            ("roundingPriority", "auto".into()),
            ("trailingZeroDisplay", "auto".into()),
        ]
    );
}

#[test]
fn signs_zeros_and_non_finite_numbers() {
    // ECMA-402 keeps negative zero's sign by default.
    assert_eq!(number("en-US", &[], -0.0), "-0");
    assert_eq!(number("en-US", &[("signDisplay", "negative")], -0.0), "0");
    assert_eq!(number("en-US", &[("signDisplay", "exceptZero")], 0.0), "0");
    assert_eq!(number("en-US", &[("signDisplay", "exceptZero")], 5.0), "+5");
    assert_eq!(number("en-US", &[("signDisplay", "never")], -5.0), "5");
    assert_eq!(
        kinds(&number::format("en-US", &[], f64::NAN).unwrap()),
        [("nan", "NaN")]
    );
    assert_eq!(
        kinds(&number::format("en-US", &[], f64::NEG_INFINITY).unwrap()),
        [("minusSign", "-"), ("infinity", "∞")]
    );
}

#[test]
fn currencies() {
    let currency = |code| [("style", "currency"), ("currency", code)];
    let parts = number::format("es-ES", &currency("EUR"), 1234.5).unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("integer", "1234"),
            ("decimal", ","),
            ("fraction", "50"),
            ("literal", "\u{a0}"),
            ("currency", "€")
        ]
    );
    assert_eq!(
        number("es-ES", &currency("USD"), 1234.5),
        "1234,50\u{a0}US$"
    );
    assert_eq!(number("en-US", &currency("EUR"), 1234.5), "€1,234.50");
    // The sign before the whole pattern (CLDR's implicit negative pattern).
    let parts = number::format("en-US", &currency("USD"), -1234.5).unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("minusSign", "-"),
            ("currency", "$"),
            ("integer", "1"),
            ("group", ","),
            ("integer", "234"),
            ("decimal", "."),
            ("fraction", "50")
        ]
    );
    // ISO 4217's minor units: none for the yen (CLDR ja's symbol is the full-width ￥).
    assert_eq!(number("ja-JP", &currency("JPY"), 1234.5), "￥1,235");
    let pairs = number::resolve("en-US", &currency("kwd")).unwrap();
    assert_eq!(resolved(&pairs, "currency"), Some("KWD"));
    assert_eq!(resolved(&pairs, "minimumFractionDigits"), Some("3"));
    let display = |code, display| {
        [
            ("style", "currency"),
            ("currency", code),
            ("currencyDisplay", display),
        ]
    };
    // CLDR en's pattern for letters next to the number, `¤ #,##0.00` with U+00A0.
    assert_eq!(
        number("en-US", &display("USD", "code"), 1234.5),
        "USD\u{a0}1,234.50"
    );
    assert_eq!(
        number("es-MX", &display("USD", "narrowSymbol"), 1234.5),
        "$1,234.50"
    );
    // Names by plural form.
    let parts = number::format("en-US", &display("USD", "name"), 1234.5).unwrap();
    assert_eq!(text(&parts), "1,234.50 US dollars");
    assert_eq!(
        parts.last().unwrap(),
        &Part {
            kind: "currency",
            value: "US dollars".into()
        }
    );
    assert_eq!(number("es-ES", &display("EUR", "name"), 2.0), "2,00 euros");
    // Deviation: CLDR nl's currency pattern has a negative subpattern of its own (`€ -12,34` in a
    // browser), which ICU4X's data leaves out.
    assert_eq!(number("nl-NL", &currency("EUR"), -12.34), "-€\u{a0}12,34");
}

#[test]
fn percentages() {
    assert_eq!(
        kinds(&number::format("en-US", &[("style", "percent")], 0.256).unwrap()),
        [("integer", "26"), ("percentSign", "%")]
    );
    // CLDR de: `#,##0 %` with U+00A0.
    let parts = number::format("de-DE", &[("style", "percent")], -0.256).unwrap();
    assert_eq!(
        kinds(&parts),
        [
            ("minusSign", "-"),
            ("integer", "26"),
            ("literal", "\u{a0}"),
            ("percentSign", "%")
        ]
    );
    assert_eq!(
        number(
            "en-US",
            &[("style", "percent"), ("maximumFractionDigits", "1")],
            0.12345
        ),
        "12.3%"
    );
    assert_eq!(
        number(
            "en-US",
            &[("style", "percent"), ("signDisplay", "exceptZero")],
            -0.0001
        ),
        "0%"
    );
    let pairs = number::resolve("en-US", &[("style", "percent")]).unwrap();
    assert_eq!(resolved(&pairs, "maximumFractionDigits"), Some("0"));
}

#[test]
fn number_options_oxyos_does_not_support_are_refused() {
    let refused = |locale: &str, options: &[(&str, &str)]| {
        let error = number::format(locale, options, 1.0).unwrap_err();
        assert!(
            error.0.ends_with("is not supported on OxyOS yet"),
            "{error}"
        );
    };
    refused("en-US", &[("notation", "compact")]);
    refused("en-US", &[("notation", "scientific")]);
    refused("en-US", &[("style", "unit")]);
    refused("en-US", &[("unit", "meter")]);
    refused("en-US", &[("compactDisplay", "short")]);
    refused(
        "en-US",
        &[
            ("style", "currency"),
            ("currency", "USD"),
            ("currencySign", "accounting"),
        ],
    );
    refused(
        "en-US",
        &[("roundingIncrement", "5"), ("maximumFractionDigits", "2")],
    );
    refused("en-US", &[("roundingPriority", "morePrecision")]);
    refused("en-US", &[("numberingSystem", "arab")]);
    refused("ar-EG-u-nu-arab", &[]);
    let error = |options: &[(&str, &str)]| number::format("en-US", options, 1.0).unwrap_err().0;
    assert_eq!(
        error(&[("style", "currency")]),
        "Currency code is required with currency style."
    );
    assert_eq!(
        error(&[("style", "currency"), ("currency", "EURO")]),
        "Invalid currency code : EURO"
    );
    assert_eq!(
        error(&[
            ("minimumFractionDigits", "4"),
            ("maximumFractionDigits", "2")
        ]),
        "maximumFractionDigits value 2 is out of range"
    );
    assert_eq!(
        error(&[("maximumFractionDigits", "101")]),
        "maximumFractionDigits value \"101\" is out of range for Intl.NumberFormat (from 0 to 100)"
    );
    assert_eq!(
        error(&[("style", "pounds")]),
        "Value pounds out of range for Intl.NumberFormat options property style"
    );
}

#[test]
fn option_bags_from_the_c_abi() {
    assert_eq!(
        options("year=numeric;hour12=true").unwrap(),
        [("year", "numeric"), ("hour12", "true")]
    );
    assert_eq!(options("").unwrap(), []);
    assert!(options("year").is_err());
}
