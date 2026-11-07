//! The query language (docs/adr/0051-search.mdx, decision 5): what a person types into Spotlight
//! or a search field, and the tokens a field holds, as one question.
//!
//! - **Words** match as prefixes of words in an item's name and keywords (`rep` finds `Report`),
//!   and as words in its contents, the last one as a prefix too (what is being typed).
//! - **"Quoted phrases"** match those words in that order, in the name or the contents.
//! - **Typed filters**, in English or Spanish, as a Mac's Spotlight takes them: `kind:pdf`
//!   (`tipo:`), `tag:Work` or `tag:"Two words"` (`etiqueta:`), `date:today` (`fecha:hoy`:
//!   `today`, `yesterday`, `week`, `month`, `year`, or a day `2026-11-07`, a month `2026-11`, a
//!   year `2026`), `name:report` (`nombre:`), which matches the name alone.
//! - **Tokens** a field holds (Finder's "Kind: PDF") are the same filters, chosen rather than
//!   typed.
//!
//! An unknown `word:value` is two words. Several kinds are any of them; several tags, every one.

use crate::category::Category;
use serde::Deserialize;

/// A filter a search field holds as a token.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Token {
    Category(Category),
    Tag(String),
    Date(String),
    Name(String),
}

/// A span of time, in milliseconds since the epoch: `from` included, `to` excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: i64,
    pub to: i64,
}

/// A question, parsed: its words as typed (the index folds them as it folds what it holds).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub words: Vec<String>,
    pub phrases: Vec<String>,
    /// Prefixes the name alone must have (`name:`).
    pub names: Vec<String>,
    /// Any of these.
    pub categories: Vec<Category>,
    /// Every one of these (case aside).
    pub tags: Vec<String>,
    pub dates: Option<Span>,
    /// Whether a filter could not be understood (`date:someday`): nothing matches it.
    pub impossible: bool,
}

impl Parsed {
    /// Whether it asks for anything at all.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
            && self.phrases.is_empty()
            && self.names.is_empty()
            && self.categories.is_empty()
            && self.tags.is_empty()
            && self.dates.is_none()
            && !self.impossible
    }

    /// Whether it asks for text (words, phrases or names), not filters alone.
    pub fn has_text(&self) -> bool {
        !self.words.is_empty() || !self.phrases.is_empty() || !self.names.is_empty()
    }
}

/// Parses `text` with `tokens`, dates as of `now` (milliseconds) in the system's time zone.
pub fn parse(text: &str, tokens: &[Token], now: i64) -> Parsed {
    let mut parsed = Parsed::default();
    for piece in pieces(text) {
        match piece {
            Piece::Phrase(phrase) => {
                if !phrase.trim().is_empty() {
                    parsed.phrases.push(phrase);
                }
            }
            Piece::Word(word) => match word.split_once(':') {
                Some((key, value)) if !value.is_empty() => {
                    if !filter(&mut parsed, key, value, now) {
                        parsed.words.extend(
                            [key, value]
                                .into_iter()
                                .filter(|part| !part.is_empty())
                                .map(String::from),
                        );
                    }
                }
                // `kind:` alone, still being typed: nothing yet.
                Some((key, _)) if is_key(key) => {}
                _ => parsed.words.push(word),
            },
            Piece::Filter(key, value) => {
                if !filter(&mut parsed, &key, &value, now) {
                    parsed.words.push(key);
                    parsed.phrases.push(value);
                }
            }
        }
    }
    for token in tokens {
        match token {
            Token::Category(category) => add_category(&mut parsed, *category),
            Token::Tag(tag) => parsed.tags.push(tag.clone()),
            Token::Date(date) => date_filter(&mut parsed, date, now),
            Token::Name(name) => parsed.names.push(name.clone()),
        }
    }
    parsed
}

fn add_category(parsed: &mut Parsed, category: Category) {
    if !parsed.categories.contains(&category) {
        parsed.categories.push(category);
    }
}

fn is_key(key: &str) -> bool {
    matches!(
        key.to_lowercase().as_str(),
        "kind" | "tipo" | "tag" | "etiqueta" | "date" | "fecha" | "name" | "nombre"
    )
}

/// Applies the typed filter `key:value`; false if `key` names none.
fn filter(parsed: &mut Parsed, key: &str, value: &str, now: i64) -> bool {
    match key.to_lowercase().as_str() {
        "kind" | "tipo" => {
            match Category::from_word(&value.to_lowercase()) {
                Some(category) => add_category(parsed, category),
                None => parsed.impossible = true,
            }
            true
        }
        "tag" | "etiqueta" => {
            parsed.tags.push(value.to_string());
            true
        }
        "date" | "fecha" => {
            date_filter(parsed, value, now);
            true
        }
        "name" | "nombre" => {
            parsed.names.push(value.to_string());
            true
        }
        _ => false,
    }
}

fn date_filter(parsed: &mut Parsed, value: &str, now: i64) {
    match span(value, now) {
        Some(span) => {
            // Two dates: the time both say.
            parsed.dates = Some(match parsed.dates {
                Some(old) => Span {
                    from: old.from.max(span.from),
                    to: old.to.min(span.to),
                },
                None => span,
            });
        }
        None => parsed.impossible = true,
    }
}

/// The span a date filter names, as of `now`, in the system's time zone: a named span (today,
/// yesterday, the last 7, 30 or 365 days), a day, a month or a year.
pub fn span(value: &str, now: i64) -> Option<Span> {
    use jiff::civil::Date;
    use jiff::{tz::TimeZone, Timestamp, ToSpan};
    let zone = TimeZone::system();
    let now_zoned = Timestamp::from_millisecond(now)
        .ok()?
        .to_zoned(zone.clone());
    let today = now_zoned.date();
    let millis = |date: Date| -> Option<i64> {
        Some(
            date.to_zoned(zone.clone())
                .ok()?
                .timestamp()
                .as_millisecond(),
        )
    };
    let days_back = |days: i64| -> Option<Span> {
        Some(Span {
            from: millis(today.checked_sub(days.days()).ok()?)?,
            to: millis(today.tomorrow().ok()?)?,
        })
    };
    let lower = value.to_lowercase();
    match lower.as_str() {
        "today" | "hoy" => days_back(0),
        "yesterday" | "ayer" => Some(Span {
            from: millis(today.yesterday().ok()?)?,
            to: millis(today)?,
        }),
        "week" | "semana" => days_back(6),
        "month" | "mes" => days_back(29),
        "year" | "año" | "ano" => days_back(364),
        _ => {
            let parts: Vec<&str> = lower.split('-').collect();
            let number = |text: &str| text.parse::<i16>().ok();
            let (from, to) = match parts.as_slice() {
                [year] if year.len() == 4 => {
                    let start = Date::new(number(year)?, 1, 1).ok()?;
                    (start, start.checked_add(1.year()).ok()?)
                }
                [year, month] if year.len() == 4 => {
                    let start = Date::new(number(year)?, number(month)? as i8, 1).ok()?;
                    (start, start.checked_add(1.month()).ok()?)
                }
                [year, month, day] if year.len() == 4 => {
                    let start =
                        Date::new(number(year)?, number(month)? as i8, number(day)? as i8).ok()?;
                    (start, start.tomorrow().ok()?)
                }
                _ => return None,
            };
            Some(Span {
                from: millis(from)?,
                to: millis(to)?,
            })
        }
    }
}

enum Piece {
    Word(String),
    Phrase(String),
    /// `key:"quoted value"`.
    Filter(String, String),
}

/// Words and quoted phrases; `key:"a value"` keeps its quotes' words together. An unclosed quote
/// runs to the end (still being typed).
fn pieces(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' {
            chars.next();
            let phrase: String = chars.by_ref().take_while(|&c| c != '"').collect();
            pieces.push(Piece::Phrase(phrase));
            continue;
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            chars.next();
            if c == '"' && word.ends_with(':') {
                let value: String = chars.by_ref().take_while(|&c| c != '"').collect();
                word.pop();
                pieces.push(Piece::Filter(word.clone(), value));
                word.clear();
                break;
            }
            word.push(c);
        }
        if !word.is_empty() {
            pieces.push(Piece::Word(word));
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 7 November 2026, noon, in UTC (the tests' time zone is the system's: spans are compared by
    /// length and order, not by value).
    const NOW: i64 = 1_794_052_800_000;

    #[test]
    fn words_phrases_and_filters() {
        let parsed = parse(
            r#"budget "quarterly report" kind:pdf tag:"Two words" tipo:imagen name:inv"#,
            &[],
            NOW,
        );
        assert_eq!(parsed.words, ["budget"]);
        assert_eq!(parsed.phrases, ["quarterly report"]);
        assert_eq!(parsed.categories, [Category::Pdf, Category::Image]);
        assert_eq!(parsed.tags, ["Two words"]);
        assert_eq!(parsed.names, ["inv"]);
        assert!(!parsed.impossible);
    }

    #[test]
    fn an_unknown_key_is_words_and_a_key_being_typed_is_nothing() {
        let parsed = parse("http://example kind:", &[], NOW);
        assert_eq!(parsed.words, ["http", "//example"]);
        assert!(parsed.categories.is_empty());
        assert!(!parsed.impossible);
    }

    #[test]
    fn an_unknown_kind_or_date_matches_nothing() {
        assert!(parse("kind:spaceship", &[], NOW).impossible);
        assert!(parse("date:someday", &[], NOW).impossible);
    }

    #[test]
    fn tokens_are_the_same_filters() {
        let parsed = parse(
            "re",
            &[
                Token::Category(Category::Pdf),
                Token::Tag("Work".into()),
                Token::Date("week".into()),
            ],
            NOW,
        );
        assert_eq!(parsed.words, ["re"]);
        assert_eq!(parsed.categories, [Category::Pdf]);
        assert_eq!(parsed.tags, ["Work"]);
        assert!(parsed.dates.is_some());
    }

    #[test]
    fn spans_hold_now_and_nest() {
        let today = span("today", NOW).unwrap();
        let week = span("week", NOW).unwrap();
        let month = span("mes", NOW).unwrap();
        let yesterday = span("ayer", NOW).unwrap();
        assert!(today.from <= NOW && NOW < today.to);
        assert!(week.from < today.from && week.to == today.to);
        assert!(month.from < week.from);
        assert_eq!(yesterday.to, today.from);
        let day = span("2026-11-07", NOW).unwrap();
        assert!((day.to - day.from) >= 23 * 3_600_000 && (day.to - day.from) <= 25 * 3_600_000);
        let year = span("2026", NOW).unwrap();
        assert!(year.from < day.from && day.to < year.to);
        assert!(span("2026-13", NOW).is_none());
    }

    #[test]
    fn two_dates_are_the_time_both_say() {
        let parsed = parse("date:2026 date:2026-11", &[], NOW);
        assert_eq!(parsed.dates, span("2026-11", NOW));
    }
}
