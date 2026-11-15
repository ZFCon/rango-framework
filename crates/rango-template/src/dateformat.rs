//! `utils.dateformat` (the `date` / `time` filters and `{% now %}`).

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_AP: [&str; 12] = ["Jan.", "Feb.", "March", "April", "May", "June", "July", "Aug.", "Sept.", "Oct.", "Nov.", "Dec."];
const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// What can be formatted: some parts may be absent (a `date` has no time).
pub struct Parts {
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    /// UTC offset in seconds (aware datetimes).
    pub offset: Option<i32>,
}

impl From<NaiveDateTime> for Parts {
    fn from(d: NaiveDateTime) -> Self {
        Parts { date: Some(d.date()), time: Some(d.time()), offset: None }
    }
}

fn ordinal_suffix(day: u32) -> &'static str {
    if (11..=13).contains(&day) {
        return "th";
    }
    match day % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

fn p_format(t: &NaiveTime) -> String {
    let (h, m) = (t.hour(), t.minute());
    if h == 0 && m == 0 {
        return "midnight".into();
    }
    if h == 12 && m == 0 {
        return "noon".into();
    }
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let ampm = if h < 12 { "a.m." } else { "p.m." };
    if m == 0 { format!("{h12} {ampm}") } else { format!("{h12}:{m:02} {ampm}") }
}

fn offset_str(off: i32) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    format!("{sign}{:02}{:02}", off.abs() / 3600, (off.abs() / 60) % 60)
}

/// `format(value, format_string)`. Characters needing missing parts yield an error string.
pub fn format(p: &Parts, fmt: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = fmt.chars();
    let need_date = |p: &Parts, c: char| p.date.ok_or_else(|| format!("The format for date objects may not contain time-related format specifiers (found '{c}')."));
    let need_time = |p: &Parts, c: char| p.time.ok_or_else(|| format!("The format for time objects may not contain date-related format specifiers (found '{c}')."));
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            'a' => {
                let t = need_time(p, c)?;
                out.push_str(if t.hour() < 12 { "a.m." } else { "p.m." });
            }
            'A' => {
                let t = need_time(p, c)?;
                out.push_str(if t.hour() < 12 { "AM" } else { "PM" });
            }
            'b' => out.push_str(&MONTHS[need_date(p, c)?.month0() as usize][..3].to_lowercase()),
            'c' => {
                let (d, t) = (need_date(p, c)?, need_time(p, c)?);
                out.push_str(&d.and_time(t).format("%Y-%m-%dT%H:%M:%S").to_string());
                if t.nanosecond() / 1000 != 0 {
                    out.push_str(&format!(".{:06}", t.nanosecond() / 1000));
                }
                if let Some(o) = p.offset {
                    out.push_str(&format!("{}{:02}:{:02}", if o < 0 { '-' } else { '+' }, o.abs() / 3600, (o.abs() / 60) % 60));
                }
            }
            'd' => out.push_str(&format!("{:02}", need_date(p, c)?.day())),
            'D' => out.push_str(&WEEKDAYS[need_date(p, c)?.weekday().num_days_from_monday() as usize][..3]),
            'e' => out.push_str(if p.offset == Some(0) { "UTC" } else { "" }),
            'E' => out.push_str(MONTHS[need_date(p, c)?.month0() as usize]),
            'f' => {
                let t = need_time(p, c)?;
                let h12 = if t.hour() % 12 == 0 { 12 } else { t.hour() % 12 };
                out.push_str(&if t.minute() == 0 { h12.to_string() } else { format!("{h12}:{:02}", t.minute()) });
            }
            'F' => out.push_str(MONTHS[need_date(p, c)?.month0() as usize]),
            'g' => {
                let t = need_time(p, c)?;
                out.push_str(&(if t.hour() % 12 == 0 { 12 } else { t.hour() % 12 }).to_string());
            }
            'G' => out.push_str(&need_time(p, c)?.hour().to_string()),
            'h' => {
                let t = need_time(p, c)?;
                out.push_str(&format!("{:02}", if t.hour() % 12 == 0 { 12 } else { t.hour() % 12 }));
            }
            'H' => out.push_str(&format!("{:02}", need_time(p, c)?.hour())),
            'i' => out.push_str(&format!("{:02}", need_time(p, c)?.minute())),
            'I' => out.push('0'),
            'j' => out.push_str(&need_date(p, c)?.day().to_string()),
            'l' => out.push_str(WEEKDAYS[need_date(p, c)?.weekday().num_days_from_monday() as usize]),
            'L' => out.push_str(if need_date(p, c)?.leap_year() { "True" } else { "False" }),
            'm' => out.push_str(&format!("{:02}", need_date(p, c)?.month())),
            'M' => out.push_str(&MONTHS[need_date(p, c)?.month0() as usize][..3]),
            'n' => out.push_str(&need_date(p, c)?.month().to_string()),
            'N' => out.push_str(MONTHS_AP[need_date(p, c)?.month0() as usize]),
            'o' => out.push_str(&need_date(p, c)?.iso_week().year().to_string()),
            'O' => out.push_str(&offset_str(p.offset.ok_or("The format for naive datetimes may not contain timezone specifiers")?)),
            'P' => out.push_str(&p_format(&need_time(p, c)?)),
            'r' => {
                let (d, t) = (need_date(p, c)?, need_time(p, c)?);
                out.push_str(&format!("{}, {} {}", &WEEKDAYS[d.weekday().num_days_from_monday() as usize][..3], d.format("%-d"), d.and_time(t).format("%b %Y %H:%M:%S")));
                out.push(' ');
                out.push_str(&offset_str(p.offset.unwrap_or(0)));
            }
            's' => out.push_str(&format!("{:02}", need_time(p, c)?.second())),
            'S' => out.push_str(ordinal_suffix(need_date(p, c)?.day())),
            't' => {
                let d = need_date(p, c)?;
                let next = if d.month() == 12 { NaiveDate::from_ymd_opt(d.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1) };
                out.push_str(&next.map_or(0, |n| (n - NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("valid")).num_days()).to_string());
            }
            'T' => out.push_str(if p.offset == Some(0) { "UTC" } else { "" }),
            'u' => out.push_str(&format!("{:06}", need_time(p, c)?.nanosecond() / 1000)),
            'U' => {
                let (d, t) = (need_date(p, c)?, need_time(p, c)?);
                out.push_str(&(d.and_time(t).and_utc().timestamp() - i64::from(p.offset.unwrap_or(0))).to_string());
            }
            'w' => out.push_str(&need_date(p, c)?.weekday().num_days_from_sunday().to_string()),
            'W' => out.push_str(&need_date(p, c)?.iso_week().week().to_string()),
            'y' => out.push_str(&format!("{:02}", need_date(p, c)?.year().rem_euclid(100))),
            'Y' => out.push_str(&need_date(p, c)?.year().to_string()),
            'z' => out.push_str(&need_date(p, c)?.ordinal().to_string()),
            'Z' => out.push_str(&p.offset.ok_or("The format for naive datetimes may not contain timezone specifiers")?.to_string()),
            other => out.push(other),
        }
    }
    Ok(out)
}

/// `DATE_FORMAT` etc. defaults for `en-us` (used when rendering dates without a filter).
pub fn default_format(p: &Parts) -> String {
    let f = match (p.date.is_some(), p.time.is_some()) {
        (true, true) => "N j, Y, P",
        (true, false) => "N j, Y",
        (false, true) => "P",
        _ => "",
    };
    format(p, f).unwrap_or_default()
}
