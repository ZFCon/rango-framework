//! `core.serializers` (json / jsonl) and the `dumpdata` / `loaddata` commands.
//!
//! Output is the fixture format: `{"model": "app.model", "pk": 1, "fields": {...}}`.
//! XML and YAML are not implemented.

use std::path::PathBuf;

use async_trait::async_trait;
use clap::{Arg, ArgAction};
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_core::management::{Command, CommandContext, CommandError};
use serde_json::{Value as Json, json};

use crate::connection::{Db, Executor};
use crate::models::fields_db::{from_db, prep_param};
use crate::models::meta::{Field, FieldType, ModelMeta, registry};
use crate::models::related;
use crate::value::Value;

fn bad(msg: impl Into<String>) -> Error {
    Error::Other(msg.into().into())
}

fn micros(nanos: u32) -> String {
    if nanos / 1000 == 0 { String::new() } else { format!(".{:06}", nanos / 1000) }
}

pub fn to_json(f: &Field, v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(x) => json!(x),
        Value::Decimal(d) => json!(match f.decimal_places {
            Some(p) => {
                let mut x = *d;
                x.rescale(p);
                x.to_string()
            }
            None => d.to_string(),
        }),
        Value::Str(s) => json!(s),
        Value::Bytes(b) => json!(String::from_utf8_lossy(b)),
        Value::Date(d) => json!(d.to_string()),
        // Python's isoformat(): microseconds, omitted when zero.
        Value::Time(t) => json!(format!("{}{}", t.format("%H:%M:%S"), micros(chrono::Timelike::nanosecond(t)))),
        Value::DateTime(d) => json!(format!("{}{}Z", d.format("%Y-%m-%dT%H:%M:%S"), micros(d.timestamp_subsec_nanos()))),
        Value::NaiveDateTime(d) => json!(format!("{}{}", d.format("%Y-%m-%dT%H:%M:%S"), micros(chrono::Timelike::nanosecond(d)))),
        Value::Duration(d) => json!(rango_core::utils::duration::duration_string(*d)),
        Value::Uuid(u) => json!(u.to_string()),
        Value::Json(j) => {
            if f.ty == FieldType::Json { j.clone() } else { j.clone() }
        }
    }
}

pub fn from_json(f: &Field, j: &Json) -> Result<Value> {
    if j.is_null() {
        return Ok(Value::Null);
    }
    let text = || j.as_str().map(str::to_owned).unwrap_or_else(|| j.to_string());
    Ok(match f.ty {
        FieldType::Boolean => Value::Bool(j.as_bool().ok_or_else(|| bad(format!("{}: expected a boolean", f.name)))?),
        FieldType::Auto | FieldType::BigAuto | FieldType::SmallAuto | FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger | FieldType::PositiveInteger | FieldType::PositiveBigInteger | FieldType::PositiveSmallInteger | FieldType::ForeignKey | FieldType::OneToOne => {
            match j {
                Json::Number(n) => Value::Int(n.as_i64().ok_or_else(|| bad("integer out of range"))?),
                Json::String(s) if s.parse::<i64>().is_ok() => Value::Int(s.parse().expect("checked")),
                other => Value::Str(other.as_str().unwrap_or_default().to_owned()),
            }
        }
        FieldType::Float => Value::Float(j.as_f64().ok_or_else(|| bad(format!("{}: expected a number", f.name)))?),
        FieldType::Decimal => Value::Decimal(text().parse().map_err(|_| bad(format!("{}: invalid decimal", f.name)))?),
        FieldType::Date => Value::Date(rango_core::utils::dateparse::parse_date(&text()).ok().flatten().ok_or_else(|| bad(format!("{}: invalid date", f.name)))?),
        FieldType::Time => Value::Time(rango_core::utils::dateparse::parse_time(&text()).ok().flatten().ok_or_else(|| bad(format!("{}: invalid time", f.name)))?),
        FieldType::DateTime => {
            let p = rango_core::utils::dateparse::parse_datetime(&text()).ok().flatten().ok_or_else(|| bad(format!("{}: invalid datetime", f.name)))?;
            let utc = p.naive - chrono::TimeDelta::seconds(i64::from(p.offset.map_or(0, |o| o.local_minus_utc())));
            if rango_core::conf::settings().use_tz { Value::DateTime(utc.and_utc()) } else { Value::NaiveDateTime(utc) }
        }
        FieldType::Duration => Value::Duration(rango_core::utils::dateparse::parse_duration(&text()).ok_or_else(|| bad(format!("{}: invalid duration", f.name)))?),
        FieldType::Uuid => Value::Uuid(text().parse().map_err(|_| bad(format!("{}: invalid uuid", f.name)))?),
        FieldType::Json => Value::Json(j.clone()),
        _ => Value::Str(text()),
    })
}

/// Models to dump: `app_label` or `app_label.Model` specs (empty = all installed).
pub fn select_models(specs: &[String]) -> Result<Vec<std::sync::Arc<ModelMeta>>> {
    let reg = registry();
    let installed: Vec<String> = rango_core::apps::apps().map(|a| a.get_app_configs().map(|c| c.label.clone()).collect()).unwrap_or_default();
    let mut out: Vec<std::sync::Arc<ModelMeta>> = Vec::new();
    if specs.is_empty() {
        out.extend(reg.models().filter(|m| (installed.is_empty() || installed.contains(&m.app_label)) && !m.abstract_ && m.managed).cloned());
    }
    for s in specs {
        match s.split_once('.') {
            Some(_) => out.push(reg.get(s).ok_or_else(|| bad(format!("Unknown model: {s}")))?.clone()),
            None => {
                let found: Vec<_> = reg.models().filter(|m| m.app_label == *s && !m.abstract_).cloned().collect();
                if found.is_empty() {
                    return Err(bad(format!("No installed app with label '{s}'.")));
                }
                out.extend(found);
            }
        }
    }
    // Auto-created many-to-many through tables are serialized with their owners.
    out.retain(|m| !reg.models().any(|o| o.many_to_many().any(|f| f.rel.as_ref().is_some_and(|r| r.through.is_none()) && format!("{}.{}_{}", o.app_label, o.object_name, f.name).eq_ignore_ascii_case(&m.label()))));
    out.sort_by(|a, b| (&a.app_label, &a.source, &a.object_name).cmp(&(&b.app_label, &b.source, &b.object_name)));
    Ok(out)
}

/// `serialize("json", ...)` of every row of `models` (`dumpdata`).
pub async fn dump(db: &dyn Executor, models: &[std::sync::Arc<ModelMeta>]) -> Result<Vec<Json>> {
    let vendor = db.vendor();
    let mut out = Vec::new();
    for m in models {
        let pk = m.pk().clone();
        let cols: Vec<&Field> = m.concrete_fields().filter(|f| !f.primary_key).collect();
        let sql = format!(
            "SELECT {} FROM {} ORDER BY {}",
            std::iter::once(&pk).chain(cols.iter().copied()).map(|f| vendor.quote(&f.column())).collect::<Vec<_>>().join(", "),
            vendor.quote(&m.db_table),
            vendor.quote(&pk.column())
        );
        for row in db.query(&sql, &[]).await? {
            let pk_val = from_db(vendor, &pk, row.values[0].clone())?;
            let mut fields = serde_json::Map::new();
            for (i, f) in cols.iter().enumerate() {
                fields.insert(f.name.clone(), to_json(f, &from_db(vendor, f, row.values[i + 1].clone())?));
            }
            for f in m.many_to_many() {
                let ids = related::current_ids(db, &m.label(), &f.name, &pk_val).await?;
                fields.insert(f.name.clone(), Json::Array(ids.iter().map(|v| to_json(&pk, v)).collect()));
            }
            out.push(json!({"model": m.label_lower(), "pk": to_json(&pk, &pk_val), "fields": fields}));
        }
    }
    Ok(out)
}

/// Format `objects` as `json` (with `indent`) or `jsonl`.
pub fn format_objects(objects: &[Json], format: &str, indent: Option<usize>) -> Result<String> {
    match format {
        "json" => match indent {
            Some(n) => {
                let mut buf = Vec::new();
                let pad = " ".repeat(n);
                let fmt = serde_json::ser::PrettyFormatter::with_indent(pad.as_bytes());
                let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
                serde::Serialize::serialize(&objects, &mut ser).map_err(|e| Error::Other(Box::new(e)))?;
                Ok(String::from_utf8_lossy(&buf).into_owned())
            }
            None => rango_core::utils::json::dumps_with(&objects, false, false).map_err(|e| Error::Other(Box::new(e))),
        },
        "jsonl" => Ok(objects.iter().map(|o| rango_core::utils::json::dumps_with(o, false, false).unwrap_or_default() + "\n").collect()),
        other => Err(bad(format!("Unknown serialization format: {other}"))),
    }
}

/// Parse a `json` / `jsonl` fixture.
pub fn parse_objects(text: &str, format: &str) -> Result<Vec<Json>> {
    match format {
        "json" => match serde_json::from_str::<Json>(text).map_err(|e| bad(format!("Problem installing fixture: {e}")))? {
            Json::Array(a) => Ok(a),
            _ => Err(bad("Fixture must be a JSON array")),
        },
        "jsonl" => text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).map_err(|e| bad(format!("Problem installing fixture: {e}")))).collect(),
        other => Err(bad(format!("Unknown serialization format: {other}"))),
    }
}

/// Insert or update the objects (what `loaddata` does); returns how many were saved.
pub async fn load_objects(db: &Db, objects: &[Json]) -> Result<usize> {
    let vendor = db.vendor();
    let reg = registry();
    let mut n = 0;
    let objs: Vec<Json> = objects.to_vec();
    for o in &objs {
        let label = o["model"].as_str().ok_or_else(|| bad("Fixture entry without \"model\""))?;
        let m = reg.get(label).ok_or_else(|| bad(format!("Invalid model identifier: '{label}'")))?.clone();
        let pk = m.pk().clone();
        let fields = o["fields"].as_object().ok_or_else(|| bad("Fixture entry without \"fields\""))?;
        let pk_val = if o["pk"].is_null() { Value::Null } else { from_json(&pk, &o["pk"])? };
        let mut cols: Vec<(&Field, Value)> = Vec::new();
        if !pk_val.is_null() {
            cols.push((&pk, pk_val.clone()));
        }
        for f in m.concrete_fields().filter(|f| !f.primary_key) {
            let key = if f.rel.is_some() { &f.name } else { &f.name };
            if let Some(j) = fields.get(key) {
                cols.push((f, from_json(f, j)?));
            }
        }
        let exists = if pk_val.is_null() {
            false
        } else {
            let sql = format!("SELECT 1 FROM {} WHERE {} = %s", vendor.quote(&m.db_table), vendor.quote(&pk.column()));
            !db.query(&sql, &[prep_param(vendor, &pk, pk_val.clone())]).await?.is_empty()
        };
        if exists {
            let sets: Vec<String> = cols.iter().filter(|(f, _)| !f.primary_key).map(|(f, _)| format!("{} = %s", vendor.quote(&f.column()))).collect();
            if !sets.is_empty() {
                let mut params: Vec<Value> = cols.iter().filter(|(f, _)| !f.primary_key).map(|(f, v)| prep_param(vendor, f, v.clone())).collect();
                params.push(prep_param(vendor, &pk, pk_val.clone()));
                db.execute(&format!("UPDATE {} SET {} WHERE {} = %s", vendor.quote(&m.db_table), sets.join(", "), vendor.quote(&pk.column())), &params).await?;
            }
        } else {
            let sql = format!("INSERT INTO {} ({}) VALUES ({})", vendor.quote(&m.db_table), cols.iter().map(|(f, _)| vendor.quote(&f.column())).collect::<Vec<_>>().join(", "), vec!["%s"; cols.len()].join(", "));
            let params: Vec<Value> = cols.iter().map(|(f, v)| prep_param(vendor, f, v.clone())).collect();
            db.execute(&sql, &params).await?;
        }
        for f in m.many_to_many() {
            if let Some(Json::Array(ids)) = fields.get(&f.name) {
                let target = reg.target_of(&m, f)?;
                let vals: Result<Vec<Value>> = ids.iter().map(|j| from_json(target.pk(), j)).collect();
                related::set(db, &m.label(), &f.name, &pk_val, &vals?).await?;
            }
        }
        n += 1;
    }
    Ok(n)
}

/// `dumpdata [app_label[.Model] ...] [--format json|jsonl] [--indent N] [-o FILE] [--database DB]`.
#[derive(Default)]
pub struct DumpData;

#[async_trait]
impl Command for DumpData {
    fn help(&self) -> &str {
        "Output the contents of the database as a fixture of the given format."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("args").num_args(0..))
            .arg(Arg::new("format").long("format").default_value("json"))
            .arg(Arg::new("indent").long("indent"))
            .arg(Arg::new("output").short('o').long("output"))
            .arg(Arg::new("database").long("database").default_value("default"))
            .arg(Arg::new("exclude").short('e').long("exclude").action(ArgAction::Append))
    }
    async fn handle(&self, _ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let specs: Vec<String> = args.get_many::<String>("args").into_iter().flatten().cloned().collect();
        let mut models = select_models(&specs).map_err(|e| CommandError::new(e.to_string()))?;
        for ex in args.get_many::<String>("exclude").into_iter().flatten() {
            models.retain(|m| m.app_label != *ex && !m.label().eq_ignore_ascii_case(ex));
        }
        let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
        let objects = dump(&db, &models).await.map_err(|e| CommandError::new(e.to_string()))?;
        let indent = args.get_one::<String>("indent").and_then(|s| s.parse().ok());
        let text = format_objects(&objects, args.get_one::<String>("format").map_or("json", String::as_str), indent).map_err(|e| CommandError::new(e.to_string()))?;
        if let Some(path) = args.get_one::<String>("output") {
            std::fs::write(path, text).map_err(|e| CommandError::new(e.to_string()))?;
            return Ok(None);
        }
        Ok(Some(text))
    }
}

rango_core::register_command!("rango.core", "dumpdata", DumpData);

/// `loaddata fixture ...`: files, or names looked up in `FIXTURE_DIRS` and `<app>/fixtures`.
#[derive(Default)]
pub struct LoadData;

#[async_trait]
impl Command for LoadData {
    fn help(&self) -> &str {
        "Installs the named fixture(s) in the database."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("args").num_args(1..).required(true)).arg(Arg::new("database").long("database").default_value("default"))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
        let mut dirs: Vec<PathBuf> = rango_core::conf::settings().fixture_dirs.iter().map(PathBuf::from).collect();
        dirs.push(PathBuf::from("."));
        if let Ok(apps) = rango_core::apps::apps() {
            dirs.extend(apps.get_app_configs().filter_map(|c| c.path.as_ref().map(|p| p.join("fixtures"))));
        }
        let mut total = 0;
        for name in args.get_many::<String>("args").into_iter().flatten() {
            let candidates: Vec<PathBuf> = if name.contains('.') { dirs.iter().map(|d| d.join(name)).chain([PathBuf::from(name)]).collect() } else { dirs.iter().flat_map(|d| [d.join(format!("{name}.json")), d.join(format!("{name}.jsonl"))]).collect() };
            let path = candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| CommandError::new(format!("No fixture named '{name}' found.")))?;
            let format = if path.extension().is_some_and(|e| e == "jsonl") { "jsonl" } else { "json" };
            let text = std::fs::read_to_string(&path).map_err(|e| CommandError::new(e.to_string()))?;
            let objects = parse_objects(&text, format).map_err(|e| CommandError::new(e.to_string()))?;
            total += load_objects(&db, &objects).await.map_err(|e| CommandError::new(format!("Problem installing fixture '{}': {e}", path.display())))?;
        }
        ctx.stdout.write(&format!("Installed {total} object(s) from {} fixture(s)", args.get_many::<String>("args").map_or(0, Iterator::count)));
        Ok(None)
    }
}

rango_core::register_command!("rango.core", "loaddata", LoadData);

#[doc(hidden)]
pub fn __link() {
    let _ = (DumpData::default as fn() -> _, LoadData::default as fn() -> _);
}

#[allow(dead_code)]
fn _unused(_: IndexMap<String, String>) {}
