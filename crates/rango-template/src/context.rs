//! `Context`: variable scopes plus per-render state.

use std::collections::HashMap;
use std::sync::Arc;

use crate::nodes::Node;
use crate::value::Value;

/// A named `{% block %}` body.
#[derive(Debug)]
pub struct BlockData {
    pub name: String,
    pub nodes: Vec<Node>,
}

/// State shared by one render (`context.render_context`).
#[derive(Default)]
pub struct RenderState {
    /// `BlockContext`: block name -> stack of overriding blocks (last = innermost child).
    pub blocks: HashMap<String, Vec<Arc<BlockData>>>,
    /// Blocks currently being rendered with the next-parent chain (for `block.super`).
    pub super_chain: Vec<(String, Vec<Arc<BlockData>>)>,
    pub cycles: HashMap<usize, usize>,
    pub ifchanged: HashMap<usize, Vec<Value>>,
    /// Names of templates rendered so far (tests, debug).
    pub used_templates: Vec<String>,
    pub include_depth: usize,
}

/// Template context: a stack of scopes (innermost last).
pub struct Context {
    scopes: Vec<HashMap<String, Value>>,
    pub autoescape: bool,
    pub render: RenderState,
}

/// `Context._reset_dicts`: the constants every template can use.
fn builtins() -> HashMap<String, Value> {
    HashMap::from([("True".to_owned(), Value::Bool(true)), ("False".to_owned(), Value::Bool(false)), ("None".to_owned(), Value::None)])
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    pub fn new() -> Context {
        Context { scopes: vec![builtins(), HashMap::new()], autoescape: true, render: RenderState::default() }
    }

    /// Build from a serializable value (a struct or map becomes the top-level variables).
    pub fn from_serialize<T: serde::Serialize>(v: &T) -> Context {
        let mut c = Context::new();
        if let Value::Map(m) = crate::value::to_value(v) {
            for (k, v) in m {
                c.set(&k, v);
            }
        }
        c
    }

    /// `context[key] = value` (innermost scope).
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.scopes.last_mut().expect("at least one scope").insert(key.to_owned(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.scopes.iter().rev().find_map(|s| s.get(key))
    }

    pub fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        if self.scopes.len() > 2 {
            self.scopes.pop();
        }
    }

    /// Add `vars` as the lowest user scope, under everything already set (`RequestContext` processors).
    pub fn insert_below(&mut self, vars: impl IntoIterator<Item = (String, Value)>) {
        self.scopes.insert(1, vars.into_iter().collect());
    }

    /// Replace all scopes (`{% include ... only %}`); returns the old scopes.
    pub(crate) fn isolate(&mut self) -> Vec<HashMap<String, Value>> {
        std::mem::replace(&mut self.scopes, vec![builtins(), HashMap::new()])
    }

    pub(crate) fn restore(&mut self, scopes: Vec<HashMap<String, Value>>) {
        self.scopes = scopes;
    }
}
