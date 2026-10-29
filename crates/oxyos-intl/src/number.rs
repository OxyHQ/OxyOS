//! `Intl.NumberFormat`: a number as a decimal, a percentage or an amount of a currency, through
//! ICU4X's decimal formatter and its (experimental) currency and percent data.
//!
//! The digit options are ECMA-402's SetNumberFormatDigitOptions, with its defaults (a decimal at
//! most 3 fraction digits, a percentage none, a currency its ISO 4217 minor unit); rounding is to
//! the shortest decimal that reads back as the same double, as browsers round, not its exact
//! binary value. A negative amount takes its sign before the whole currency pattern, CLDR's
//! implicit negative pattern; the few locales whose pattern has an explicit one (`nl`: `€ -12,34`)
//! print `-€ 12,34`. `NaN` and `∞` are CLDR's root symbols in every locale. `useGrouping: "always"`
//! (and `true`) groups as `auto` does, ICU4X having no stronger strategy, so `es` still writes
//! `1234`. Digits are Latin in every locale (a browser writes `ar-EG`'s in Arabic-Indic digits).
//!
//! Refused: `notation` other than `standard`, `style: unit` (and `unit`, `unitDisplay`),
//! `compactDisplay`, `currencySign: accounting`, `roundingIncrement` other than 1,
//! `roundingPriority` other than `auto`, and numbering systems but `latn`.

use super::{choice, latin_digits, number, push, unsupported, Error, Part, Runs};
use fixed_decimal::{Decimal, FloatPrecision, Sign, SignedRoundingMode, UnsignedRoundingMode};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_experimental::dimension::provider::currency::essentials::{
    CurrencyEssentialsV1, PatternSelection, PlaceholderValue,
};
use icu_experimental::dimension::provider::currency::extended::CurrencyExtendedDataV1;
use icu_experimental::dimension::provider::currency::patterns::CurrencyPatternsDataV1;
use icu_experimental::dimension::provider::percent::PercentEssentialsV1;
use icu_locale_core::extensions::unicode::key;
use icu_locale_core::Locale;
use icu_pattern::{DoublePlaceholderKey, PatternItem};
use icu_plurals::{PluralOperands, PluralRules};
use icu_provider::prelude::*;

const SERVICE: &str = "NumberFormat";

/// The parts of `value` formatted with `options` (as the application gave them) in `locale`, one
/// `oxyos_intl_resolve` gave.
pub fn format(locale: &str, options: &[(&str, &str)], value: f64) -> Result<Vec<Part>, Error> {
    NumberFormat::new(locale, options)?.format(value)
}

/// `resolvedOptions()`, in ECMA-402's order.
pub fn resolve(
    locale: &str,
    options: &[(&str, &str)],
) -> Result<Vec<(&'static str, String)>, Error> {
    let format = NumberFormat::new(locale, options)?;
    let options = &format.options;
    let mut resolved = vec![
        ("locale", format.locale.to_string()),
        ("numberingSystem", "latn".into()),
        ("style", options.style.into()),
    ];
    if let Some(currency) = &options.currency {
        resolved.push(("currency", currency.clone()));
        resolved.push(("currencyDisplay", options.currency_display.into()));
        resolved.push(("currencySign", "standard".into()));
    }
    resolved.push(("minimumIntegerDigits", options.digits.integer.to_string()));
    match options.digits.rounding {
        Rounding::Fraction(minimum, maximum) => {
            resolved.push(("minimumFractionDigits", minimum.to_string()));
            resolved.push(("maximumFractionDigits", maximum.to_string()));
        }
        Rounding::Significant(minimum, maximum) => {
            resolved.push(("minimumSignificantDigits", minimum.to_string()));
            resolved.push(("maximumSignificantDigits", maximum.to_string()));
        }
    }
    resolved.extend([
        ("useGrouping", options.grouping.into()),
        ("notation", "standard".into()),
        ("signDisplay", options.sign_display.into()),
        ("roundingIncrement", "1".into()),
        ("roundingMode", options.rounding_mode.into()),
        ("roundingPriority", "auto".into()),
        ("trailingZeroDisplay", options.trailing_zeros.into()),
    ]);
    Ok(resolved)
}

/// How a number is rounded: to fraction digits (at least, at most) or significant digits.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rounding {
    Fraction(u8, u8),
    Significant(u8, u8),
}

#[derive(Clone, Copy, Debug)]
struct Digits {
    integer: u8,
    rounding: Rounding,
}

/// The options, checked, with ECMA-402's defaults.
struct Options {
    style: &'static str,
    currency: Option<String>,
    currency_display: &'static str,
    digits: Digits,
    /// `auto`, `always`, `min2` or `false`.
    grouping: &'static str,
    sign_display: &'static str,
    rounding_mode: &'static str,
    trailing_zeros: &'static str,
}

impl Options {
    fn parse(options: &[(&str, &str)]) -> Result<Self, Error> {
        let mut style = "decimal";
        let mut currency = None;
        let mut currency_display = "symbol";
        let (mut integer, mut fraction, mut significant) = (None, (None, None), (None, None));
        let mut grouping = "auto";
        let mut sign_display = "auto";
        let mut rounding_mode = "halfExpand";
        let mut trailing_zeros = "auto";
        for &(name, value) in options {
            let pick = |allowed| choice(SERVICE, name, value, allowed);
            match name {
                "style" => match pick(&["decimal", "percent", "currency", "unit"])? {
                    "unit" => return Err(unsupported("Intl.NumberFormat's style unit")),
                    chosen => style = chosen,
                },
                "currency" => {
                    if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                        return Err(Error(format!("Invalid currency code : {value}")));
                    }
                    currency = Some(value.to_ascii_uppercase());
                }
                "currencyDisplay" => {
                    currency_display = pick(&["symbol", "narrowSymbol", "code", "name"])?
                }
                "currencySign" => {
                    if pick(&["standard", "accounting"])? == "accounting" {
                        return Err(unsupported("Intl.NumberFormat's currencySign accounting"));
                    }
                }
                "notation" => {
                    let notation = pick(&["standard", "scientific", "engineering", "compact"])?;
                    if notation != "standard" {
                        return Err(unsupported(format!(
                            "Intl.NumberFormat's notation {notation}"
                        )));
                    }
                }
                "unit" | "unitDisplay" | "compactDisplay" => {
                    return Err(unsupported(format!("Intl.NumberFormat's {name} option")))
                }
                "minimumIntegerDigits" => integer = Some(number(SERVICE, name, value, 1, 21)?),
                "minimumFractionDigits" => fraction.0 = Some(value),
                "maximumFractionDigits" => fraction.1 = Some(value),
                "minimumSignificantDigits" => significant.0 = Some(value),
                "maximumSignificantDigits" => significant.1 = Some(value),
                "useGrouping" => {
                    grouping = match pick(&["always", "auto", "min2", "true", "false"])? {
                        "true" => "always",
                        chosen => chosen,
                    }
                }
                "signDisplay" => {
                    sign_display = pick(&["auto", "never", "always", "exceptZero", "negative"])?
                }
                "roundingMode" => {
                    rounding_mode = pick(&[
                        "ceil",
                        "floor",
                        "expand",
                        "trunc",
                        "halfCeil",
                        "halfFloor",
                        "halfExpand",
                        "halfTrunc",
                        "halfEven",
                    ])?
                }
                "roundingIncrement" => {
                    let increments = [
                        1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000,
                    ];
                    let increment = value.parse::<u16>().ok().filter(|i| increments.contains(i));
                    match increment {
                        Some(1) => {}
                        Some(_) => {
                            return Err(unsupported(format!(
                                "Intl.NumberFormat's roundingIncrement {value}"
                            )))
                        }
                        None => {
                            return Err(Error(format!(
                                "roundingIncrement value {value} is out of range"
                            )))
                        }
                    }
                }
                "roundingPriority" => {
                    let priority = pick(&["auto", "morePrecision", "lessPrecision"])?;
                    if priority != "auto" {
                        return Err(unsupported(format!(
                            "Intl.NumberFormat's roundingPriority {priority}"
                        )));
                    }
                }
                "trailingZeroDisplay" => trailing_zeros = pick(&["auto", "stripIfInteger"])?,
                "numberingSystem" if value.eq_ignore_ascii_case("latn") => {}
                "numberingSystem" => {
                    return Err(unsupported(format!("The numbering system {value}")))
                }
                "localeMatcher" => _ = pick(&["lookup", "best fit"])?,
                _ => return Err(Error(format!("Intl.NumberFormat has no option {name}"))),
            }
        }
        if style == "currency" && currency.is_none() {
            return Err(Error(
                "Currency code is required with currency style.".into(),
            ));
        }
        // SetNumberFormatDigitOptions, with roundingPriority auto and standard notation.
        let (fraction_default, fraction_maximum_default) = match (style, &currency) {
            ("currency", Some(code)) => (minor_unit(code), minor_unit(code)),
            ("percent", _) => (0, 0),
            _ => (0, 3),
        };
        let digits = |name, value: Option<&str>, minimum, maximum| {
            value
                .map(|value| number(SERVICE, name, value, minimum, maximum))
                .transpose()
        };
        let rounding = if significant.0.or(significant.1).is_some() {
            let minimum = digits("minimumSignificantDigits", significant.0, 1, 21)?.unwrap_or(1);
            let maximum = digits("maximumSignificantDigits", significant.1, 1, 21)?.unwrap_or(21);
            if minimum > maximum {
                return Err(Error(format!(
                    "maximumSignificantDigits value {maximum} is out of range"
                )));
            }
            Rounding::Significant(minimum, maximum)
        } else {
            let minimum = digits("minimumFractionDigits", fraction.0, 0, 100)?;
            let maximum = digits("maximumFractionDigits", fraction.1, 0, 100)?;
            match (minimum, maximum) {
                (None, None) => Rounding::Fraction(fraction_default, fraction_maximum_default),
                (None, Some(maximum)) => Rounding::Fraction(fraction_default.min(maximum), maximum),
                (Some(minimum), None) => {
                    Rounding::Fraction(minimum, fraction_maximum_default.max(minimum))
                }
                (Some(minimum), Some(maximum)) if minimum > maximum => {
                    return Err(Error(format!(
                        "maximumFractionDigits value {maximum} is out of range"
                    )))
                }
                (Some(minimum), Some(maximum)) => Rounding::Fraction(minimum, maximum),
            }
        };
        Ok(Self {
            style,
            currency_display: if currency.is_some() {
                currency_display
            } else {
                "symbol"
            },
            currency: currency.filter(|_| style == "currency"),
            digits: Digits {
                integer: integer.unwrap_or(1),
                rounding,
            },
            grouping,
            sign_display,
            rounding_mode,
            trailing_zeros,
        })
    }
}

/// ECMA-402's CurrencyDigits: the currency's minor unit in ISO 4217, else 2.
fn minor_unit(code: &str) -> u8 {
    match code {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        "CLF" | "UYW" => 4,
        _ => 2,
    }
}

/// How a style wraps the number.
enum Wrapping {
    None,
    Percent(DataPayload<PercentEssentialsV1>),
    /// The currency's symbol (or code) and the pattern it goes into.
    Symbol(DataPayload<CurrencyEssentialsV1>, String, PatternSelection),
    /// The currency's names and patterns by plural form.
    Name(
        DataPayload<CurrencyExtendedDataV1>,
        DataPayload<CurrencyPatternsDataV1>,
        PluralRules,
    ),
}

struct NumberFormat {
    locale: Locale,
    options: Options,
    formatter: DecimalFormatter,
    wrapping: Wrapping,
}

impl NumberFormat {
    fn new(locale: &str, options: &[(&str, &str)]) -> Result<Self, Error> {
        let options = Options::parse(options)?;
        let locale = super::formatting_locale(locale, &[key!("nu")])?;
        let mut preferences = DecimalFormatterPreferences::from(&locale);
        preferences.numbering_system = latin_digits(&locale);
        let mut decimal_options = DecimalFormatterOptions::default();
        decimal_options.grouping_strategy = Some(match options.grouping {
            "always" => GroupingStrategy::Always,
            "min2" => GroupingStrategy::Min2,
            "false" => GroupingStrategy::Never,
            _ => GroupingStrategy::Auto,
        });
        let missing = |error: DataError| Error(format!("No number formats for {locale}: {error}"));
        let formatter = DecimalFormatter::try_new(preferences, decimal_options).map_err(missing)?;
        let data_locale = DataLocale::from(&locale);
        let wrapping = match (options.style, &options.currency) {
            ("percent", _) => Wrapping::Percent(load(&data_locale, "").map_err(missing)?),
            ("currency", Some(code)) if options.currency_display == "name" => {
                let rules = PluralRules::try_new_cardinal((&locale).into()).map_err(missing)?;
                Wrapping::Name(
                    load(&data_locale, code).map_err(missing)?,
                    load(&data_locale, "").map_err(missing)?,
                    rules,
                )
            }
            ("currency", Some(code)) => {
                let essentials: DataPayload<CurrencyEssentialsV1> =
                    load(&data_locale, "").map_err(missing)?;
                let (symbol, selection) =
                    currency_symbol(essentials.get(), code, options.currency_display);
                Wrapping::Symbol(essentials, symbol, selection)
            }
            _ => Wrapping::None,
        };
        Ok(Self {
            locale,
            options,
            formatter,
            wrapping,
        })
    }

    fn format(&self, value: f64) -> Result<Vec<Part>, Error> {
        // NaN is neither negative nor, for exceptZero, signed (ECMA-402's GetNumberFormatPattern).
        let negative = value.is_sign_negative() && !value.is_nan();
        let mut decimal = if value.is_finite() {
            let mut decimal = Decimal::try_from_f64(value.abs(), FloatPrecision::RoundTrip)
                .map_err(|_| Error(format!("{value} cannot be formatted")))?;
            if self.options.style == "percent" {
                decimal.multiply_pow10(2);
            }
            decimal.sign = if negative { Sign::Negative } else { Sign::None };
            self.round(&mut decimal);
            decimal
        } else {
            Decimal::from(0)
        };
        let zero = decimal.absolute.is_zero() && !value.is_infinite();
        let signed = if negative {
            Sign::Negative
        } else {
            Sign::Positive
        };
        let sign = match self.options.sign_display {
            "auto" if negative => Sign::Negative,
            "always" => signed,
            "exceptZero" if !zero => signed,
            "negative" if negative && !zero => Sign::Negative,
            _ => Sign::None,
        };
        // A percentage's sign is its pattern's; the others are the decimal's.
        decimal.sign = if self.options.style == "percent" {
            Sign::None
        } else {
            sign
        };
        let mut number = Vec::new();
        for (inside, text) in Runs::of(&self.formatter.format(&decimal)) {
            let kind = match inside.last().map(|part| part.value) {
                Some("integer") if value.is_nan() => "nan",
                Some("integer") if value.is_infinite() => "infinity",
                Some("integer") => "integer",
                Some("group") => "group",
                Some("decimal") => "decimal",
                Some("fraction") => "fraction",
                Some("minusSign") => "minusSign",
                Some("plusSign") => "plusSign",
                _ => "literal",
            };
            let text = match kind {
                "nan" => "NaN",
                "infinity" => "∞",
                _ => &text,
            };
            push(&mut number, kind, text);
        }
        let mut parts = Vec::new();
        match &self.wrapping {
            Wrapping::None => parts = number,
            Wrapping::Percent(essentials) => {
                let essentials = essentials.get();
                let (sign, text) = match sign {
                    Sign::Negative => ("minusSign", &*essentials.minus_sign),
                    Sign::Positive => ("plusSign", &*essentials.plus_sign),
                    Sign::None => ("", ""),
                };
                if sign.is_empty() {
                    for item in essentials.unsigned_pattern.iter() {
                        match item {
                            PatternItem::Literal(literal) => percent_literal(&mut parts, literal),
                            PatternItem::Placeholder(_) => parts.extend(number.iter().cloned()),
                        }
                    }
                } else {
                    for item in essentials.signed_pattern.iter() {
                        match item {
                            PatternItem::Literal(literal) => percent_literal(&mut parts, literal),
                            PatternItem::Placeholder(DoublePlaceholderKey::Place0) => {
                                parts.extend(number.iter().cloned())
                            }
                            PatternItem::Placeholder(DoublePlaceholderKey::Place1) => {
                                push(&mut parts, sign, text)
                            }
                        }
                    }
                }
            }
            Wrapping::Symbol(essentials, symbol, selection) => {
                let essentials = essentials.get();
                let pattern = match selection {
                    PatternSelection::StandardAlphaNextToNumber => essentials
                        .standard_alpha_next_to_number_pattern
                        .as_ref()
                        .or(essentials.standard_pattern.as_ref()),
                    PatternSelection::Standard => essentials.standard_pattern.as_ref(),
                }
                .ok_or_else(|| Error(format!("No currency formats for {}", self.locale)))?;
                // CLDR's implicit negative pattern: the sign before the whole pattern.
                let signed = number
                    .iter()
                    .take_while(|part| matches!(part.kind, "minusSign" | "plusSign"))
                    .count();
                parts.extend(number.drain(..signed));
                wrap(&mut parts, pattern, &number, "currency", symbol);
            }
            Wrapping::Name(names, patterns, rules) => {
                let operands = PluralOperands::from(&decimal);
                let name = names.get().display_names.get(operands, rules);
                let pattern = patterns.get().patterns.get(operands, rules);
                wrap(&mut parts, pattern, &number, "currency", name);
            }
        }
        Ok(parts)
    }

    /// Rounds and pads `decimal` as the digit options say.
    fn round(&self, decimal: &mut Decimal) {
        let digits = self.options.digits;
        let mode = match self.options.rounding_mode {
            "ceil" => SignedRoundingMode::Ceil,
            "floor" => SignedRoundingMode::Floor,
            "expand" => SignedRoundingMode::Unsigned(UnsignedRoundingMode::Expand),
            "trunc" => SignedRoundingMode::Unsigned(UnsignedRoundingMode::Trunc),
            "halfCeil" => SignedRoundingMode::HalfCeil,
            "halfFloor" => SignedRoundingMode::HalfFloor,
            "halfTrunc" => SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfTrunc),
            "halfEven" => SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfEven),
            _ => SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
        };
        let shown = match digits.rounding {
            Rounding::Fraction(minimum, maximum) => {
                decimal.round_with_mode(-i16::from(maximum), mode);
                -i16::from(minimum)
            }
            Rounding::Significant(minimum, maximum) => {
                if !decimal.absolute.is_zero() {
                    let start = decimal.absolute.nonzero_magnitude_start();
                    decimal.round_with_mode(start - i16::from(maximum) + 1, mode);
                }
                // Rounding may have carried into a new digit (9.99 to 10.0).
                decimal.absolute.nonzero_magnitude_start() - i16::from(minimum) + 1
            }
        };
        decimal.absolute.trim_end();
        let integer = decimal.absolute.nonzero_magnitude_end() >= 0;
        if !(self.options.trailing_zeros == "stripIfInteger" && integer) {
            decimal.absolute.pad_end(shown.min(0));
        }
        decimal.absolute.pad_start(i16::from(digits.integer));
    }
}

/// Data from ICU4X's experimental components, for `locale` and the attributes (a currency code)
/// or none.
fn load<M>(locale: &DataLocale, attributes: &str) -> Result<DataPayload<M>, DataError>
where
    M: DataMarker,
    super::data::Baked: DataProvider<M>,
{
    let attributes = DataMarkerAttributes::try_from_str(attributes)
        .map_err(|_| DataErrorKind::IdentifierNotFound.into_error())?;
    let request = DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, locale),
        ..Default::default()
    };
    Ok(super::data::Baked.load(request)?.payload)
}

/// The currency's symbol (`symbol`, `narrowSymbol`) or code (`code`), and which of the locale's
/// patterns it goes into (CLDR's own for an alphabetic symbol next to the number: `USD 1.00`).
fn currency_symbol(
    essentials: &icu_experimental::dimension::provider::currency::essentials::CurrencyEssentials,
    code: &str,
    display: &str,
) -> (String, PatternSelection) {
    let config = tinystr::TinyAsciiStr::<3>::try_from_str(code)
        .ok()
        .and_then(|code| {
            essentials
                .pattern_config_map
                .get_copied(&code.to_unvalidated())
        })
        .unwrap_or(essentials.default_pattern_config);
    let (placeholder, selection) = match display {
        "narrowSymbol" => (
            config.narrow_placeholder_value,
            config.narrow_pattern_selection,
        ),
        "code" => (None, PatternSelection::StandardAlphaNextToNumber),
        _ => (
            config.short_placeholder_value,
            config.short_pattern_selection,
        ),
    };
    let symbol = match placeholder {
        Some(PlaceholderValue::Index(index)) => essentials.placeholders.get(index.into()),
        Some(PlaceholderValue::ISO) | None => None,
    };
    (symbol.unwrap_or(code).into(), selection)
}

/// `pattern` with the number's parts for `{0}` and `{1}` as one part of `kind`.
fn wrap(
    parts: &mut Vec<Part>,
    pattern: &icu_pattern::DoublePlaceholderPattern,
    number: &[Part],
    kind: &'static str,
    text: &str,
) {
    for item in pattern.iter() {
        match item {
            PatternItem::Literal(literal) => push(parts, "literal", literal),
            PatternItem::Placeholder(DoublePlaceholderKey::Place0) => {
                parts.extend(number.iter().cloned())
            }
            PatternItem::Placeholder(DoublePlaceholderKey::Place1) => push(parts, kind, text),
        }
    }
}

/// A percent pattern's literal text: its percent sign, and the rest (spaces, bidi marks).
fn percent_literal(parts: &mut Vec<Part>, literal: &str) {
    let mut rest = literal;
    while let Some(at) = rest.find(['%', '٪', '﹪', '％']) {
        push(parts, "literal", &rest[..at]);
        let sign = rest[at..].chars().next().map_or(0, char::len_utf8);
        push(parts, "percentSign", &rest[at..at + sign]);
        rest = &rest[at + sign..];
    }
    push(parts, "literal", rest);
}
