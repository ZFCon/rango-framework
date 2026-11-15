//! D1-style parity test for the core runtime: replays ~500 cases recorded from the reference data
//! (see `tools/gen_core_golden.py`) against rango-core and reports every mismatch at once.
//! Also checks the default settings table against the `global_settings`.

use std::collections::HashSet;
use std::time::Duration;

use rango_core::conf::{self, Settings};
use rango_core::exceptions::ValidationError;
use rango_core::paginator::{PageItem, Paginator};
use rango_core::signing::{self, Signer, SignerOptions, TimestampSigner};
use rango_core::utils::crypto::{self, HashAlgorithm};
use rango_core::utils::{dateparse, duration, encoding, html, http, ipv6, text};
use rango_core::validators::{self as v, Validator};
use serde_json::{Value, json};

type Out = Result<Value, String>;

fn s(a: &[Value], i: usize) -> &str {
    a[i].as_str()
        .unwrap_or_else(|| panic!("arg {i} is not a string: {:?}", a[i]))
}
fn opt_s(a: &[Value], i: usize) -> Option<&str> {
    a[i].as_str()
}
fn n(a: &[Value], i: usize) -> i64 {
    a[i].as_i64()
        .unwrap_or_else(|| a[i].as_f64().expect("number") as i64)
}
fn f(a: &[Value], i: usize) -> f64 {
    a[i].as_f64().expect("number")
}
fn b(a: &[Value], i: usize) -> bool {
    a[i].as_bool().expect("bool")
}
fn strings(a: &[Value], i: usize) -> Vec<String> {
    a[i].as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_str().expect("str").to_owned())
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}
fn ok(v: impl Into<Value>) -> Out {
    Ok(v.into())
}

fn signer_opts(a: &[Value], key: usize, salt: usize) -> SignerOptions {
    SignerOptions {
        key: opt_s(a, key).map(str::to_owned),
        salt: opt_s(a, salt).map(str::to_owned),
        ..SignerOptions::default()
    }
}

fn vmsg(r: Result<(), ValidationError>) -> Out {
    match r {
        Ok(()) => ok("ok"),
        Err(err) => Ok(json!({
            "error": "ValidationError",
            "messages": err.messages(),
            "codes": err.error_list().iter().map(|x| x.code().map_or(Value::Null, |c| json!(c))).collect::<Vec<_>>(),
        })),
    }
}

fn iso_time(t: chrono::NaiveTime) -> String {
    use chrono::Timelike;
    let base = t.format("%H:%M:%S").to_string();
    match t.nanosecond() / 1000 {
        0 => base,
        us => format!("{base}.{us:06}"),
    }
}

fn iso_dt(p: dateparse::ParsedDateTime) -> String {
    let date = p.naive.date().format("%Y-%m-%d");
    let tz = p.offset.map_or(String::new(), |o| {
        let secs = o.local_minus_utc();
        let sign = if secs < 0 { '-' } else { '+' };
        format!(
            "{sign}{:02}:{:02}",
            secs.abs() / 3600,
            (secs.abs() / 60) % 60
        )
    });
    format!("{date}T{}{tz}", iso_time(p.naive.time()))
}

fn td(d: chrono::TimeDelta) -> Value {
    let us = d.num_microseconds().expect("fits");
    let day = 86_400_000_000i64;
    json!({ "days": us.div_euclid(day), "seconds": us.rem_euclid(day) / 1_000_000, "microseconds": us.rem_euclid(1_000_000) })
}

fn paginate(a: &[Value]) -> Out {
    let objs: Vec<i64> = a[0]
        .as_array()
        .expect("list")
        .iter()
        .map(|x| x.as_i64().expect("int"))
        .collect();
    let (per, orph, empty, number) = (n(a, 1) as usize, n(a, 2) as usize, b(a, 3), n(a, 4));
    let (side, ends) = (n(a, 5) as usize, n(a, 6) as usize);
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(e)?;
    rt.block_on(async {
        let p = Paginator::new(objs, per).orphans(orph).allow_empty_first_page(empty);
        let mut out = json!({ "count": p.count().await.map_err(e)?, "num_pages": p.num_pages().await.map_err(e)? });
        out["page"] = match p.page(number).await.map_err(e)? {
            Ok(pg) => json!({
                "items": pg.object_list, "start": pg.start_index(), "end": pg.end_index(),
                "has_next": pg.has_next(), "has_prev": pg.has_previous()
            }),
            Err(rango_core::paginator::InvalidPage::EmptyPage(_)) => json!({ "error": "EmptyPage" }),
            Err(_) => json!({ "error": "PageNotAnInteger" }),
        };
        out["elided"] = match p.get_elided_page_range(number, side, ends).await.map_err(e)? {
            Ok(items) => json!(
                items.iter().map(|i| match i { PageItem::Page(n) => json!(n), PageItem::Ellipsis => json!("…") }).collect::<Vec<_>>()
            ),
            Err(_) => json!({ "error": "EmptyPage" }),
        };
        Ok(out)
    })
}

fn call(name: &str, a: &[Value], expected: &Value) -> Out {
    match name {
        // ---- signing
        "Signer.sign" => {
            let mut o = signer_opts(a, 0, 1);
            o.algorithm = opt_s(a, 2).map(|x| x.parse().expect("algorithm"));
            ok(Signer::new(o).map_err(e)?.sign(s(a, 3)))
        }
        "TimestampSigner.sign" => ok(TimestampSigner::new(signer_opts(a, 0, 1))
            .map_err(e)?
            .sign_at(s(a, 3), n(a, 2))),
        "TimestampSigner.unsign" => {
            let mut o = signer_opts(a, 0, 1);
            o.fallback_keys = a[5].is_array().then(|| strings(a, 5));
            let max_age = a[3].as_f64().map(Duration::from_secs_f64);
            ok(TimestampSigner::new(o)
                .map_err(e)?
                .unsign_at(s(a, 2), max_age, f(a, 4))
                .map_err(e)?)
        }
        "signing.dumps" => {
            let signer = TimestampSigner::new(signer_opts(a, 1, 2)).map_err(e)?;
            let (compress, ts) = (b(a, 3), n(a, 4));
            let mine = signer.sign_object_at(&a[0], compress, ts).map_err(e)?;
            if !compress {
                return ok(mine);
            }
            // zlib output may differ byte-for-byte between implementations: require that each
            // side can read the other's value instead.
            let theirs: Value = signer
                .unsign_object_at(expected.as_str().unwrap_or(""), None, ts as f64)
                .map_err(e)?;
            let back: Value = signer.unsign_object_at(&mine, None, ts as f64).map_err(e)?;
            if theirs == a[0] && back == a[0] {
                Ok(expected.clone())
            } else {
                Err(format!("compressed roundtrip: {theirs} / {back}"))
            }
        }
        "signing.loads" => {
            let signer = TimestampSigner::new(signer_opts(a, 1, 2)).map_err(e)?;
            let max_age = a[3].as_f64().map(Duration::from_secs_f64);
            signer
                .unsign_object_at::<Value>(s(a, 0), max_age, f(a, 4))
                .map_err(e)
        }
        "signing.cookie_sign" => {
            let salt = signing::cookie_signer_salt(s(a, 0), s(a, 1));
            ok(signing::get_cookie_signer(&salt)
                .map_err(e)?
                .sign_at(s(a, 2), n(a, 3)))
        }
        "b62_encode" => ok(signing::b62_encode(n(a, 0))),
        "b62_decode" => signing::b62_decode(s(a, 0))
            .map(Value::from)
            .ok_or_else(|| "invalid".into()),
        // ---- crypto
        "salted_hmac" => {
            let alg: HashAlgorithm = s(a, 3).parse().map_err(e)?;
            ok(hex(&crypto::salted_hmac(
                s(a, 0).as_bytes(),
                s(a, 1).as_bytes(),
                s(a, 2).as_bytes(),
                alg,
            )))
        }
        "pbkdf2" => {
            let alg: HashAlgorithm = s(a, 4).parse().map_err(e)?;
            ok(hex(&crypto::pbkdf2(
                s(a, 0).as_bytes(),
                s(a, 1).as_bytes(),
                n(a, 2) as u32,
                n(a, 3) as usize,
                alg,
            )))
        }
        "constant_time_compare" => ok(crypto::constant_time_compare(s(a, 0), s(a, 1))),
        // ---- text
        "capfirst" => ok(text::capfirst(s(a, 0))),
        "slugify" => ok(text::slugify(s(a, 0), b(a, 1))),
        "get_valid_filename" => text::get_valid_filename(s(a, 0))
            .map(Value::from)
            .map_err(e),
        "camel_case_to_spaces" => ok(text::camel_case_to_spaces(s(a, 0))),
        "get_text_list" => ok(text::get_text_list(&strings(a, 0), opt_s(a, 1))),
        "phone2numeric" => ok(text::phone2numeric(s(a, 0))),
        "smart_split" => ok(json!(text::smart_split(s(a, 0)))),
        "unescape_string_literal" => text::unescape_string_literal(s(a, 0))
            .map(Value::from)
            .ok_or_else(|| "not a literal".into()),
        "wrap" => ok(text::wrap(s(a, 0), n(a, 1) as usize)),
        "normalize_newlines" => ok(text::normalize_newlines(s(a, 0))),
        "Truncator.chars" => ok(text::Truncator::new(s(a, 0)).chars(n(a, 1), opt_s(a, 2))),
        "Truncator.words" => ok(text::Truncator::new(s(a, 0)).words(n(a, 1), opt_s(a, 2))),
        "trim_value" => {
            let (v, suffix) = text::trim_value(s(a, 0), n(a, 1) as usize);
            ok(json!([v, suffix]))
        }
        // ---- html
        "escape" => ok(html::escape(s(a, 0)).into_string()),
        "escapejs" => ok(html::escapejs(s(a, 0)).into_string()),
        "json_script" => html::json_script(&a[0], opt_s(a, 1))
            .map(|x| Value::from(x.into_string()))
            .map_err(e),
        "linebreaks" => ok(html::linebreaks(s(a, 0), false)),
        "strip_tags" => html::strip_tags(s(a, 0)).map(Value::from).map_err(e),
        "strip_spaces_between_tags" => ok(html::strip_spaces_between_tags(s(a, 0))),
        "avoid_wrapping" => ok(html::avoid_wrapping(s(a, 0))),
        // ---- http
        "urlencode" => {
            let pairs: Vec<(String, String)> = a[0]
                .as_array()
                .expect("pairs")
                .iter()
                .map(|p| (p[0].as_str().unwrap().into(), p[1].as_str().unwrap().into()))
                .collect();
            ok(http::urlencode(pairs))
        }
        "http_date" => ok(http::http_date(Some(f(a, 0)))),
        "parse_http_date" => {
            Ok(http::parse_http_date_safe(s(a, 0)).map_or(Value::Null, Value::from))
        }
        "int_to_base36" => ok(http::int_to_base36(n(a, 0) as u64)),
        "base36_to_int" => http::base36_to_int(s(a, 0)).map(Value::from),
        "urlsafe_base64_encode" => ok(http::urlsafe_base64_encode(s(a, 0).as_bytes())),
        "urlsafe_base64_decode" => {
            let bytes = http::urlsafe_base64_decode(s(a, 0)).map_err(e)?;
            String::from_utf8(bytes).map(Value::from).map_err(e)
        }
        "parse_etags" => ok(json!(http::parse_etags(s(a, 0)))),
        "quote_etag" => ok(http::quote_etag(s(a, 0))),
        "is_same_domain" => ok(http::is_same_domain(s(a, 0), s(a, 1))),
        "url_has_allowed_host_and_scheme" => {
            let hosts: HashSet<String> = strings(a, 1).into_iter().collect();
            ok(http::url_has_allowed_host_and_scheme(
                s(a, 0),
                &hosts,
                b(a, 2),
            ))
        }
        "escape_leading_slashes" => ok(http::escape_leading_slashes(s(a, 0))),
        "parse_header_parameters" => {
            let (key, mut params) = http::parse_header_parameters(s(a, 0))?;
            params.sort();
            ok(json!([key, params]))
        }
        "content_disposition_header" => {
            Ok(http::content_disposition_header(b(a, 0), opt_s(a, 1))
                .map_or(Value::Null, Value::from))
        }
        // ---- encoding
        "iri_to_uri" => ok(encoding::iri_to_uri(s(a, 0))),
        "uri_to_iri" => ok(encoding::uri_to_iri(s(a, 0))),
        "escape_uri_path" => ok(encoding::escape_uri_path(s(a, 0))),
        "filepath_to_uri" => ok(encoding::filepath_to_uri(s(a, 0))),
        // ---- ipv6
        "clean_ipv6_address" => {
            ipv6::clean_ipv6_address(s(a, 0), b(a, 1), None, ipv6::MAX_IPV6_ADDRESS_LENGTH)
                .map(Value::from)
                .map_err(e)
        }
        "is_valid_ipv6_address" => ok(ipv6::is_valid_ipv6_address(s(a, 0))),
        // ---- dateparse / duration
        "parse_date" => dateparse::parse_date(s(a, 0))
            .map(|d| d.map_or(Value::Null, |d| d.format("%Y-%m-%d").to_string().into()))
            .map_err(e),
        "parse_time" => dateparse::parse_time(s(a, 0))
            .map(|t| t.map_or(Value::Null, |t| iso_time(t).into()))
            .map_err(e),
        "parse_datetime" => dateparse::parse_datetime(s(a, 0))
            .map(|d| d.map_or(Value::Null, |d| iso_dt(d).into()))
            .map_err(e),
        "parse_duration" => Ok(dateparse::parse_duration(s(a, 0)).map_or(Value::Null, td)),
        "duration_string" | "duration_iso_string" => {
            let d = chrono::TimeDelta::days(n(a, 0))
                + chrono::TimeDelta::seconds(n(a, 1))
                + chrono::TimeDelta::microseconds(n(a, 2));
            ok(if name == "duration_string" {
                duration::duration_string(d)
            } else {
                duration::duration_iso_string(d)
            })
        }
        "paginate" => paginate(a),
        "normalize" => rango_core::utils::regex_helper::normalize(s(a, 0)).map(|v| json!(v.into_iter().map(|(f, p)| json!([f, p])).collect::<Vec<_>>())),
        "urljoin" => ok(http::urljoin(s(a, 0), s(a, 1))),
        // ---- validators
        "validate_email" => vmsg(v::validate_email(s(a, 0))),
        "validate_slug" => vmsg(v::validate_slug(s(a, 0))),
        "validate_unicode_slug" => vmsg(v::validate_unicode_slug(s(a, 0))),
        "validate_ipv4_address" => vmsg(v::validate_ipv4_address(s(a, 0))),
        "validate_ipv6_address" => vmsg(v::validate_ipv6_address(s(a, 0))),
        "validate_ipv46_address" => vmsg(v::validate_ipv46_address(s(a, 0))),
        "validate_integer" => vmsg(v::validate_integer(s(a, 0))),
        "validate_comma_separated_integer_list" => {
            vmsg(v::validate_comma_separated_integer_list(s(a, 0)))
        }
        "validate_domain_name" => vmsg(v::validate_domain_name(s(a, 0))),
        "URLValidator" => vmsg(v::UrlValidator::new().map_err(e)?.validate(s(a, 0))),
        "MaxValueValidator" | "MinValueValidator" => {
            let max = name == "MaxValueValidator";
            if a[0].is_i64() && a[1].is_i64() {
                let (l, x) = (n(a, 0), n(a, 1));
                vmsg(if max {
                    v::MaxValueValidator::new(l).validate(&x)
                } else {
                    v::MinValueValidator::new(l).validate(&x)
                })
            } else {
                let (l, x) = (f(a, 0), f(a, 1));
                vmsg(if max {
                    v::MaxValueValidator::new(l).validate(&x)
                } else {
                    v::MinValueValidator::new(l).validate(&x)
                })
            }
        }
        "MinLengthValidator" => {
            vmsg(v::MinLengthValidator::new(n(a, 0) as usize).validate(s(a, 1)))
        }
        "MaxLengthValidator" => {
            vmsg(v::MaxLengthValidator::new(n(a, 0) as usize).validate(s(a, 1)))
        }
        "StepValueValidator" => vmsg(v::StepValueValidator::new(f(a, 0)).validate(&f(a, 1))),
        "ProhibitNullCharactersValidator" => {
            vmsg(v::ProhibitNullCharactersValidator::default().validate(s(a, 0)))
        }
        "FileExtensionValidator" => {
            let exts = strings(a, 0);
            let refs: Vec<&str> = exts.iter().map(String::as_str).collect();
            vmsg(v::FileExtensionValidator::new(Some(&refs)).validate(s(a, 1)))
        }
        other => Err(format!("no Rust mapping for oracle function {other:?}")),
    }
}

fn numbers_equal(x: &Value, y: &Value) -> bool {
    match (x, y) {
        (Value::Number(p), Value::Number(q)) => p.as_f64() == q.as_f64(),
        (Value::Array(p), Value::Array(q)) => {
            p.len() == q.len() && p.iter().zip(q).all(|(a, b)| numbers_equal(a, b))
        }
        (Value::Object(p), Value::Object(q)) => {
            p.len() == q.len()
                && p.iter()
                    .all(|(k, a)| q.get(k).is_some_and(|b| numbers_equal(a, b)))
        }
        _ => x == y,
    }
}

#[test]
fn core_matches_rango() {
    conf::configure(Settings {
        secret_key: "test-secret-key".into(),
        secret_key_fallbacks: vec!["old-secret-key".into()],
        use_i18n: false,
        time_zone: "UTC".into(),
        ..Settings::default()
    })
    .expect("configure");

    let golden: Vec<Value> =
        serde_json::from_str(include_str!("golden/core.json")).expect("golden json");
    let mut failures = Vec::new();
    for case in &golden {
        let name = case["fn"].as_str().expect("fn");
        let args = case["args"].as_array().expect("args");
        let expected = &case["expected"];
        let expects_error = expected.get("error").is_some_and(|e| e.is_string())
            && expected.get("messages").is_none();
        let actual = call(name, args, expected);
        let pass = match (&actual, expects_error) {
            (Err(_), true) => true,
            (Ok(got), false) => numbers_equal(got, expected),
            _ => false,
        };
        if !pass {
            failures.push(format!(
                "{name}({}) expected {expected}, got {actual:?}",
                Value::Array(args.clone())
            ));
        }
    }

    // Default settings: same names, same values as the global_settings (rebranded dotted paths).
    let defaults: Value =
        serde_json::from_str(include_str!("golden/global_settings.json")).expect("settings json");
    let ours = serde_json::to_value(Settings::default()).expect("serialize settings");
    for (k, want) in defaults.as_object().expect("object") {
        match ours.get(k) {
            Some(got) if numbers_equal(got, want) => {}
            got => failures.push(format!("setting {k}: expected {want}, got {got:?}")),
        }
    }
    let extra: Vec<_> = ours
        .as_object()
        .expect("object")
        .keys()
        .filter(|k| defaults.get(k.as_str()).is_none())
        .collect();
    if !extra.is_empty() {
        failures.push(format!("settings not in the reference implementation: {extra:?}"));
    }

    assert!(
        failures.is_empty(),
        "{} of {} oracle cases differ from the reference implementation:\n{}",
        failures.len(),
        golden.len() + defaults.as_object().map_or(0, |m| m.len()),
        failures.join("\n")
    );
}
