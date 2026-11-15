//! Oracle test: ~150 form cases run through the reference data (tools/gen_forms_golden.py).

use std::sync::Arc;

use rango_core::conf::{self, Settings};
use rango_forms::{Attrs, Choice, Data, FileData, Form, FormSet, Value, Widget, fields::*};
use rango_http::UploadedFile;
use serde_json::{Value as J, json};

fn attrs(j: &J) -> Attrs {
    j.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), rango_forms::Attr::Str(v.as_str().unwrap_or_default().to_owned()))).collect()).unwrap_or_default()
}

fn widget(j: &J) -> Widget {
    let (name, a) = match j {
        J::String(s) => (s.as_str(), Attrs::new()),
        J::Array(v) => (v[0].as_str().unwrap(), attrs(&v[1])),
        _ => unreachable!(),
    };
    let w = match name {
        "Textarea" => Widget::textarea(),
        "PasswordInput" => Widget::password_input(),
        "HiddenInput" => Widget::hidden_input(),
        "RadioSelect" => Widget::radio_select(),
        "CheckboxSelectMultiple" => Widget::checkbox_select_multiple(),
        "DateInput" => Widget::date_input(),
        "MultipleHiddenInput" => Widget::multiple_hidden_input(),
        "SelectDateWidget" => Widget::select_date(None),
        "NullBooleanSelect" => Widget::null_boolean_select(),
        other => panic!("widget {other}"),
    };
    w.with_attrs(a)
}

fn choices(j: &J) -> Vec<Choice> {
    j.as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let k = c[0].as_str().unwrap().to_owned();
            match &c[1] {
                J::Array(items) => Choice::Group(k, items.iter().map(|i| (Value::from(i[0].as_str().unwrap()), i[1].as_str().unwrap().to_owned())).collect()),
                l => Choice::option(k.as_str(), l.as_str().unwrap()),
            }
        })
        .collect()
}

fn value_of(j: &J) -> Value {
    match j {
        J::String(s) => Value::Str(s.clone()),
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => n.as_i64().map_or_else(|| Value::Float(n.as_f64().unwrap()), Value::Int),
        J::Null => Value::None,
        J::Object(o) => {
            let (k, v) = o.iter().next().unwrap();
            let v = v.as_str().unwrap();
            match k.as_str() {
                "date" => Value::Date(v.parse().unwrap()),
                "datetime" => Value::DateTime(chrono::DateTime::parse_from_rfc3339(v).unwrap().to_utc()),
                "time" => Value::Time(v.parse().unwrap()),
                _ => panic!(),
            }
        }
        _ => panic!(),
    }
}

fn build(cls: &str, kw: &serde_json::Map<String, J>) -> Field {
    let mut f = match cls {
        "CharField" => Field::char(),
        "IntegerField" => Field::integer(),
        "FloatField" => Field::float(),
        "DecimalField" => Field::decimal(),
        "DateField" => Field::date(),
        "TimeField" => Field::time(),
        "DateTimeField" => Field::datetime(),
        "DurationField" => Field::duration(),
        "RegexField" => Field::regex(kw["regex"].as_str().unwrap()),
        "EmailField" => Field::email(),
        "URLField" => Field::url(),
        "BooleanField" => Field::boolean(),
        "NullBooleanField" => Field::null_boolean(),
        "ChoiceField" => Field::choice(choices(&kw["choices"])),
        "TypedChoiceField" => Field::typed_choice(choices(&kw["choices"]), |s| s.parse::<i64>().map(Value::Int).map_err(|_| ())),
        "MultipleChoiceField" => Field::multiple_choice(choices(&kw["choices"])),
        "TypedMultipleChoiceField" => Field::typed_multiple_choice(choices(&kw["choices"]), |s| s.parse::<i64>().map(Value::Int).map_err(|_| ())),
        "SlugField" => Field::slug(),
        "UUIDField" => Field::uuid(),
        "JSONField" => Field::json(),
        "GenericIPAddressField" => Field::generic_ip_address(kw.get("protocol").and_then(J::as_str).unwrap_or("both"), false),
        "SplitDateTimeField" => Field::split_datetime(),
        "FileField" => Field::file(),
        other => panic!("field {other}"),
    };
    for (k, v) in kw {
        f = match k.as_str() {
            "max_length" => f.max_length(v.as_u64().unwrap() as usize),
            "min_length" => f.min_length(v.as_u64().unwrap() as usize),
            "strip" => f.strip(v.as_bool().unwrap()),
            "empty_value" => f.empty_value(value_of(v)),
            "required" => f.required(v.as_bool().unwrap()),
            "label" => f.label(v.as_str().unwrap()),
            "help_text" => f.help_text(v.as_str().unwrap()),
            "initial" => f.initial(value_of(v)),
            "disabled" => f.disabled(v.as_bool().unwrap()),
            "show_hidden_initial" => f.show_hidden_initial(v.as_bool().unwrap()),
            "min_value" => f.min_value(if cls == "DecimalField" { Value::Decimal(v.to_string().parse().unwrap()) } else { value_of(v) }),
            "max_value" => f.max_value(if cls == "DecimalField" { Value::Decimal(v.to_string().parse().unwrap()) } else { value_of(v) }),
            "step_size" => f.step_size(v.as_f64().unwrap()),
            "max_digits" => f.max_digits(v.as_u64().unwrap() as u32),
            "decimal_places" => f.decimal_places(v.as_u64().unwrap() as u32),
            "input_formats" => f.input_formats(&v.as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect::<Vec<_>>()),
            "widget" => f.widget(widget(v)),
            "regex" | "choices" | "coerce" | "protocol" => f,
            other => panic!("kwarg {other}"),
        };
    }
    f
}

fn jsonify(v: &Value) -> J {
    match v {
        Value::None | Value::Cleared => J::Null,
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::Str(s) => json!(s),
        Value::Decimal(d) => json!({"decimal": d.to_string()}),
        Value::Date(d) => json!({"date": d.to_string()}),
        Value::Time(t) => json!({"time": t.format("%H:%M:%S").to_string()}),
        Value::DateTime(d) => json!({"datetime": d.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()}),
        Value::Duration(d) => json!({"duration": format!("{:?}", d.num_microseconds().unwrap() as f64 / 1e6)}),
        Value::Uuid(u) => json!({"uuid": u.to_string()}),
        Value::Json(j) => j.clone(),
        Value::List(l) => J::Array(l.iter().map(jsonify).collect()),
        Value::File(f) => json!({"file": f.name}),
        other => json!(other.to_text()),
    }
}

#[test]
fn forms_match_rango() {
    conf::configure(Settings { use_tz: true, time_zone: "UTC".into(), secret_key: "x".into(), forms_urlfield_assume_https: true, static_url: Some("/static/".into()), ..Settings::default() }).unwrap();
    let golden: J = serde_json::from_str(include_str!("golden/forms.json")).unwrap();
    let cases: Vec<J> = golden["forms"].as_array().unwrap().clone();
    let mut failures = Vec::new();
    for c in &cases {
        let mut form = Form::new("F");
        for f in c["fields"].as_array().unwrap() {
            form = form.field(f[0].as_str().unwrap(), build(f[1].as_str().unwrap(), f[2].as_object().unwrap()));
        }
        if let Some(k) = c.get("form").and_then(J::as_object) {
            for (key, v) in k {
                form = match key.as_str() {
                    "prefix" => form.prefix(v.as_str().unwrap()),
                    "auto_id" => form.auto_id(v.as_str()),
                    "label_suffix" => form.label_suffix(v.as_str().unwrap()),
                    "required_css_class" => form.required_css_class(v.as_str().unwrap()),
                    "error_css_class" => form.error_css_class(v.as_str().unwrap()),
                    "field_order" => form.field_order(&v.as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect::<Vec<_>>()),
                    other => panic!("form kwarg {other}"),
                };
            }
        }
        if let Some(i) = c.get("initial").and_then(J::as_object) {
            form = form.initial(i.iter().map(|(k, v)| (k.clone(), value_of(v))));
        }
        if !c["data"].is_null() || c.get("files").is_some() {
            let data = Data::from_lists(c["data"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect())));
            let files = FileData::from_lists(c.get("files").and_then(J::as_object).into_iter().flatten().map(|(k, v)| {
                (k.clone(), v.as_array().unwrap().iter().map(|f| UploadedFile::from_bytes(f[0].as_str().unwrap(), None, f[1].as_str().unwrap().as_bytes().to_vec())).collect())
            }));
            form = form.bound(data, files);
        }
        let form = Arc::new(form);
        let mut got = serde_json::Map::new();
        for o in c["out"].as_array().unwrap() {
            let o = o.as_str().unwrap();
            let v = match o {
                "is_valid" => json!(form.is_valid()),
                "errors" => form.errors().get_json_data(false),
                "cleaned" => J::Object(form.cleaned_data().iter().map(|(k, v)| (k.clone(), jsonify(v))).collect()),
                "changed_data" => json!(form.changed_data()),
                "as_div" => json!(form.as_div()),
                "as_p" => json!(form.as_p()),
                "as_table" => json!(form.as_table()),
                "as_ul" => json!(form.as_ul()),
                o if o.starts_with("bf:") => json!(form.get(&o[3..]).unwrap().render()),
                o if o.starts_with("label:") => json!(form.get(&o[6..]).unwrap().label_tag(None, None, None, None)),
                o if o.starts_with("subwidgets:") => json!(form.get(&o[11..]).unwrap().subwidgets().iter().map(|w| w.tag(true)).collect::<Vec<_>>()),
                other => panic!("out {other}"),
            };
            got.insert(o.to_owned(), v);
        }
        let expected = &c["expected"];
        for (k, v) in &got {
            if v != &expected[k] {
                failures.push(format!("fields={} data={} out={k}\n   expected {}\n   got      {}", c["fields"], c["data"], expected[k], v));
            }
        }
    }
    for c in golden["formsets"].as_array().unwrap() {
        let mut proto = Form::new("TestForm");
        for f in c["fields"].as_array().unwrap() {
            proto = proto.field(f[0].as_str().unwrap(), build(f[1].as_str().unwrap(), f[2].as_object().unwrap()));
        }
        let mut fs = FormSet::new(proto);
        for (k, v) in c["opts"].as_object().unwrap() {
            fs = match k.as_str() {
                "extra" => fs.extra(v.as_u64().unwrap() as usize),
                "min_num" => fs.min_num(v.as_i64().unwrap()),
                "max_num" => fs.max_num(v.as_i64().unwrap()),
                "can_delete" => fs.can_delete(v.as_bool().unwrap()),
                "can_order" => fs.can_order(v.as_bool().unwrap()),
                "validate_min" => fs.validate_min(v.as_bool().unwrap()),
                "validate_max" => fs.validate_max(v.as_bool().unwrap()),
                other => panic!("formset opt {other}"),
            };
        }
        if let Some(p) = c["prefix"].as_str() {
            fs = fs.prefix(p);
        }
        if let Some(init) = c["initial"].as_array() {
            fs = fs.with_initial(init.iter().map(|m| m.as_object().unwrap().iter().map(|(k, v)| (k.clone(), value_of(v))).collect()).collect());
        }
        if let Some(d) = c["data"].as_object() {
            fs = fs.bound(Data::from_lists(d.iter().map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect()))), FileData::new());
        }
        for o in c["out"].as_array().unwrap() {
            let o = o.as_str().unwrap();
            let idx = |forms: Vec<&Form>| json!(forms.iter().map(|f| fs.forms().iter().position(|x| std::ptr::eq(x, *f)).unwrap()).collect::<Vec<_>>());
            let v = match o {
                "is_valid" => json!(fs.is_valid()),
                "total" => json!(fs.total_form_count()),
                "non_form_errors" => json!(fs.non_form_errors().messages()),
                "errors" => J::Array(fs.errors().iter().map(|e| e.get_json_data(false)).collect()),
                "cleaned" => match fs.cleaned_data() {
                    Some(l) => J::Array(l.iter().map(|m| J::Object(m.iter().map(|(k, v)| (k.clone(), jsonify(v))).collect())).collect()),
                    None => J::Null,
                },
                "deleted" => idx(fs.deleted_forms()),
                "ordered" => idx(fs.ordered_forms().unwrap_or_default()),
                "empty_form" => json!(fs.empty_form().as_p()),
                "as_div" => json!(fs.as_div()),
                "as_p" => json!(fs.as_p()),
                "as_table" => json!(fs.as_table()),
                "as_ul" => json!(fs.as_ul()),
                other => panic!("formset out {other}"),
            };
            if v != c["expected"][o] {
                failures.push(format!("formset fields={} opts={} data={} out={o}\n   expected {}\n   got      {}", c["fields"], c["opts"], c["data"], c["expected"][o], v));
            }
        }
    }
    assert!(failures.is_empty(), "{} of {} form outputs differ from the reference implementation:\n{}", failures.len(), cases.len(), failures.iter().take(12).map(|f| f.chars().take(1500).collect::<String>()).collect::<Vec<_>>().join("\n"));
}
