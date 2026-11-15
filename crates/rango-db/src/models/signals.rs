//! Model signals (`db.models.signals`). Senders are model labels (`polls.Question`).

use indexmap::IndexMap;
use rango_core::dispatch::Signal;

use crate::value::Value;

/// What receivers get: a snapshot of the instance (typed instances cannot cross the signal boundary).
#[derive(Clone, Debug)]
pub struct ModelEvent {
    pub label: String,
    pub pk: Value,
    pub created: bool,
    pub update_fields: Option<Vec<String>>,
    /// Concrete field values keyed by attname (empty for delete signals).
    pub values: IndexMap<String, Value>,
    pub using: String,
}

pub static PRE_SAVE: Signal<ModelEvent> = Signal::new();
pub static POST_SAVE: Signal<ModelEvent> = Signal::new();
pub static PRE_DELETE: Signal<ModelEvent> = Signal::new();
pub static POST_DELETE: Signal<ModelEvent> = Signal::new();

/// `m2m_changed` actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum M2mAction {
    PreAdd,
    PostAdd,
    PreRemove,
    PostRemove,
    PreClear,
    PostClear,
}

#[derive(Clone, Debug)]
pub struct M2mEvent {
    pub action: M2mAction,
    /// Label of the model owning the field.
    pub label: String,
    pub field: String,
    pub pk: Value,
    pub pk_set: Vec<Value>,
}

pub static M2M_CHANGED: Signal<M2mEvent> = Signal::new();
