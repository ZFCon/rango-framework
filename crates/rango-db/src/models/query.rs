//! `Query`: the builder behind QuerySets (the `db.models.sql.Query`).
//!
//! It owns the alias map (joins), the WHERE tree and the SELECT/ORDER/GROUP state; the
//! [`compile`](crate::models::sql) module turns it into SQL.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::models::expressions::Expr;
use crate::models::meta::{Field, FieldType, ModelMeta, Registry};
use crate::models::q::{Connector, Q, QChild, Rhs};
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinType {
    Base,
    Inner,
    LeftOuter,
}

/// One entry of the alias map.
#[derive(Clone, Debug)]
pub struct Join {
    pub table: String,
    pub alias: String,
    pub parent_alias: Option<String>,
    pub join_type: JoinType,
    /// `(parent column, this table's column)` pairs of the ON clause.
    pub join_cols: Vec<(String, String)>,
    pub nullable: bool,
    /// Identity of the relation path used to create the join (for reuse).
    pub relation_key: String,
    pub refcount: usize,
}

/// A resolved column reference.
#[derive(Clone, Debug)]
pub struct Col {
    pub alias: String,
    pub field: Field,
    pub model: String,
}

impl Col {
    pub fn column(&self) -> String {
        self.field.column()
    }
}

/// Left-hand side of a resolved lookup.
#[derive(Clone, Debug)]
pub enum Lhs {
    Col(Col),
    Transform { name: String, inner: Box<Lhs> },
    /// An annotation / expression used as a filter target.
    Expr(Expr),
}

impl Lhs {
    pub fn base_col(&self) -> Option<&Col> {
        match self {
            Lhs::Col(c) => Some(c),
            Lhs::Transform { inner, .. } => inner.base_col(),
            Lhs::Expr(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedLookup {
    pub lhs: Lhs,
    pub lookup: String,
    pub rhs: Rhs,
    /// Resolved reference columns for `F()` right-hand sides.
    pub rhs_col: Option<Col>,
}

#[derive(Clone, Debug)]
pub enum WhereChild {
    Lookup(ResolvedLookup),
    Node(WhereNode),
    Expr(Expr),
    /// Matches nothing (`EmptyResultSet`).
    Nothing,
}

#[derive(Clone, Debug)]
pub struct WhereNode {
    pub connector: Connector,
    pub negated: bool,
    pub children: Vec<WhereChild>,
}

impl Default for WhereNode {
    fn default() -> Self {
        WhereNode { connector: Connector::And, negated: false, children: Vec::new() }
    }
}

impl WhereNode {
    /// `Node.add(data, conn_type)` including the squashing of same-connector children.
    pub fn add(&mut self, data: WhereNode, conn: Connector) {
        if self.connector != conn {
            let old = WhereNode { connector: self.connector, negated: self.negated, children: std::mem::take(&mut self.children) };
            self.connector = conn;
            self.negated = false;
            self.children = vec![WhereChild::Node(old)];
        }
        if !data.negated && (data.connector == conn || data.children.len() == 1) {
            self.children.extend(data.children);
        } else {
            self.children.push(WhereChild::Node(data));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectRelated {
    None,
    All,
    Fields(Vec<String>),
}

#[derive(Clone, Debug)]
pub enum GroupBy {
    None,
    /// `True`: group by every selected non-aggregate column.
    All,
}

/// Set operations (`union`, `intersection`, `difference`).
#[derive(Clone, Debug)]
pub struct Combinator {
    pub kind: String,
    pub all: bool,
    pub queries: Vec<Query>,
}

/// One path step found by `names_to_path`.
#[derive(Clone, Debug)]
pub(crate) struct PathInfo {
    pub from_label: String,
    pub to_label: String,
    pub join_cols: Vec<(String, String)>,
    /// Multi-valued (reverse FK / M2M leg)?
    pub m2m: bool,
    pub direct: bool,
    pub nullable: bool,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct Query {
    pub registry: Arc<Registry>,
    pub model: Arc<ModelMeta>,
    pub alias_map: IndexMap<String, Join>,
    table_map: HashMap<String, Vec<String>>,
    pub where_: WhereNode,
    /// `None` = all concrete fields of the model (and select_related models).
    pub select: Option<Vec<(Col, Option<String>)>>,
    pub deferred: Option<(bool, Vec<String>)>,
    pub annotations: IndexMap<String, Expr>,
    pub annotation_mask: Option<Vec<String>>,
    pub values_names: Option<Vec<String>>,
    pub group_by: GroupBy,
    pub having: Vec<Expr>,
    pub order_by: Vec<Expr>,
    pub default_ordering: bool,
    pub low_mark: usize,
    pub high_mark: Option<usize>,
    pub distinct: bool,
    pub distinct_fields: Vec<String>,
    pub select_related: SelectRelated,
    pub select_for_update: Option<String>,
    pub combinator: Option<Combinator>,
    pub alias_prefix: String,
    pub subquery: bool,
    pub explain: bool,
    pub extra_where: Vec<(String, Vec<Value>)>,
    pub empty: bool,
    pub filter_is_sticky: bool,
    pub(crate) used_aliases: HashSet<String>,
}

impl Query {
    pub fn new(registry: Arc<Registry>, model: Arc<ModelMeta>) -> Query {
        Query {
            registry,
            model,
            alias_map: IndexMap::new(),
            table_map: HashMap::new(),
            where_: WhereNode::default(),
            select: None,
            deferred: None,
            annotations: IndexMap::new(),
            annotation_mask: None,
            values_names: None,
            group_by: GroupBy::None,
            having: Vec::new(),
            order_by: Vec::new(),
            default_ordering: true,
            low_mark: 0,
            high_mark: None,
            distinct: false,
            distinct_fields: Vec::new(),
            select_related: SelectRelated::None,
            select_for_update: None,
            combinator: None,
            alias_prefix: "T".into(),
            subquery: false,
            explain: false,
            extra_where: Vec::new(),
            empty: false,
            filter_is_sticky: false,
            used_aliases: HashSet::new(),
        }
    }

    /// The base table alias (created on first use).
    pub fn base_alias(&mut self) -> String {
        if let Some((a, _)) = self.alias_map.iter().find(|(_, j)| j.join_type == JoinType::Base) {
            return a.clone();
        }
        let table = self.model.db_table.clone();
        let alias = self.table_alias(&table, true).0;
        self.alias_map.insert(
            alias.clone(),
            Join { table, alias: alias.clone(), parent_alias: None, join_type: JoinType::Base, join_cols: vec![], nullable: false, relation_key: String::new(), refcount: 1 },
        );
        alias
    }

    /// `table_alias(table_name, create)`: first use gets the table name, later ones `T<n>`.
    fn table_alias(&mut self, table: &str, create: bool) -> (String, bool) {
        let list = self.table_map.entry(table.to_owned()).or_default();
        if !create && !list.is_empty() {
            return (list[0].clone(), false);
        }
        if !list.is_empty() {
            let alias = format!("{}{}", self.alias_prefix, self.alias_map.len() + 1);
            list.push(alias.clone());
            return (alias, true);
        }
        list.push(table.to_owned());
        (table.to_owned(), true)
    }

    fn promote(&mut self, alias: &str) {
        let mut cur = Some(alias.to_owned());
        while let Some(a) = cur {
            let Some(j) = self.alias_map.get_mut(&a) else { break };
            if j.join_type == JoinType::Inner {
                j.join_type = JoinType::LeftOuter;
            } else {
                break;
            }
            cur = j.parent_alias.clone();
        }
    }

    pub(crate) fn promote_joins(&mut self, aliases: &HashSet<String>) {
        let mut aliases: Vec<String> = aliases.iter().cloned().collect();
        aliases.sort();
        for a in aliases {
            self.promote(&a);
        }
    }

    pub(crate) fn demote_joins(&mut self, aliases: &HashSet<String>) {
        for a in aliases {
            if let Some(j) = self.alias_map.get_mut(a) {
                if j.join_type == JoinType::LeftOuter && !j.nullable_forced_outer() {
                    j.join_type = JoinType::Inner;
                }
            }
        }
    }

    /// `join(join, reuse)`: reuse an equal join or create one.
    fn join(&mut self, table: &str, parent: &str, cols: Vec<(String, String)>, nullable: bool, key: &str, reuse: Option<&HashSet<String>>) -> String {
        let existing = self
            .alias_map
            .iter()
            .find(|(a, j)| {
                j.table == table
                    && j.parent_alias.as_deref() == Some(parent)
                    && j.relation_key == key
                    && j.join_type != JoinType::Base
                    && reuse.is_none_or(|r| r.contains(*a))
            })
            .map(|(a, _)| a.clone());
        if let Some(a) = existing {
            self.alias_map.get_mut(&a).expect("exists").refcount += 1;
            return a;
        }
        let parent_outer = self.alias_map.get(parent).is_some_and(|p| p.join_type == JoinType::LeftOuter);
        let join_type = if parent_outer || nullable { JoinType::LeftOuter } else { JoinType::Inner };
        let (alias, _) = self.table_alias(table, true);
        self.alias_map.insert(
            alias.clone(),
            Join { table: table.to_owned(), alias: alias.clone(), parent_alias: Some(parent.to_owned()), join_type, join_cols: cols, nullable, relation_key: key.to_owned(), refcount: 1 },
        );
        alias
    }

    /// Resolve one relation hop starting at `opts`.
    pub(crate) fn name_to_path_step(&self, opts: &ModelMeta, name: &str) -> Result<Option<(PathInfo, Arc<ModelMeta>, Option<Field>)>> {
        let reg = Arc::clone(&self.registry);
        // Forward relation.
        if let Some(f) = opts.get_field(name).filter(|f| f.is_relation() && f.name == name) {
            let target = reg.target_of(opts, f)?;
            let rel = f.rel.as_ref().expect("relation");
            if f.ty == FieldType::ManyToMany {
                // Two legs are produced by the caller through `m2m_legs`.
                let through = reg.through_of(opts, f)?;
                let (from_fk, to_fk) = m2m_fk_names(opts, &through, target)?;
                let info = PathInfo {
                    from_label: opts.label(),
                    to_label: target.label(),
                    join_cols: vec![(opts.pk().column(), from_fk.clone())],
                    m2m: true,
                    direct: true,
                    nullable: false,
                    key: format!("m2m:{}.{}:{}", opts.label(), f.name, through.label()),
                };
                let _ = to_fk;
                return Ok(Some((info, Arc::clone(target), Some(f.clone()))));
            }
            let to_col = match &rel.to_field {
                Some(n) => target.get_field(n).map(|t| t.column()).unwrap_or_else(|| target.pk().column()),
                None => target.pk().column(),
            };
            let info = PathInfo {
                from_label: opts.label(),
                to_label: target.label(),
                join_cols: vec![(f.column(), to_col)],
                m2m: false,
                direct: true,
                nullable: f.null,
                key: format!("fk:{}.{}", opts.label(), f.name),
            };
            return Ok(Some((info, Arc::clone(target), Some(f.clone()))));
        }
        // Reverse relation (by related query name).
        for ro in reg.related_objects(&opts.label()) {
            if ro.query_name == name {
                let owner = reg.get_or_err(&ro.model)?;
                let fk = owner.get_field(&ro.field).expect("relation field exists").clone();
                if fk.ty == FieldType::ManyToMany {
                    let through = reg.through_of(owner, &fk)?;
                    let (from_fk, to_fk) = m2m_fk_names(owner, through, reg.get_or_err(&opts.label())?)?;
                    let _ = from_fk;
                    let info = PathInfo {
                        from_label: opts.label(),
                        to_label: owner.label(),
                        join_cols: vec![(opts.pk().column(), to_fk)],
                        m2m: true,
                        direct: false,
                        nullable: false,
                        key: format!("rm2m:{}.{}:{}", owner.label(), fk.name, through.label()),
                    };
                    return Ok(Some((info, Arc::clone(owner), Some(fk))));
                }
                let rel = fk.rel.as_ref().expect("relation");
                let parent_col = match &rel.to_field {
                    Some(n) => opts.get_field(n).map(|t| t.column()).unwrap_or_else(|| opts.pk().column()),
                    None => opts.pk().column(),
                };
                let info = PathInfo {
                    from_label: opts.label(),
                    to_label: owner.label(),
                    join_cols: vec![(parent_col, fk.column())],
                    m2m: fk.ty != FieldType::OneToOne,
                    direct: false,
                    nullable: true,
                    key: format!("rev:{}.{}", owner.label(), fk.name),
                };
                return Ok(Some((info, Arc::clone(owner), Some(fk))));
            }
        }
        Ok(None)
    }

    /// `setup_joins`: create (or reuse) the joins for relation hops `names` and return the final alias.
    #[allow(clippy::type_complexity)]
    pub(crate) fn setup_joins(
        &mut self,
        names: &[String],
        can_reuse: Option<&HashSet<String>>,
    ) -> Result<(String, Arc<ModelMeta>, usize, Vec<String>)> {
        let mut alias = self.base_alias();
        let mut opts = Arc::clone(&self.model);
        let mut consumed = 0;
        let mut joins = vec![alias.clone()];
        let reg = Arc::clone(&self.registry);
        for name in names {
            let name = if name == "pk" { opts.pk().name.clone() } else { name.clone() };
            let Some((info, target, field)) = self.name_to_path_step(&opts, &name)? else { break };
            if info.key.starts_with("m2m:") || info.key.starts_with("rm2m:") {
                // Two joins: parent -> through -> target.
                let owner_opts: Arc<ModelMeta> = if info.direct { Arc::clone(&opts) } else { Arc::clone(&target) };
                let fk = field.expect("m2m field");
                let through = reg.through_of(&owner_opts, &fk)?;
                let target_meta = if info.direct { Arc::clone(&target) } else { Arc::clone(&opts) };
                let (from_fk, to_fk) = m2m_fk_names(&owner_opts, through, &target_meta)?;
                let (leg1_cols, leg2_cols) = if info.direct {
                    (vec![(opts.pk().column(), from_fk.clone())], vec![(to_fk.clone(), target.pk().column())])
                } else {
                    (vec![(opts.pk().column(), to_fk.clone())], vec![(from_fk.clone(), target.pk().column())])
                };
                let reuse = can_reuse;
                let a1 = self.join(&through.db_table, &alias, leg1_cols, false, &format!("{}#1", info.key), reuse);
                let a2 = self.join(&target.db_table, &a1, leg2_cols, false, &format!("{}#2", info.key), reuse);
                self.used_aliases.insert(a1.clone());
                self.used_aliases.insert(a2.clone());
                joins.push(a1);
                joins.push(a2.clone());
                alias = a2;
            } else {
                let reuse = if info.m2m { can_reuse } else { None };
                let a = self.join(&target.db_table, &alias, info.join_cols.clone(), info.nullable, &info.key, reuse);
                if info.m2m {
                    self.used_aliases.insert(a.clone());
                }
                joins.push(a.clone());
                alias = a;
            }
            opts = target;
            consumed += 1;
        }
        Ok((alias, opts, consumed, joins))
    }

    /// Alias for the base table / relation path of an attribute path (`question__pub_date`) used by
    /// ordering, `values()` and select_related.
    pub fn resolve_path(&mut self, path: &str, can_reuse: Option<&HashSet<String>>) -> Result<Col> {
        let names: Vec<String> = path.split("__").map(str::to_owned).collect();
        let (alias, opts, consumed, _joins) = self.setup_joins(&names, can_reuse)?;
        let rest = &names[consumed..];
        let fname = match rest {
            [one] => one.clone(),
            [] => opts.pk().name.clone(),
            _ => return Err(Error::field_error(format!("Cannot resolve keyword '{}' into field.", rest[0]))),
        };
        let fname = if fname == "pk" { opts.pk().name.clone() } else { fname };
        let field = opts
            .get_field(&fname)
            .filter(|f| f.concrete())
            .cloned()
            .ok_or_else(|| Error::field_error(format!("Cannot resolve keyword '{fname}' into field. Choices are: {}", self.choices(&opts))))?;
        // Trim a trailing forward join when the target is the join column.
        let (alias, field) = self.trim_join(alias, &opts, field);
        Ok(Col { alias, model: opts.label(), field })
    }

    fn choices(&self, opts: &ModelMeta) -> String {
        let mut names: Vec<String> = opts.fields.iter().map(|f| f.name.clone()).collect();
        names.extend(self.registry.related_objects(&opts.label()).iter().map(|r| r.query_name.clone()));
        names.extend(self.annotations.keys().cloned());
        names.sort();
        names.join(", ")
    }

    /// `trim_joins`: `choice__question__id` -> `choice.question_id` when `id` is the join target.
    fn trim_join(&mut self, alias: String, opts: &ModelMeta, field: Field) -> (String, Field) {
        let Some(j) = self.alias_map.get(&alias).cloned() else { return (alias, field) };
        if j.join_type == JoinType::Base || j.relation_key.starts_with("rev:") || j.relation_key.contains('#') {
            return (alias, field);
        }
        // Forward FK/O2O join: trim when the requested column is the join's target column.
        if let Some((parent_col, target_col)) = j.join_cols.first() {
            if field.column() == *target_col && field.primary_key == (opts.pk().column() == *target_col) {
                let parent_alias = j.parent_alias.clone().expect("non-base join has parent");
                let parent_table = self.alias_map.get(&parent_alias).map(|p| p.table.clone()).unwrap_or_default();
                let found = self.registry.models().find(|m| m.db_table == parent_table).cloned();
                if let Some(parent_meta) = found {
                    if let Some(pf) = parent_meta.concrete_fields().find(|f| f.column() == *parent_col).cloned() {
                        self.unref(&alias);
                        return (parent_alias, pf);
                    }
                }
            }
        }
        (alias, field)
    }

    fn unref(&mut self, alias: &str) {
        if let Some(j) = self.alias_map.get_mut(alias) {
            j.refcount = j.refcount.saturating_sub(1);
        }
    }

    // ------------------------------------------------------------------ filtering

    /// `add_q(q)`: AND a Q object into the WHERE tree.
    pub fn add_q(&mut self, q: &Q) -> Result<()> {
        if !self.filter_is_sticky {
            self.used_aliases.clear();
        }
        let existing: HashSet<String> = self.used_aliases.clone();
        let mut used = existing;
        let clause = self.build_q(q, &mut used, false)?;
        self.where_.add(clause.0, Connector::And);
        self.used_aliases = used;
        Ok(())
    }

    /// `filter(~Q)` shortcut used by `exclude()`.
    pub fn add_q_negated(&mut self, q: &Q) -> Result<()> {
        self.add_q(&q.clone().not())
    }

    fn build_q(&mut self, q: &Q, used: &mut HashSet<String>, parent_negated: bool) -> Result<(WhereNode, HashSet<String>)> {
        let negated = q.negated;
        let current_negated = negated ^ parent_negated;
        let mut target = WhereNode { connector: q.connector, negated, children: Vec::new() };
        let mut promoter = JoinPromoter::new(q.connector, q.children.len(), current_negated);
        let mut needed_inner_all = HashSet::new();
        for child in &q.children {
            match child {
                QChild::Q(sub) => {
                    let (clause, needed) = self.build_q(sub, used, current_negated)?;
                    if !clause.children.is_empty() {
                        target.children.push(WhereChild::Node(clause));
                    }
                    promoter.add_votes(&needed);
                    needed_inner_all.extend(needed);
                }
                QChild::Expr(e) => {
                    target.children.push(WhereChild::Expr(e.clone()));
                }
                QChild::Lookup { path, rhs } => {
                    let (clause, needed) = self.build_filter(path, rhs, used, current_negated)?;
                    match clause {
                        // `Node.add`: a same-connector, non-negated clause is flattened into its parent.
                        WhereChild::Node(n) if !n.negated && (n.connector == target.connector || n.children.len() == 1) => target.children.extend(n.children),
                        other => target.children.push(other),
                    }
                    promoter.add_votes(&needed);
                    needed_inner_all.extend(needed);
                }
            }
        }
        promoter.update_join_types(self);
        Ok((target, needed_inner_all))
    }

    /// Split `path__lookup` into (relation/field names, transforms, lookup name).
    fn split_lookup(&mut self, path: &str, can_reuse: &HashSet<String>) -> Result<(Lhs, String, bool, HashSet<String>)> {
        let _ = can_reuse;
        let parts: Vec<String> = path.split("__").map(str::to_owned).collect();
        // Annotation reference?
        if let Some(first) = parts.first() {
            if self.annotations.contains_key(first) && !self.model.fields.iter().any(|f| f.name == *first) {
                let lookup = parts.get(1).cloned().unwrap_or_else(|| "exact".into());
                let used = HashSet::new();
                return Ok((Lhs::Expr(Expr::F(first.clone())), lookup, false, used));
            }
        }
        let reuse = self.used_aliases.clone();
        let (alias, opts, consumed, joins) = self.setup_joins(&parts, Some(&reuse))?;
        let used_joins: HashSet<String> = joins.into_iter().filter(|a| self.alias_map.get(a).is_some_and(|j| j.join_type != JoinType::Base)).collect();
        let mut rest: Vec<String> = parts[consumed..].to_vec();
        // The field.
        // `question__in=[..]` / `question__isnull=true`: the path ends on the relation itself and the
        // rest is a lookup, so compare the target's primary key (as `question=1` does).
        let relation_lookup = consumed > 0 && rest.first().is_some_and(|r| r != "pk" && opts.get_field(r).is_none() && (crate::models::lookups::is_lookup(r) || crate::models::lookups::is_transform(r)));
        let (field, alias, opts, traversed_relation_target) = if rest.is_empty() || relation_lookup {
            // `question=1` style: filter on the relation itself -> target pk.
            if consumed > 0 {
                let f = opts.pk().clone();
                let (alias, f) = self.trim_join(alias, &opts, f);
                (f, alias, opts, true)
            } else {
                return Err(Error::field_error("Cannot filter on empty path".to_owned()));
            }
        } else {
            let name = if rest[0] == "pk" { opts.pk().name.clone() } else { rest[0].clone() };
            if let Some(f) = opts.get_field(&name).filter(|f| f.concrete()).cloned() {
                rest.remove(0);
                if f.is_relation() {
                    // `question=1` where question is an FK on this model (no join needed): compare the column.
                    (f, alias, opts, false)
                } else {
                    let (alias, f) = self.trim_join(alias, &opts, f);
                    (f, alias, opts, false)
                }
            } else if let Some(f) = opts.get_field(&name).filter(|f| f.many_to_many_()).cloned() {
                // filter(authors=1): join through and compare the target pk.
                let _ = f;
                return Err(Error::field_error(format!("Unsupported lookup '{name}' for many-to-many field")));
            } else {
                return Err(Error::field_error(format!(
                    "Cannot resolve keyword '{}' into field. Choices are: {}",
                    rest[0],
                    self.choices(&opts)
                )));
            }
        };
        let _ = traversed_relation_target;
        let col = Col { alias, model: opts.label(), field };
        // Remaining parts: transforms then (optionally) a lookup.
        let mut lhs = Lhs::Col(col);
        let mut lookup = "exact".to_owned();
        let mut explicit = false;
        let n = rest.len();
        for (i, p) in rest.iter().enumerate() {
            let is_last = i + 1 == n;
            if is_last && crate::models::lookups::is_lookup(p) {
                lookup = p.clone();
                explicit = true;
            } else if crate::models::lookups::is_transform(p) {
                lhs = Lhs::Transform { name: p.clone(), inner: Box::new(lhs) };
            } else if is_last {
                return Err(Error::field_error(format!("Unsupported lookup '{p}' for field")));
            } else {
                return Err(Error::field_error(format!("Unsupported lookup '{p}'")));
            }
        }
        Ok((lhs, lookup, explicit, used_joins))
    }

    /// `build_filter`: one `path__lookup=value` condition.
    fn build_filter(&mut self, path: &str, rhs: &Rhs, used: &mut HashSet<String>, current_negated: bool) -> Result<(WhereChild, HashSet<String>)> {
        let saved = std::mem::take(&mut self.used_aliases);
        self.used_aliases = used.clone();
        let result = self.split_lookup(path, &HashSet::new());
        let after = std::mem::replace(&mut self.used_aliases, saved);
        used.extend(after);
        let (lhs, lookup, _explicit, used_joins) = result?;
        // `exact` with None is `isnull=True`.
        let (lookup, rhs) = match (&lookup[..], rhs) {
            ("exact", Rhs::Val(Value::Null)) => ("isnull".to_owned(), Rhs::Val(Value::Bool(true))),
            _ => (lookup, rhs.clone()),
        };
        // Resolve F() on the right-hand side to a column.
        let (rhs, rhs_col) = match rhs {
            Rhs::Expr(e) => match &*e {
                Expr::F(p) => {
                    let col = self.resolve_path(p, None)?;
                    (Rhs::Expr(e), Some(col))
                }
                _ => (Rhs::Expr(e), None),
            },
            other => (other, None),
        };
        let nullable = lhs.base_col().is_some_and(|c| c.field.null) || lhs.base_col().is_some_and(|c| self.alias_map.get(&c.alias).is_some_and(|j| j.join_type == JoinType::LeftOuter));
        let resolved = ResolvedLookup { lhs: lhs.clone(), lookup: lookup.clone(), rhs: rhs.clone(), rhs_col: rhs_col.clone() };

        let is_isnull_true = lookup == "isnull" && matches!(&rhs, Rhs::Val(Value::Bool(true)));
        let require_outer = is_isnull_true && !current_negated;
        let used_joins_out: HashSet<String> = if require_outer { HashSet::new() } else { used_joins.clone() };

        let mut clause = WhereNode::default();
        clause.children.push(WhereChild::Lookup(resolved));
        if current_negated && (lookup != "isnull" || matches!(&rhs, Rhs::Val(Value::Bool(false)))) && !matches!(&rhs, Rhs::Val(Value::Null)) {
            // The `col IS NOT NULL` guard that makes NOT (col = x) behave as expected with NULLs.
            if lookup != "isnull" && nullable {
                if let Some(c) = lhs.base_col() {
                    clause.children.push(WhereChild::Lookup(ResolvedLookup {
                        lhs: Lhs::Col(c.clone()),
                        lookup: "isnull".into(),
                        rhs: Rhs::Val(Value::Bool(false)),
                        rhs_col: None,
                    }));
                }
            }
            if let Some(rc) = &rhs_col {
                if rc.field.null {
                    clause.children.push(WhereChild::Lookup(ResolvedLookup {
                        lhs: Lhs::Col(rc.clone()),
                        lookup: "isnull".into(),
                        rhs: Rhs::Val(Value::Bool(false)),
                        rhs_col: None,
                    }));
                }
            }
        }
        let child = if clause.children.len() == 1 { clause.children.pop().expect("one child") } else { WhereChild::Node(clause) };
        let needed: HashSet<String> = used_joins_out;
        Ok((child, needed))
    }
}

impl Join {
    fn nullable_forced_outer(&self) -> bool {
        false
    }
}

/// Foreign-key column names of an M2M through model: (column pointing at `owner`, column pointing at `target`).
pub(crate) fn m2m_fk_names(owner: &ModelMeta, through: &ModelMeta, target: &ModelMeta) -> Result<(String, String)> {
    let fks: Vec<&Field> = through.fields.iter().filter(|f| f.ty == FieldType::ForeignKey).collect();
    let owner_label = owner.label();
    let target_label = target.label();
    let find = |label: &str, skip: Option<&str>| {
        fks.iter().find(|f| f.rel.as_ref().is_some_and(|r| r.to.eq_ignore_ascii_case(label) && Some(f.name.as_str()) != skip)).map(|f| (f.name.clone(), f.column()))
    };
    let from = find(&owner_label, None).ok_or_else(|| Error::field_error("through model lacks FK to owner".to_owned()))?;
    let to = find(&target_label, Some(&from.0)).or_else(|| find(&target_label, None)).ok_or_else(|| Error::field_error("through model lacks FK to target".to_owned()))?;
    Ok((from.1, to.1))
}

/// Join type promotion/demotion for AND/OR trees (`JoinPromoter`).
struct JoinPromoter {
    effective_or: bool,
    num_children: usize,
    votes: HashMap<String, usize>,
}

impl JoinPromoter {
    fn new(connector: Connector, num_children: usize, negated: bool) -> Self {
        let effective_or = match (connector, negated) {
            (Connector::And, true) => true,
            (Connector::Or, true) => false,
            (c, false) => c == Connector::Or,
        };
        Self { effective_or, num_children, votes: HashMap::new() }
    }

    fn add_votes(&mut self, aliases: &HashSet<String>) {
        for a in aliases {
            *self.votes.entry(a.clone()).or_default() += 1;
        }
    }

    fn update_join_types(&self, q: &mut Query) {
        let (mut to_promote, mut to_demote) = (HashSet::new(), HashSet::new());
        for (alias, votes) in &self.votes {
            if self.effective_or && *votes < self.num_children {
                to_promote.insert(alias.clone());
            }
            if !self.effective_or || (self.effective_or && *votes == self.num_children) {
                to_demote.insert(alias.clone());
            }
        }
        q.promote_joins(&to_promote);
        q.demote_joins(&to_demote);
    }
}
