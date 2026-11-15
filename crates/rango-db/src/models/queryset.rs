//! `QuerySet`: lazy, chainable query builder and the terminal operations that run it.

use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::connection::Executor;
use crate::models::base::{Model, RowData};
use crate::models::deletion::delete_pks;
use crate::models::expressions::Expr;
use crate::models::fields_db::{from_db, prep_param};
use crate::models::meta::{FieldType, ModelMeta, registry};
use crate::models::q::{Q, Rhs};
use crate::models::query::{Combinator, GroupBy, Query, SelectRelated};
use crate::models::sql::{SelectKind, SelectPlan, UpdateValue, contains_agg, insert_sql, parse_ordering};
use crate::value::{FromValue, Row, ToValue, Value};

const MAX_GET_RESULTS: usize = 21;

pub struct QuerySet<M: Model> {
    pub(crate) query: Query,
    prefetch: Vec<String>,
    error: Option<Error>,
    _m: PhantomData<fn() -> M>,
}

impl<M: Model> Clone for QuerySet<M> {
    fn clone(&self) -> Self {
        QuerySet { query: self.query.clone(), prefetch: self.prefetch.clone(), error: None, _m: PhantomData }
    }
}

/// `Model.DoesNotExist`.
pub fn does_not_exist(model: &str) -> Error {
    Error::ObjectDoesNotExist { model: Some(model.to_owned()), message: format!("{model} matching query does not exist.") }
}

impl<M: Model> Default for QuerySet<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Model> QuerySet<M> {
    /// `Model.objects.all()`.
    pub fn new() -> Self {
        Self::for_label(&M::label())
    }

    /// A queryset for a model chosen at runtime (`apps.get_model(label)`).
    pub fn for_label(label: &str) -> Self {
        let reg = registry();
        match reg.get_or_err(label) {
            Ok(meta) => QuerySet { query: Query::new(Arc::clone(&reg), Arc::clone(meta)), prefetch: Vec::new(), error: None, _m: PhantomData },
            Err(e) => {
                let dummy = ModelMeta::new("unknown", "Unknown");
                QuerySet { query: Query::new(Arc::clone(&reg), Arc::new(dummy)), prefetch: Vec::new(), error: Some(e), _m: PhantomData }
            }
        }
    }

    /// The underlying query (for subqueries and inspection).
    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn into_query(self) -> Query {
        self.query
    }

    fn fail(mut self, e: Error) -> Self {
        self.error.get_or_insert(e);
        self
    }

    fn check(&self) -> Result<()> {
        match &self.error {
            Some(Error::FieldError(m)) => Err(Error::FieldError(m.clone())),
            Some(Error::LookupError(m)) => Err(Error::LookupError(m.clone())),
            Some(other) => Err(Error::Other(other.to_string().into())),
            None => Ok(()),
        }
    }

    // ------------------------------------------------------------------ chainable

    pub fn all(self) -> Self {
        self
    }

    /// `filter(Q(...))`.
    pub fn filter_q(mut self, q: Q) -> Self {
        if let Err(e) = self.query.add_q(&self.rewrite_pk(q)) {
            return self.fail(e);
        }
        self
    }

    /// `exclude(Q(...))`.
    pub fn exclude_q(mut self, q: Q) -> Self {
        if let Err(e) = self.query.add_q_negated(&self.rewrite_pk(q)) {
            return self.fail(e);
        }
        self
    }

    fn rewrite_pk(&self, q: Q) -> Q {
        q
    }

    /// `filter(path__lookup=value)`.
    pub fn filter_path(self, path: &str, value: impl ToValue) -> Result<Self> {
        Ok(self.filter_q(Q::lookup(path, value)))
    }

    /// `filter(path__lookup=value)` with a string path (runtime-checked; errors surface at the
    /// terminal call). Prefer the typed [`filter`](Self::filter) with `filter!`.
    pub fn filter_kw(self, path: &str, value: impl ToValue) -> Self {
        self.filter_q(Q::lookup(path, value))
    }

    /// `exclude(path__lookup=value)` with a string path.
    pub fn exclude_kw(self, path: &str, value: impl ToValue) -> Self {
        self.exclude_q(Q::lookup(path, value))
    }

    /// `filter(...)` with typed field handles: `qs.filter(filter!(question_text__startswith = "What"))`
    /// or `qs.filter(|f| f.question_text().startswith("What"))`. Misspelled fields do not compile.
    pub fn filter(self, cond: impl FnOnce(&M::Fields) -> Q) -> Self {
        let q = cond(&M::Fields::default());
        self.filter_q(q)
    }

    /// `exclude(...)` with typed field handles.
    pub fn exclude(self, cond: impl FnOnce(&M::Fields) -> Q) -> Self {
        let q = cond(&M::Fields::default());
        self.exclude_q(q)
    }

    /// `filter(path__lookup=F("other"))` / any expression.
    pub fn filter_expr(self, path: &str, e: Expr) -> Self {
        self.filter_q(Q::compare_expr(path, e))
    }

    /// `order_by("-pub_date", "question__text")`; `order_by(&[])` clears all ordering.
    pub fn order_by(mut self, fields: &[&str]) -> Self {
        self.query.order_by = fields.iter().map(|f| if *f == "?" { Expr::F("?".into()).asc() } else { parse_ordering(f) }).collect();
        self.query.default_ordering = !fields.is_empty() && false;
        self
    }

    /// `order_by(expr, ...)` with expressions.
    pub fn order_by_expr(mut self, exprs: Vec<Expr>) -> Self {
        self.query.order_by = exprs;
        self.query.default_ordering = false;
        self
    }

    /// `reverse()`.
    pub fn reverse(mut self) -> Self {
        if self.query.order_by.is_empty() {
            self.query.order_by = self.query.model.ordering.iter().map(|o| parse_ordering(o)).collect();
        }
        self.query.order_by = self
            .query
            .order_by
            .iter()
            .map(|e| match e {
                Expr::Ordered { expr, descending, nulls_first } => Expr::Ordered { expr: expr.clone(), descending: !descending, nulls_first: nulls_first.map(|n| !n) },
                other => other.clone().desc(),
            })
            .collect();
        self.query.default_ordering = false;
        self
    }

    pub fn distinct(mut self) -> Self {
        self.query.distinct = true;
        self
    }

    /// `values(*fields)`: use [`values_rows`](Self::values_rows) to run it.
    pub fn values(mut self, fields: &[&str]) -> Self {
        let names: Vec<String> = if fields.is_empty() {
            self.query.model.concrete_fields().map(|f| f.attname()).collect()
        } else {
            fields.iter().map(|s| (*s).to_owned()).collect()
        };
        self.query.values_names = Some(names);
        self
    }

    /// `annotate(name=expr)`.
    pub fn annotate(mut self, name: &str, expr: Expr) -> Self {
        if contains_agg(&expr) {
            self.query.group_by = GroupBy::All;
        }
        self.query.annotations.insert(name.to_owned(), expr);
        self
    }

    pub fn select_related(mut self, fields: &[&str]) -> Self {
        self.query.select_related = if fields.is_empty() { SelectRelated::All } else { SelectRelated::Fields(fields.iter().map(|s| (*s).to_owned()).collect()) };
        self
    }

    pub fn prefetch_related(mut self, lookups: &[&str]) -> Self {
        self.prefetch.extend(lookups.iter().map(|s| (*s).to_owned()));
        self
    }

    pub fn only(mut self, fields: &[&str]) -> Self {
        self.query.deferred = Some((true, fields.iter().map(|s| (*s).to_owned()).collect()));
        self
    }

    pub fn defer(mut self, fields: &[&str]) -> Self {
        self.query.deferred = Some((false, fields.iter().map(|s| (*s).to_owned()).collect()));
        self
    }

    /// `none()`.
    pub fn none(mut self) -> Self {
        self.query.empty = true;
        self
    }

    /// `qs[low:high]`.
    pub fn slice(mut self, low: usize, high: Option<usize>) -> Self {
        let base = self.query.low_mark;
        self.query.low_mark = base + low;
        if let Some(h) = high {
            let new_high = base + h;
            self.query.high_mark = Some(self.query.high_mark.map_or(new_high, |old| old.min(new_high)));
        }
        self
    }

    pub fn select_for_update(mut self, mode: &str) -> Self {
        self.query.select_for_update = Some(mode.to_owned());
        self
    }

    fn combine(mut self, kind: &str, all: bool, others: Vec<Query>) -> Self {
        self.query.combinator = Some(Combinator { kind: kind.to_owned(), all, queries: others });
        self
    }

    pub fn union(self, other: QuerySet<M>, all: bool) -> Self {
        self.combine("UNION", all, vec![other.query])
    }

    pub fn intersection(self, other: QuerySet<M>) -> Self {
        self.combine("INTERSECT", false, vec![other.query])
    }

    pub fn difference(self, other: QuerySet<M>) -> Self {
        self.combine("EXCEPT", false, vec![other.query])
    }

    /// `ordered`: does the query have an ordering?
    pub fn ordered(&self) -> bool {
        !self.query.order_by.is_empty() || (self.query.default_ordering && !self.query.model.ordering.is_empty())
    }

    // ------------------------------------------------------------------ running

    /// The SQL this queryset would run (`str(qs.query)` with `%s` placeholders).
    pub fn sql(&self, vendor: crate::backends::Vendor) -> Result<Option<crate::models::sql::Compiled>> {
        self.check()?;
        Ok(self.query.compile_select(vendor)?.map(|(c, _)| c))
    }

    pub(crate) async fn fetch_data(&self, db: &dyn Executor) -> Result<(Vec<RowData>, SelectPlan, Vec<Row>)> {
        self.check()?;
        let vendor = db.vendor();
        let Some((compiled, plan)) = self.query.compile_select(vendor)? else { return Ok((vec![], SelectPlan::default(), vec![])) };
        let rows = db.query(&compiled.sql, &compiled.params).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(build_row_data(vendor, &plan, row)?);
        }
        Ok((out, plan, rows))
    }

    /// Evaluate the queryset into model instances (`list(qs)`).
    pub async fn fetch(&self, db: &dyn Executor) -> Result<Vec<M>> {
        let (mut data, _, _) = self.fetch_data(db).await?;
        if !self.prefetch.is_empty() {
            crate::models::prefetch::prefetch(db, &self.query.model, &mut data, &self.prefetch).await?;
        }
        data.into_iter().map(M::from_data).collect()
    }

    /// Evaluate a `values()` queryset into one map per row.
    pub async fn values_rows(&self, db: &dyn Executor) -> Result<Vec<IndexMap<String, Value>>> {
        self.check()?;
        let vendor = db.vendor();
        let Some((compiled, plan)) = self.query.compile_select(vendor)? else { return Ok(vec![]) };
        let rows = db.query(&compiled.sql, &compiled.params).await?;
        rows.iter()
            .map(|row| {
                let mut m = IndexMap::new();
                for (item, v) in plan.items.iter().zip(&row.values) {
                    match &item.kind {
                        SelectKind::Value { name, field } => {
                            let v = match field {
                                Some(f) => from_db(vendor, f, v.clone())?,
                                None => v.clone(),
                            };
                            m.insert(name.clone(), v);
                        }
                        SelectKind::Annotation(n) => {
                            m.insert(n.clone(), v.clone());
                        }
                        SelectKind::Field { field, .. } => {
                            m.insert(field.attname(), from_db(vendor, field, v.clone())?);
                        }
                    }
                }
                Ok(m)
            })
            .collect()
    }

    /// `values_list(*fields)`.
    pub async fn values_list(&self, db: &dyn Executor) -> Result<Vec<Vec<Value>>> {
        Ok(self.values_rows(db).await?.into_iter().map(|m| m.into_values().collect()).collect())
    }

    /// `values_list(field, flat=True)`.
    pub async fn values_flat<T: FromValue>(&self, db: &dyn Executor) -> Result<Vec<T>> {
        self.values_rows(db).await?.into_iter().map(|m| T::from_value(m.into_values().next().unwrap_or(Value::Null))).collect()
    }

    /// `get()`: exactly one match or `DoesNotExist` / `MultipleObjectsReturned`.
    pub async fn get(&self, db: &dyn Executor) -> Result<M> {
        let mut qs = self.clone();
        if qs.query.high_mark.is_none() && qs.query.low_mark == 0 {
            qs.query.order_by.clear();
            qs.query.default_ordering = false;
            qs.query.high_mark = Some(MAX_GET_RESULTS);
        }
        let mut found = qs.fetch(db).await?;
        let name = self.query.model.object_name.clone();
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(does_not_exist(&name)),
            n => Err(Error::MultipleObjectsReturned {
                model: Some(name.clone()),
                message: format!("get() returned more than one {name} -- it returned {}!", if n < MAX_GET_RESULTS { n.to_string() } else { format!("more than {}", MAX_GET_RESULTS - 1) }),
            }),
        }
    }

    /// `first()`.
    pub async fn first(&self, db: &dyn Executor) -> Result<Option<M>> {
        let qs = if self.ordered() { self.clone() } else { self.clone().order_by(&["pk"]) };
        Ok(qs.slice(0, Some(1)).fetch(db).await?.into_iter().next())
    }

    /// `last()`.
    pub async fn last(&self, db: &dyn Executor) -> Result<Option<M>> {
        let qs = if self.ordered() { self.clone() } else { self.clone().order_by(&["pk"]) };
        Ok(qs.reverse().slice(0, Some(1)).fetch(db).await?.into_iter().next())
    }

    /// `earliest(field)`.
    pub async fn earliest(&self, db: &dyn Executor, field: &str) -> Result<M> {
        self.clone().order_by(&[field]).slice(0, Some(1)).fetch(db).await?.into_iter().next().ok_or_else(|| does_not_exist(&self.query.model.object_name))
    }

    /// `latest(field)`.
    pub async fn latest(&self, db: &dyn Executor, field: &str) -> Result<M> {
        let desc = format!("-{field}");
        self.clone().order_by(&[&desc]).slice(0, Some(1)).fetch(db).await?.into_iter().next().ok_or_else(|| does_not_exist(&self.query.model.object_name))
    }

    /// `count()`.
    pub async fn count(&self, db: &dyn Executor) -> Result<usize> {
        self.check()?;
        let Some(c) = self.query.compile_count(db.vendor())? else { return Ok(0) };
        let rows = db.query(&c.sql, &c.params).await?;
        Ok(rows.first().and_then(|r| r.values.first()).and_then(Value::as_i64).unwrap_or(0) as usize)
    }

    /// `exists()`.
    pub async fn exists(&self, db: &dyn Executor) -> Result<bool> {
        self.check()?;
        let Some(c) = self.query.compile_exists(db.vendor())? else { return Ok(false) };
        Ok(!db.query(&c.sql, &c.params).await?.is_empty())
    }

    /// `aggregate(name=Expr, ...)`.
    pub async fn aggregate(&self, db: &dyn Executor, aggs: Vec<(&str, Expr)>) -> Result<IndexMap<String, Value>> {
        self.check()?;
        let mut q = self.query.clone();
        q.order_by.clear();
        q.default_ordering = false;
        q.select = Some(vec![]);
        q.values_names = None;
        q.annotations.clear();
        q.group_by = GroupBy::None;
        q.select_related = SelectRelated::None;
        for (n, e) in &aggs {
            q.annotations.insert((*n).to_owned(), e.clone());
        }
        let vendor = db.vendor();
        let Some((compiled, plan)) = q.compile_select(vendor)? else { return Ok(aggs.iter().map(|(n, _)| ((*n).to_owned(), Value::Null)).collect()) };
        let rows = db.query(&compiled.sql, &compiled.params).await?;
        let mut out = IndexMap::new();
        if let Some(row) = rows.first() {
            for (item, v) in plan.items.iter().zip(&row.values) {
                if let SelectKind::Annotation(n) = &item.kind {
                    out.insert(n.clone(), v.clone());
                }
            }
        }
        // Normalize numeric aggregates over decimal/date fields.
        for (n, e) in &aggs {
            if let (Expr::Agg { expr, .. }, Some(v)) = (e, out.get_mut(*n)) {
                if let Expr::F(path) = &**expr {
                    if let Ok(col) = self.query.clone().resolve_path(path, None) {
                        if matches!(col.field.ty, FieldType::Decimal | FieldType::Date | FieldType::DateTime | FieldType::Time) && !matches!(e, Expr::Agg { kind: crate::models::expressions::AggKind::Count, .. }) {
                            *v = from_db(vendor, &col.field, v.clone())?;
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// `in_bulk(id_list)`: map of pk → instance.
    pub async fn in_bulk(&self, db: &dyn Executor, ids: Vec<Value>) -> Result<IndexMap<String, M>> {
        if ids.is_empty() {
            return Ok(IndexMap::new());
        }
        let found = self.clone().filter_q(Q::is_in("pk", ids)).fetch(db).await?;
        Ok(found.into_iter().map(|m| (format!("{:?}", m.pk_value()), m)).collect())
    }

    // ------------------------------------------------------------------ writes

    /// `create(**kwargs)`: save a new instance.
    pub async fn create(&self, db: &dyn Executor, mut obj: M) -> Result<M> {
        obj.save_with(db, true, None).await?;
        Ok(obj)
    }

    /// `bulk_create(objs, batch_size=...)`; returns the number of rows inserted.
    pub async fn bulk_create(&self, db: &dyn Executor, mut objs: Vec<M>, batch_size: Option<usize>) -> Result<usize> {
        if objs.is_empty() {
            return Ok(0);
        }
        let meta = M::meta()?;
        let vendor = db.vendor();
        let fields: Vec<_> = meta.concrete_fields().filter(|f| !(f.primary_key && f.ty.is_auto() && objs[0].pk_value().is_null())).cloned().collect();
        let refs: Vec<&_> = fields.iter().collect();
        let per = batch_size.unwrap_or(999 / fields.len().max(1)).max(1);
        let mut total = 0;
        for chunk in objs.chunks_mut(per) {
            let mut params = Vec::new();
            for o in chunk.iter_mut() {
                o.pre_save(true);
                let data = o.to_data();
                params.extend(fields.iter().map(|f| prep_param(vendor, f, data.get(&f.attname()).cloned().unwrap_or(Value::Null))));
            }
            let sql = insert_sql(vendor, &meta, &refs, chunk.len(), false);
            total += db.execute(&sql, &params).await? as usize;
        }
        Ok(total)
    }

    /// `bulk_update(objs, fields)`.
    pub async fn bulk_update(&self, db: &dyn Executor, objs: &mut [M], fields: &[&str]) -> Result<usize> {
        let mut n = 0;
        for o in objs.iter_mut() {
            o.save_with(db, false, Some(fields)).await?;
            n += 1;
        }
        Ok(n)
    }

    /// `update(**kwargs)`: rows matched.
    pub async fn update(&self, db: &dyn Executor, values: Vec<(&str, UpdateValue)>) -> Result<u64> {
        self.check()?;
        let vals: Vec<(String, UpdateValue)> = values.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        let Some(c) = self.query.compile_update(db.vendor(), &vals)? else { return Ok(0) };
        db.execute(&c.sql, &c.params).await
    }

    /// `delete()`: `(total deleted, per model)`.
    pub async fn delete(&self, db: &dyn Executor) -> Result<(usize, IndexMap<String, usize>)> {
        self.check()?;
        let label = self.query.model.label();
        let reg = registry();
        let simple = reg.related_objects(&label).is_empty()
            && !crate::models::signals::PRE_DELETE.has_listeners(Some(&label))
            && !crate::models::signals::POST_DELETE.has_listeners(Some(&label));
        if simple {
            let Some(c) = self.query.compile_delete(db.vendor())? else { return Ok((0, IndexMap::new())) };
            let n = db.execute(&c.sql, &c.params).await? as usize;
            return Ok((n, IndexMap::from([(label, n)])));
        }
        let mut q = self.query.clone();
        q.select = None;
        q.values_names = Some(vec![self.query.model.pk().name.clone()]);
        q.order_by.clear();
        q.default_ordering = false;
        q.annotations.clear();
        let pks: Vec<Value> = QuerySet::<M> { query: q, prefetch: vec![], error: None, _m: PhantomData }
            .values_rows(db)
            .await?
            .into_iter()
            .filter_map(|m| m.into_values().next())
            .collect();
        delete_pks(db, &label, pks).await
    }

    /// `get_or_create(**lookup, defaults=...)`: `(object, created)`.
    pub async fn get_or_create(&self, db: &dyn Executor, lookup: Q, create: impl FnOnce() -> M + Send) -> Result<(M, bool)> {
        match self.clone().filter_q(lookup).get(db).await {
            Ok(o) => Ok((o, false)),
            Err(Error::ObjectDoesNotExist { .. }) => Ok((self.create(db, create()).await?, true)),
            Err(e) => Err(e),
        }
    }

    /// `update_or_create(defaults=...)`: `(object, created)`.
    pub async fn update_or_create(
        &self,
        db: &dyn Executor,
        lookup: Q,
        update: impl FnOnce(&mut M) + Send,
        create: impl FnOnce() -> M + Send,
    ) -> Result<(M, bool)> {
        match self.clone().filter_q(lookup).get(db).await {
            Ok(mut o) => {
                update(&mut o);
                o.save(db).await?;
                Ok((o, false))
            }
            Err(Error::ObjectDoesNotExist { .. }) => Ok((self.create(db, create()).await?, true)),
            Err(e) => Err(e),
        }
    }

    /// `explain()`.
    pub async fn explain(&self, db: &dyn Executor) -> Result<String> {
        self.check()?;
        let Some(c) = self.sql(db.vendor())? else { return Ok(String::new()) };
        let rows = db.query(&format!("EXPLAIN {}", c.sql), &c.params).await?;
        Ok(rows.iter().map(|r| r.values.iter().map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| format!("{v:?}"))).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n"))
    }

    /// `raw(sql, params)`: map result columns onto the model's fields by name.
    pub async fn raw(db: &dyn Executor, sql: &str, params: &[Value]) -> Result<Vec<M>> {
        let meta = M::meta()?;
        let vendor = db.vendor();
        let rows = db.query(sql, params).await?;
        rows.iter()
            .map(|row| {
                let mut data = RowData::default();
                for (col, v) in row.columns.iter().zip(&row.values) {
                    match meta.concrete_fields().find(|f| f.column() == *col || f.attname() == *col) {
                        Some(f) => {
                            data.fields.insert(f.attname(), from_db(vendor, f, v.clone())?);
                        }
                        None => {
                            data.annotations.insert(col.clone(), v.clone());
                        }
                    }
                }
                M::from_data(data)
            })
            .collect()
    }
}

/// Group a result row into instance data (base model + `select_related` models + annotations).
pub(crate) fn build_row_data(vendor: crate::backends::Vendor, plan: &SelectPlan, row: &Row) -> Result<RowData> {
    let mut data = RowData::default();
    let mut related: Vec<RowData> = plan.related.iter().map(|_| RowData::default()).collect();
    for (item, v) in plan.items.iter().zip(&row.values) {
        match &item.kind {
            SelectKind::Field { group: 0, field } => {
                data.fields.insert(field.attname(), from_db(vendor, field, v.clone())?);
            }
            SelectKind::Field { group, field } => {
                related[group - 1].fields.insert(field.attname(), from_db(vendor, field, v.clone())?);
            }
            SelectKind::Annotation(n) => {
                data.annotations.insert(n.clone(), v.clone());
            }
            SelectKind::Value { name, field } => {
                let v = match field {
                    Some(f) => from_db(vendor, f, v.clone())?,
                    None => v.clone(),
                };
                data.annotations.insert(name.clone(), v);
            }
        }
    }
    // Attach related rows (skip all-NULL rows from LEFT OUTER JOINs), deepest paths last.
    let mut order: Vec<usize> = (0..related.len()).collect();
    order.sort_by_key(|&i| plan.related[i].0.matches("__").count());
    for i in order {
        let rd = std::mem::take(&mut related[i]);
        if rd.fields.values().all(Value::is_null) {
            continue;
        }
        let path = &plan.related[i].0;
        let mut target = &mut data;
        let parts: Vec<&str> = path.split("__").collect();
        for (j, p) in parts.iter().enumerate() {
            if j + 1 == parts.len() {
                target.related.insert((*p).to_owned(), rd);
                break;
            }
            match target.related.get_mut(*p) {
                Some(t) => target = t,
                None => break,
            }
        }
    }
    Ok(data)
}

#[allow(dead_code)]
fn _unused(_: HashSet<String>, _: HashMap<String, String>, _: Rhs) {}
