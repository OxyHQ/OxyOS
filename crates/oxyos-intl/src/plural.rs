//! `Intl.PluralRules`: a number's plural category (`one`, `few`, `other`...) in a locale, cardinal
//! or ordinal, through ICU4X's CLDR plural rules.
//!
//! The category is of the number as `Intl.NumberFormat` would write it with the same digit
//! options (ECMA-402's ResolvePlural): with `minimumFractionDigits: 1`, English's `1` is `1.0`,
//! which is `other`. The digit options are ECMA-402's SetNumberFormatDigitOptions with
//! PluralRules' defaults (at most 3 fraction digits), rounding half away from zero. A number that
//! is not finite is `other`.
//!
//! Refused: `roundingIncrement`, `roundingMode`, `roundingPriority` and `trailingZeroDisplay`
//! (ECMA-402's newer digit options), which OxyOS does not take yet.

use super::{choice, number, unsupported, Error};
use crate::number::{round, rounding, Digits, Rounding};
use fixed_decimal::{Decimal, FloatPrecision};
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralOperands, PluralRules};

const SERVICE: &str = "PluralRules";

/// The plural category of `value` in `locale` (one `oxyos_intl_resolve` gave) with `options` (as
/// the application gave them): `zero`, `one`, `two`, `few`, `many` or `other`.
pub fn select(locale: &str, options: &[(&str, &str)], value: f64) -> Result<&'static str, Error> {
    let plural = Plural::new(locale, options)?;
    if !value.is_finite() {
        return Ok("other");
    }
    let mut decimal = Decimal::try_from_f64(value.abs(), FloatPrecision::RoundTrip)
        .map_err(|_| Error(format!("{value} has no plural category")))?;
    round(&mut decimal, plural.digits, "halfExpand", "auto");
    Ok(name(
        plural.rules.category_for(PluralOperands::from(&decimal)),
    ))
}

/// `resolvedOptions()`, in ECMA-402's order; `pluralCategories` the locale's, in CLDR's order
/// (`zero`, `one`, `two`, `few`, `many`, `other`), joined by `,`.
pub fn resolve(
    locale: &str,
    options: &[(&str, &str)],
) -> Result<Vec<(&'static str, String)>, Error> {
    let plural = Plural::new(locale, options)?;
    let mut resolved = vec![
        ("locale", plural.locale.to_string()),
        ("type", plural.kind.into()),
        ("minimumIntegerDigits", plural.digits.integer.to_string()),
    ];
    match plural.digits.rounding {
        Rounding::Fraction(minimum, maximum) => {
            resolved.push(("minimumFractionDigits", minimum.to_string()));
            resolved.push(("maximumFractionDigits", maximum.to_string()));
        }
        Rounding::Significant(minimum, maximum) => {
            resolved.push(("minimumSignificantDigits", minimum.to_string()));
            resolved.push(("maximumSignificantDigits", maximum.to_string()));
        }
    }
    let categories: Vec<&str> = plural.rules.categories().map(name).collect();
    resolved.extend([
        ("pluralCategories", categories.join(",")),
        ("roundingIncrement", "1".into()),
        ("roundingMode", "halfExpand".into()),
        ("roundingPriority", "auto".into()),
        ("trailingZeroDisplay", "auto".into()),
    ]);
    Ok(resolved)
}

/// A category's CLDR name.
fn name(category: PluralCategory) -> &'static str {
    match category {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

struct Plural {
    locale: Locale,
    /// `cardinal` or `ordinal`.
    kind: &'static str,
    digits: Digits,
    rules: PluralRules,
}

impl Plural {
    fn new(locale: &str, options: &[(&str, &str)]) -> Result<Self, Error> {
        let mut kind = "cardinal";
        let (mut integer, mut fraction, mut significant) = (None, (None, None), (None, None));
        for &(name, value) in options {
            match name {
                "type" => kind = choice(SERVICE, name, value, &["cardinal", "ordinal"])?,
                "minimumIntegerDigits" => integer = Some(number(SERVICE, name, value, 1, 21)?),
                "minimumFractionDigits" => fraction.0 = Some(value),
                "maximumFractionDigits" => fraction.1 = Some(value),
                "minimumSignificantDigits" => significant.0 = Some(value),
                "maximumSignificantDigits" => significant.1 = Some(value),
                "localeMatcher" => _ = choice(SERVICE, name, value, &["lookup", "best fit"])?,
                "roundingIncrement"
                | "roundingMode"
                | "roundingPriority"
                | "trailingZeroDisplay" => {
                    return Err(unsupported(format!("Intl.PluralRules' {name} option")))
                }
                _ => return Err(Error(format!("Intl.PluralRules has no option {name}"))),
            }
        }
        let digits = Digits {
            integer: integer.unwrap_or(1),
            rounding: rounding(SERVICE, fraction, significant, (0, 3))?,
        };
        // PluralRules has no Unicode extension keywords of its own.
        let locale = super::formatting_locale(locale, &[])?;
        let preferences = (&locale).into();
        let rules = match kind {
            "ordinal" => PluralRules::try_new_ordinal(preferences),
            _ => PluralRules::try_new_cardinal(preferences),
        }
        .map_err(|error| Error(format!("No plural rules for {locale}: {error}")))?;
        Ok(Self {
            locale,
            kind,
            digits,
            rules,
        })
    }
}
