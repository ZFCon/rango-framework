//! `contrib.auth.password_validation`.

use std::collections::HashSet;
use std::sync::LazyLock;

use rango_core::conf::settings;
use rango_core::exceptions::ValidationError;
use rango_core::utils::translation::{gettext, ngettext};

/// the list of common passwords (data file, one per line).
static COMMON: LazyLock<HashSet<&'static str>> = LazyLock::new(|| include_str!("../../data/common-passwords.txt").lines().map(str::trim).collect());

/// User attributes the similarity validator compares against (`(attribute, verbose name)`).
pub type UserAttrs<'a> = &'a [(&'a str, &'a str, String)];

fn quick_ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (a.chars().count(), b.chars().count());
    if la + lb == 0 {
        return 1.0;
    }
    let mut avail: std::collections::HashMap<char, i64> = std::collections::HashMap::new();
    for c in b.chars() {
        *avail.entry(c).or_default() += 1;
    }
    let mut matches = 0;
    for c in a.chars() {
        let n = avail.entry(c).or_default();
        if *n > 0 {
            matches += 1;
        }
        *n -= 1;
    }
    2.0 * f64::from(matches) / (la + lb) as f64
}

/// `validate_password(password, user)` for the configured `AUTH_PASSWORD_VALIDATORS`.
///
/// `user` lists `(attribute name, verbose name, value)` of the user being edited.
pub fn validate_password(password: &str, user: UserAttrs) -> Result<(), ValidationError> {
    let mut errors = Vec::new();
    for v in &settings().auth_password_validators {
        let name = v.get("NAME").and_then(|n| n.as_str()).unwrap_or("");
        let opts = v.get("OPTIONS");
        let opt_u = |k: &str, d: u64| opts.and_then(|o| o.get(k)).and_then(|x| x.as_u64()).unwrap_or(d);
        match name.rsplit('.').next().unwrap_or("") {
            "MinimumLengthValidator" => {
                let min = opt_u("min_length", 8) as usize;
                if password.chars().count() < min {
                    errors.push(
                        ValidationError::new(ngettext("This password is too short. It must contain at least %(min_length)d character.", "This password is too short. It must contain at least %(min_length)d characters.", min as u64))
                            .with_code("password_too_short")
                            .with_params([("min_length", min)]),
                    );
                }
            }
            "NumericPasswordValidator" => {
                if !password.is_empty() && password.chars().all(|c| c.is_ascii_digit()) {
                    errors.push(ValidationError::new(gettext("This password is entirely numeric.")).with_code("password_entirely_numeric"));
                }
            }
            "CommonPasswordValidator" => {
                if COMMON.contains(password.to_lowercase().trim()) {
                    errors.push(ValidationError::new(gettext("This password is too common.")).with_code("password_too_common"));
                }
            }
            "UserAttributeSimilarityValidator" => {
                let max = opts.and_then(|o| o.get("max_similarity")).and_then(|x| x.as_f64()).unwrap_or(0.7);
                let pw = password.to_lowercase();
                'attrs: for (_, verbose, value) in user {
                    if value.is_empty() {
                        continue;
                    }
                    let lowered = value.to_lowercase();
                    let parts: Vec<&str> = lowered.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|s| !s.is_empty()).collect();
                    for part in parts.iter().copied().chain(std::iter::once(lowered.as_str())) {
                        if quick_ratio(&pw, part) >= max {
                            errors.push(ValidationError::new(gettext("The password is too similar to the %(verbose_name)s.")).with_code("password_too_similar").with_params([("verbose_name", *verbose)]));
                            break 'attrs;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(ValidationError::from_list(errors)) }
}

/// `password_validators_help_texts()`.
pub fn password_validators_help_texts() -> Vec<String> {
    settings()
        .auth_password_validators
        .iter()
        .filter_map(|v| {
            let name = v.get("NAME")?.as_str()?.rsplit('.').next()?.to_owned();
            Some(match name.as_str() {
                "MinimumLengthValidator" => {
                    let min = v.get("OPTIONS").and_then(|o| o.get("min_length")).and_then(|x| x.as_u64()).unwrap_or(8);
                    ngettext("Your password must contain at least %(min_length)d character.", "Your password must contain at least %(min_length)d characters.", min).replace("%(min_length)d", &min.to_string())
                }
                "NumericPasswordValidator" => gettext("Your password can’t be entirely numeric."),
                "CommonPasswordValidator" => gettext("Your password can’t be a commonly used password."),
                "UserAttributeSimilarityValidator" => gettext("Your password can’t be too similar to your other personal information."),
                _ => return None,
            })
        })
        .collect()
}
