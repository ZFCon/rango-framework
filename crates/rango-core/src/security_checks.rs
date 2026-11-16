//! `core.checks.security`: the `check --deploy` security checks.
//!
//! Message texts live in `data/security_checks.json`; only the conditions are code.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use serde::Deserialize;

use crate::checks::{CheckContext, CheckMessage};
use crate::conf::try_settings;

#[derive(Deserialize)]
struct Msg {
    level: u32,
    msg: String,
    hint: Option<String>,
}

static MESSAGES: LazyLock<HashMap<String, Msg>> = LazyLock::new(|| serde_json::from_str(include_str!("../data/security_checks.json")).expect("security_checks.json"));

fn msg(id: &str) -> CheckMessage {
    let m = &MESSAGES[id];
    let mut c = CheckMessage::new(m.level, m.msg.clone());
    c.hint = m.hint.clone();
    c.id = Some(id.to_owned());
    c
}

const REFERRER: [&str; 8] = ["no-referrer", "no-referrer-when-downgrade", "origin", "origin-when-cross-origin", "same-origin", "strict-origin", "strict-origin-when-cross-origin", "unsafe-url"];
const COOP: [&str; 3] = ["same-origin", "same-origin-allow-popups", "unsafe-none"];

fn secret_ok(key: &str) -> bool {
    key.len() >= 50 && key.chars().collect::<BTreeSet<_>>().len() >= 5 && !key.starts_with("rango-insecure-")
}

async fn check(_: &CheckContext) -> Vec<CheckMessage> {
    let Ok(s) = try_settings() else { return vec![] };
    let has = |name: &str| s.middleware.iter().any(|m| m == name);
    let (security_mw, xframe_mw, csrf_mw, session_mw) = (has("rango.middleware.security.SecurityMiddleware"), has("rango.middleware.clickjacking.XFrameOptionsMiddleware"), has("rango.middleware.csrf.CsrfViewMiddleware"), has("rango.contrib.sessions.middleware.SessionMiddleware"));
    let session_app = s.installed_apps.iter().any(|a| a == "rango.contrib.sessions");
    let mut out = Vec::new();
    if !security_mw {
        out.push(msg("security.W001"));
    }
    if !xframe_mw {
        out.push(msg("security.W002"));
    }
    if !csrf_mw {
        out.push(msg("security.W003"));
    }
    if security_mw && s.secure_hsts_seconds == 0 {
        out.push(msg("security.W004"));
    }
    if security_mw && s.secure_hsts_seconds != 0 && !s.secure_hsts_include_subdomains {
        out.push(msg("security.W005"));
    }
    if security_mw && !s.secure_content_type_nosniff {
        out.push(msg("security.W006"));
    }
    if security_mw && !s.secure_ssl_redirect {
        out.push(msg("security.W008"));
    }
    if !secret_ok(s.secret_key().unwrap_or("")) {
        out.push(msg("security.W009"));
    }
    // sessions
    if !s.session_cookie_secure {
        let mut e = Vec::new();
        if session_app {
            e.push("security.W010");
        }
        if session_mw {
            e.push("security.W011");
        }
        if e.len() > 1 {
            e = vec!["security.W012"];
        }
        out.extend(e.into_iter().map(msg));
    }
    if !s.session_cookie_httponly {
        let mut e = Vec::new();
        if session_app {
            e.push("security.W013");
        }
        if session_mw {
            e.push("security.W014");
        }
        if e.len() > 1 {
            e = vec!["security.W015"];
        }
        out.extend(e.into_iter().map(msg));
    }
    if !(s.csrf_use_sessions || !csrf_mw || s.csrf_cookie_secure) {
        out.push(msg("security.W016"));
    }
    if s.debug {
        out.push(msg("security.W018"));
    }
    if xframe_mw && s.x_frame_options != "DENY" {
        out.push(msg("security.W019"));
    }
    if !s.debug && s.allowed_hosts.is_empty() {
        out.push(msg("security.W020"));
    }
    if security_mw && s.secure_hsts_seconds != 0 && !s.secure_hsts_preload {
        out.push(msg("security.W021"));
    }
    if security_mw && s.secure_referrer_policy.is_empty() {
        out.push(msg("security.W022"));
    } else if !s.secure_referrer_policy.is_empty() && !s.secure_referrer_policy.split(',').map(|p| p.trim()).all(|p| REFERRER.contains(&p)) {
        out.push(msg("security.E023"));
    }
    if security_mw && !COOP.contains(&s.secure_cross_origin_opener_policy.as_str()) {
        out.push(msg("security.E024"));
    }
    if s.secret_key_fallbacks.iter().any(|k| !secret_ok(k)) {
        out.push(msg("security.W025"));
    }
    out
}

crate::register_check!(tags = ["security"], deploy = true, check);
