//! Migration operations (`db.migrations.operations`).

use std::sync::Arc;

use futures_util::future::BoxFuture;
use rango_core::exceptions::{Error, Result};

use crate::backends::Vendor;
use crate::connection::Executor;
use crate::migrations::schema::SchemaEditor;
use crate::migrations::state::ProjectState;
use crate::models::meta::{Field, Index, ModelMeta};

/// Callable run by `RunCode` (the `RunPython`).
pub type CodeFn = Arc<dyn for<'a> Fn(&'a dyn Executor) -> BoxFuture<'a, Result<()>> + Send + Sync>;

#[derive(Clone)]
pub enum Operation {
    CreateModel(ModelMeta),
    DeleteModel { label: String },
    RenameModel { old: String, new_name: String },
    AddField { label: String, field: Field },
    RemoveField { label: String, name: String },
    AlterField { label: String, field: Field },
    RenameField { label: String, old: String, new_name: String },
    AddIndex { label: String, index: Index },
    RemoveIndex { label: String, name: String },
    AlterUniqueTogether { label: String, groups: Vec<Vec<String>> },
    RunSql { forward: Vec<String>, reverse: Vec<String> },
    RunCode { forward: CodeFn, reverse: Option<CodeFn> },
}

impl std::fmt::Debug for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

fn model<'a>(state: &'a ProjectState, label: &str) -> Result<&'a ModelMeta> {
    state.get(label).ok_or_else(|| Error::LookupError(format!("Model '{label}' is not in the project state.")))
}

impl Operation {
    /// `describe()`: shown by `migrate`/`showmigrations`.
    pub fn describe(&self) -> String {
        match self {
            Operation::CreateModel(m) => format!("Create model {}", m.object_name),
            Operation::DeleteModel { label } => format!("Delete model {}", label.rsplit('.').next().unwrap_or(label)),
            Operation::RenameModel { old, new_name } => format!("Rename model {} to {new_name}", old.rsplit('.').next().unwrap_or(old)),
            Operation::AddField { label, field } => format!("Add field {} to {}", field.name, label.rsplit('.').next().unwrap_or(label).to_lowercase()),
            Operation::RemoveField { label, name } => format!("Remove field {name} from {}", label.rsplit('.').next().unwrap_or(label).to_lowercase()),
            Operation::AlterField { label, field } => format!("Alter field {} on {}", field.name, label.rsplit('.').next().unwrap_or(label).to_lowercase()),
            Operation::RenameField { label, old, new_name } => format!("Rename field {old} on {} to {new_name}", label.rsplit('.').next().unwrap_or(label).to_lowercase()),
            Operation::AddIndex { label, index } => format!("Create index {} on field(s) {} of model {}", index.name.clone().unwrap_or_default(), index.fields.join(", "), label.rsplit('.').next().unwrap_or(label)),
            Operation::RemoveIndex { label, name } => format!("Remove index {name} from {}", label.rsplit('.').next().unwrap_or(label)),
            Operation::AlterUniqueTogether { label, .. } => format!("Alter unique_together for {} ", label.rsplit('.').next().unwrap_or(label)),
            Operation::RunSql { .. } => "Raw SQL operation".into(),
            Operation::RunCode { .. } => "Raw Rust code".into(),
        }
    }

    /// `state_forwards`.
    pub fn state_forwards(&self, state: &mut ProjectState) -> Result<()> {
        match self {
            Operation::CreateModel(m) => state.add(m.clone()),
            Operation::DeleteModel { label } => {
                state.remove(label);
            }
            Operation::RenameModel { old, new_name } => {
                if let Some(mut m) = state.remove(old) {
                    let default_table = format!("{}_{}", m.app_label, m.object_name.to_lowercase());
                    let was_default = m.db_table == default_table;
                    m.object_name = new_name.clone();
                    if was_default {
                        m.db_table = format!("{}_{}", m.app_label, new_name.to_lowercase());
                    }
                    let old_label = old.to_lowercase();
                    let new_label = m.label();
                    state.add(m);
                    for other in state.models.values_mut() {
                        for f in &mut other.fields {
                            if let Some(r) = &mut f.rel {
                                if r.to.to_lowercase() == old_label {
                                    r.to = new_label.clone();
                                }
                            }
                        }
                    }
                }
            }
            Operation::AddField { label, field } => {
                state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?.fields.push(field.clone());
            }
            Operation::RemoveField { label, name } => {
                state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?.fields.retain(|f| f.name != *name);
            }
            Operation::AlterField { label, field } => {
                let m = state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?;
                if let Some(f) = m.fields.iter_mut().find(|f| f.name == field.name) {
                    *f = field.clone();
                }
            }
            Operation::RenameField { label, old, new_name } => {
                let m = state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?;
                if let Some(f) = m.fields.iter_mut().find(|f| f.name == *old) {
                    f.name = new_name.clone();
                }
            }
            Operation::AddIndex { label, index } => {
                state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?.indexes.push(index.clone());
            }
            Operation::RemoveIndex { label, name } => {
                state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?.indexes.retain(|i| i.name.as_deref() != Some(name));
            }
            Operation::AlterUniqueTogether { label, groups } => {
                state.get_mut(label).ok_or_else(|| Error::LookupError(label.clone()))?.unique_together = groups.clone();
            }
            Operation::RunSql { .. } | Operation::RunCode { .. } => {}
        }
        Ok(())
    }

    /// `database_forwards`: emit DDL for `from -> to`.
    pub fn database_forwards(&self, editor: &mut SchemaEditor<'_>, from: &ProjectState, to: &ProjectState) -> Result<()> {
        let vendor = editor.vendor;
        match self {
            Operation::CreateModel(m) => {
                let m = model(to, &m.label())?;
                editor.create_model(m)
            }
            Operation::DeleteModel { label } => editor.delete_model(model(from, label)?),
            Operation::RenameModel { old, new_name } => {
                let (o, n) = (model(from, old)?, model(to, &format!("{}.{new_name}", old.split('.').next().unwrap_or("")))?);
                if o.db_table != n.db_table {
                    editor.rename_table(&o.db_table, &n.db_table);
                }
                Ok(())
            }
            Operation::AddField { label, field } => editor.add_field(model(to, label)?, field),
            Operation::RemoveField { label, name } => {
                let m = model(from, label)?;
                let f = m.fields.iter().find(|f| f.name == *name).ok_or_else(|| Error::FieldDoesNotExist(name.clone()))?.clone();
                editor.remove_field(m, &f)
            }
            Operation::AlterField { label, field } => {
                let old = model(from, label)?.fields.iter().find(|f| f.name == field.name).cloned().ok_or_else(|| Error::FieldDoesNotExist(field.name.clone()))?;
                editor.alter_field(model(to, label)?, &old, field)
            }
            Operation::RenameField { label, old, new_name } => {
                let m = model(from, label)?;
                let of = m.fields.iter().find(|f| f.name == *old).ok_or_else(|| Error::FieldDoesNotExist(old.clone()))?;
                let mut nf = of.clone();
                nf.name = new_name.clone();
                if of.column() != nf.column() {
                    editor.rename_column(&m.db_table, &of.column(), &nf.column());
                }
                Ok(())
            }
            Operation::AddIndex { label, index } => {
                editor.add_index(model(to, label)?, index);
                Ok(())
            }
            Operation::RemoveIndex { name, .. } => {
                editor.remove_index(name);
                Ok(())
            }
            Operation::AlterUniqueTogether { label, groups } => {
                let old = model(from, label)?.unique_together.clone();
                editor.alter_unique_together(model(to, label)?, &old, groups)
            }
            Operation::RunSql { forward, .. } => {
                for s in forward {
                    editor.sql.0.push(s.clone());
                }
                let _ = vendor;
                Ok(())
            }
            Operation::RunCode { .. } => Ok(()),
        }
    }

    /// The inverse operation, if it has one.
    pub fn reverse(&self, from: &ProjectState) -> Option<Operation> {
        Some(match self {
            Operation::CreateModel(m) => Operation::DeleteModel { label: m.label() },
            Operation::DeleteModel { label } => Operation::CreateModel(from.get(label)?.clone()),
            Operation::RenameModel { old, new_name } => Operation::RenameModel { old: format!("{}.{new_name}", old.split('.').next()?), new_name: old.rsplit('.').next()?.to_owned() },
            Operation::AddField { label, field } => Operation::RemoveField { label: label.clone(), name: field.name.clone() },
            Operation::RemoveField { label, name } => Operation::AddField { label: label.clone(), field: from.get(label)?.fields.iter().find(|f| f.name == *name)?.clone() },
            Operation::AlterField { label, field } => Operation::AlterField { label: label.clone(), field: from.get(label)?.fields.iter().find(|f| f.name == field.name)?.clone() },
            Operation::RenameField { label, old, new_name } => Operation::RenameField { label: label.clone(), old: new_name.clone(), new_name: old.clone() },
            Operation::AddIndex { label, index } => Operation::RemoveIndex { label: label.clone(), name: index.name.clone()? },
            Operation::RemoveIndex { label, name } => Operation::AddIndex { label: label.clone(), index: from.get(label)?.indexes.iter().find(|i| i.name.as_deref() == Some(name))?.clone() },
            Operation::AlterUniqueTogether { label, .. } => Operation::AlterUniqueTogether { label: label.clone(), groups: from.get(label)?.unique_together.clone() },
            Operation::RunSql { forward, reverse } => Operation::RunSql { forward: reverse.clone(), reverse: forward.clone() },
            Operation::RunCode { forward, reverse } => Operation::RunCode { forward: reverse.clone()?, reverse: Some(forward.clone()) },
        })
    }

    pub fn is_noop_for(&self, _vendor: Vendor) -> bool {
        false
    }
}
