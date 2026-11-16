//! Built-in filters (`template.defaultfilters`) and the `humanize` library.

use std::str::FromStr;

use chrono::{NaiveDateTime, Timelike};
use rango_core::utils::html::{escape, escapejs, linebreaks, strip_tags};
use rango_core::utils::text::{Truncator, capfirst, phone2numeric, slugify, wrap};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::dateformat::{Parts, format as date_format};
use crate::engine::{FilterFn, Library, TemplateError};
use crate::value::Value;

type R = Result<Value, TemplateError>;

fn s(v: &Value) -> String {
    v.to_str()
}

fn esc_if(autoescape: bool, v: &Value) -> String {
    if autoescape && !v.is_safe() { escape(&s(v)).into_string() } else { s(v) }
}

/// Plain string result; `Filter::is_safe` (applied by the caller) re-marks it safe's `stringfilter`.
fn string_result(_orig: &Value, out: String) -> R {
    Ok(Value::Str(out))
}

fn dt_parts(v: &Value) -> Option<Parts> {
    match v {
        Value::Date(d) => Some(Parts { date: Some(*d), time: None, offset: None }),
        Value::Time(t) => Some(Parts { date: None, time: Some(*t), offset: None }),
        Value::DateTime(d, off) => Some(Parts { date: Some(d.date()), time: Some(d.time()), offset: *off }),
        _ => None,
    }
}

fn named_format(arg: &str) -> String {
    match arg {
        "DATE_FORMAT" | "DATETIME_FORMAT" | "SHORT_DATE_FORMAT" | "SHORT_DATETIME_FORMAT" | "TIME_FORMAT" | "YEAR_MONTH_FORMAT" | "MONTH_DAY_FORMAT" => rango_core::utils::formats::get_format_str(arg),
        other => other.to_owned(),
    }
}

fn int_arg(a: Option<&Value>) -> Option<i64> {
    a.and_then(Value::as_i64)
}

fn floatformat(v: &Value, arg: Option<&Value>, _ae: bool) -> R {
    let arg_s = arg.map_or("-1".to_owned(), s);
    let (mut arg_str, mut use_l10n, mut force_grouping) = (arg_s.clone(), true, false);
    if arg_str.ends_with('u') {
        arg_str.pop();
        use_l10n = false;
    }
    if arg_str.ends_with('g') {
        arg_str.pop();
        force_grouping = true;
    }
    let _ = (use_l10n, force_grouping);
    let Ok(p) = arg_str.parse::<i32>() else { return Ok(Value::Str(s(v))) };
    let text = match v {
        Value::Float(f) => format!("{f}"),
        Value::Decimal(d) => d.to_string(),
        other => s(other),
    };
    let Ok(d) = Decimal::from_str(text.trim()).or_else(|_| Decimal::from_scientific(text.trim())) else {
        return Ok(Value::Str(String::new()));
    };
    let abs_p = p.unsigned_abs();
    let is_whole = d.fract().is_zero();
    if p < 0 && is_whole {
        let mut out = format!("{}", d.round_dp(0));
        if out == "-0" {
            out = "0".into();
        }
        return Ok(Value::Safe(out));
    }
    let rounded = d.round_dp_with_strategy(abs_p, RoundingStrategy::MidpointAwayFromZero);
    let mut out = format!("{:.*}", abs_p as usize, rounded);
    if rounded.is_zero() && out.starts_with('-') {
        out.remove(0);
    }
    Ok(Value::Safe(out))
}

fn title(v: &Value, _: Option<&Value>, _: bool) -> R {
    let t = s(v);
    let mut out = String::with_capacity(t.len());
    let mut prev_letter = false;
    for c in t.chars() {
        if c.is_alphabetic() {
            if prev_letter { out.extend(c.to_lowercase()) } else { out.extend(c.to_uppercase()) }
            prev_letter = true;
        } else {
            out.push(c);
            prev_letter = false;
        }
    }
    // the reference implementation: re.sub(r"([a-z])'([A-Z])", lambda m: m[0].lower(), t)
    let chars: Vec<char> = out.chars().collect();
    let mut fixed = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i].is_ascii_lowercase() && chars[i + 1] == '\'' && chars[i + 2].is_ascii_uppercase() {
            fixed.push(chars[i]);
            fixed.push('\'');
            fixed.extend(chars[i + 2].to_lowercase());
            i += 3;
        } else {
            fixed.push(chars[i]);
            i += 1;
        }
    }
    string_result(v, fixed)
}

fn urlize_text(text: &str, autoescape: bool, trim: Option<usize>) -> String {
    use std::sync::LazyLock;
    static URL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)\b((?:https?://|www\.)[^\s<>]+)").expect("valid regex"));
    let mut out = String::new();
    let mut last = 0;
    let esc = |t: &str| if autoescape { escape(t).into_string() } else { t.to_owned() };
    for m in URL.find_iter(text) {
        out.push_str(&esc(&text[last..m.start()]));
        let mut url = m.as_str().to_owned();
        let mut trailing = String::new();
        while url.ends_with(['.', ',', ':', ';', '!']) {
            trailing.insert(0, url.pop().expect("non-empty"));
        }
        let href = if url.to_lowercase().starts_with("http") { url.clone() } else { format!("http://{url}") };
        let shown = match trim {
            Some(n) if url.chars().count() > n => format!("{}…", url.chars().take(n.saturating_sub(1)).collect::<String>()),
            _ => url.clone(),
        };
        out.push_str(&format!("<a href=\"{}\" rel=\"nofollow\">{}</a>{}", escape(&href), esc(&shown), esc(&trailing)));
        last = m.end();
    }
    out.push_str(&esc(&text[last..]));
    out
}

fn timesince_impl(d: &NaiveDateTime, now: &NaiveDateTime, depth: usize) -> String {
    const CHUNKS: [(i64, &str, &str); 6] =
        [(60 * 60 * 24 * 365, "year", "years"), (60 * 60 * 24 * 30, "month", "months"), (60 * 60 * 24 * 7, "week", "weeks"), (60 * 60 * 24, "day", "days"), (60 * 60, "hour", "hours"), (60, "minute", "minutes")];
    let mut since = (*now - *d).num_seconds();
    if since <= 0 {
        return "0\u{a0}minutes".into();
    }
    let mut i = CHUNKS.iter().position(|(secs, ..)| since / secs != 0).unwrap_or(CHUNKS.len() - 1);
    let mut parts = Vec::new();
    while i < CHUNKS.len() && parts.len() < depth {
        let (secs, one, many) = CHUNKS[i];
        let count = since / secs;
        if count == 0 {
            break;
        }
        parts.push(format!("{count}\u{a0}{}", if count == 1 { one } else { many }));
        since -= secs * count;
        i += 1;
    }
    parts.join(", ")
}

fn now_naive() -> NaiveDateTime {
    rango_core::utils::timezone::now().naive_utc()
}

fn dictsort_key(item: &Value, key: &str) -> Value {
    let mut cur = item.clone();
    for bit in key.split('.') {
        cur = match &cur {
            Value::Map(m) => m.get(bit).cloned().unwrap_or(Value::None),
            Value::List(l) => bit.parse::<usize>().ok().and_then(|i| l.get(i).cloned()).unwrap_or(Value::None),
            _ => Value::None,
        };
    }
    cur
}

fn unordered_list_fmt(items: &[Value], tabs: usize, autoescape: bool) -> String {
    let indent = "\t".repeat(tabs);
    let mut output = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let item = &items[i];
        let mut sub = String::new();
        if let Some(Value::List(children)) = items.get(i + 1) {
            sub = format!("\n{indent}<ul>\n{}\n{indent}</ul>\n{indent}", unordered_list_fmt(children, tabs + 1, autoescape));
            i += 1;
        }
        output.push(format!("{indent}<li>{}{sub}</li>", esc_if(autoescape, item)));
        i += 1;
    }
    output.join("\n")
}

fn py_format(fmt: &str, v: &Value) -> Option<String> {
    // Minimal printf-style subset: %s %d %i %f %.Nf %x %o %e %05d %-5s %%.
    let spec = fmt.strip_prefix('%')?;
    let (flags, rest): (String, &str) = {
        let n = spec.chars().take_while(|c| "-+ #0".contains(*c)).count();
        (spec[..n].to_owned(), &spec[n..])
    };
    let wn = rest.chars().take_while(char::is_ascii_digit).count();
    let width: usize = rest[..wn].parse().unwrap_or(0);
    let rest = &rest[wn..];
    let (prec, conv) = match rest.strip_prefix('.') {
        Some(r) => {
            let pn = r.chars().take_while(char::is_ascii_digit).count();
            (r[..pn].parse::<usize>().ok(), &r[pn..])
        }
        None => (None, rest),
    };
    let body = match conv {
        "s" => {
            let t = s(v);
            prec.map_or(t.clone(), |p| t.chars().take(p).collect())
        }
        "d" | "i" => format!("{}", v.as_f64()? as i64),
        "f" | "F" => format!("{:.*}", prec.unwrap_or(6), v.as_f64()?),
        "e" => format!("{:.*e}", prec.unwrap_or(6), v.as_f64()?),
        "x" => format!("{:x}", v.as_i64()?),
        "X" => format!("{:X}", v.as_i64()?),
        "o" => format!("{:o}", v.as_i64()?),
        _ => return None,
    };
    let body = if flags.contains('+') && v.as_f64().is_some_and(|f| f >= 0.0) && "dfiF".contains(conv) { format!("+{body}") } else { body };
    Some(if body.len() >= width {
        body
    } else if flags.contains('-') {
        format!("{body:<width$}")
    } else if flags.contains('0') && conv != "s" {
        let (sign, digits) = body.strip_prefix(['-', '+']).map_or(("", body.as_str()), |d| (&body[..1], d));
        format!("{sign}{digits:0>w$}", w = width - sign.len())
    } else {
        format!("{body:>width$}")
    })
}

fn float_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    a.cmp_py(b).unwrap_or(std::cmp::Ordering::Equal)
}

pub fn default_filters() -> Library {
    let mut l = Library::new();
    macro_rules! f {
        ($name:literal, $safe:expr, $ae:expr, $func:expr) => {
            l.filter($name, $func as FilterFn, $safe, $ae);
        };
    }
    f!("add", false, false, |v, a, _| {
        let arg = a.cloned().unwrap_or(Value::None);
        let as_int = |x: &Value| match x {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(i64::from(*b)),
            Value::Float(f) if f.is_finite() => Some(f.trunc() as i64),
            Value::Str(t) | Value::Safe(t) => t.trim().parse::<i64>().ok(),
            _ => None,
        };
        if let (Some(x), Some(y)) = (as_int(v), as_int(&arg)) {
            return Ok(Value::Int(x + y));
        }
        Ok(match (v, &arg) {
            (Value::List(x), Value::List(y)) => Value::List(x.iter().chain(y).cloned().collect()),
            (Value::Str(x) | Value::Safe(x), Value::Str(y) | Value::Safe(y)) => Value::Str(format!("{x}{y}")),
            (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
            (Value::Float(x), Value::Int(y)) | (Value::Int(y), Value::Float(x)) => Value::Float(x + *y as f64),
            _ => Value::Str(String::new()),
        })
    });
    f!("addslashes", true, false, |v, _, _| string_result(v, s(v).replace('\\', "\\\\").replace('"', "\\\"").replace('\'', "\\'")));
    f!("capfirst", true, false, |v, _, _| string_result(v, capfirst(&s(v))));
    f!("escapejs", false, false, |v, _, _| Ok(Value::Safe(escapejs(&s(v)).into_string())));
    f!("floatformat", true, false, floatformat);
    f!("get_digit", false, false, |v, a, _| {
        let (Some(n), Some(arg)) = (v.as_i64(), int_arg(a)) else { return Ok(v.clone()) };
        if arg < 1 {
            return Ok(Value::Int(n));
        }
        let digits: Vec<char> = n.to_string().chars().collect();
        Ok(Value::Int(digits.iter().rev().nth(arg as usize - 1).and_then(|c| c.to_digit(10)).map_or(0, i64::from)))
    });
    f!("iriencode", true, false, |v, _, _| string_result(v, rango_core::utils::encoding::iri_to_uri(&s(v))));
    f!("linenumbers", true, true, |v, _, ae| {
        let text = s(v);
        let lines: Vec<&str> = text.split('\n').collect();
        let width = lines.len().to_string().len();
        let out: Vec<String> = lines.iter().enumerate().map(|(i, line)| format!("{:0width$}. {}", i + 1, if ae && !v.is_safe() { escape(line).into_string() } else { (*line).to_owned() })).collect();
        Ok(Value::Safe(out.join("\n")))
    });
    f!("lower", true, false, |v, _, _| string_result(v, s(v).to_lowercase()));
    f!("make_list", false, false, |v, _, _| Ok(Value::List(s(v).chars().map(|c| Value::Str(c.to_string())).collect())));
    f!("slugify", true, false, |v, _, _| Ok(Value::Safe(slugify(&s(v), false))));
    f!("stringformat", true, false, |v, a, _| Ok(a.and_then(|f| py_format(&format!("%{}", s(f)), v)).map_or(Value::Str(String::new()), Value::Str)));
    f!("title", true, false, title);
    f!("truncatechars", true, false, |v, a, _| match int_arg(a) {
        Some(n) => Ok(Value::Str(Truncator::new(&s(v)).chars(n, None))),
        None => Ok(v.clone()),
    });
    f!("truncatechars_html", true, false, |v, a, _| match int_arg(a) {
        Some(n) => Ok(Value::Str(Truncator::new(&s(v)).chars(n, None))),
        None => Ok(v.clone()),
    });
    f!("truncatewords", true, false, |v, a, _| match int_arg(a) {
        Some(n) => Ok(Value::Str(Truncator::new(&s(v)).words(n, Some(" …")))),
        None => Ok(v.clone()),
    });
    f!("truncatewords_html", true, false, |v, a, _| match int_arg(a) {
        Some(n) => Ok(Value::Str(Truncator::new(&s(v)).words(n, Some(" …")))),
        None => Ok(v.clone()),
    });
    f!("upper", false, false, |v, _, _| string_result(v, s(v).to_uppercase()));
    f!("urlencode", false, false, |v, a, _| Ok(Value::Str(rango_core::utils::http::quote(&s(v), &a.map_or("/".to_owned(), s)))));
    f!("urlize", true, true, |v, _, ae| Ok(Value::Safe(urlize_text(&s(v), ae && !v.is_safe(), None))));
    f!("urlizetrunc", true, true, |v, a, ae| Ok(Value::Safe(urlize_text(&s(v), ae && !v.is_safe(), int_arg(a).map(|n| n as usize)))));
    f!("wordcount", false, false, |v, _, _| Ok(Value::Int(s(v).split_whitespace().count() as i64)));
    f!("wordwrap", true, false, |v, a, _| string_result(v, wrap(&s(v), int_arg(a).unwrap_or(0).max(0) as usize)));
    f!("ljust", true, false, |v, a, _| string_result(v, format!("{:<w$}", s(v), w = int_arg(a).unwrap_or(0).max(0) as usize)));
    f!("rjust", true, false, |v, a, _| string_result(v, format!("{:>w$}", s(v), w = int_arg(a).unwrap_or(0).max(0) as usize)));
    f!("center", true, false, |v, a, _| {
        let (t, w) = (s(v), int_arg(a).unwrap_or(0).max(0) as usize);
        let len = t.chars().count();
        if w <= len {
            return string_result(v, t);
        }
        let total = w - len;
        // str.center: extra padding goes on the left when the total is odd and the width is odd.
        let left = total / 2 + (total & w & 1);
        string_result(v, format!("{}{t}{}", " ".repeat(left), " ".repeat(total - left)))
    });
    f!("cut", true, false, |v, a, _| {
        let cut = a.map_or(String::new(), s);
        let out = s(v).replace(&cut, "");
        Ok(if v.is_safe() && cut != ";" { Value::Safe(out) } else { Value::Str(out) })
    });
    f!("escape", true, false, |v, _, _| Ok(if v.is_safe() { v.clone() } else { Value::Safe(escape(&s(v)).into_string()) }));
    f!("force_escape", true, false, |v, _, _| Ok(Value::Safe(escape(&s(v)).into_string())));
    f!("linebreaks", true, true, |v, _, ae| Ok(Value::Safe(linebreaks(&s(v), ae && !v.is_safe()))));
    f!("linebreaksbr", true, true, |v, _, ae| {
        let t = s(v).replace("\r\n", "\n").replace('\r', "\n");
        let t = if ae && !v.is_safe() { escape(&t).into_string() } else { t };
        Ok(Value::Safe(t.replace('\n', "<br>")))
    });
    f!("safe", false, false, |v, _, _| Ok(Value::Safe(s(v))));
    f!("safeseq", true, false, |v, _, _| match v {
        Value::List(l) => Ok(Value::List(l.iter().map(|x| Value::Safe(s(x))).collect())),
        other => Ok(other.clone()),
    });
    f!("striptags", true, false, |v, _, _| strip_tags(&s(v)).map(Value::Str).map_err(|e| TemplateError::Render(e.to_string())));
    f!("last", false, false, |v, _, _| Ok(match v {
        Value::List(l) => l.last().cloned().unwrap_or(Value::Str(String::new())),
        Value::Str(t) | Value::Safe(t) => t.chars().last().map_or(Value::Str(String::new()), |c| Value::Str(c.to_string())),
        _ => Value::Str(String::new()),
    }));
    f!("first", false, false, |v, _, _| Ok(match v {
        Value::List(l) => l.first().cloned().unwrap_or(Value::Str(String::new())),
        Value::Str(t) | Value::Safe(t) => t.chars().next().map_or(Value::Str(String::new()), |c| Value::Str(c.to_string())),
        _ => Value::Str(String::new()),
    }));
    f!("join", true, true, |v, a, ae| {
        let sep = a.cloned().unwrap_or(Value::Str(String::new()));
        let Value::List(items) = v else { return Ok(v.clone()) };
        let parts: Vec<String> = items.iter().map(|i| esc_if(ae, i)).collect();
        let sep_s = if ae && !sep.is_safe() { escape(&s(&sep)).into_string() } else { s(&sep) };
        Ok(Value::Safe(parts.join(&sep_s)))
    });
    f!("length", false, false, |v, _, _| Ok(Value::Int(v.len().unwrap_or(0) as i64)));
    f!("random", false, false, |v, _, _| match v {
        Value::List(l) if !l.is_empty() => Ok(l[rand::RngExt::random_range(&mut rand::rng(), 0..l.len())].clone()),
        other => Ok(other.clone()),
    });
    f!("slice", true, false, |v, a, _| {
        let spec = a.map_or(String::new(), s);
        let bits: Vec<Option<i64>> = spec.split(':').map(|b| b.trim().parse().ok()).collect();
        let norm = |i: i64, len: i64| if i < 0 { (len + i).max(0) } else { i.min(len) };
        let apply = |len: i64| -> Option<(i64, i64, i64)> {
            let step = bits.get(2).copied().flatten().unwrap_or(1);
            if step == 0 {
                return None;
            }
            let (start, stop) = (bits.first().copied().flatten().map_or(0, |x| norm(x, len)), bits.get(1).copied().flatten().map_or(len, |x| norm(x, len)));
            Some((start, stop, step))
        };
        match v {
            Value::List(l) => {
                let Some((a0, b0, st)) = apply(l.len() as i64) else { return Ok(v.clone()) };
                let out: Vec<Value> = if st > 0 { (a0..b0).step_by(st as usize).map(|i| l[i as usize].clone()).collect() } else { vec![] };
                Ok(Value::List(out))
            }
            Value::Str(t) | Value::Safe(t) => {
                let cs: Vec<char> = t.chars().collect();
                let Some((a0, b0, st)) = apply(cs.len() as i64) else { return Ok(v.clone()) };
                let out: String = if st > 0 { (a0..b0).step_by(st as usize).map(|i| cs[i as usize]).collect() } else { String::new() };
                string_result(v, out)
            }
            other => Ok(other.clone()),
        }
    });
    f!("unordered_list", true, true, |v, _, ae| match v {
        Value::List(l) => Ok(Value::Safe(unordered_list_fmt(l, 1, ae))),
        other => Ok(other.clone()),
    });
    f!("divisibleby", false, false, |v, a, _| match (v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)), a.and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))) {
        (Some(x), Some(y)) if y != 0 => Ok(Value::Bool(x % y == 0)),
        (Some(_), Some(_)) => Err(TemplateError::Render("integer modulo by zero".into())),
        _ => Err(TemplateError::Render("invalid literal for int()".into())),
    });
    f!("date", false, false, |v, a, _| {
        let Some(p) = dt_parts(v) else { return Ok(Value::Str(String::new())) };
        let fmt = named_format(&a.map_or("DATE_FORMAT".to_owned(), s));
        Ok(date_format(&p, &fmt).map_or(Value::Str(String::new()), Value::Str))
    });
    f!("time", false, false, |v, a, _| {
        let Some(p) = dt_parts(v) else { return Ok(Value::Str(String::new())) };
        let fmt = named_format(&a.map_or("TIME_FORMAT".to_owned(), s));
        Ok(date_format(&p, &fmt).map_or(Value::Str(String::new()), Value::Str))
    });
    f!("timesince", false, false, |v, a, _| match v {
        Value::DateTime(d, _) => {
            let now = match a {
                Some(Value::DateTime(n, _)) => *n,
                _ => now_naive(),
            };
            Ok(Value::Str(timesince_impl(d, &now, 2)))
        }
        Value::Date(d) => {
            let now = match a {
                Some(Value::DateTime(n, _)) => *n,
                _ => now_naive(),
            };
            Ok(Value::Str(timesince_impl(&d.and_hms_opt(0, 0, 0).expect("midnight"), &now, 2)))
        }
        _ => Ok(Value::Str(String::new())),
    });
    f!("timeuntil", false, false, |v, a, _| match v {
        Value::DateTime(d, _) => {
            let now = match a {
                Some(Value::DateTime(n, _)) => *n,
                _ => now_naive(),
            };
            Ok(Value::Str(timesince_impl(&now, d, 2)))
        }
        _ => Ok(Value::Str(String::new())),
    });
    f!("default", false, false, |v, a, _| Ok(if v.truthy() { v.clone() } else { a.cloned().unwrap_or(Value::Str(String::new())) }));
    f!("default_if_none", false, false, |v, a, _| Ok(if matches!(v, Value::None) { a.cloned().unwrap_or(Value::Str(String::new())) } else { v.clone() }));
    f!("dictsort", false, false, |v, a, _| {
        let Value::List(items) = v else { return Ok(Value::Str(String::new())) };
        let key = a.map_or(String::new(), s);
        let mut out = items.clone();
        out.sort_by(|x, y| float_cmp(&dictsort_key(x, &key), &dictsort_key(y, &key)));
        Ok(Value::List(out))
    });
    f!("dictsortreversed", false, false, |v, a, _| {
        let Value::List(items) = v else { return Ok(Value::Str(String::new())) };
        let key = a.map_or(String::new(), s);
        let mut out = items.clone();
        out.sort_by(|x, y| float_cmp(&dictsort_key(y, &key), &dictsort_key(x, &key)));
        Ok(Value::List(out))
    });
    f!("yesno", false, false, |v, a, _| {
        let arg = a.map_or("yes,no,maybe".to_owned(), s);
        let bits: Vec<&str> = arg.split(',').collect();
        if bits.len() < 2 {
            return Ok(v.clone());
        }
        let (yes, no, maybe) = (bits[0], bits[1], bits.get(2).copied().unwrap_or(bits[1]));
        Ok(Value::Str(
            match v {
                Value::None => maybe,
                x if x.truthy() => yes,
                _ => no,
            }
            .to_owned(),
        ))
    });
    f!("filesizeformat", false, false, |v, _, _| {
        let Some(n) = v.as_f64() else { return Ok(Value::Str("0\u{a0}bytes".into())) };
        let (neg, n) = (n < 0.0, n.abs());
        let units = [("KB", 1u64 << 10), ("MB", 1 << 20), ("GB", 1 << 30), ("TB", 1u64 << 40), ("PB", 1u64 << 50)];
        let sign = if neg { "-" } else { "" };
        let out = if n < 1024.0 {
            format!("{sign}{}\u{a0}{}", n as i64, if n as i64 == 1 { "byte" } else { "bytes" })
        } else {
            let (name, size) = units.iter().rev().find(|(_, sz)| n >= *sz as f64).copied().unwrap_or(units[0]);
            let val = (n / size as f64 * 10.0).round() / 10.0;
            format!("{sign}{val:.1}\u{a0}{name}")
        };
        Ok(Value::Str(out))
    });
    f!("pluralize", false, false, |v, a, _| {
        let arg = a.map_or("s".to_owned(), s);
        let bits: Vec<&str> = arg.split(',').collect();
        let (singular, plural) = match bits.as_slice() {
            [p] => ("", *p),
            [s1, p] => (*s1, *p),
            _ => return Ok(Value::Str(String::new())),
        };
        let one = match v {
            Value::List(l) => l.len() == 1,
            Value::Str(t) | Value::Safe(t) if t.parse::<f64>().is_err() => t.chars().count() == 1,
            other => other.as_f64().is_some_and(|f| f == 1.0),
        };
        Ok(Value::Str(if one { singular } else { plural }.to_owned()))
    });
    f!("phone2numeric", true, false, |v, _, _| string_result(v, phone2numeric(&s(v))));
    f!("pprint", false, false, |v, _, _| Ok(Value::Str(v.repr())));
    f!("json_script", false, false, |v, a, _| {
        let json: serde_json::Value = match v {
            Value::List(l) => serde_json::Value::Array(l.iter().map(to_json).collect()),
            other => to_json(other),
        };
        let id = a.map(s);
        rango_core::utils::html::json_script(&json, id.as_deref()).map(|x| Value::Safe(x.into_string())).map_err(|e| TemplateError::Render(e.to_string()))
    });
    l
}

fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::None => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f).map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::List(l) => serde_json::Value::Array(l.iter().map(to_json).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
        other => serde_json::Value::String(s(other)),
    }
}

/// `humanize` and other optional libraries (the `static`/`i18n` tag libraries live in `tags`).
pub fn extra_library(name: &str) -> Option<Library> {
    if name != "humanize" {
        return None;
    }
    let mut l = Library::new();
    l.filter("intcomma", |v, _, _| {
        let t = s(v);
        let (int_part, frac) = t.split_once('.').map_or((t.as_str(), ""), |(a, b)| (a, b));
        let (sign, digits) = int_part.strip_prefix('-').map_or(("", int_part), |d| ("-", d));
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        Ok(Value::Str(format!("{sign}{out}{}{frac}", if frac.is_empty() { "" } else { "." })))
    }, false, false);
    l.filter("ordinal", |v, _, _| {
        let Some(n) = v.as_i64() else { return Ok(v.clone()) };
        let suffix = if (11..=13).contains(&(n % 100)) { "th" } else { ["th", "st", "nd", "rd", "th", "th", "th", "th", "th", "th"][(n % 10) as usize] };
        Ok(Value::Str(format!("{n}{suffix}")))
    }, true, false);
    l.filter("apnumber", |v, _, _| {
        let Some(n) = v.as_i64() else { return Ok(v.clone()) };
        let words = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
        Ok(if (1..=9).contains(&n) { Value::Str(words[(n - 1) as usize].into()) } else { Value::Str(n.to_string()) })
    }, true, false);
    l.filter("naturaltime", |v, _, _| match v {
        Value::DateTime(d, _) => {
            let now = now_naive();
            let t = timesince_impl(d, &now, 1);
            Ok(Value::Str(if *d <= now { format!("{t} ago") } else { format!("{} from now", timesince_impl(&now, d, 1)) }))
        }
        other => Ok(other.clone()),
    }, false, false);
    let _ = |t: chrono::NaiveTime| t.hour();
    Some(l)
}
