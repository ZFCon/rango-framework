//! D3: template rendering parity with the reference.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rango_core::conf::{self, Settings};
use rango_template::{Context, Engine, Value};
use serde_json::Value as J;

fn conv(j: &J) -> Value {
    match j {
        J::Object(m) if m.len() == 1 => {
            if let Some(d) = m.get("__date__").and_then(J::as_str) {
                return Value::Date(NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap());
            }
            if let Some(d) = m.get("__datetime__").and_then(J::as_str) {
                return Value::DateTime(NaiveDateTime::parse_from_str(d, "%Y-%m-%dT%H:%M:%S").unwrap(), None);
            }
            if let Some(t) = m.get("__time__").and_then(J::as_str) {
                return Value::Time(NaiveTime::parse_from_str(t, "%H:%M:%S").unwrap());
            }
            if let Some(s) = m.get("__safe__").and_then(J::as_str) {
                return Value::Safe(s.to_owned());
            }
            Value::from(j.clone())
        }
        J::Array(a) => Value::List(a.iter().map(conv).collect()),
        J::Object(m) => Value::Map(m.iter().map(|(k, v)| (k.clone(), conv(v))).collect()),
        other => Value::from(other.clone()),
    }
}

#[test]
fn templates_match_rango() {
    conf::configure(Settings { static_url: Some("/static/".into()), use_tz: false, ..Settings::default() }).unwrap();
    let golden: J = serde_json::from_str(include_str!("golden/templates.json")).unwrap();
    let shared: Vec<(String, String)> = golden["shared"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned())).collect();
    let mut failures = Vec::new();
    let cases = golden["cases"].as_array().unwrap();
    for case in cases {
        let tpl = case["template"].as_str().unwrap();
        let mut engine = Engine::new();
        engine.autoescape = case["autoescape"].as_bool().unwrap_or(true);
        let mut tpls: Vec<(&str, &str)> = shared.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        tpls.push(("main", tpl));
        engine.locmem(&tpls);
        let mut ctx = Context::new();
        if let J::Object(m) = &case["context"] {
            for (k, v) in m {
                ctx.set(k, conv(v));
            }
        }
        let got = engine.render_to_string("main", &mut ctx);
        let expected = &case["expected"];
        let ok = match (expected.get("output"), &got) {
            (Some(o), Ok(g)) => o.as_str() == Some(g.as_str()) || (tpl.contains("lorem") && !g.is_empty()),
            (None, Err(_)) => true,
            _ => false,
        };
        if !ok {
            failures.push(format!("{tpl:?} ctx={}\n   expected {expected}\n   got      {got:?}", case["context"]));
        }
    }
    assert!(failures.is_empty(), "{} of {} template cases differ from the reference implementation:\n{}", failures.len(), cases.len(), failures.join("\n"));
}
