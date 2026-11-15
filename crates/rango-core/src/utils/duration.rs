//! `utils.duration`.

use chrono::TimeDelta;

/// `duration_string()`: `[D ]HH:MM:SS[.ffffff]`.
pub fn duration_string(d: TimeDelta) -> String {
    let (days, hours, minutes, seconds, micros) = components(d);
    let mut s = format!("{hours:02}:{minutes:02}:{seconds:02}");
    if days != 0 {
        s = format!("{days} {s}");
    }
    if micros != 0 {
        s.push_str(&format!(".{micros:06}"));
    }
    s
}

/// `duration_iso_string()`.
pub fn duration_iso_string(d: TimeDelta) -> String {
    let (sign, d) = if d < TimeDelta::zero() {
        ("-", -d)
    } else {
        ("", d)
    };
    let (days, hours, minutes, seconds, micros) = components(d);
    let ms = if micros != 0 {
        format!(".{micros:06}")
    } else {
        String::new()
    };
    format!("{sign}P{days}DT{hours:02}H{minutes:02}M{seconds:02}{ms}S")
}

/// `duration_microseconds()`.
pub fn duration_microseconds(d: TimeDelta) -> i64 {
    d.num_microseconds().unwrap_or(i64::MAX)
}

/// Python `timedelta` normalization: `days` may be negative, the rest are non-negative.
fn components(d: TimeDelta) -> (i64, i64, i64, i64, i64) {
    let total_us = d.num_microseconds().unwrap_or(0);
    let day_us = 86_400_000_000i64;
    let days = total_us.div_euclid(day_us);
    let rem = total_us.rem_euclid(day_us);
    let micros = rem % 1_000_000;
    let secs = rem / 1_000_000;
    (days, secs / 3600, (secs / 60) % 60, secs % 60, micros)
}
