//! `utils.formats`: locale-specific date, time and number formats.
//!
//! The per-locale values live in `data/locale_formats.json`.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::Value as Json;

use super::translation::{get_language, to_locale};
use crate::conf::try_settings;

static LOCALES: LazyLock<HashMap<String, HashMap<String, Json>>> = LazyLock::new(|| serde_json::from_str(include_str!("../../data/locale_formats.json")).expect("locale_formats.json"));

const ISO_DATE: [&str; 1] = ["%Y-%m-%d"];
const ISO_TIME: [&str; 3] = ["%H:%M:%S", "%H:%M:%S.%f", "%H:%M"];
const ISO_DATETIME: [&str; 4] = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M:%S.%f", "%Y-%m-%d %H:%M", "%Y-%m-%d"];

fn locale_value(name: &str, lang: &str) -> Option<&'static Json> {
    let locale = to_locale(lang);
    let mut candidates = vec![locale.clone()];
    if let Some((base, _)) = locale.split_once('_') {
        candidates.push(base.to_owned());
    }
    candidates.iter().find_map(|l| LOCALES.get(l).and_then(|m| m.get(name)))
}

fn setting_value(name: &str) -> Json {
    let Ok(s) = try_settings() else { return Json::Null };
    match name {
        "DATE_FORMAT" => s.date_format.clone().into(),
        "DATETIME_FORMAT" => s.datetime_format.clone().into(),
        "TIME_FORMAT" => s.time_format.clone().into(),
        "YEAR_MONTH_FORMAT" => s.year_month_format.clone().into(),
        "MONTH_DAY_FORMAT" => s.month_day_format.clone().into(),
        "SHORT_DATE_FORMAT" => s.short_date_format.clone().into(),
        "SHORT_DATETIME_FORMAT" => s.short_datetime_format.clone().into(),
        "FIRST_DAY_OF_WEEK" => s.first_day_of_week.into(),
        "DECIMAL_SEPARATOR" => s.decimal_separator.clone().into(),
        "THOUSAND_SEPARATOR" => s.thousand_separator.clone().into(),
        "NUMBER_GROUPING" => s.number_grouping.clone(),
        "DATE_INPUT_FORMATS" => s.date_input_formats.clone().into(),
        "TIME_INPUT_FORMATS" => s.time_input_formats.clone().into(),
        "DATETIME_INPUT_FORMATS" => s.datetime_input_formats.clone().into(),
        _ => Json::Null,
    }
}

/// `get_format(name, lang)`: the locale's value, else the setting's. `*_INPUT_FORMATS` also accept the ISO formats.
pub fn get_format(name: &str, lang: Option<&str>) -> Json {
    let lang = lang.map_or_else(get_language, str::to_owned);
    let from_locale = locale_value(name, &lang).cloned();
    // Only values found in a locale module get the ISO input formats appended.
    let add_iso = from_locale.is_some();
    let mut v = from_locale.unwrap_or_else(|| setting_value(name));
    let iso: &[&str] = match name {
        "DATE_INPUT_FORMATS" => &ISO_DATE,
        "TIME_INPUT_FORMATS" => &ISO_TIME,
        "DATETIME_INPUT_FORMATS" => &ISO_DATETIME,
        _ => &[],
    };
    if let (true, Json::Array(a)) = (add_iso, &mut v) {
        for f in iso {
            if !a.iter().any(|x| x == f) {
                a.push((*f).into());
            }
        }
    }
    v
}

/// A string format (`DATE_FORMAT`, `DECIMAL_SEPARATOR`, ...).
pub fn get_format_str(name: &str) -> String {
    match get_format(name, None) {
        Json::String(s) => s,
        other => other.to_string(),
    }
}

/// A list of `strftime` patterns (`DATE_INPUT_FORMATS`, ...).
pub fn get_format_list(name: &str) -> Vec<String> {
    match get_format(name, None) {
        Json::Array(a) => a.into_iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        _ => vec![],
    }
}

/// `NUMBER_GROUPING` as a list of group sizes (`3` or `(3, 2, 0)`).
pub fn number_grouping() -> Vec<usize> {
    match get_format("NUMBER_GROUPING", None) {
        Json::Number(n) => vec![n.as_u64().unwrap_or(0) as usize],
        Json::Array(a) => a.iter().filter_map(|x| x.as_u64().map(|n| n as usize)).collect(),
        _ => vec![0],
    }
}

/// `numberformat.format(number, decimal_sep, decimal_pos, grouping, thousand_sep, force_grouping)` for a decimal string.
pub fn number_format(number: &str, decimal_sep: &str, decimal_pos: Option<usize>, grouping: &[usize], thousand_sep: &str, force_grouping: bool) -> String {
    let (neg, digits) = number.strip_prefix('-').map_or((false, number), |r| (true, r));
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    let mut frac = frac.to_owned();
    if let Some(p) = decimal_pos {
        frac.truncate(p);
        while frac.len() < p {
            frac.push('0');
        }
    }
    let use_grouping = (force_grouping || try_settings().is_ok_and(|s| s.use_thousand_separator)) && grouping.first().is_some_and(|g| *g > 0) && !thousand_sep.is_empty();
    let mut int_out = int.to_owned();
    if use_grouping {
        // Group sizes apply right-to-left; the last size repeats.
        let (mut parts, mut rest, mut idx) = (Vec::new(), int.to_owned(), 0usize);
        loop {
            let size = *grouping.get(idx).unwrap_or_else(|| grouping.last().expect("non-empty"));
            if size == 0 || rest.len() <= size {
                parts.push(rest);
                break;
            }
            let cut = rest.len() - size;
            parts.push(rest[cut..].to_owned());
            rest.truncate(cut);
            idx += 1;
        }
        parts.reverse();
        int_out = parts.join(thousand_sep);
    }
    let mut out = String::new();
    if neg && (int.chars().any(|c| c != '0') || frac.chars().any(|c| c != '0')) {
        out.push('-');
    }
    out.push_str(&int_out);
    if !frac.is_empty() {
        out.push_str(decimal_sep);
        out.push_str(&frac);
    }
    out
}

/// `localize`/`number_format(value)` of a decimal string with the active locale's separators.
pub fn localize_number(number: &str, decimal_pos: Option<usize>) -> String {
    number_format(number, &get_format_str("DECIMAL_SEPARATOR"), decimal_pos, &number_grouping(), &get_format_str("THOUSAND_SEPARATOR"), false)
}
