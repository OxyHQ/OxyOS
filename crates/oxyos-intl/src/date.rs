//! `Intl.DateTimeFormat`: a time value (milliseconds since the epoch, UTC) in a locale and time
//! zone, through ICU4X's field sets.
//!
//! ECMA-402 lets each field have its own width; an ICU4X field set has one length for all of them,
//! chosen here from the textual month (`long` long, `short` and `narrow` medium), else the weekday
//! (`long` long, `short` medium, `narrow` short), else the era, else short (numeric). So the
//! weekday and era are as wide as the month (`{weekday: "long", month: "numeric"}` gives
//! `Thu, 10/29/2026`, a browser `Thursday, 10/29/2026`), and a narrow month or weekday comes out
//! abbreviated (`Oct`, `Thu`). A `2-digit` month, day or hour pads every numeric field of the
//! pattern, as ICU4X's column alignment does; a `2-digit` year has two digits only where ICU4X's
//! short length gives them (near years, short dates). `hour12: true` is the 12-hour clock numbered
//! 1 to 12 (`h12`) in every locale, `false` the 24-hour one from 0 (`h23`).
//!
//! The calendar is the Gregorian one even where a locale's default is another (a browser gives
//! `th-TH` Buddhist years). Named zones are those the system's database has (`Asia/Calcutta` only
//! with Debian's tzdata-legacy), reported with its case and not resolved through links; a system
//! zone without a name (a POSIX `TZ` rule) is taken as UTC.
//!
//! Refused: `timeZoneName`, `dayPeriod`, `timeStyle` full and long (they show the zone's name),
//! `hourCycle` h24, calendars but `gregory`, numbering systems but `latn`, and the combinations of
//! fields ICU4X has no field set for (a year or weekday without the month and day between them, a
//! time without its hour or with seconds but no minutes, a month or year without a day beside a
//! time).

use super::{boolean, choice, latin_digits, number, push, unsupported, Error, Part, Runs};
use icu_calendar::{Date, Gregorian};
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder};
use icu_datetime::fieldsets::enums::CompositeDateTimeFieldSet;
use icu_datetime::options::{Alignment, Length, SubsecondDigits, TimePrecision, YearStyle};
use icu_datetime::preferences::HourCycle;
use icu_datetime::{DateTimeFormatterPreferences, FixedCalendarDateTimeFormatter};
use icu_locale_core::extensions::unicode::key;
use icu_time::{DateTime, Time};
use jiff::tz::{Offset, TimeZone};
use jiff::Timestamp;

const SERVICE: &str = "DateTimeFormat";
const TEXT: &[&str] = &["long", "short", "narrow"];
const NUMERIC: &[&str] = &["numeric", "2-digit"];
const STYLES: &[&str] = &["full", "long", "medium", "short"];

/// The parts of `epoch_ms` formatted with `options` (as the application gave them) in `locale`,
/// one `oxyos_intl_resolve` gave.
pub fn format(locale: &str, options: &[(&str, &str)], epoch_ms: f64) -> Result<Vec<Part>, Error> {
    let format = DateFormat::new(locale, options)?;
    format.format(epoch_ms)
}

/// `resolvedOptions()`: the locale, calendar, numbering system, time zone, hour cycle (when there
/// is an hour) and the fields or styles the pattern has, in ECMA-402's order.
pub fn resolve(
    locale: &str,
    options: &[(&str, &str)],
) -> Result<Vec<(&'static str, String)>, Error> {
    let format = DateFormat::new(locale, options)?;
    // The pattern can depend on the date (an era for years before the common era); today's is
    // the one an application sees.
    let now = Timestamp::now().as_millisecond() as f64;
    let pattern = format.pattern(now)?;
    let mut resolved = vec![
        ("locale", format.locale.clone()),
        ("calendar", "gregory".into()),
        ("numberingSystem", "latn".into()),
        ("timeZone", format.zone_name.clone()),
    ];
    let fields = fields(&pattern);
    if let Some(cycle) = fields.hour_cycle {
        resolved.push(("hourCycle", cycle.into()));
        resolved.push(("hour12", matches!(cycle, "h11" | "h12").to_string()));
    }
    match (format.styles.0, format.styles.1) {
        (None, None) => {
            for (name, value) in [
                ("weekday", fields.weekday),
                ("era", fields.era),
                ("year", fields.year),
                ("month", fields.month),
                ("day", fields.day),
                ("hour", fields.hour),
                ("minute", fields.minute),
                ("second", fields.second),
            ] {
                if let Some(value) = value {
                    resolved.push((name, value.into()));
                }
            }
            if let Some(digits) = fields.fractional {
                resolved.push(("fractionalSecondDigits", digits.to_string()));
            }
        }
        (date, time) => {
            if let Some(date) = date {
                resolved.push(("dateStyle", date.into()));
            }
            if let Some(time) = time {
                resolved.push(("timeStyle", time.into()));
            }
        }
    }
    Ok(resolved)
}

/// The options as the application gave them, checked.
#[derive(Default)]
struct Options {
    weekday: Option<&'static str>,
    era: Option<&'static str>,
    year: Option<&'static str>,
    month: Option<&'static str>,
    day: Option<&'static str>,
    hour: Option<&'static str>,
    minute: Option<&'static str>,
    second: Option<&'static str>,
    fractional: Option<u8>,
    hour12: Option<bool>,
    hour_cycle: Option<&'static str>,
    date_style: Option<&'static str>,
    time_style: Option<&'static str>,
    time_zone: Option<String>,
}

impl Options {
    fn parse(options: &[(&str, &str)]) -> Result<Self, Error> {
        let mut bag = Options::default();
        for &(name, value) in options {
            let pick = |allowed| choice(SERVICE, name, value, allowed);
            match name {
                "weekday" => bag.weekday = Some(pick(TEXT)?),
                "era" => bag.era = Some(pick(TEXT)?),
                "year" => bag.year = Some(pick(NUMERIC)?),
                "month" => {
                    bag.month = Some(pick(&["numeric", "2-digit", "long", "short", "narrow"])?)
                }
                "day" => bag.day = Some(pick(NUMERIC)?),
                "hour" => bag.hour = Some(pick(NUMERIC)?),
                "minute" => bag.minute = Some(pick(NUMERIC)?),
                "second" => bag.second = Some(pick(NUMERIC)?),
                "fractionalSecondDigits" => {
                    bag.fractional = Some(number(SERVICE, name, value, 1, 3)?)
                }
                "hour12" => bag.hour12 = Some(boolean(SERVICE, name, value)?),
                "hourCycle" => bag.hour_cycle = Some(pick(&["h11", "h12", "h23", "h24"])?),
                "dateStyle" => bag.date_style = Some(pick(STYLES)?),
                "timeStyle" => bag.time_style = Some(pick(STYLES)?),
                "timeZone" => bag.time_zone = Some(value.into()),
                "calendar" if value.eq_ignore_ascii_case("gregory") => {}
                "calendar" => return Err(unsupported(format!("The calendar {value}"))),
                "numberingSystem" if value.eq_ignore_ascii_case("latn") => {}
                "numberingSystem" => {
                    return Err(unsupported(format!("The numbering system {value}")))
                }
                // Resolution and matching are done by the time a format is made.
                "localeMatcher" => _ = pick(&["lookup", "best fit"])?,
                "formatMatcher" => _ = pick(&["basic", "best fit"])?,
                "timeZoneName" | "dayPeriod" => {
                    return Err(unsupported(format!("Intl.DateTimeFormat's {name} option")))
                }
                _ => return Err(Error(format!("Intl.DateTimeFormat has no option {name}"))),
            }
        }
        let fields = [
            ("weekday", bag.weekday.is_some()),
            ("era", bag.era.is_some()),
            ("year", bag.year.is_some()),
            ("month", bag.month.is_some()),
            ("day", bag.day.is_some()),
            ("hour", bag.hour.is_some()),
            ("minute", bag.minute.is_some()),
            ("second", bag.second.is_some()),
            ("fractionalSecondDigits", bag.fractional.is_some()),
        ];
        let styled = bag.date_style.or(bag.time_style).is_some();
        if let Some((field, _)) = fields.iter().find(|(_, given)| *given).filter(|_| styled) {
            return Err(Error(format!(
                "Can't set option {field} when dateStyle or timeStyle is used"
            )));
        }
        // ECMA-402's ToDateTimeOptions: no field (an era is not one) and no style is a date.
        if !styled && fields.iter().all(|(name, given)| *name == "era" || !given) {
            bag.year = Some("numeric");
            bag.month = Some("numeric");
            bag.day = Some("numeric");
        }
        Ok(bag)
    }

    /// The ICU4X field set the options ask for.
    fn field_set(&self) -> Result<FieldSetBuilder, Error> {
        let mut builder = FieldSetBuilder::new();
        if self.date_style.or(self.time_style).is_some() {
            let length = |style| match style {
                "full" | "long" => Length::Long,
                "medium" => Length::Medium,
                _ => Length::Short,
            };
            builder.date_fields = self.date_style.map(|style| match style {
                "full" => DateFields::YMDE,
                _ => DateFields::YMD,
            });
            builder.time_precision = match self.time_style {
                Some("full" | "long") => {
                    return Err(unsupported(
                        "timeStyle full and long (they show the time zone's name)",
                    ))
                }
                Some("medium") => Some(TimePrecision::Second),
                Some(_) => Some(TimePrecision::Minute),
                None => None,
            };
            // CLDR joins a date and a time as the date's style says.
            builder.length = self.date_style.or(self.time_style).map(length);
            return Ok(builder);
        }
        builder.date_fields = match (
            self.year.is_some(),
            self.month.is_some(),
            self.day.is_some(),
            self.weekday.is_some(),
        ) {
            (false, false, false, false) => None,
            (false, false, true, false) => Some(DateFields::D),
            (false, true, true, false) => Some(DateFields::MD),
            (true, true, true, false) => Some(DateFields::YMD),
            (false, false, true, true) => Some(DateFields::DE),
            (false, true, true, true) => Some(DateFields::MDE),
            (true, true, true, true) => Some(DateFields::YMDE),
            (false, false, false, true) => Some(DateFields::E),
            (false, true, false, false) => Some(DateFields::M),
            (true, true, false, false) => Some(DateFields::YM),
            (true, false, false, false) => Some(DateFields::Y),
            _ => {
                return Err(unsupported(
                    "A year or weekday without the month and day between them",
                ))
            }
        };
        if self.era.is_some() && self.year.is_none() {
            return Err(unsupported("An era without the year"));
        }
        builder.time_precision = match (self.hour, self.minute, self.second, self.fractional) {
            (None, None, None, None) => None,
            (Some(_), None, None, None) => Some(TimePrecision::Hour),
            (Some(_), Some(_), None, None) => Some(TimePrecision::Minute),
            (Some(_), Some(_), Some(_), None) => Some(TimePrecision::Second),
            (Some(_), Some(_), Some(_), Some(digits)) => {
                Some(TimePrecision::Subsecond(match digits {
                    1 => SubsecondDigits::S1,
                    2 => SubsecondDigits::S2,
                    _ => SubsecondDigits::S3,
                }))
            }
            _ => {
                return Err(unsupported(
                    "A time without its hour, minutes or seconds before the smaller fields",
                ))
            }
        };
        let period = matches!(
            builder.date_fields,
            Some(DateFields::M | DateFields::YM | DateFields::Y)
        );
        if period && builder.time_precision.is_some() {
            return Err(unsupported(
                "A month or year without the day, beside a time",
            ));
        }
        builder.length = Some(match (self.month, self.weekday, self.era) {
            (Some("long"), ..) => Length::Long,
            (Some("short" | "narrow"), ..) => Length::Medium,
            (Some(_), ..) => Length::Short,
            (None, Some(text), _) | (None, None, Some(text)) => match text {
                "long" => Length::Long,
                "short" => Length::Medium,
                _ => Length::Short,
            },
            (None, None, None) => Length::Short,
        });
        if self.year.is_some() {
            builder.year_style = Some(match (self.era, self.year) {
                (Some(_), _) => YearStyle::WithEra,
                (None, Some("2-digit")) => YearStyle::Auto,
                _ => YearStyle::Full,
            });
        }
        let two_digits = [self.month, self.day, self.hour].contains(&Some("2-digit"));
        // A weekday of its own has no numeric field to align.
        if two_digits && builder.date_fields != Some(DateFields::E) {
            builder.alignment = Some(Alignment::Column);
        }
        Ok(builder)
    }

    /// The hour cycle asked for, if any: `hour12` overrides `hourCycle`.
    fn hour_cycle(&self) -> Result<Option<HourCycle>, Error> {
        Ok(match (self.hour12, self.hour_cycle) {
            (Some(true), _) => Some(HourCycle::H12),
            (Some(false), _) => Some(HourCycle::H23),
            (None, Some("h11")) => Some(HourCycle::H11),
            (None, Some("h12")) => Some(HourCycle::H12),
            (None, Some("h23")) => Some(HourCycle::H23),
            (None, Some(_)) => return Err(unsupported("The hour cycle h24")),
            (None, None) => None,
        })
    }
}

/// A format made from a locale and options.
struct DateFormat {
    formatter: FixedCalendarDateTimeFormatter<Gregorian, CompositeDateTimeFieldSet>,
    locale: String,
    zone: TimeZone,
    zone_name: String,
    styles: (Option<&'static str>, Option<&'static str>),
}

impl DateFormat {
    fn new(locale: &str, options: &[(&str, &str)]) -> Result<Self, Error> {
        let options = Options::parse(options)?;
        let mut parsed = super::formatting_locale(locale, &[key!("ca"), key!("hc"), key!("nu")])?;
        let mut preferences = DateTimeFormatterPreferences::from(&parsed);
        preferences.numbering_system = latin_digits(&parsed);
        if let Some(cycle) = options.hour_cycle()? {
            // ResolveLocale: an option unlike the locale's keyword takes its place.
            if preferences.hour_cycle != Some(cycle) || options.hour12.is_some() {
                parsed.extensions.unicode.keywords.remove(key!("hc"));
            }
            preferences.hour_cycle = Some(cycle);
        }
        let field_set = options
            .field_set()?
            .build_composite_datetime()
            .map_err(|error| unsupported(format!("That combination of fields ({error:?})")))?;
        let formatter = FixedCalendarDateTimeFormatter::try_new_unstable(
            &super::data::Baked,
            preferences,
            field_set,
        )
        .map_err(|error| Error(format!("No date formats for {locale}: {error}")))?;
        let (zone_name, zone) = zone(options.time_zone.as_deref())?;
        Ok(Self {
            formatter,
            locale: parsed.to_string(),
            zone,
            zone_name,
            styles: (options.date_style, options.time_style),
        })
    }

    fn format(&self, epoch_ms: f64) -> Result<Vec<Part>, Error> {
        let datetime = wall_clock(epoch_ms, &self.zone)?;
        let mut parts = Vec::new();
        for (inside, text) in Runs::of(&self.formatter.format(&datetime)) {
            let field = inside.iter().find(|part| part.category == "datetime");
            let innermost = inside.last().map(|part| (part.category, part.value));
            let kind = match field.map(|part| part.value) {
                // ICU4X marks the whole of `05.123` as the second, its digits as a decimal's.
                Some("second") => match innermost {
                    Some(("decimal", "decimal")) => "literal",
                    Some(("decimal", "fraction")) => "fractionalSecond",
                    _ => "second",
                },
                Some("era") => "era",
                Some("year") => "year",
                Some("month") => "month",
                Some("day") => "day",
                Some("weekday") => "weekday",
                Some("dayPeriod") => "dayPeriod",
                Some("hour") => "hour",
                Some("minute") => "minute",
                _ => "literal",
            };
            push(&mut parts, kind, &text);
        }
        Ok(parts)
    }

    /// The CLDR pattern the format uses for `epoch_ms`, such as `h:mm a`.
    fn pattern(&self, epoch_ms: f64) -> Result<String, Error> {
        let datetime = wall_clock(epoch_ms, &self.zone)?;
        Ok(
            writeable::Writeable::write_to_string(&self.formatter.format(&datetime).pattern())
                .into(),
        )
    }
}

/// ECMA-402's time zone: `UTC` (and its aliases), an offset such as `+01:00`, a named IANA zone
/// (`europe/madrid` as `Europe/Madrid`), or the system's when absent. Its name and rules.
fn zone(option: Option<&str>) -> Result<(String, TimeZone), Error> {
    const UTC: &[&str] = &[
        "UTC",
        "Etc/UTC",
        "Etc/UCT",
        "UCT",
        "Etc/Universal",
        "Universal",
        "Etc/Zulu",
        "Zulu",
        "GMT",
        "Etc/GMT",
        "GMT0",
        "Etc/GMT0",
        "GMT+0",
        "Etc/GMT+0",
        "GMT-0",
        "Etc/GMT-0",
        "Greenwich",
        "Etc/Greenwich",
    ];
    let utc = |name: &str| UTC.iter().any(|alias| alias.eq_ignore_ascii_case(name));
    let Some(name) = option else {
        // A system zone without a name (a POSIX `TZ` rule, a copied /etc/localtime) cannot be
        // reported as ECMA-402 requires; UTC is used for both instead.
        let system = TimeZone::try_system().ok();
        return Ok(match system.as_ref().and_then(TimeZone::iana_name) {
            Some(name) if !utc(name) && name != "Etc/Unknown" => {
                (name.into(), system.unwrap_or(TimeZone::UTC))
            }
            _ => ("UTC".into(), TimeZone::UTC),
        });
    };
    let invalid = || Error(format!("Invalid time zone specified: {name}"));
    if utc(name) {
        return Ok(("UTC".into(), TimeZone::UTC));
    }
    if let Some(sign) = name.strip_prefix(['+', '-']).map(|_| &name[..1]) {
        let digits: String = name[1..].chars().filter(|c| *c != ':').collect();
        let well_formed = matches!(
            (name.len(), name.find(':')),
            (3, None) | (5, None) | (6, Some(3))
        ) && digits.chars().all(|c| c.is_ascii_digit());
        if !well_formed {
            return Err(invalid());
        }
        let hours: i32 = digits[..2].parse().map_err(|_| invalid())?;
        let minutes: i32 = match &digits[2..] {
            "" => 0,
            minutes => minutes.parse().map_err(|_| invalid())?,
        };
        if hours > 23 || minutes > 59 {
            return Err(invalid());
        }
        let seconds = (hours * 60 + minutes) * 60 * if sign == "-" { -1 } else { 1 };
        let offset = Offset::from_seconds(seconds).map_err(|_| invalid())?;
        let sign = if seconds < 0 { '-' } else { '+' };
        return Ok((
            format!("{sign}{hours:02}:{minutes:02}"),
            TimeZone::fixed(offset),
        ));
    }
    let zone = TimeZone::get(name).map_err(|_| invalid())?;
    let name = zone.iana_name().unwrap_or(name).to_string();
    Ok((name, zone))
}

/// The date and time on the zone's clocks at `epoch_ms` (ECMA-402's time value).
fn wall_clock(epoch_ms: f64, zone: &TimeZone) -> Result<DateTime<Gregorian>, Error> {
    // TimeClip: a time value is at most 100 million days from the epoch, in whole milliseconds.
    if !epoch_ms.is_finite() || epoch_ms.abs() > 8.64e15 {
        return Err(Error("Invalid time value".into()));
    }
    let instant = Timestamp::from_millisecond(epoch_ms.trunc() as i64)
        .map_err(|_| unsupported("A date outside the years -9999 to 9999"))?;
    let local = zone.to_datetime(instant);
    let invalid = |_| Error("Invalid time value".into());
    // Within jiff's range, each field fits ICU4X's.
    let date = Date::try_new_gregorian(local.year().into(), local.month() as u8, local.day() as u8)
        .map_err(invalid)?;
    let time = Time::try_new(
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
        local.subsec_nanosecond() as u32,
    )
    .map_err(invalid)?;
    Ok(DateTime { date, time })
}

/// What a CLDR pattern shows, as resolvedOptions names it.
#[derive(Default)]
struct Fields {
    weekday: Option<&'static str>,
    era: Option<&'static str>,
    year: Option<&'static str>,
    month: Option<&'static str>,
    day: Option<&'static str>,
    hour: Option<&'static str>,
    minute: Option<&'static str>,
    second: Option<&'static str>,
    fractional: Option<usize>,
    hour_cycle: Option<&'static str>,
}

/// The fields of a CLDR pattern: runs of one letter outside quotes (UTS 35's date field symbols).
fn fields(pattern: &str) -> Fields {
    let mut fields = Fields::default();
    let mut symbols: Vec<(char, usize)> = Vec::new();
    let mut quoted = false;
    for c in pattern.chars() {
        match c {
            '\'' => quoted = !quoted,
            c if !quoted && c.is_ascii_alphabetic() => match symbols.last_mut() {
                Some((last, count)) if *last == c => *count += 1,
                _ => symbols.push((c, 1)),
            },
            // Anything else separates runs.
            _ => symbols.push((' ', 0)),
        }
    }
    let numeric = |count| if count == 2 { "2-digit" } else { "numeric" };
    let text = |count| match count {
        4 => "long",
        5 => "narrow",
        _ => "short",
    };
    for (symbol, count) in symbols {
        match symbol {
            'G' => fields.era = Some(text(count)),
            'y' | 'u' | 'Y' => fields.year = Some(numeric(count)),
            'M' | 'L' => {
                fields.month = Some(match count {
                    1 | 2 => numeric(count),
                    _ => text(count),
                })
            }
            'd' => fields.day = Some(numeric(count)),
            'E' | 'c' | 'e' => fields.weekday = Some(text(count)),
            'h' | 'H' | 'K' | 'k' => {
                fields.hour = Some(numeric(count));
                fields.hour_cycle = Some(match symbol {
                    'h' => "h12",
                    'H' => "h23",
                    'K' => "h11",
                    _ => "h24",
                });
            }
            'm' => fields.minute = Some(numeric(count)),
            's' => fields.second = Some(numeric(count)),
            'S' => fields.fractional = Some(count),
            _ => {}
        }
    }
    fields
}
