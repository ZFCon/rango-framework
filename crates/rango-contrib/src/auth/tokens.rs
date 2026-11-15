//! `contrib.auth.tokens.PasswordResetTokenGenerator` (stable tokens).

use chrono::{NaiveDate, Utc};
use rango_core::conf::settings;
use rango_core::utils::crypto::{HashAlgorithm, constant_time_compare, salted_hmac};

use super::models::AbstractBaseUser;

const KEY_SALT: &str = "rango.contrib.auth.tokens.PasswordResetTokenGenerator";

fn base36(mut n: i64) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}

fn num_days(d: chrono::NaiveDate) -> i64 {
    (d - NaiveDate::from_ymd_opt(2001, 1, 1).expect("valid")).num_days()
}

fn hash(user: &dyn AbstractBaseUser, timestamp: i64) -> String {
    let login = user.last_login().map_or(String::new(), |t| t.format("%Y-%m-%d %H:%M:%S%.6f%:z").to_string().replace(".000000", "").replace("+00:00", "+00:00"));
    let value = format!("{}{}{login}{timestamp}{}", user.pk().as_i64().map_or_else(String::new, |i| i.to_string()), user.password(), user.email());
    let secret = settings().secret_key().map(str::to_owned).unwrap_or_default();
    let mac = salted_hmac(KEY_SALT.as_bytes(), value.as_bytes(), secret.as_bytes(), HashAlgorithm::Sha256);
    mac.iter().map(|b| format!("{b:02x}")).collect::<String>().chars().step_by(2).collect()
}

/// `make_token(user)`.
pub fn make_token(user: &dyn AbstractBaseUser) -> String {
    let ts = num_days(Utc::now().date_naive());
    format!("{}-{}", base36(ts), hash(user, ts))
}

/// `check_token(user, token)`: valid for `PASSWORD_RESET_TIMEOUT` seconds.
pub fn check_token(user: &dyn AbstractBaseUser, token: &str) -> bool {
    let Some((ts_b36, h)) = token.split_once('-') else { return false };
    let Ok(ts) = i64::from_str_radix(ts_b36, 36) else { return false };
    if !constant_time_compare(&hash(user, ts), h) {
        return false;
    }
    let timeout_days = settings().password_reset_timeout / 86_400;
    num_days(Utc::now().date_naive()) - ts <= timeout_days
}
