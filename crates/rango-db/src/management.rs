//! `sqlflush`, `flush`, `dbshell` and `inspectdb`.

use async_trait::async_trait;
use clap::{Arg, ArgAction};
use rango_core::conf::settings;
use rango_core::management::{Command, CommandContext, CommandError};

use crate::backends::Vendor;
use crate::connection::{Db, Executor};
use crate::models::meta::{FieldType, registry};
use crate::value::Value;

fn err(e: impl ToString) -> CommandError {
    CommandError::new(e.to_string())
}

/// Tables of the installed models (and their auto-created many-to-many tables), sorted.
pub fn model_tables() -> Vec<String> {
    let installed: Vec<String> = rango_core::apps::apps().map(|a| a.get_app_configs().map(|c| c.label.clone()).collect()).unwrap_or_default();
    let mut t: Vec<String> = registry().models().filter(|m| m.managed && !m.abstract_ && m.proxy_for.is_none() && (installed.is_empty() || installed.contains(&m.app_label))).map(|m| m.db_table.clone()).collect();
    t.sort();
    t.dedup();
    t
}

/// `connection.ops.sql_flush(...)`.
pub fn sql_flush(vendor: Vendor, tables: &[String]) -> Vec<String> {
    match vendor {
        Vendor::Sqlite => {
            let mut out: Vec<String> = tables.iter().map(|t| format!("DELETE FROM {};", vendor.quote(t))).collect();
            if !tables.is_empty() {
                out.push(format!("DELETE FROM \"sqlite_sequence\" WHERE \"name\" IN ({});", tables.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ")));
            }
            out
        }
        Vendor::Postgresql => {
            if tables.is_empty() { vec![] } else { vec![format!("TRUNCATE {} RESTART IDENTITY CASCADE;", tables.iter().map(|t| vendor.quote(t)).collect::<Vec<_>>().join(", "))] }
        }
    }
}

#[derive(Default)]
pub struct SqlFlush;

#[async_trait]
impl Command for SqlFlush {
    fn help(&self) -> &str {
        "Returns a list of the SQL statements required to return all tables in the database to the state they were in just after they were installed."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("database").long("database").default_value("default"))
    }
    async fn handle(&self, _ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
        Ok(Some(sql_flush(db.vendor(), &model_tables()).join("\n")))
    }
}

rango_core::register_command!("rango.core", "sqlflush", SqlFlush);

/// `flush --noinput`: empty every model table, then re-run `post_migrate` (content types, permissions).
#[derive(Default)]
pub struct Flush;

#[async_trait]
impl Command for Flush {
    fn help(&self) -> &str {
        "Removes ALL DATA from the database, including data added during migrations."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("noinput").long("noinput").visible_alias("no-input").action(ArgAction::SetTrue)).arg(Arg::new("database").long("database").default_value("default"))
    }
    async fn handle(&self, _ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        if !args.get_flag("noinput") {
            return Err(CommandError::new("flush deletes all data; pass --noinput to confirm."));
        }
        let alias = args.get_one::<String>("database").map_or("default", String::as_str).to_owned();
        let db = Db::using(&alias);
        let stmts = sql_flush(db.vendor(), &model_tables());
        db.atomic(|tx| {
            Box::pin(async move {
                for s in stmts {
                    tx.execute_batch(&s).await?;
                }
                Ok(())
            })
        })
        .await
        .map_err(err)?;
        let loader = crate::migrations::MigrationLoader::load(None).map_err(err)?;
        let mut apps: Vec<String> = Vec::new();
        for (a, _) in &loader.order {
            if !apps.contains(a) {
                apps.push(a.clone());
            }
        }
        for app in apps {
            crate::migrations::executor::POST_MIGRATE.send(Some(&app), crate::migrations::executor::MigrateEvent { app_label: app.clone(), using: alias.clone() }).await;
        }
        Ok(None)
    }
}

rango_core::register_command!("rango.core", "flush", Flush);

/// `dbshell`: run the database's command-line client.
#[derive(Default)]
pub struct DbShell;

#[async_trait]
impl Command for DbShell {
    fn help(&self) -> &str {
        "Runs the command-line client for the specified database."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("database").long("database").default_value("default"))
    }
    async fn handle(&self, _ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let alias = args.get_one::<String>("database").map_or("default", String::as_str);
        let s = settings();
        let cfg = s.databases.get(alias).ok_or_else(|| CommandError::new(format!("The connection '{alias}' doesn't exist.")))?;
        let status = match Vendor::from_engine(&cfg.engine).map_err(err)? {
            Vendor::Sqlite => std::process::Command::new("sqlite3").arg(&cfg.name).status(),
            Vendor::Postgresql => {
                let mut c = std::process::Command::new("psql");
                if !cfg.user.is_empty() {
                    c.args(["-U", &cfg.user]);
                }
                if !cfg.host.is_empty() {
                    c.args(["-h", &cfg.host]);
                }
                if !cfg.port.is_empty() {
                    c.args(["-p", &cfg.port]);
                }
                if !cfg.password.is_empty() {
                    c.env("PGPASSWORD", &cfg.password);
                }
                c.arg(&cfg.name).status()
            }
        };
        match status {
            Ok(s) if s.success() => Ok(None),
            Ok(s) => Err(CommandError::new(format!("dbshell exited with {s}"))),
            Err(e) => Err(CommandError::new(format!("You appear not to have the database client installed ({e}).")))
        }
    }
}

rango_core::register_command!("rango.core", "dbshell", DbShell);

// ----------------------------------------------------------------------------- inspectdb

struct Col {
    name: String,
    sql_type: String,
    null: bool,
    pk: bool,
    fk: Option<(String, String)>,
}

fn rust_field(c: &Col) -> (String, String) {
    let t = c.sql_type.to_lowercase();
    let (len, base) = match t.split_once('(') {
        Some((b, rest)) => (rest.trim_end_matches(')').split(',').next().and_then(|n| n.trim().parse::<u32>().ok()), b.trim().to_owned()),
        None => (None, t.clone()),
    };
    let (ty, attr) = match base.as_str() {
        b if b.contains("int") && b.contains("big") => ("i64", None),
        b if b.contains("int") && b.contains("small") => ("i16", None),
        b if b.contains("int") || b == "serial" => ("i32", None),
        "bool" | "boolean" => ("bool", None),
        "real" | "float" | "double" | "double precision" => ("f64", None),
        "numeric" | "decimal" => ("rust_decimal::Decimal", None),
        "date" => ("chrono::NaiveDate", None),
        "time" | "time without time zone" => ("chrono::NaiveTime", None),
        b if b.starts_with("timestamp") || b == "datetime" => ("chrono::DateTime<chrono::Utc>", None),
        "uuid" => ("uuid::Uuid", None),
        "json" | "jsonb" => ("serde_json::Value", None),
        "text" | "clob" => ("String", None),
        b if b.contains("char") || b == "varchar" => ("String", len.map(|n| format!("max_length = {n}"))),
        _ => ("String", None),
    };
    let ty = if c.null && !c.pk { format!("Option<{ty}>") } else { ty.to_owned() };
    let mut attrs: Vec<String> = Vec::new();
    if c.pk {
        attrs.push("primary_key".into());
    }
    if let Some(a) = attr {
        attrs.push(a);
    }
    if c.null {
        attrs.push("null".into());
        attrs.push("blank".into());
    }
    (ty, attrs.join(", "))
}

fn camel(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).map(|p| p.chars().next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + &p[f.len_utf8()..])).collect()
}

async fn tables_and_columns(db: &Db) -> crate::Result<Vec<(String, Vec<Col>)>> {
    let mut out = Vec::new();
    match db.vendor() {
        Vendor::Sqlite => {
            let names: Vec<String> = db.query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name", &[]).await?.into_iter().filter_map(|r| if let Some(Value::Str(s)) = r.values.first() { Some(s.clone()) } else { None }).collect();
            for t in names {
                let fks: std::collections::HashMap<String, (String, String)> = db.query(&format!("PRAGMA foreign_key_list(\"{t}\")"), &[]).await?.into_iter().filter_map(|r| match r.values.as_slice() {
                    [_, _, Value::Str(tt), Value::Str(from), Value::Str(to), ..] => Some((from.clone(), (tt.clone(), to.clone()))),
                    _ => None,
                }).collect();
                let cols = db.query(&format!("PRAGMA table_info(\"{t}\")"), &[]).await?.into_iter().filter_map(|r| match r.values.as_slice() {
                    [_, Value::Str(name), Value::Str(ty), Value::Int(notnull), _, Value::Int(pk)] => Some(Col { fk: fks.get(name).cloned(), name: name.clone(), sql_type: ty.clone(), null: *notnull == 0, pk: *pk > 0 }),
                    _ => None,
                }).collect();
                out.push((t, cols));
            }
        }
        Vendor::Postgresql => {
            let names: Vec<String> = db.query("SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' ORDER BY table_name", &[]).await?.into_iter().filter_map(|r| if let Some(Value::Str(s)) = r.values.first() { Some(s.clone()) } else { None }).collect();
            for t in names {
                let pks: Vec<String> = db.query("SELECT kcu.column_name FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name WHERE tc.table_name = %s AND tc.constraint_type = 'PRIMARY KEY'", &[Value::Str(t.clone())]).await?.into_iter().filter_map(|r| if let Some(Value::Str(s)) = r.values.first() { Some(s.clone()) } else { None }).collect();
                let cols = db.query("SELECT column_name, data_type, is_nullable, character_maximum_length FROM information_schema.columns WHERE table_name = %s AND table_schema = current_schema() ORDER BY ordinal_position", &[Value::Str(t.clone())]).await?.into_iter().filter_map(|r| match r.values.as_slice() {
                    [Value::Str(n), Value::Str(ty), Value::Str(nl), len] => Some(Col { fk: None, name: n.clone(), sql_type: match len { Value::Int(l) => format!("{ty}({l})"), _ => ty.clone() }, null: nl == "YES", pk: pks.contains(n) }),
                    _ => None,
                }).collect();
                out.push((t, cols));
            }
        }
    }
    Ok(out)
}

/// `inspectdb`: Rust model source for the tables of `db` (all, or only `only`).
pub async fn inspect(db: &Db, app: &str, only: &[String]) -> crate::Result<String> {
    let mut out = String::from("// Generated by `inspectdb`. Review field types and relationships before use.\nuse rango::db::models::Model;\n");
    for (table, cols) in tables_and_columns(db).await? {
        if !only.is_empty() && !only.contains(&table) {
            continue;
        }
        out.push_str(&format!("\n#[derive(Model)]\n#[model(app_label = {app:?}, db_table = {table:?}, managed = false)]\npub struct {} {{\n", camel(&table)));
        for c in &cols {
            let (mut ty, mut attrs) = rust_field(c);
            if let Some((target, _)) = &c.fk {
                ty = if c.null { format!("Option<rango::db::models::ForeignKey<{}>>", camel(target)) } else { format!("rango::db::models::ForeignKey<{}>", camel(target)) };
                attrs.clear();
            }
            let fname = c.name.strip_suffix("_id").filter(|_| c.fk.is_some()).unwrap_or(&c.name);
            out.push_str(&format!("    {}pub {fname}: {ty},\n", if attrs.is_empty() { String::new() } else { format!("#[field({attrs})] ") }));
        }
        out.push_str("}\n");
    }
    Ok(out)
}

#[derive(Default)]
pub struct InspectDb;

#[async_trait]
impl Command for InspectDb {
    fn help(&self) -> &str {
        "Introspects the database tables in the given database and outputs a Rango model module."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("table").num_args(0..)).arg(Arg::new("database").long("database").default_value("default")).arg(Arg::new("app_label").long("app-label").default_value("app"))
    }
    async fn handle(&self, _ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
        let only: Vec<String> = args.get_many::<String>("table").into_iter().flatten().cloned().collect();
        let app = args.get_one::<String>("app_label").map_or("app", String::as_str);
        let out = inspect(&db, app, &only).await.map_err(err)?;
        Ok(Some(out))
    }
}

rango_core::register_command!("rango.core", "inspectdb", InspectDb);

#[doc(hidden)]
pub fn __link() {
    let _ = (SqlFlush::default as fn() -> _, Flush::default as fn() -> _, DbShell::default as fn() -> _, InspectDb::default as fn() -> _);
}

#[allow(dead_code)]
fn _t(_: FieldType) {}
