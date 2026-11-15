//! `Q` objects: composable filter conditions.

use crate::models::expressions::Expr;
use crate::models::query::Query;
use crate::value::{ToValue, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connector {
    And,
    Or,
}

/// Right-hand side of a lookup.
#[derive(Clone, Debug)]
pub enum Rhs {
    Val(Value),
    /// For `in`: a list of values.
    List(Vec<Value>),
    /// `range`: inclusive bounds.
    Range(Value, Value),
    /// An expression (`F("other")`, `Subquery`, ...).
    Expr(Box<Expr>),
    /// A queryset used as a value (`pk__in=other_qs`).
    Query(Box<Query>),
}

#[derive(Clone, Debug)]
pub enum QChild {
    /// `path__lookup=rhs` — the final lookup name is part of `path` (parsed by the compiler).
    Lookup { path: String, rhs: Rhs },
    Q(Q),
    /// A boolean expression such as `Exists(...)`.
    Expr(Expr),
}

/// `Q(...)`.
#[derive(Clone, Debug)]
pub struct Q {
    pub connector: Connector,
    pub negated: bool,
    pub children: Vec<QChild>,
}

impl Default for Q {
    fn default() -> Self {
        Q { connector: Connector::And, negated: false, children: Vec::new() }
    }
}

impl Q {
    pub fn new() -> Q {
        Q::default()
    }

    /// `Q(path=value)` where `path` may end in a lookup (`name__startswith`).
    pub fn lookup(path: &str, value: impl ToValue) -> Q {
        Q { children: vec![QChild::Lookup { path: path.to_owned(), rhs: Rhs::Val(value.to_value()) }], ..Q::default() }
    }

    pub fn lookup_rhs(path: &str, rhs: Rhs) -> Q {
        Q { children: vec![QChild::Lookup { path: path.to_owned(), rhs }], ..Q::default() }
    }

    /// `Q(path__in=[...])`.
    pub fn is_in<T: ToValue>(path: &str, values: impl IntoIterator<Item = T>) -> Q {
        Q::lookup_rhs(&format!("{path}__in"), Rhs::List(values.into_iter().map(|v| v.to_value()).collect()))
    }

    /// `Q(path=F("other"))`.
    pub fn compare_expr(path: &str, e: Expr) -> Q {
        Q::lookup_rhs(path, Rhs::Expr(Box::new(e)))
    }

    pub fn expr(e: Expr) -> Q {
        Q { children: vec![QChild::Expr(e)], ..Q::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    fn combine(self, other: Q, connector: Connector) -> Q {
        if other.is_empty() {
            return self;
        }
        if self.is_empty() {
            return other;
        }
        Q { connector, negated: false, children: vec![QChild::Q(self), QChild::Q(other)] }
    }

    pub fn and(self, other: Q) -> Q {
        self.combine(other, Connector::And)
    }

    pub fn or(self, other: Q) -> Q {
        self.combine(other, Connector::Or)
    }

    /// `~q`: negate (the reference implementation flips `negated` on a copy rather than wrapping).
    pub fn not(mut self) -> Q {
        self.negated = !self.negated;
        self
    }
}

impl std::ops::BitAnd for Q {
    type Output = Q;
    fn bitand(self, rhs: Q) -> Q {
        self.and(rhs)
    }
}
impl std::ops::BitOr for Q {
    type Output = Q;
    fn bitor(self, rhs: Q) -> Q {
        self.or(rhs)
    }
}
impl std::ops::Not for Q {
    type Output = Q;
    fn not(self) -> Q {
        Q::not(self)
    }
}
