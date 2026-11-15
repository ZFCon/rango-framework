//! `makemigrations`: diff two project states into operations (`MigrationAutodetector`),
//! name the migration and write it as Rust source.

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::migrations::operations::Operation;
use crate::migrations::state::ProjectState;
use crate::models::meta::{Default_, Field, FieldType, ModelMeta, OnDelete};
use crate::value::Value;

/// Operations per app label.
pub fn changes(from: &ProjectState, to: &ProjectState) -> Result<IndexMap<String, Vec<Operation>>> {
    let mut out: IndexMap<String, Vec<Operation>> = IndexMap::new();
    let mut push = |app: &str, op: Operation| out.entry(app.to_owned()).or_default().push(op);

    // --- created models, relation targets first
    let mut pending: Vec<&ModelMeta> = to.models.iter().filter(|(k, _)| !from.models.contains_key(*k)).map(|(_, m)| m).collect();
    let mut created: Vec<String> = Vec::new();
    let mut deferred_fields: Vec<(String, Field)> = Vec::new();
    while !pending.is_empty() {
        let ready: Vec<usize> = pending
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.fields.iter().filter(|f| f.is_relation()).all(|f| {
                    let t = f.rel.as_ref().map(|r| if r.to == "self" { m.label() } else { r.to.clone() }).unwrap_or_default().to_lowercase();
                    t == m.label_lower() || created.contains(&t) || !to.models.contains_key(&t) || from.models.contains_key(&t)
                })
            })
            .map(|(i, _)| i)
            .collect();
        let pick = ready.first().copied();
        let (m, cut) = match pick {
            Some(i) => (pending.remove(i), false),
            None => (pending.remove(0), true), // cycle: create without relation fields, add them afterwards
        };
        let mut model = m.clone();
        if cut {
            let (rel, plain): (Vec<Field>, Vec<Field>) = model.fields.into_iter().partition(|f| f.is_relation() && f.rel.as_ref().is_some_and(|r| r.to.to_lowercase() != m.label_lower()));
            deferred_fields.extend(rel.into_iter().map(|f| (m.label(), f)));
            model.fields = plain;
        }
        created.push(m.label_lower());
        push(&m.app_label, Operation::CreateModel(model));
    }
    for (label, f) in deferred_fields {
        let app = label.split('.').next().unwrap_or("").to_owned();
        push(&app, Operation::AddField { label, field: f });
    }

    // --- changed models
    for (key, new) in &to.models {
        let Some(old) = from.models.get(key) else { continue };
        for f in &new.fields {
            match old.fields.iter().find(|o| o.name == f.name) {
                None => {
                    if !f.null && f.default.is_none() && !f.many_to_many_() && !f.ty.is_auto() && f.db_default.is_none() {
                        return Err(Error::Other(
                            format!(
                                "You are trying to add a non-nullable field '{}' to {} without a default; we can't do that (the database needs something to populate existing rows). Give the field a default (#[field(default = ...)]) or make it nullable.",
                                f.name,
                                new.object_name.to_lowercase()
                            )
                            .into(),
                        ));
                    }
                    push(&new.app_label, Operation::AddField { label: new.label(), field: f.clone() });
                }
                Some(o) if o != f => push(&new.app_label, Operation::AlterField { label: new.label(), field: f.clone() }),
                Some(_) => {}
            }
        }
        for o in &old.fields {
            if !new.fields.iter().any(|f| f.name == o.name) {
                push(&new.app_label, Operation::RemoveField { label: new.label(), name: o.name.clone() });
            }
        }
        for idx in &new.indexes {
            if !old.indexes.contains(idx) {
                push(&new.app_label, Operation::AddIndex { label: new.label(), index: idx.clone() });
            }
        }
        for idx in &old.indexes {
            if !new.indexes.contains(idx) {
                if let Some(n) = &idx.name {
                    push(&new.app_label, Operation::RemoveIndex { label: new.label(), name: n.clone() });
                }
            }
        }
        if old.unique_together != new.unique_together {
            push(&new.app_label, Operation::AlterUniqueTogether { label: new.label(), groups: new.unique_together.clone() });
        }
    }

    // --- deleted models
    for (key, old) in &from.models {
        if !to.models.contains_key(key) {
            push(&old.app_label, Operation::DeleteModel { label: old.label() });
        }
    }
    Ok(out)
}

/// Apps referenced by relation fields of `ops` (cross-app dependencies).
pub fn referenced_apps(ops: &[Operation], app: &str) -> Vec<String> {
    let mut apps = Vec::new();
    let mut note = |f: &Field| {
        if let Some(r) = &f.rel {
            if let Some(a) = r.to.split('.').next().filter(|a| *a != app && r.to != "self") {
                if !apps.contains(&a.to_owned()) {
                    apps.push(a.to_owned());
                }
            }
        }
    };
    for op in ops {
        match op {
            Operation::CreateModel(m) => m.fields.iter().for_each(&mut note),
            Operation::AddField { field, .. } | Operation::AlterField { field, .. } => note(field),
            _ => {}
        }
    }
    apps
}

/// `suggest_name`.
pub fn suggest_name(ops: &[Operation], initial: bool) -> String {
    if initial {
        return "initial".into();
    }
    let lower = |l: &str| l.rsplit('.').next().unwrap_or(l).to_lowercase();
    if let [op] = ops {
        return match op {
            Operation::CreateModel(m) => m.object_name.to_lowercase(),
            Operation::DeleteModel { label } => format!("delete_{}", lower(label)),
            Operation::AddField { label, field } => format!("{}_{}", lower(label), field.name.to_lowercase()),
            Operation::RemoveField { label, name } => format!("remove_{}_{}", lower(label), name.to_lowercase()),
            Operation::AlterField { label, field } => format!("alter_{}_{}", lower(label), field.name.to_lowercase()),
            Operation::RenameField { label, old, new_name } => format!("rename_{old}_{}_{new_name}", lower(label)),
            Operation::AddIndex { label, .. } => format!("{}_index", lower(label)),
            Operation::AlterUniqueTogether { label, .. } => format!("alter_{}_unique_together", lower(label)),
            _ => "auto".into(),
        };
    }
    format!("auto_{}", rango_core::utils::timezone::now().format("%Y%m%d_%H%M"))
}

// ---------------------------------------------------------------------------- writer

fn v_code(v: &Value) -> String {
    match v {
        Value::Null => "Value::Null".into(),
        Value::Bool(b) => format!("Value::Bool({b})"),
        Value::Int(i) => format!("Value::Int({i})"),
        Value::Float(f) => format!("Value::Float({f:?})"),
        Value::Str(s) => format!("Value::Str({s:?}.to_owned())"),
        other => format!("Value::Str({:?}.to_owned())", crate::models::fields_db::adapt_to_text(other)),
    }
}

/// Rust source reproducing `f`.
pub fn field_code(f: &Field) -> String {
    let mut s = match (&f.rel, f.ty) {
        (Some(r), FieldType::ForeignKey | FieldType::OneToOne | FieldType::ManyToMany) => {
            let od = match &r.on_delete {
                OnDelete::Cascade => "OnDelete::Cascade",
                OnDelete::Protect => "OnDelete::Protect",
                OnDelete::Restrict => "OnDelete::Restrict",
                OnDelete::SetNull => "OnDelete::SetNull",
                OnDelete::SetDefault => "OnDelete::SetDefault",
                OnDelete::DoNothing => "OnDelete::DoNothing",
                OnDelete::SetValue(_) => "OnDelete::SetNull",
            };
            match f.ty {
                FieldType::ForeignKey => format!("Field::foreign_key({:?}, {:?}, {od})", f.name, r.to),
                FieldType::OneToOne => format!("Field::one_to_one({:?}, {:?}, {od})", f.name, r.to),
                _ => format!("Field::many_to_many({:?}, {:?})", f.name, r.to),
            }
        }
        _ => format!("Field::new({:?}, FieldType::{:?})", f.name, f.ty),
    };
    let mut set = Vec::new();
    let base = Field::new(&f.name, f.ty);
    let base = if f.rel.is_some() {
        match f.ty {
            FieldType::ForeignKey => Field::foreign_key(&f.name, "x", OnDelete::Cascade),
            FieldType::OneToOne => Field::one_to_one(&f.name, "x", OnDelete::Cascade),
            _ => Field::many_to_many(&f.name, "x"),
        }
    } else {
        base
    };
    if f.primary_key != base.primary_key {
        set.push(format!("f.primary_key = {};", f.primary_key));
    }
    if f.null {
        set.push("f.null = true;".into());
    }
    if f.blank {
        set.push("f.blank = true;".into());
    }
    if f.unique != base.unique {
        set.push(format!("f.unique = {};", f.unique));
    }
    if f.db_index != base.db_index {
        set.push(format!("f.db_index = {};", f.db_index));
    }
    if f.editable != base.editable {
        set.push(format!("f.editable = {};", f.editable));
    }
    if let Some(n) = f.max_length {
        set.push(format!("f.max_length = Some({n});"));
    }
    if let Some(n) = f.max_digits {
        set.push(format!("f.max_digits = Some({n});"));
    }
    if let Some(n) = f.decimal_places {
        set.push(format!("f.decimal_places = Some({n});"));
    }
    if f.auto_now {
        set.push("f.auto_now = true;".into());
    }
    if f.auto_now_add {
        set.push("f.auto_now_add = true;".into());
    }
    if let Some(Default_::Value(v)) = &f.default {
        set.push(format!("f.default = Some(Default_::Value({}));", v_code(v)));
    }
    if let Some(c) = &f.db_column {
        set.push(format!("f.db_column = Some({c:?}.to_owned());"));
    }
    if let Some(c) = &f.verbose_name {
        set.push(format!("f.verbose_name = Some({c:?}.to_owned());"));
    }
    if !f.help_text.is_empty() {
        set.push(format!("f.help_text = {:?}.to_owned();", f.help_text));
    }
    if let Some(r) = &f.rel {
        if let Some(n) = &r.related_name {
            set.push(format!("f.rel.as_mut().unwrap().related_name = Some({n:?}.to_owned());"));
        }
        if let Some(n) = &r.related_query_name {
            set.push(format!("f.rel.as_mut().unwrap().related_query_name = Some({n:?}.to_owned());"));
        }
        if let Some(n) = &r.to_field {
            set.push(format!("f.rel.as_mut().unwrap().to_field = Some({n:?}.to_owned());"));
        }
        if let Some(n) = &r.through {
            set.push(format!("f.rel.as_mut().unwrap().through = Some({n:?}.to_owned());"));
        }
        if !r.db_constraint {
            set.push("f.rel.as_mut().unwrap().db_constraint = false;".into());
        }
    }
    if !set.is_empty() {
        s = format!("{s}.with(|f| {{ {} }})", set.join(" "));
    }
    s
}

fn model_code(m: &ModelMeta) -> String {
    let mut s = format!("ModelMeta::new({:?}, {:?})", m.app_label, m.object_name);
    let default_table = format!("{}_{}", m.app_label, m.object_name.to_lowercase());
    if m.db_table != default_table {
        s.push_str(&format!(".db_table({:?})", m.db_table));
    }
    if !m.ordering.is_empty() {
        s.push_str(&format!(".ordering(&[{}])", m.ordering.iter().map(|o| format!("{o:?}")).collect::<Vec<_>>().join(", ")));
    }
    for g in &m.unique_together {
        s.push_str(&format!(".unique_together(&[{}])", g.iter().map(|o| format!("{o:?}")).collect::<Vec<_>>().join(", ")));
    }
    for f in &m.fields {
        s.push_str(&format!("\n                .field({})", field_code(f)));
    }
    s
}

fn op_code(op: &Operation) -> String {
    match op {
        Operation::CreateModel(m) => format!("Operation::CreateModel(\n            {})", model_code(m)),
        Operation::DeleteModel { label } => format!("Operation::DeleteModel {{ label: {label:?}.to_owned() }}"),
        Operation::RenameModel { old, new_name } => format!("Operation::RenameModel {{ old: {old:?}.to_owned(), new_name: {new_name:?}.to_owned() }}"),
        Operation::AddField { label, field } => format!("Operation::AddField {{ label: {label:?}.to_owned(), field: {} }}", field_code(field)),
        Operation::RemoveField { label, name } => format!("Operation::RemoveField {{ label: {label:?}.to_owned(), name: {name:?}.to_owned() }}"),
        Operation::AlterField { label, field } => format!("Operation::AlterField {{ label: {label:?}.to_owned(), field: {} }}", field_code(field)),
        Operation::RenameField { label, old, new_name } => format!("Operation::RenameField {{ label: {label:?}.to_owned(), old: {old:?}.to_owned(), new_name: {new_name:?}.to_owned() }}"),
        Operation::AddIndex { label, index } => {
            let name = match &index.name {
                Some(n) => format!("Some({n:?}.to_owned())"),
                None => "None".into(),
            };
            let fields = index.fields.iter().map(|f| format!("{f:?}.to_owned()")).collect::<Vec<_>>().join(", ");
            format!("Operation::AddIndex {{ label: {label:?}.to_owned(), index: Index {{ name: {name}, fields: vec![{fields}], condition: None, include: vec![] }} }}")
        }
        Operation::RemoveIndex { label, name } => format!("Operation::RemoveIndex {{ label: {label:?}.to_owned(), name: {name:?}.to_owned() }}"),
        Operation::AlterUniqueTogether { label, groups } => format!(
            "Operation::AlterUniqueTogether {{ label: {label:?}.to_owned(), groups: vec![{}] }}",
            groups.iter().map(|g| format!("vec![{}]", g.iter().map(|f| format!("{f:?}.to_owned()")).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>().join(", ")
        ),
        Operation::RunSql { forward, reverse } => format!(
            "Operation::RunSql {{ forward: vec![{}], reverse: vec![{}] }}",
            forward.iter().map(|s| format!("{s:?}.to_owned()")).collect::<Vec<_>>().join(", "),
            reverse.iter().map(|s| format!("{s:?}.to_owned()")).collect::<Vec<_>>().join(", ")
        ),
        Operation::RunCode { .. } => "/* RunCode cannot be serialized */ Operation::RunSql { forward: vec![], reverse: vec![] }".into(),
    }
}

/// Source of a migration module.
pub fn migration_source(app: &str, name: &str, initial: bool, deps: &[(String, String)], ops: &[Operation]) -> String {
    let mut s = String::new();
    s.push_str(&format!("//! Generated by `makemigrations` (Rango {}) on {}.\n\n", rango_core::management::VERSION, rango_core::utils::timezone::now().format("%Y-%m-%d %H:%M")));
    s.push_str("#![allow(unused_imports)]\n\nuse rango::db::migrations::{Migration, Operation};\nuse rango::db::models::meta::Default_;\nuse rango::db::models::{Field, FieldType, Index, ModelMeta, OnDelete};\nuse rango::db::Value;\n\n");
    s.push_str(&format!("pub fn migration() -> Migration {{\n    Migration::new({app:?}, {name:?})"));
    if initial {
        s.push_str("\n        .initial()");
    }
    for (a, n) in deps {
        s.push_str(&format!("\n        .dependency({a:?}, {n:?})"));
    }
    for op in ops {
        s.push_str(&format!("\n        .operation({})", op_code(op)));
    }
    s.push_str("\n}\n\nrango::db::register_migration!(migration);\n");
    s
}
