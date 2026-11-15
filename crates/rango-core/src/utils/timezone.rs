//! `utils.timezone`.
//!
//! Rust's types already distinguish naive from aware datetimes, so `is_aware`/`is_naive`
//! disappear. `settings.TIME_ZONE` is parsed as an IANA name; unlike the reference implementation, the process
//! `TZ` environment variable is *not* modified.

use std::future::Future;

use chrono::{DateTime, FixedOffset, LocalResult, NaiveDateTime, Offset, TimeZone, Utc};
pub use chrono_tz::Tz;

use crate::conf::try_settings;
use crate::exceptions::{Error, Result};

tokio::task_local! {
    static CURRENT_TZ: Tz;
}

/// `get_fixed_timezone(offset_minutes)`.
pub fn get_fixed_timezone(offset_minutes: i32) -> Option<FixedOffset> {
    FixedOffset::east_opt(offset_minutes * 60)
}

/// `get_default_timezone()`: `settings.TIME_ZONE`.
pub fn get_default_timezone() -> Result<Tz> {
    let name = try_settings()?.time_zone.clone();
    name.parse::<Tz>()
        .map_err(|_| Error::improperly_configured(format!("Incorrect timezone setting: {name}")))
}

/// `get_current_timezone()`: the innermost [`override_tz`] scope, else the default.
pub fn get_current_timezone() -> Result<Tz> {
    match CURRENT_TZ.try_with(|tz| *tz) {
        Ok(tz) => Ok(tz),
        Err(_) => get_default_timezone(),
    }
}

/// `timezone.override(tz)` for async code (tasks spawned inside do not inherit it).
pub async fn override_tz<F: Future>(tz: Tz, fut: F) -> F::Output {
    CURRENT_TZ.scope(tz, fut).await
}

/// `timezone.override(tz)` for synchronous code.
pub fn override_tz_sync<R>(tz: Tz, body: impl FnOnce() -> R) -> R {
    CURRENT_TZ.sync_scope(tz, body)
}

/// `now()` as an aware UTC datetime (use [`localtime`] to convert; with `USE_TZ = false`
/// the reference implementation would return naive local time, see [`now_naive`]).
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// `now()` when `USE_TZ = false`: naive local time in the default timezone.
pub fn now_naive() -> Result<NaiveDateTime> {
    Ok(Utc::now()
        .with_timezone(&get_default_timezone()?)
        .naive_local())
}

/// `localtime(value, timezone=None)`.
pub fn localtime(value: DateTime<Utc>, tz: Option<Tz>) -> Result<DateTime<Tz>> {
    let tz = match tz {
        Some(tz) => tz,
        None => get_current_timezone()?,
    };
    Ok(value.with_timezone(&tz))
}

/// `make_aware(value, timezone)`: ambiguous times take the first occurrence (`fold=0`) and
/// non-existent times use the offset in force before the gap, as Python's `zoneinfo` does.
pub fn make_aware(value: NaiveDateTime, tz: Tz) -> DateTime<Tz> {
    match tz.from_local_datetime(&value) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(first, _) => first,
        LocalResult::None => {
            let before = tz
                .offset_from_utc_datetime(&(value - chrono::TimeDelta::hours(24)))
                .fix();
            let utc = value - chrono::TimeDelta::seconds(i64::from(before.local_minus_utc()));
            tz.from_utc_datetime(&utc)
        }
    }
}

/// `make_naive(value, timezone)`.
pub fn make_naive<Z: TimeZone>(value: &DateTime<Z>, tz: Tz) -> NaiveDateTime {
    value.with_timezone(&tz).naive_local()
}
