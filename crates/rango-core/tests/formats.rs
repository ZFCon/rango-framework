use rango_core::conf::{self, Settings};
use rango_core::utils::formats::{get_format, number_format, number_grouping};
use rango_core::utils::translation::{activate, deactivate};
use serde_json::Value as J;

#[test]
fn locale_formats_match_rango() {
    conf::configure(Settings { secret_key: "x".into(), ..Settings::default() }).unwrap();
    let g: J = serde_json::from_str(include_str!("golden/formats.json")).unwrap();
    let mut bad = Vec::new();
    for c in g["formats"].as_array().unwrap() {
        let (lang, name) = (c["lang"].as_str().unwrap(), c["name"].as_str().unwrap());
        let got = get_format(name, Some(lang));
        if got != c["value"] {
            bad.push(format!("{lang} {name}: expected {} got {got}", c["value"]));
        }
    }
    for c in g["numbers"].as_array().unwrap() {
        let lang = c["lang"].as_str().unwrap();
        activate(lang);
        let sep = get_format("DECIMAL_SEPARATOR", None).as_str().unwrap().to_owned();
        let tsep = get_format("THOUSAND_SEPARATOR", None).as_str().unwrap().to_owned();
        let got = number_format(c["number"].as_str().unwrap(), &sep, c["pos"].as_u64().map(|p| p as usize), &number_grouping(), &tsep, c["force"].as_bool().unwrap());
        deactivate();
        if got != c["expected"].as_str().unwrap() {
            bad.push(format!("number {lang} {c}: got {got:?}"));
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.iter().take(15).cloned().collect::<Vec<_>>().join("\n"));
}
