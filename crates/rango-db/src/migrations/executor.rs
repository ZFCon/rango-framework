//! Migration graph, loader, recorder and executor (`db.migrations.{loader,recorder,executor}`).

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::backends::Vendor;
use crate::connection::{Db, Executor};
use crate::migrations::operations::Operation;
use crate::migrations::schema::SchemaEditor;
use crate::migrations::state::ProjectState;
use crate::value::Value;

/// Payload of [`PRE_MIGRATE`] / [`POST_MIGRATE`].
#[derive(Clone, Debug)]
pub struct MigrateEvent {
    pub app_label: String,
    pub using: String,
}

/// `db.models.signals.pre_migrate` (sender: app label).
pub static PRE_MIGRATE: rango_core::dispatch::Signal<MigrateEvent> = rango_core::dispatch::Signal::new();
/// `db.models.signals.post_migrate` (sender: app label).
pub static POST_MIGRATE: rango_core::dispatch::Signal<MigrateEvent> = rango_core::dispatch::Signal::new();

/// A migration (`db.migrations.Migration`).
#[derive(Clone, Debug)]
pub struct Migration {
    pub app_label: String,
    pub name: String,
    pub dependencies: Vec<(String, String)>,
    pub operations: Vec<Operation>,
    pub initial: bool,
    pub replaces: Vec<(String, String)>,
}

impl Migration {
    pub fn new(app_label: &str, name: &str) -> Self {
        Migration { app_label: app_label.to_owned(), name: name.to_owned(), dependencies: vec![], operations: vec![], initial: false, replaces: vec![] }
    }
    pub fn initial(mut self) -> Self {
        self.initial = true;
        self
    }
    pub fn dependency(mut self, app: &str, name: &str) -> Self {
        self.dependencies.push((app.to_owned(), name.to_owned()));
        self
    }
    pub fn operation(mut self, op: Operation) -> Self {
        self.operations.push(op);
        self
    }
    pub fn key(&self) -> (String, String) {
        (self.app_label.clone(), self.name.clone())
    }
}

/// Link-time registration of a migration (generated migration files emit this).
pub struct MigrationRegistration {
    pub build: fn() -> Migration,
}

inventory::collect!(MigrationRegistration);

/// `register_migration!(migration)` where `migration: fn() -> Migration`.
#[macro_export]
macro_rules! register_migration {
    ($f:path) => {
        $crate::inventory::submit! { $crate::migrations::executor::MigrationRegistration { build: $f } }
    };
}

pub type Key = (String, String);

/// `MigrationLoader`: all registered migrations plus their dependency graph.
pub struct MigrationLoader {
    pub migrations: IndexMap<Key, Migration>,
    /// Topological order (dependencies first).
    pub order: Vec<Key>,
}

impl MigrationLoader {
    pub fn load(apps: Option<&[String]>) -> Result<Self> {
        let mut all: Vec<Migration> = inventory::iter::<MigrationRegistration>.into_iter().map(|r| (r.build)()).collect();
        all.retain(|m| apps.is_none_or(|a| a.contains(&m.app_label)));
        all.sort_by(|a, b| (&a.app_label, &a.name).cmp(&(&b.app_label, &b.name)));
        let migrations: IndexMap<Key, Migration> = all.into_iter().map(|m| (m.key(), m)).collect();
        let mut loader = MigrationLoader { migrations, order: vec![] };
        loader.order = loader.toposort()?;
        Ok(loader)
    }

    /// A loader over explicit migrations (tests, tooling).
    pub fn from_migrations(all: Vec<Migration>) -> Result<Self> {
        let migrations: IndexMap<Key, Migration> = all.into_iter().map(|m| (m.key(), m)).collect();
        let mut loader = MigrationLoader { migrations, order: vec![] };
        loader.order = loader.toposort()?;
        Ok(loader)
    }

    /// Resolve `__first__` / `__latest__`.
    fn resolve(&self, dep: &Key) -> Option<Key> {
        match dep.1.as_str() {
            "__first__" => self.migrations.keys().find(|k| k.0 == dep.0).cloned(),
            "__latest__" => self.leaf(&dep.0),
            _ => self.migrations.contains_key(dep).then(|| dep.clone()),
        }
    }

    fn toposort(&self) -> Result<Vec<Key>> {
        let mut out = Vec::new();
        let mut state: HashMap<Key, u8> = HashMap::new();
        fn visit(l: &MigrationLoader, k: &Key, state: &mut HashMap<Key, u8>, out: &mut Vec<Key>) -> Result<()> {
            match state.get(k) {
                Some(2) => return Ok(()),
                Some(1) => return Err(Error::improperly_configured(format!("Circular dependency in migrations at {}.{}", k.0, k.1))),
                _ => {}
            }
            state.insert(k.clone(), 1);
            let m = &l.migrations[k];
            for d in &m.dependencies {
                let dk = l.resolve(d).ok_or_else(|| Error::LookupError(format!("Migration {}.{} dependencies reference nonexistent parent node ('{}', '{}')", k.0, k.1, d.0, d.1)))?;
                visit(l, &dk, state, out)?;
            }
            state.insert(k.clone(), 2);
            out.push(k.clone());
            Ok(())
        }
        for k in self.migrations.keys() {
            visit(self, k, &mut state, &mut out)?;
        }
        Ok(out)
    }

    /// The latest migration of `app` (the one nothing else in the app depends on).
    pub fn leaf(&self, app: &str) -> Option<Key> {
        let in_app: Vec<&Key> = self.migrations.keys().filter(|k| k.0 == app).collect();
        let depended: HashSet<&Key> = self.migrations.values().filter(|m| m.app_label == app).flat_map(|m| m.dependencies.iter().filter(|d| d.0 == app)).collect();
        self.order.iter().filter(|k| k.0 == app).rev().find(|k| !depended.contains(k)).cloned().or_else(|| in_app.last().map(|k| (*k).clone()))
    }

    /// Project state after applying `keys` in order.
    pub fn state_for(&self, keys: &[Key]) -> Result<ProjectState> {
        let mut state = ProjectState::new();
        for k in keys {
            for op in &self.migrations[k].operations {
                op.state_forwards(&mut state)?;
            }
        }
        Ok(state)
    }

    pub fn full_state(&self) -> Result<ProjectState> {
        self.state_for(&self.order)
    }

    /// Ancestors of `key` (including it), dependencies first.
    pub fn with_dependencies(&self, key: &Key) -> Vec<Key> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        fn go(l: &MigrationLoader, k: &Key, seen: &mut HashSet<Key>, out: &mut Vec<Key>) {
            if !seen.insert(k.clone()) {
                return;
            }
            for d in &l.migrations[k].dependencies {
                if let Some(dk) = l.resolve(d) {
                    go(l, &dk, seen, out);
                }
            }
            out.push(k.clone());
        }
        go(self, key, &mut seen, &mut out);
        out
    }
}

const SQLITE_RECORDER: &str = r#"CREATE TABLE IF NOT EXISTS "rango_migrations" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "app" varchar(255) NOT NULL, "name" varchar(255) NOT NULL, "applied" datetime NOT NULL)"#;
const PG_RECORDER: &str = r#"CREATE TABLE IF NOT EXISTS "rango_migrations" ("id" bigint NOT NULL PRIMARY KEY GENERATED BY DEFAULT AS IDENTITY, "app" varchar(255) NOT NULL, "name" varchar(255) NOT NULL, "applied" timestamp with time zone NOT NULL)"#;

/// `MigrationRecorder`.
pub async fn ensure_recorder(db: &dyn Executor) -> Result<()> {
    db.execute_batch(if db.vendor() == Vendor::Sqlite { SQLITE_RECORDER } else { PG_RECORDER }).await
}

/// Applied migrations (`recorder.applied_migrations()`).
pub async fn applied(db: &dyn Executor) -> Result<HashSet<Key>> {
    ensure_recorder(db).await?;
    Ok(db.query(r#"SELECT "app", "name" FROM "rango_migrations""#, &[]).await?.into_iter().map(|r| (r.values[0].as_str().unwrap_or("").to_owned(), r.values[1].as_str().unwrap_or("").to_owned())).collect())
}

async fn record(db: &dyn Executor, k: &Key, applied: bool) -> Result<()> {
    if applied {
        let now = rango_core::utils::timezone::now();
        let v = if db.vendor() == Vendor::Sqlite { Value::Str(crate::models::fields_db::fmt_naive_datetime(&now.naive_utc())) } else { Value::DateTime(now) };
        db.execute(r#"INSERT INTO "rango_migrations" ("app", "name", "applied") VALUES (%s, %s, %s)"#, &[Value::Str(k.0.clone()), Value::Str(k.1.clone()), v]).await?;
    } else {
        db.execute(r#"DELETE FROM "rango_migrations" WHERE "app" = %s AND "name" = %s"#, &[Value::Str(k.0.clone()), Value::Str(k.1.clone())]).await?;
    }
    Ok(())
}

/// SQL a migration generates for `vendor` (`sqlmigrate`).
pub fn migration_sql(loader: &MigrationLoader, key: &Key, vendor: Vendor, backwards: bool) -> Result<Vec<String>> {
    let idx = loader.order.iter().position(|k| k == key).ok_or_else(|| Error::LookupError(format!("No migration {}.{}", key.0, key.1)))?;
    let mut state = loader.state_for(&loader.order[..idx])?;
    let ops = &loader.migrations[key].operations;
    let mut sql = Vec::new();
    let mut states = vec![state.clone()];
    for op in ops {
        op.state_forwards(&mut state)?;
        states.push(state.clone());
    }
    let plan: Vec<(Operation, ProjectState, ProjectState)> = if backwards {
        ops.iter().enumerate().rev().filter_map(|(i, op)| Some((op.reverse(&states[i])?, states[i + 1].clone(), states[i].clone()))).collect()
    } else {
        ops.iter().enumerate().map(|(i, op)| (op.clone(), states[i].clone(), states[i + 1].clone())).collect()
    };
    let mut carried: Vec<String> = Vec::new();
    for (op, from, to) in plan {
        let reg = crate::migrations::state::merged_registry(&from, &to)?;
        let mut editor = SchemaEditor::new(vendor, &reg).with_deferred(std::mem::take(&mut carried));
        op.database_forwards(&mut editor, &from, &to)?;
        carried = editor.take_deferred();
        sql.extend(editor.sql.0);
    }
    sql.extend(carried);
    Ok(sql)
}

/// What `migrate` should reach.
#[derive(Clone, Debug)]
pub enum Target {
    All,
    App(String),
    /// `(app, migration name or "zero")`.
    Migration(String, String),
}

/// `migrate`: returns human-readable lines ("Applying polls.0001_initial... OK").
pub async fn migrate(db: &Db, loader: &MigrationLoader, target: &Target) -> Result<Vec<String>> {
    let vendor = db.vendor();
    let done = applied(db).await?;
    let mut lines = Vec::new();

    // Backward target?
    if let Target::Migration(app, name) = target {
        let keys: Vec<Key> = loader.order.iter().filter(|k| k.0 == *app).cloned().collect();
        let keep: Vec<Key> = if name == "zero" { vec![] } else { vec![keys.iter().find(|k| k.1.starts_with(name.as_str())).cloned().ok_or_else(|| Error::LookupError(format!("Cannot find a migration matching '{name}' from app '{app}'.")))?] };
        let keep_all: HashSet<Key> = keep.iter().flat_map(|k| loader.with_dependencies(k)).collect();
        let to_undo: Vec<Key> = keys.iter().rev().filter(|k| done.contains(*k) && !keep_all.contains(*k)).cloned().collect();
        for k in to_undo {
            let sqls = migration_sql(loader, &k, vendor, true)?;
            let kk = k.clone();
            db.atomic(|tx| {
                Box::pin(async move {
                    for s in sqls {
                        tx.execute_batch(&s).await?;
                    }
                    record(tx, &kk, false).await?;
                    Ok(())
                })
            })
            .await?;
            lines.push(format!("  Unapplying {}.{}... OK", k.0, k.1));
        }
    }

    let wanted: Vec<Key> = match target {
        Target::All => loader.order.clone(),
        Target::App(a) => loader.order.iter().filter(|k| k.0 == *a).flat_map(|k| loader.with_dependencies(k)).collect::<HashSet<Key>>().into_iter().collect(),
        Target::Migration(app, name) if name != "zero" => {
            let k = loader.order.iter().filter(|k| k.0 == *app).find(|k| k.1.starts_with(name.as_str())).cloned();
            k.map(|k| loader.with_dependencies(&k)).unwrap_or_default()
        }
        _ => vec![],
    };
    let ordered: Vec<Key> = loader.order.iter().filter(|k| wanted.contains(k)).cloned().collect();
    let done = applied(db).await?;
    for k in ordered.iter().filter(|k| !done.contains(*k)) {
        let idx = loader.order.iter().position(|x| x == k).expect("in order");
        let mut state = loader.state_for(&loader.order[..idx])?;
        let mut statements: Vec<(Operation, Vec<String>)> = Vec::new();
        let mut carried: Vec<String> = Vec::new();
        for op in &loader.migrations[k].operations {
            let from = state.clone();
            op.state_forwards(&mut state)?;
            let reg = crate::migrations::state::merged_registry(&from, &state)?;
            let mut editor = SchemaEditor::new(vendor, &reg).with_deferred(std::mem::take(&mut carried));
            op.database_forwards(&mut editor, &from, &state)?;
            carried = editor.take_deferred();
            statements.push((op.clone(), editor.sql.0));
        }
        if let Some(last) = statements.last_mut() {
            last.1.extend(carried);
        }
        let key = k.clone();
        db.atomic(|tx| {
            Box::pin(async move {
                for (op, sqls) in statements {
                    for s in sqls {
                        tx.execute_batch(&s).await?;
                    }
                    if let Operation::RunCode { forward, .. } = &op {
                        forward(tx).await?;
                    }
                }
                record(tx, &key, true).await
            })
        })
        .await?;
        lines.push(format!("  Applying {}.{}... OK", k.0, k.1));
    }
    if lines.is_empty() {
        lines.push("  No migrations to apply.".into());
    }
    // `post_migrate`: once per app, after everything is applied (contenttypes and permissions hook in here).
    let mut apps: Vec<String> = Vec::new();
    for (a, _) in &loader.order {
        if !apps.contains(a) {
            apps.push(a.clone());
        }
    }
    for app in apps {
        POST_MIGRATE.send(Some(&app), MigrateEvent { app_label: app.clone(), using: db.alias().to_owned() }).await;
    }
    Ok(lines)
}
