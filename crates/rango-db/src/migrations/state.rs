//! `ProjectState`: the model definitions implied by a sequence of migrations.

use indexmap::IndexMap;
use rango_core::exceptions::Result;

use crate::models::meta::{ModelMeta, Registry};

#[derive(Clone, Debug, Default)]
pub struct ProjectState {
    /// Keyed by lower-cased label (`polls.question`).
    pub models: IndexMap<String, ModelMeta>,
}

impl ProjectState {
    pub fn new() -> Self {
        Self::default()
    }

    /// State describing the models currently compiled into the program.
    pub fn from_registry(reg: &Registry, apps: Option<&[String]>) -> Self {
        let mut s = ProjectState::new();
        let mut all: Vec<&std::sync::Arc<crate::models::meta::ModelMeta>> = reg.models().collect();
        all.sort_by(|a, b| (&a.app_label, &a.source, &a.object_name).cmp(&(&b.app_label, &b.source, &b.object_name)));
        for m in all {
            // Auto-created M2M through models are implied by their M2M field.
            if reg.models().any(|o| o.many_to_many().any(|f| f.rel.as_ref().is_some_and(|r| r.through.is_none()) && format!("{}.{}_{}", o.app_label, o.object_name, f.name).eq_ignore_ascii_case(&m.label()))) {
                continue;
            }
            if apps.is_none_or(|a| a.contains(&m.app_label)) && m.managed {
                s.models.insert(m.label_lower(), (**m).clone());
            }
        }
        s
    }

    pub fn get(&self, label: &str) -> Option<&ModelMeta> {
        self.models.get(&label.to_lowercase())
    }

    pub fn get_mut(&mut self, label: &str) -> Option<&mut ModelMeta> {
        self.models.get_mut(&label.to_lowercase())
    }

    pub fn add(&mut self, m: ModelMeta) {
        self.models.insert(m.label_lower(), m);
    }

    pub fn remove(&mut self, label: &str) -> Option<ModelMeta> {
        self.models.shift_remove(&label.to_lowercase())
    }

    /// Resolve relations: the registry operations and the schema editor work against.
    pub fn registry(&self) -> Result<Registry> {
        Registry::new(self.models.values().cloned().collect())
    }
}

/// Registry covering both sides of an operation (deleted models only exist in `from`).
pub fn merged_registry(from: &ProjectState, to: &ProjectState) -> Result<Registry> {
    let mut models = to.models.clone();
    for (k, m) in &from.models {
        models.entry(k.clone()).or_insert_with(|| m.clone());
    }
    Registry::new(models.into_values().collect())
}
