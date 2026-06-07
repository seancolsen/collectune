//! Per-column value formatters.
//!
//! A column's metadata may carry a `formatter` JSON blob selecting a typed transform
//! that reshapes each cell's display text. Cell values arrive as plain strings (Arrow's
//! `ArrayFormatter` output), so every formatter parses the string it is given and
//! returns `None` when it can't — the caller then falls back to the raw value.

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Unit, Zoned};
use serde::Deserialize;

/// A typed value formatter, parsed from a column's `formatter` metadata blob. The
/// `type` discriminant selects the variant; each variant carries its own config.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum Formatter {
    /// Formats a number to a min/max number of fraction digits (no thousands grouping).
    #[serde(rename_all = "camelCase")]
    Number {
        #[serde(default)]
        decimal_places: DecimalPlaces,
    },
    /// Formats a timestamp via a jiff `strftime` format string.
    Timestamp { format: String },
    /// Formats a number of seconds as `M:SS`.
    Duration {},
    /// Formats a timestamp as the time elapsed relative to now.
    #[serde(rename_all = "camelCase")]
    RelativeTime {
        #[serde(default)]
        units: Vec<RelativeUnit>,
    },
}

/// The user-facing `decimalPlaces` spec: a plain number, or a one/two-element list of
/// `[min]` / `[min, max]` fraction digits.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(crate) enum DecimalPlaces {
    Single(usize),
    List(Vec<usize>),
}

impl Default for DecimalPlaces {
    fn default() -> Self {
        DecimalPlaces::List(Vec::new())
    }
}

/// A unit available to the `relativeTime` formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RelativeUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

impl Formatter {
    /// Formats `value` for display, or returns `None` when it can't be parsed (the
    /// caller then renders the raw value unchanged).
    pub(crate) fn format(&self, value: &str) -> Option<String> {
        match self {
            Formatter::Number { decimal_places } => {
                let (min, max) = resolve_decimals(decimal_places);
                format_number(value, min, max)
            }
            Formatter::Timestamp { format } => format_timestamp(value, format),
            Formatter::Duration {} => format_duration(value),
            Formatter::RelativeTime { units } => {
                let then = parse_zoned(value)?;
                relative_between(&then, &Zoned::now(), units)
            }
        }
    }
}

/// Resolves a [`DecimalPlaces`] spec into a `(min, max)` pair, swapping if min > max.
fn resolve_decimals(spec: &DecimalPlaces) -> (usize, usize) {
    let (mut min, mut max) = match spec {
        DecimalPlaces::Single(n) => (*n, *n),
        DecimalPlaces::List(list) => match list.as_slice() {
            [] => (0, 0),
            [n] => (*n, *n),
            [min, max, ..] => (*min, *max),
        },
    };
    if min > max {
        std::mem::swap(&mut min, &mut max);
    }
    (min, max)
}

/// Formats a number to at most `max` and at least `min` fraction digits, trimming
/// trailing zeros in between. No thousands grouping.
fn format_number(value: &str, min: usize, max: usize) -> Option<String> {
    let n: f64 = value.trim().parse().ok()?;
    let mut s = format!("{n:.max$}");
    // `s` now has exactly `max` fraction digits. Trim trailing zeros down to `min`.
    if max > min
        && let Some(dot) = s.find('.')
    {
        // Shortest acceptable length: the integer part, plus a dot and `min` digits.
        let min_len = if min == 0 { dot } else { dot + 1 + min };
        while s.len() > min_len && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    Some(s)
}

/// Formats a number of seconds as `M:SS` (minutes unpadded, seconds zero-padded),
/// rounded to the nearest second.
fn format_duration(value: &str) -> Option<String> {
    let secs: f64 = value.trim().parse().ok()?;
    let total = secs.round() as i64;
    let sign = if total < 0 { "-" } else { "" };
    let total = total.abs();
    let minutes = total / 60;
    let seconds = total % 60;
    Some(format!("{sign}{minutes}:{seconds:02}"))
}

/// Formats a parsed timestamp via a jiff `strftime` format string. Tries a civil
/// datetime, then a date, then integer epoch seconds.
fn format_timestamp(value: &str, format: &str) -> Option<String> {
    let v = value.trim();
    // Arrow renders datetimes space-separated (`2026-06-07 14:30:45`); ISO uses `T`.
    if let Ok(dt) = v.replacen(' ', "T", 1).parse::<DateTime>() {
        return Some(dt.strftime(format).to_string());
    }
    if let Ok(date) = v.parse::<Date>() {
        return Some(date.strftime(format).to_string());
    }
    let secs: i64 = v.parse().ok()?;
    let zoned = Timestamp::from_second(secs)
        .ok()?
        .to_zoned(TimeZone::system());
    Some(zoned.strftime(format).to_string())
}

/// Parses a cell value into a zoned instant, for relative-time math. Tries a civil
/// datetime, then a date (start of day), then integer epoch seconds — all in the
/// system time zone.
fn parse_zoned(value: &str) -> Option<Zoned> {
    let v = value.trim();
    if let Ok(dt) = v.replacen(' ', "T", 1).parse::<DateTime>() {
        return dt.to_zoned(TimeZone::system()).ok();
    }
    if let Ok(date) = v.parse::<Date>() {
        return date.to_zoned(TimeZone::system()).ok();
    }
    let secs: i64 = v.parse().ok()?;
    Some(
        Timestamp::from_second(secs)
            .ok()?
            .to_zoned(TimeZone::system()),
    )
}

/// Renders the elapsed time between `then` and `now` using the largest of the requested
/// `units` that yields a non-zero integer; falls back to the smallest unit (a `0`).
fn relative_between(then: &Zoned, now: &Zoned, units: &[RelativeUnit]) -> Option<String> {
    let mut order: Vec<RelativeUnit> = if units.is_empty() {
        RelativeUnit::all()
    } else {
        units.to_vec()
    };
    // Largest unit first.
    order.sort_by_key(|u| std::cmp::Reverse(u.rank()));
    order.dedup();
    let smallest = *order.last()?;

    let mut chosen = None;
    for &unit in &order {
        let n = elapsed_in(then, now, unit.to_unit())?;
        if n.abs() >= 1 {
            chosen = Some((unit, n));
            break;
        }
    }
    let (unit, n) = match chosen {
        Some(c) => c,
        None => (smallest, elapsed_in(then, now, smallest.to_unit())?),
    };

    let count = n.abs();
    let noun = unit.noun(count);
    Some(if n < 0 {
        format!("in {count} {noun}")
    } else {
        format!("{count} {noun} ago")
    })
}

/// The signed whole number of `unit`s elapsed from `then` to `now` (calendar-aware,
/// truncated toward zero). Positive when `then` is in the past.
fn elapsed_in(then: &Zoned, now: &Zoned, unit: Unit) -> Option<i64> {
    let span = then.until(now).ok()?;
    let total = span.total((unit, then)).ok()?;
    Some(total.trunc() as i64)
}

impl RelativeUnit {
    fn all() -> Vec<Self> {
        use RelativeUnit::{Days, Hours, Minutes, Months, Weeks, Years};
        vec![Minutes, Hours, Days, Weeks, Months, Years]
    }

    /// Ordering rank, smallest unit (minutes) lowest.
    fn rank(self) -> u8 {
        match self {
            Self::Minutes => 0,
            Self::Hours => 1,
            Self::Days => 2,
            Self::Weeks => 3,
            Self::Months => 4,
            Self::Years => 5,
        }
    }

    fn to_unit(self) -> Unit {
        match self {
            Self::Minutes => Unit::Minute,
            Self::Hours => Unit::Hour,
            Self::Days => Unit::Day,
            Self::Weeks => Unit::Week,
            Self::Months => Unit::Month,
            Self::Years => Unit::Year,
        }
    }

    /// The unit's noun, pluralized for any `count` other than 1.
    fn noun(self, count: i64) -> String {
        let base = match self {
            Self::Minutes => "minute",
            Self::Hours => "hour",
            Self::Days => "day",
            Self::Weeks => "week",
            Self::Months => "month",
            Self::Years => "year",
        };
        if count == 1 {
            base.to_string()
        } else {
            format!("{base}s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoned(y: i16, m: i8, d: i8, h: i8, min: i8) -> Zoned {
        jiff::civil::date(y, m, d)
            .at(h, min, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
    }

    #[test]
    fn number_min_max_fraction_digits() {
        let f = Formatter::Number {
            decimal_places: DecimalPlaces::List(vec![0, 1]),
        };
        assert_eq!(f.format("3").as_deref(), Some("3"));
        assert_eq!(f.format("3.0").as_deref(), Some("3"));
        assert_eq!(f.format("3.14").as_deref(), Some("3.1"));
        assert_eq!(f.format("1234567.8").as_deref(), Some("1234567.8"));
        assert_eq!(f.format("-3.0").as_deref(), Some("-3"));
    }

    #[test]
    fn number_keeps_minimum_digits() {
        let f = Formatter::Number {
            decimal_places: DecimalPlaces::List(vec![2, 4]),
        };
        assert_eq!(f.format("3.5").as_deref(), Some("3.50"));
        assert_eq!(f.format("3").as_deref(), Some("3.00"));
    }

    #[test]
    fn number_bad_input_is_none() {
        let f = Formatter::Number {
            decimal_places: DecimalPlaces::default(),
        };
        assert_eq!(f.format(""), None);
        assert_eq!(f.format("N/A"), None);
    }

    #[test]
    fn duration_mm_ss() {
        let f = Formatter::Duration {};
        assert_eq!(f.format("245").as_deref(), Some("4:05"));
        assert_eq!(f.format("245.0").as_deref(), Some("4:05"));
        assert_eq!(f.format("0").as_deref(), Some("0:00"));
        assert_eq!(f.format("66.4").as_deref(), Some("1:06"));
        assert_eq!(f.format("3600").as_deref(), Some("60:00"));
        assert_eq!(f.format("nope"), None);
    }

    #[test]
    fn timestamp_strftime() {
        let f = Formatter::Timestamp {
            format: "%Y-%m-%d".to_string(),
        };
        assert_eq!(
            f.format("2026-06-07 14:30:45").as_deref(),
            Some("2026-06-07")
        );
        assert_eq!(f.format("2026-06-07").as_deref(), Some("2026-06-07"));
        assert_eq!(f.format("not a date"), None);
    }

    #[test]
    fn relative_picks_largest_fitting_unit() {
        let now = zoned(2026, 6, 7, 12, 0);
        let then = zoned(2026, 6, 3, 12, 0); // 4 days earlier
        let out = relative_between(&then, &now, &[]).unwrap();
        assert_eq!(out, "4 days ago");
    }

    #[test]
    fn relative_singular_and_zero_fallback() {
        let now = zoned(2026, 6, 7, 12, 0);
        let one_day = zoned(2026, 6, 6, 12, 0);
        assert_eq!(
            relative_between(&one_day, &now, &[RelativeUnit::Days]).unwrap(),
            "1 day ago"
        );
        let seconds_ago = zoned(2026, 6, 7, 11, 59);
        assert_eq!(
            relative_between(&seconds_ago, &now, &[RelativeUnit::Minutes]).unwrap(),
            "1 minute ago"
        );
        let just_now = zoned(2026, 6, 7, 12, 0);
        assert_eq!(
            relative_between(&just_now, &now, &[RelativeUnit::Minutes]).unwrap(),
            "0 minutes ago"
        );
    }

    #[test]
    fn relative_future_uses_in() {
        let now = zoned(2026, 6, 7, 12, 0);
        let later = zoned(2026, 6, 10, 12, 0); // 3 days later
        assert_eq!(relative_between(&later, &now, &[]).unwrap(), "in 3 days");
    }
}
