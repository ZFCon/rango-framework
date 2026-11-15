//! `utils.dateparse`.
//!
//! Each parser returns `Ok(None)` when the input is not well formatted and `Err` when it is
//! well formatted but not a real date/time (the reference implementation: returns `None` / raises `ValueError`).

use std::sync::LazyLock;

use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use regex::{Captures, Regex};

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid regex")
}

static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(?P<year>\d{4})-(?P<month>\d{1,2})-(?P<day>\d{1,2})$"));
static TIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?P<hour>\d{1,2}):(?P<minute>\d{1,2})(?::(?P<second>\d{1,2})(?:[.,](?P<microsecond>\d{1,6})\d{0,6})?)?$",
    )
});
static DATETIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?P<year>\d{4})-(?P<month>\d{1,2})-(?P<day>\d{1,2})[T ](?P<hour>\d{1,2}):(?P<minute>\d{1,2})(?::(?P<second>\d{1,2})(?:[.,](?P<microsecond>\d{1,6})\d{0,6})?)?\s*(?P<tzinfo>Z|[+-]\d{2}(?::?\d{2})?)?$",
    )
});
static STANDARD_DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    // The Python lookahead `(?=\d+:\d+)` is expressed by trying the "with hours" form first.
    re(
        r"^(?:(?P<days>-?\d+) (?:days?, )?)?(?P<sign>-?)(?:(?P<hours>\d+):(?P<minutes_h>\d+):)?(?:(?P<minutes>\d+):)?(?P<seconds>\d+)(?:[.,](?P<microseconds>\d{1,6})\d{0,6})?$",
    )
});
static ISO8601_DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?P<sign>[-+]?)P(?:(?P<days>\d+(?:[.,]\d+)?)D)?(?:T(?:(?P<hours>\d+(?:[.,]\d+)?)H)?(?:(?P<minutes>\d+(?:[.,]\d+)?)M)?(?:(?P<seconds>\d+(?:[.,]\d+)?)S)?)?$",
    )
});
static POSTGRES_INTERVAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:(?P<days>-?\d+) (?:days? ?))?(?:(?P<sign>[-+])?(?P<hours>\d+):(?P<minutes>\d\d):(?P<seconds>\d\d)(?:\.(?P<microseconds>\d{1,6}))?)?$",
    )
});

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(pub String);

fn int(c: &Captures, name: &str) -> Option<u32> {
    c.name(name).and_then(|m| m.as_str().parse().ok())
}

fn micro(c: &Captures) -> u32 {
    c.name("microsecond")
        .map_or(0, |m| format!("{:0<6}", m.as_str()).parse().unwrap_or(0))
}

static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d{4})-?(\d{2})-?(\d{2})$"));
static ISO_TIME: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?P<h>\d{2})(?::?(?P<m>\d{2})(?::?(?P<s>\d{2})(?:[.,](?P<f>\d+))?)?)?(?P<tz>Z|[+-]\d{2}(?::?\d{2}(?::?\d{2}(?:[.,]\d+)?)?)?)?$",
    )
});

/// Python 3.11+ `date.fromisoformat` (`YYYY-MM-DD` or `YYYYMMDD`).
fn iso_date(value: &str) -> Option<NaiveDate> {
    let c = ISO_DATE.captures(value)?;
    NaiveDate::from_ymd_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?)
}

/// Python 3.11+ `time.fromisoformat`: the time and its UTC offset in seconds, if any.
fn iso_time(value: &str) -> Option<(NaiveTime, Option<i32>)> {
    let c = ISO_TIME.captures(value.strip_prefix('T').unwrap_or(value))?;
    let num = |n: &str| c.name(n).map_or(Some(0u32), |m| m.as_str().parse().ok());
    let micro: u32 = c.name("f").map_or(0, |m| {
        format!("{:0<6}", &m.as_str()[..m.as_str().len().min(6)])
            .parse()
            .unwrap_or(0)
    });
    let time = NaiveTime::from_hms_micro_opt(num("h")?, num("m")?, num("s")?, micro)?;
    let offset = match c.name("tz").map(|m| m.as_str()) {
        None => None,
        Some("Z") => Some(0),
        Some(tz) => {
            let digits: Vec<i32> = tz[1..]
                .split(|ch: char| !ch.is_ascii_digit())
                .flat_map(|p| {
                    // "0530" (no colons) is one chunk: split into pairs.
                    p.as_bytes()
                        .chunks(2)
                        .map(|b| std::str::from_utf8(b).unwrap_or("0").parse().unwrap_or(0))
                        .collect::<Vec<_>>()
                })
                .collect();
            let secs = digits.first().copied().unwrap_or(0) * 3600
                + digits.get(1).copied().unwrap_or(0) * 60
                + digits.get(2).copied().unwrap_or(0);
            Some(if tz.starts_with('-') { -secs } else { secs })
        }
    };
    Some((time, offset))
}

/// `parse_date()`.
pub fn parse_date(value: &str) -> Result<Option<NaiveDate>, ParseError> {
    if let Some(d) = iso_date(value) {
        return Ok(Some(d));
    }
    let Some(c) = DATE_RE.captures(value) else {
        return Ok(None);
    };
    NaiveDate::from_ymd_opt(
        int(&c, "year").unwrap_or(0) as i32,
        int(&c, "month").unwrap_or(0),
        int(&c, "day").unwrap_or(0),
    )
    .map(Some)
    .ok_or_else(|| ParseError("day is out of range for month".into()))
}

/// `parse_time()` (offsets are not supported: such input is "not well formatted").
pub fn parse_time(value: &str) -> Result<Option<NaiveTime>, ParseError> {
    if let Some((t, _)) = iso_time(value) {
        return Ok(Some(t));
    }
    let Some(c) = TIME_RE.captures(value) else {
        return Ok(None);
    };
    NaiveTime::from_hms_micro_opt(
        int(&c, "hour").unwrap_or(0),
        int(&c, "minute").unwrap_or(0),
        int(&c, "second").unwrap_or(0),
        micro(&c),
    )
    .map(Some)
    .ok_or_else(|| ParseError("time out of range".into()))
}

/// A parsed datetime: naive unless the input carried an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedDateTime {
    pub naive: NaiveDateTime,
    pub offset: Option<FixedOffset>,
}

/// `parse_datetime()`.
pub fn parse_datetime(value: &str) -> Result<Option<ParsedDateTime>, ParseError> {
    // `datetime.fromisoformat` first: a bare date, or date + any one separator char + time.
    let mut chars = value.chars();
    let (date_part, rest) = if value.len() >= 10 && value.is_char_boundary(10) {
        value.split_at(10)
    } else {
        (value, "")
    };
    let compact = value.len() >= 8
        && value.is_char_boundary(8)
        && value[..8].bytes().all(|b| b.is_ascii_digit());
    let _ = chars.next();
    let split = if iso_date(date_part).is_some() {
        Some((date_part, rest))
    } else if compact {
        Some(value.split_at(8))
    } else {
        None
    };
    if let Some((d, rest)) = split
        && let Some(date) = iso_date(d)
    {
        let parsed = if rest.is_empty() {
            Some((NaiveTime::MIN, None))
        } else {
            let mut it = rest.chars();
            it.next().and_then(|_| iso_time(it.as_str()))
        };
        if let Some((time, off)) = parsed {
            return Ok(Some(ParsedDateTime {
                naive: NaiveDateTime::new(date, time),
                offset: off.and_then(FixedOffset::east_opt),
            }));
        }
    }
    let Some(c) = DATETIME_RE.captures(value) else {
        return Ok(None);
    };
    let date = NaiveDate::from_ymd_opt(
        int(&c, "year").unwrap_or(0) as i32,
        int(&c, "month").unwrap_or(0),
        int(&c, "day").unwrap_or(0),
    )
    .ok_or_else(|| ParseError("day is out of range for month".into()))?;
    let time = NaiveTime::from_hms_micro_opt(
        int(&c, "hour").unwrap_or(0),
        int(&c, "minute").unwrap_or(0),
        int(&c, "second").unwrap_or(0),
        micro(&c),
    )
    .ok_or_else(|| ParseError("time out of range".into()))?;
    let offset = match c.name("tzinfo").map(|m| m.as_str()) {
        None => None,
        Some("Z") => FixedOffset::east_opt(0),
        Some(tz) => {
            let mins = if tz.len() > 3 {
                tz[tz.len() - 2..].parse::<i32>().unwrap_or(0)
            } else {
                0
            };
            let total = 60 * tz[1..3].parse::<i32>().unwrap_or(0) + mins;
            let total = if tz.starts_with('-') { -total } else { total };
            Some(
                FixedOffset::east_opt(total * 60)
                    .ok_or_else(|| ParseError("offset out of range".into()))?,
            )
        }
    };
    Ok(Some(ParsedDateTime {
        naive: NaiveDateTime::new(date, time),
        offset,
    }))
}

/// `parse_duration()`: `'%d %H:%M:%S.%f'`, ISO 8601, or PostgreSQL's day-time interval.
pub fn parse_duration(value: &str) -> Option<TimeDelta> {
    let (c, is_iso) = if let Some(c) = STANDARD_DURATION_RE.captures(value).filter(|c| std_ok(c)) {
        (c, false)
    } else if let Some(c) = ISO8601_DURATION_RE.captures(value) {
        (c, true)
    } else {
        let c = POSTGRES_INTERVAL_RE.captures(value)?;
        (c, false)
    };
    let f = |name: &str| -> f64 {
        c.name(name)
            .map_or(0.0, |m| m.as_str().replace(',', ".").parse().unwrap_or(0.0))
    };
    let sign = if c.name("sign").is_some_and(|m| m.as_str() == "-") {
        -1.0
    } else {
        1.0
    };
    let micros_str = c
        .name("microseconds")
        .map_or(String::new(), |m| format!("{:0<6}", m.as_str()));
    let micros: f64 = micros_str.parse().unwrap_or(0.0);
    let hours = if c.name("hours").is_some() {
        f("hours")
    } else {
        0.0
    };
    let minutes = if c.name("minutes_h").is_some() {
        f("minutes_h")
    } else {
        f("minutes")
    };
    let rest_us =
        ((hours * 3600.0 + minutes * 60.0 + f("seconds")) * 1e6 + micros).round_ties_even();
    let days_us = (f("days") * 86_400.0 * 1e6).round_ties_even();
    let total = if is_iso {
        sign * days_us + sign * rest_us
    } else {
        days_us + sign * rest_us
    };
    Some(TimeDelta::microseconds(total as i64))
}

/// the lookahead `(?=\d+:\d+)` after `hours:`: both `hours` and `minutes_h` must be set together.
fn std_ok(c: &Captures) -> bool {
    c.name("hours").is_some() == c.name("minutes_h").is_some()
}
