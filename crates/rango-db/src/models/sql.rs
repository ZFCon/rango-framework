//! SQL generation (the `SQLCompiler`, `WhereNode.as_sql`, lookup SQL, backend operators).
//!
//! Placeholders are `%s` (like the `sql_with_params()`); backends convert them.

use std::collections::{HashMap, HashSet};

use rango_core::conf::try_settings;
use rango_core::exceptions::{Error, Result};

use crate::backends::Vendor;
use crate::models::expressions::{AggKind, Expr, Op};
use crate::models::fields_db::prep_param;
use crate::models::lookups::like_escape;
use crate::models::meta::{Field, FieldType, ModelMeta};
use crate::models::q::{Connector, Rhs};
use crate::models::query::{Col, GroupBy, JoinType, Lhs, Query, ResolvedLookup, SelectRelated, WhereChild, WhereNode};
use crate::value::{ToValue, Value};

/// A compiled statement.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Value>,
}

/// Why a WHERE node produced no SQL.
enum Sql {
    Text(String, Vec<Value>),
    /// `EmptyResultSet`: can never match.
    Nothing,
    /// `FullResultSet`: always matches (condition can be dropped).
    Everything,
}

/// What one selected column maps back to.
#[derive(Clone, Debug)]
pub enum SelectKind {
    /// Column of model instance `group` (0 = base model, n = select_related models in order).
    Field { group: usize, field: Field },
    Annotation(String),
    /// An expression/`values()` entry named `name`.
    Value { name: String, field: Option<Field> },
}

#[derive(Clone, Debug)]
pub struct SelectItem {
    pub kind: SelectKind,
}

/// The select layout of a compiled query (needed to build model instances from rows).
#[derive(Clone, Debug, Default)]
pub struct SelectPlan {
    pub items: Vec<SelectItem>,
    /// Related models loaded by `select_related`: (relation name path, model label).
    pub related: Vec<(String, String)>,
}

pub struct Compiler<'a> {
    pub q: &'a mut Query,
    pub vendor: Vendor,
    rename: HashMap<String, String>,
    outer: Option<&'a OuterCtx>,
    having_parts: Vec<(String, Vec<Value>)>,
    number_cols: bool,
}

/// Enough of the enclosing query to resolve `OuterRef`.
pub struct OuterCtx {
    pub model: std::sync::Arc<ModelMeta>,
    pub base_alias: String,
}

fn q_name(vendor: Vendor, n: &str) -> String {
    vendor.quote(n)
}

fn type_of(field: &Field) -> FieldType {
    field.ty
}

impl<'a> Compiler<'a> {
    pub fn new(q: &'a mut Query, vendor: Vendor) -> Self {
        Compiler { q, vendor, rename: HashMap::new(), outer: None, having_parts: Vec::new(), number_cols: false }
    }

    fn alias(&self, a: &str) -> String {
        self.rename.get(a).cloned().unwrap_or_else(|| a.to_owned())
    }

    /// `quote_name_unless_alias`: generated aliases (`T3`, `U0`) stay bare, table names are quoted.
    fn quote_alias(&self, orig: &str) -> String {
        let a = self.alias(orig);
        let generated = self.q.alias_map.get(orig).is_some_and(|j| j.table != orig);
        if generated || self.rename.contains_key(orig) { a } else { q_name(self.vendor, &a) }
    }

    fn col_sql(&self, c: &Col) -> String {
        format!("{}.{}", self.quote_alias(&c.alias), q_name(self.vendor, &c.column()))
    }

    // ------------------------------------------------------------------ expressions

    fn expr_sql(&mut self, e: &Expr, params: &mut Vec<Value>) -> Result<(String, Option<Field>)> {
        Ok(match e {
            Expr::F(path) => {
                if let Some(a) = self.q.annotations.get(path).cloned() {
                    if !self.q.model.fields.iter().any(|f| f.name == *path) {
                        return self.expr_sql(&a, params);
                    }
                }
                let col = self.q.resolve_path(path, None)?;
                (self.col_sql(&col), Some(col.field.clone()))
            }
            Expr::Val(v) => {
                params.push(v.clone());
                ("%s".into(), None)
            }
            Expr::OuterRef(path) => {
                let outer = self.outer.ok_or_else(|| Error::field_error("OuterRef used outside of a subquery".to_owned()))?;
                let name = if path == "pk" { outer.model.pk().name.clone() } else { path.clone() };
                let f = outer
                    .model
                    .get_field(&name)
                    .ok_or_else(|| Error::field_error(format!("Cannot resolve outer reference '{path}'")))?
                    .clone();
                (format!("({}.{})", q_name(self.vendor, &outer.base_alias), q_name(self.vendor, &f.column())), Some(f))
            }
            Expr::Func { name, args } => {
                let mut parts = Vec::new();
                let mut first = None;
                for a in args {
                    let (s, f) = self.expr_sql(a, params)?;
                    first = first.or(f);
                    parts.push(s);
                }
                (self.func_sql(name, &parts), first)
            }
            Expr::Combined(l, op, r) => {
                let (ls, lf) = self.expr_sql(l, params)?;
                let (rs, _) = self.expr_sql(r, params)?;
                let sql = if *op == Op::Pow && self.vendor == Vendor::Sqlite {
                    format!("POWER({ls}, {rs})")
                } else {
                    format!("{ls} {} {rs}", op.sql())
                };
                (format!("({sql})"), lf)
            }
            Expr::Neg(inner) => {
                let (s, f) = self.expr_sql(inner, params)?;
                (format!("-{s}"), f)
            }
            Expr::Case { whens, default } => {
                let mut sql = String::from("CASE");
                for (cond, then) in whens {
                    let w = self.q_to_where(cond)?;
                    let (ws, wp) = match self.where_sql(&w)? {
                        Sql::Text(s, p) => (s, p),
                        Sql::Everything => ("1=1".to_owned(), vec![]),
                        Sql::Nothing => continue,
                    };
                    params.extend(wp);
                    let (ts, _) = self.expr_sql(then, params)?;
                    sql.push_str(&format!(" WHEN {ws} THEN {ts}"));
                }
                if let Some(d) = default {
                    let (ds, _) = self.expr_sql(d, params)?;
                    sql.push_str(&format!(" ELSE {ds}"));
                }
                sql.push_str(" END");
                (sql, None)
            }
            Expr::Agg { kind, expr, distinct, filter } => {
                let (inner, field) = self.expr_sql(expr, params)?;
                let inner = if matches!(**expr, Expr::Star) { "*".to_owned() } else { inner };
                let d = if *distinct { "DISTINCT " } else { "" };
                let sql = if let Some(fq) = filter {
                    let w = self.q_to_where(fq)?;
                    match self.where_sql(&w)? {
                        Sql::Text(ws, wp) => {
                            if self.vendor == Vendor::Postgresql {
                                params.extend(wp);
                                format!("{}({d}{inner}) FILTER (WHERE {ws})", kind.function())
                            } else {
                                // SQLite < 3.30 compatibility form used by the reference implementation: CASE WHEN ... THEN x ELSE NULL END
                                params.extend(wp);
                                format!("{}({d}CASE WHEN {ws} THEN {inner} ELSE NULL END)", kind.function())
                            }
                        }
                        _ => format!("{}({d}{inner})", kind.function()),
                    }
                } else {
                    format!("{}({d}{inner})", kind.function())
                };
                let out_field = match kind {
                    AggKind::Count => Some(Field::new("count", FieldType::Integer)),
                    AggKind::Avg => Some(Field::new("avg", FieldType::Float)),
                    _ => field,
                };
                (sql, out_field)
            }
            Expr::Star => ("*".into(), None),
            Expr::Subquery(sub) => {
                let (sql, p) = self.compile_subquery(sub, false)?;
                params.extend(p);
                (format!("({sql})"), None)
            }
            Expr::Exists(sub) => {
                let (sql, p) = self.compile_subquery(sub, true)?;
                params.extend(p);
                (format!("EXISTS({sql})"), Some(Field::new("exists", FieldType::Boolean)))
            }
            Expr::Cast(inner, ty) => {
                let (s, _) = self.expr_sql(inner, params)?;
                let db_type = match (self.vendor, ty) {
                    (_, FieldType::Integer) => "integer",
                    (_, FieldType::BigInteger) => "bigint",
                    (Vendor::Sqlite, FieldType::Float) => "real",
                    (_, FieldType::Float) => "double precision",
                    (Vendor::Sqlite, FieldType::Decimal) => "decimal",
                    (_, FieldType::Decimal) => "numeric",
                    (Vendor::Sqlite, FieldType::Char | FieldType::Text) => "text",
                    (_, FieldType::Char) => "varchar",
                    (_, FieldType::Text) => "text",
                    (_, FieldType::Boolean) => "bool",
                    (_, FieldType::Date) => "date",
                    _ => "text",
                };
                (format!("CAST({s} AS {db_type})"), Some(Field::new("cast", *ty)))
            }
            Expr::Ordered { expr, .. } => self.expr_sql(expr, params)?,
            Expr::Raw(sql, p) => {
                params.extend(p.clone());
                (sql.clone(), None)
            }
        })
    }

    fn func_sql(&self, name: &str, args: &[String]) -> String {
        match (name, self.vendor) {
            ("CONCAT", Vendor::Sqlite) => format!("({})", args.join(" || ")),
            ("CONCAT", Vendor::Postgresql) => format!("CONCAT({})", args.join(", ")),
            ("GREATEST", Vendor::Sqlite) => format!("MAX({})", args.join(", ")),
            ("LEAST", Vendor::Sqlite) => format!("MIN({})", args.join(", ")),
            ("CEILING", Vendor::Sqlite) => format!("CEIL({})", args.join(", ")),
            _ => format!("{name}({})", args.join(", ")),
        }
    }

    // ------------------------------------------------------------------ subqueries

    fn compile_subquery(&mut self, sub: &Query, exists: bool) -> Result<(String, Vec<Value>)> {
        let base = self.q.base_alias();
        let outer = OuterCtx { model: self.q.model.clone(), base_alias: self.alias(&base) };
        let mut q = sub.clone();
        q.subquery = true;
        q.alias_prefix = if self.q.alias_prefix == "U" { "V".into() } else { "U".into() };
        let mut c = Compiler { q: &mut q, vendor: self.vendor, rename: HashMap::new(), outer: Some(&outer), having_parts: Vec::new(), number_cols: false };
        let (sql, p) = if exists { c.exists_sql_inner()? } else { c.select_sql()?.map(|(s, p, _)| (s, p)).unwrap_or(("SELECT 1 WHERE 0".into(), vec![])) };
        Ok((sql, p))
    }

    // ------------------------------------------------------------------ where tree

    pub fn q_to_where(&mut self, q: &crate::models::q::Q) -> Result<WhereNode> {
        // Build a temporary where tree for Q objects used inside expressions.
        let saved = std::mem::take(&mut self.q.where_);
        let used = self.q.used_aliases.clone();
        let r = self.q.add_q(q);
        let node = std::mem::replace(&mut self.q.where_, saved);
        self.q.used_aliases = used;
        r?;
        Ok(node)
    }

    fn where_sql(&mut self, node: &WhereNode) -> Result<Sql> {
        let mut parts: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let full_count = node.children.len();
        let mut empty_children = 0;
        let mut full_children = 0;
        for child in &node.children {
            let r = match child {
                WhereChild::Lookup(l) => self.lookup_sql(l)?,
                WhereChild::Node(n) => self.where_sql(n)?,
                WhereChild::Expr(e) => {
                    let mut p = Vec::new();
                    let (s, _) = self.expr_sql(e, &mut p)?;
                    Sql::Text(s, p)
                }
                WhereChild::Nothing => Sql::Nothing,
            };
            match r {
                Sql::Text(s, p) => {
                    parts.push(s);
                    params.extend(p);
                }
                Sql::Nothing => {
                    empty_children += 1;
                    if node.connector == Connector::And {
                        break;
                    }
                }
                Sql::Everything => {
                    full_children += 1;
                    if node.connector == Connector::Or {
                        break;
                    }
                }
            }
        }
        // the reference implementation: AND with an empty child is empty; OR with a full child is full.
        let (matches_nothing, matches_everything) = match (node.connector, node.negated) {
            (Connector::And, false) => (empty_children > 0, false),
            (Connector::Or, false) => (empty_children == full_count && full_count > 0, full_children > 0),
            (Connector::And, true) => (false, empty_children > 0),
            (Connector::Or, true) => (full_children > 0, empty_children == full_count && full_count > 0),
        };
        if matches_nothing && !node.negated {
            return Ok(Sql::Nothing);
        }
        if matches_everything && !node.negated {
            return Ok(Sql::Everything);
        }
        if node.negated {
            if empty_children > 0 && node.connector == Connector::And {
                return Ok(Sql::Everything);
            }
            if full_children > 0 && node.connector == Connector::Or {
                return Ok(Sql::Nothing);
            }
        }
        if parts.is_empty() {
            return Ok(if node.negated { Sql::Nothing } else { Sql::Everything });
        }
        let joiner = if node.connector == Connector::And { " AND " } else { " OR " };
        let mut sql = parts.join(joiner);
        if node.negated {
            sql = format!("NOT ({sql})");
        } else if parts.len() > 1 {
            sql = format!("({sql})");
        }
        Ok(Sql::Text(sql, params))
    }

    fn year_bounds(&self, field: &Field, year: i64) -> Result<(Value, Value)> {
        use chrono::{NaiveDate, TimeZone, Utc};
        let (first, last) = (
            NaiveDate::from_ymd_opt(year as i32, 1, 1).ok_or_else(|| Error::field_error("year out of range".to_owned()))?,
            NaiveDate::from_ymd_opt(year as i32, 12, 31).ok_or_else(|| Error::field_error("year out of range".to_owned()))?,
        );
        if field.ty == FieldType::Date {
            return Ok((Value::Date(first), Value::Date(last)));
        }
        let (f, l) = (first.and_hms_opt(0, 0, 0).expect("valid"), last.and_hms_micro_opt(23, 59, 59, 999_999).expect("valid"));
        let use_tz = try_settings().map(|s| s.use_tz).unwrap_or(true);
        if !use_tz {
            return Ok((Value::NaiveDateTime(f), Value::NaiveDateTime(l)));
        }
        let tz = rango_core::utils::timezone::get_current_timezone().unwrap_or(rango_core::utils::timezone::Tz::UTC);
        let aware = |n| rango_core::utils::timezone::make_aware(n, tz).with_timezone(&Utc);
        let _ = Utc.timestamp_opt(0, 0);
        Ok((Value::DateTime(aware(f)), Value::DateTime(aware(l))))
    }

    fn transform_sql(&self, name: &str, inner_sql: &str, field: &Field) -> (String, Option<FieldType>) {
        let is_dt = matches!(field.ty, FieldType::DateTime);
        match name {
            "lower" => (format!("LOWER({inner_sql})"), Some(field.ty)),
            "upper" => (format!("UPPER({inner_sql})"), Some(field.ty)),
            "length" => (format!("LENGTH({inner_sql})"), Some(FieldType::Integer)),
            "date" => match self.vendor {
                Vendor::Sqlite => (format!("rango_datetime_cast_date({inner_sql}, 'UTC', 'UTC')"), Some(FieldType::Date)),
                Vendor::Postgresql => (format!("({inner_sql} AT TIME ZONE 'UTC')::date"), Some(FieldType::Date)),
            },
            "time" => (format!("({inner_sql} AT TIME ZONE 'UTC')::time"), Some(FieldType::Time)),
            part => {
                let ty = Some(FieldType::Integer);
                match self.vendor {
                    Vendor::Sqlite => {
                        if is_dt {
                            (format!("rango_datetime_extract('{part}', {inner_sql}, 'UTC', 'UTC')"), ty)
                        } else {
                            (format!("rango_date_extract('{part}', {inner_sql})"), ty)
                        }
                    }
                    Vendor::Postgresql => {
                        let src = if is_dt { format!("{inner_sql} AT TIME ZONE 'UTC'") } else { inner_sql.to_owned() };
                        let sql = match part {
                            "week_day" => format!("EXTRACT('dow' FROM {src}) + 1"),
                            "iso_week_day" => format!("EXTRACT('isodow' FROM {src})"),
                            "iso_year" => format!("EXTRACT('isoyear' FROM {src})"),
                            p => format!("EXTRACT('{p}' FROM {src})"),
                        };
                        (sql, ty)
                    }
                }
            }
        }
    }

    fn lhs_sql(&mut self, lhs: &Lhs, params: &mut Vec<Value>) -> Result<(String, Field)> {
        Ok(match lhs {
            Lhs::Col(c) => (self.col_sql(c), c.field.clone()),
            Lhs::Transform { name, inner } => {
                let (isql, f) = self.lhs_sql(inner, params)?;
                let (sql, ty) = self.transform_sql(name, &isql, &f);
                let mut out = f.clone();
                if let Some(t) = ty {
                    out.ty = t;
                    out.rel = None;
                }
                (sql, out)
            }
            Lhs::Expr(e) => {
                let (s, f) = self.expr_sql(e, params)?;
                (s, f.unwrap_or_else(|| Field::new("expr", FieldType::Integer)))
            }
        })
    }

    fn lookup_sql(&mut self, l: &ResolvedLookup) -> Result<Sql> {
        let mut params = Vec::new();
        let vendor = self.vendor;
        // year__<cmp> optimisation on date(time) fields.
        if let Lhs::Transform { name, inner } = &l.lhs {
            if name == "year" {
                if let (Some(c), Rhs::Val(Value::Int(y))) = (inner.base_col(), &l.rhs) {
                    if matches!(c.field.ty, FieldType::Date | FieldType::DateTime) && matches!(l.lookup.as_str(), "exact" | "gt" | "gte" | "lt" | "lte") {
                        let (first, last) = self.year_bounds(&c.field, *y)?;
                        let lhs = self.col_sql(c);
                        let (op, ps): (&str, Vec<Value>) = match l.lookup.as_str() {
                            "exact" => ("BETWEEN %s AND %s", vec![first, last]),
                            "gt" => ("> %s", vec![last]),
                            "gte" => (">= %s", vec![first]),
                            "lt" => ("< %s", vec![first]),
                            _ => ("<= %s", vec![last]),
                        };
                        params.extend(ps.into_iter().map(|p| prep_param(vendor, &c.field, p)));
                        return Ok(Sql::Text(format!("{lhs} {op}"), params));
                    }
                }
            }
        }
        let (lhs_sql, field) = self.lhs_sql(&l.lhs, &mut params)?;
        if let Lhs::Expr(e) = &l.lhs {
            let agg = match e {
                Expr::F(name) => self.q.annotations.get(name).is_some_and(contains_agg),
                other => contains_agg(other),
            };
            if agg {
                // Aggregate conditions belong in HAVING.
                let mut inner = l.clone();
                inner.lhs = Lhs::Expr(Expr::Raw("\u{0}AGG".into(), vec![]));
                let _ = inner;
                let r = self.lookup_sql_with_lhs(l, lhs_sql, field, params)?;
                if let Sql::Text(s, p) = r {
                    self.having_parts.push((s, p));
                    return Ok(Sql::Everything);
                }
                return Ok(r);
            }
        }
        self.lookup_sql_with_lhs(l, lhs_sql, field, params)
    }

    fn lookup_sql_with_lhs(&mut self, l: &ResolvedLookup, lhs_sql: String, field: Field, mut params: Vec<Value>) -> Result<Sql> {
        let vendor = self.vendor;
        let pg_cast = |s: &str, lookup: &str| -> String {
            if vendor != Vendor::Postgresql {
                return s.to_owned();
            }
            let base = match lookup {
                "iexact" | "contains" | "icontains" | "startswith" | "istartswith" | "endswith" | "iendswith" | "regex" | "iregex" => {
                    if field.ty == FieldType::GenericIpAddress { format!("HOST({s})") } else { format!("{s}::text") }
                }
                _ => s.to_owned(),
            };
            if matches!(lookup, "iexact" | "icontains" | "istartswith" | "iendswith") { format!("UPPER({base})") } else { base }
        };
        let prep = |v: Value| prep_param(vendor, &field, v);

        // rhs as SQL (expression) or parameter value.
        macro_rules! rhs_expr {
            () => {{
                match &l.rhs {
                    Rhs::Expr(e) => match (&**e, &l.rhs_col) {
                        (Expr::F(_), Some(c)) => Some((format!("({})", self.col_sql(c)), Vec::<Value>::new())),
                        (e, _) => {
                            let mut p = Vec::new();
                            let (s, _) = self.expr_sql(e, &mut p)?;
                            Some((if s.starts_with('(') { s } else { format!("({s})") }, p))
                        }
                    },
                    Rhs::Query(sub) => {
                        let (s, p) = self.compile_subquery(sub, false)?;
                        Some((format!("({s})"), p))
                    }
                    _ => None,
                }
            }};
        }

        let lookup = l.lookup.as_str();
        match lookup {
            "isnull" => {
                let is_null = matches!(&l.rhs, Rhs::Val(Value::Bool(true)));
                Ok(Sql::Text(format!("{lhs_sql} IS {}NULL", if is_null { "" } else { "NOT " }), params))
            }
            "in" => {
                if let Some((s, p)) = rhs_expr!() {
                    params.extend(p);
                    return Ok(Sql::Text(format!("{lhs_sql} IN {s}"), params));
                }
                let Rhs::List(vals) = &l.rhs else { return Err(Error::field_error("'in' lookup needs a list".to_owned())) };
                let vals: Vec<&Value> = vals.iter().filter(|v| !v.is_null()).collect();
                if vals.is_empty() {
                    return Ok(Sql::Nothing);
                }
                params.extend(vals.iter().map(|v| prep((*v).clone())));
                Ok(Sql::Text(format!("{lhs_sql} IN ({})", vec!["%s"; vals.len()].join(", ")), params))
            }
            "range" => {
                let Rhs::Range(a, b) = &l.rhs else { return Err(Error::field_error("'range' lookup needs two values".to_owned())) };
                params.push(prep(a.clone()));
                params.push(prep(b.clone()));
                Ok(Sql::Text(format!("{lhs_sql} BETWEEN %s AND %s"), params))
            }
            "exact" | "gt" | "gte" | "lt" | "lte" => {
                let op = match lookup {
                    "exact" => "=",
                    "gt" => ">",
                    "gte" => ">=",
                    "lt" => "<",
                    _ => "<=",
                };
                if let Some((s, p)) = rhs_expr!() {
                    params.extend(p);
                    return Ok(Sql::Text(format!("{lhs_sql} {op} {s}"), params));
                }
                // BooleanField exact True/False compares the column itself.
                if lookup == "exact" && field.ty == FieldType::Boolean {
                    if let Rhs::Val(Value::Bool(b)) = &l.rhs {
                        return Ok(Sql::Text(if *b { lhs_sql } else { format!("NOT {lhs_sql}") }, params));
                    }
                }
                let Rhs::Val(v) = &l.rhs else { return Err(Error::field_error(format!("bad right-hand side for '{lookup}'"))) };
                params.push(prep(coerce_for_field(&field, v.clone())));
                Ok(Sql::Text(format!("{lhs_sql} {op} %s"), params))
            }
            "regex" | "iregex" => {
                let Rhs::Val(v) = &l.rhs else { return Err(Error::field_error("regex needs a value".to_owned())) };
                params.push(v.clone());
                let lhs = pg_cast(&lhs_sql, lookup);
                Ok(Sql::Text(
                    match (vendor, lookup) {
                        (Vendor::Sqlite, "regex") => format!("{lhs} REGEXP %s"),
                        (Vendor::Sqlite, _) => format!("{lhs} REGEXP '(?i)' || %s"),
                        (_, "regex") => format!("{lhs} ~ %s"),
                        (_, _) => format!("{lhs} ~* %s"),
                    },
                    params,
                ))
            }
            "iexact" | "contains" | "icontains" | "startswith" | "istartswith" | "endswith" | "iendswith" => {
                let lhs = pg_cast(&lhs_sql, lookup);
                let esc = if vendor == Vendor::Sqlite { " ESCAPE '\\'" } else { "" };
                if let Some((s, p)) = rhs_expr!() {
                    // Expression right-hand side: escape on the database side.
                    let pattern_esc = if vendor == Vendor::Sqlite {
                        format!("REPLACE(REPLACE(REPLACE({s}, '\\', '\\\\'), '%%', '\\%%'), '_', '\\_')")
                    } else {
                        format!("REPLACE(REPLACE(REPLACE({s}, E'\\\\', E'\\\\\\\\'), E'%%', E'\\\\%%'), E'_', E'\\\\_')")
                    };
                    params.extend(p);
                    let up = |x: &str| if lookup.starts_with('i') { format!("UPPER({x})") } else { x.to_owned() };
                    let tail = match lookup {
                        "iexact" => return Ok(Sql::Text(if vendor == Vendor::Sqlite { format!("{lhs} LIKE {s}{esc}") } else { format!("{lhs} = UPPER({s})") }, params)),
                        "contains" | "icontains" => format!("LIKE '%%' || {} || '%%'{}", up(&pattern_esc), if vendor == Vendor::Sqlite { " ESCAPE '\\'" } else { "" }),
                        "startswith" | "istartswith" => format!("LIKE {} || '%%'{}", up(&pattern_esc), if vendor == Vendor::Sqlite { " ESCAPE '\\'" } else { "" }),
                        _ => format!("LIKE '%%' || {}{}", up(&pattern_esc), if vendor == Vendor::Sqlite { " ESCAPE '\\'" } else { "" }),
                    };
                    return Ok(Sql::Text(format!("{lhs} {tail}"), params));
                }
                let Rhs::Val(v) = &l.rhs else { return Err(Error::field_error(format!("bad right-hand side for '{lookup}'"))) };
                let text = match v {
                    Value::Str(s) => s.clone(),
                    other => crate::models::fields_db::adapt_to_text(other),
                };
                if lookup == "iexact" && vendor == Vendor::Postgresql {
                    params.push(Value::Str(text));
                    return Ok(Sql::Text(format!("{lhs} = UPPER(%s)"), params));
                }
                let escaped = like_escape(&text);
                params.push(Value::Str(match lookup {
                    "iexact" => escaped,
                    "contains" | "icontains" => format!("%{escaped}%"),
                    "startswith" | "istartswith" => format!("{escaped}%"),
                    _ => format!("%{escaped}"),
                }));
                let op = if vendor == Vendor::Postgresql && lookup.starts_with('i') { "LIKE UPPER(%s)".to_owned() } else { format!("LIKE %s{esc}") };
                Ok(Sql::Text(format!("{lhs} {op}"), params))
            }
            other => Err(Error::field_error(format!("Unsupported lookup '{other}'"))),
        }
    }

    // ------------------------------------------------------------------ select

    /// Compute the select layout and (SQL, params) of a SELECT query. `None` = EmptyResultSet.
    pub fn select_sql(&mut self) -> Result<Option<(String, Vec<Value>, SelectPlan)>> {
        if self.q.empty {
            return Ok(None);
        }
        if let Some(comb) = self.q.combinator.clone() {
            return self.combined_sql(&comb);
        }
        let vendor = self.vendor;
        let base = self.q.base_alias();
        if self.q.subquery {
            // Subquery aliases are renamed to U0, U1, ... to avoid clashing with the outer query.
            let prefix = self.q.alias_prefix.clone();
            self.rename = self.q.alias_map.keys().enumerate().map(|(i, a)| (a.clone(), format!("{prefix}{i}"))).collect();
        }
        let mut plan = SelectPlan::default();
        let mut select_params: Vec<Value> = Vec::new();
        let mut cols: Vec<String> = Vec::new();
        let mut group_cols: Vec<String> = Vec::new();
        let mut has_agg = false;

        // Columns.
        if let Some(names) = self.q.values_names.clone() {
            for name in &names {
                if let Some(a) = self.q.annotations.get(name).cloned() {
                    let mut p = Vec::new();
                    let (s, f) = self.expr_sql(&a, &mut p)?;
                    select_params.extend(p);
                    has_agg |= contains_agg(&a);
                    cols.push(format!("{s} AS {}", q_name(vendor, name)));
                    if !contains_agg(&a) {
                        group_cols.push(s);
                    }
                    plan.items.push(SelectItem { kind: SelectKind::Value { name: name.clone(), field: f } });
                } else {
                    let col = self.q.resolve_path(name, None)?;
                    let s = self.col_sql(&col);
                    cols.push(format!("{s} AS {}", q_name(vendor, name)));
                    group_cols.push(s);
                    plan.items.push(SelectItem { kind: SelectKind::Value { name: name.clone(), field: Some(col.field.clone()) } });
                }
            }
        } else if let Some(sel) = self.q.select.clone() {
            for (col, _) in &sel {
                let s = self.col_sql(col);
                cols.push(s.clone());
                group_cols.push(s);
                plan.items.push(SelectItem { kind: SelectKind::Field { group: 0, field: col.field.clone() } });
            }
        } else {
            let model = self.q.model.clone();
            let deferred = self.q.deferred.clone();
            for f in model.concrete_fields() {
                if let Some((only, names)) = &deferred {
                    let listed = names.iter().any(|n| *n == f.name || *n == f.attname());
                    if (*only && !listed && !f.primary_key) || (!*only && listed && !f.primary_key) {
                        continue;
                    }
                }
                let s = format!("{}.{}", self.quote_alias(&base), q_name(vendor, &f.column()));
                cols.push(s.clone());
                group_cols.push(s);
                plan.items.push(SelectItem { kind: SelectKind::Field { group: 0, field: f.clone() } });
            }
            // select_related
            let related = self.select_related_paths()?;
            for (i, (path, label)) in related.iter().enumerate() {
                let meta = self.q.registry.get_or_err(label)?.clone();
                let col_alias = self.alias_for_related_path(path)?;
                for f in meta.concrete_fields() {
                    let s = format!("{}.{}", self.quote_alias(&col_alias), q_name(vendor, &f.column()));
                    cols.push(s.clone());
                    group_cols.push(s);
                    plan.items.push(SelectItem { kind: SelectKind::Field { group: i + 1, field: f.clone() } });
                }
                plan.related.push((path.clone(), label.clone()));
            }
        }
        // Annotations (not in values mode: values() handles its own names).
        if self.q.values_names.is_none() {
            let names: Vec<String> = match &self.q.annotation_mask {
                Some(m) => m.clone(),
                None => self.q.annotations.keys().cloned().collect(),
            };
            for name in names {
                let Some(a) = self.q.annotations.get(&name).cloned() else { continue };
                let mut p = Vec::new();
                let (s, f) = self.expr_sql(&a, &mut p)?;
                select_params.extend(p);
                has_agg |= contains_agg(&a);
                cols.push(format!("{s} AS {}", q_name(vendor, &name)));
                let _ = f;
                plan.items.push(SelectItem { kind: SelectKind::Annotation(name.clone()) });
            }
        }

        // WHERE / HAVING
        let where_node = self.q.where_.clone();
        let (where_sql, where_params) = match self.where_sql(&where_node)? {
            Sql::Nothing => return Ok(None),
            Sql::Everything => (String::new(), vec![]),
            Sql::Text(s, p) => (s, p),
        };
        let mut extra_where = Vec::new();
        let mut extra_params = Vec::new();
        for (s, p) in &self.q.extra_where {
            extra_where.push(s.clone());
            extra_params.extend(p.clone());
        }
        let having = self.q.having.clone();
        let mut having_sql = Vec::new();
        let mut having_params = Vec::new();
        for (hs, hp) in std::mem::take(&mut self.having_parts) {
            having_sql.push(hs);
            having_params.extend(hp);
        }
        for h in &having {
            let mut p = Vec::new();
            let (s, _) = self.expr_sql(h, &mut p)?;
            having_sql.push(s);
            having_params.extend(p);
        }

        // ORDER BY
        let (order_sql, order_params, order_cols) = self.order_by_sql()?;

        // GROUP BY
        let mut group_sql = String::new();
        if has_agg || matches!(self.q.group_by, GroupBy::All) {
            let mut g = group_cols.clone();
            for oc in &order_cols {
                if !g.contains(oc) {
                    g.push(oc.clone());
                }
            }
            if vendor.group_by_pk_only() && self.q.values_names.is_none() && self.q.select.is_none() {
                let pk = self.q.model.pk().column();
                g = vec![format!("{}.{}", self.quote_alias(&base), q_name(vendor, &pk))];
            }
            if !g.is_empty() {
                group_sql = format!(" GROUP BY {}", g.join(", "));
            }
        }

        // FROM + joins (after every path has been resolved).
        let from_sql = self.from_sql();

        if self.number_cols {
            cols = cols.into_iter().enumerate().map(|(i, c)| if c.contains(" AS ") { c } else { format!("{c} AS {}", q_name(vendor, &format!("col{}", i + 1))) }).collect();
        }
        let mut sql = String::from("SELECT ");
        if self.q.distinct {
            sql.push_str("DISTINCT ");
        }
        sql.push_str(&cols.join(", "));
        sql.push_str(&format!(" FROM {from_sql}"));
        let mut all_where = Vec::new();
        if !where_sql.is_empty() {
            all_where.push(where_sql);
        }
        all_where.extend(extra_where);
        if !all_where.is_empty() {
            sql.push_str(&format!(" WHERE {}", all_where.join(" AND ")));
        }
        sql.push_str(&group_sql);
        if !having_sql.is_empty() {
            sql.push_str(&format!(" HAVING {}", having_sql.join(" AND ")));
        }
        if !order_sql.is_empty() {
            sql.push_str(&format!(" ORDER BY {order_sql}"));
        }
        sql.push_str(&self.limit_sql());
        if let Some(mode) = &self.q.select_for_update {
            if vendor == Vendor::Postgresql {
                sql.push_str(&format!(" FOR UPDATE{}", if mode.is_empty() { String::new() } else { format!(" {mode}") }));
            }
        }
        let mut params = select_params;
        params.extend(where_params);
        params.extend(extra_params);
        params.extend(having_params);
        params.extend(order_params);
        Ok(Some((sql, params, plan)))
    }

    fn limit_sql(&self) -> String {
        let (low, high) = (self.q.low_mark, self.q.high_mark);
        match (high, low) {
            (None, 0) => String::new(),
            (Some(h), l) => format!(" LIMIT {}{}", h - l, if l > 0 { format!(" OFFSET {l}") } else { String::new() }),
            (None, l) => match self.vendor {
                Vendor::Sqlite => format!(" LIMIT -1 OFFSET {l}"),
                Vendor::Postgresql => format!(" OFFSET {l}"),
            },
        }
    }

    fn select_related_paths(&mut self) -> Result<Vec<(String, String)>> {
        let sr = self.q.select_related.clone();
        let mut out = Vec::new();
        match sr {
            SelectRelated::None => {}
            SelectRelated::All => {
                let model = self.q.model.clone();
                for f in model.fields.iter().filter(|f| matches!(f.ty, FieldType::ForeignKey | FieldType::OneToOne) && !f.null) {
                    let target = self.q.registry.target_of(&model, f)?.label();
                    out.push((f.name.clone(), target));
                }
            }
            SelectRelated::Fields(names) => {
                for path in names {
                    let mut opts = self.q.model.clone();
                    let mut walked = Vec::new();
                    for part in path.split("__") {
                        let f = opts
                            .get_field(part)
                            .filter(|f| matches!(f.ty, FieldType::ForeignKey | FieldType::OneToOne))
                            .cloned()
                            .ok_or_else(|| Error::field_error(format!("Invalid field name(s) given in select_related: '{part}'. Choices are the relation fields of {}", opts.object_name)))?;
                        walked.push(part.to_owned());
                        let target = self.q.registry.target_of(&opts, &f)?.clone();
                        let p = walked.join("__");
                        if !out.iter().any(|(q, _): &(String, String)| *q == p) {
                            out.push((p, target.label()));
                        }
                        opts = target;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Ensure joins exist for a select_related path and return the final alias.
    fn alias_for_related_path(&mut self, path: &str) -> Result<String> {
        let names: Vec<String> = path.split("__").map(str::to_owned).collect();
        let before = self.q.alias_map.len();
        let (alias, _opts, consumed, _) = self.q.setup_joins(&names, None)?;
        if consumed != names.len() {
            return Err(Error::field_error(format!("Invalid select_related path '{path}'")));
        }
        // select_related on nullable FKs must be LEFT OUTER (setup_joins already does that).
        let _ = before;
        Ok(alias)
    }

    fn order_by_sql(&mut self) -> Result<(String, Vec<Value>, Vec<String>)> {
        let vendor = self.vendor;
        let mut items = self.q.order_by.clone();
        if items.is_empty() && self.q.default_ordering {
            // Meta.ordering applies unless the query aggregates.
            let aggregating = self.q.annotations.values().any(contains_agg);
            if !aggregating {
                for o in self.q.model.ordering.clone() {
                    items.push(parse_ordering(&o));
                }
            }
        }
        let mut parts = Vec::new();
        let mut params = Vec::new();
        let mut cols = Vec::new();
        for item in items {
            let (expr, desc, nulls) = match item {
                Expr::Ordered { expr, descending, nulls_first } => (*expr, descending, nulls_first),
                e => (e, false, None),
            };
            let (mut s, _) = match &expr {
                Expr::F(p) if p == "?" => ("RANDOM()".to_owned(), None),
                Expr::F(p) if p == "pk" => {
                    let col = self.q.resolve_path("pk", None)?;
                    (self.col_sql(&col), Some(col.field))
                }
                other => {
                    let mut p = Vec::new();
                    let r = self.expr_sql(other, &mut p)?;
                    params.extend(p);
                    r
                }
            };
            let is_agg = contains_agg(&expr) || matches!(&expr, Expr::F(p) if self.q.annotations.get(p).is_some_and(contains_agg));
            if !cols.contains(&s) && !is_agg && !matches!(&expr, Expr::F(p) if p == "?") {
                cols.push(s.clone());
            }
            s.push_str(if desc { " DESC" } else { " ASC" });
            if let Some(nf) = nulls {
                s.push_str(if nf { " NULLS FIRST" } else { " NULLS LAST" });
            }
            parts.push(s);
        }
        let _ = vendor;
        Ok((parts.join(", "), params, cols))
    }

    fn from_sql(&self) -> String {
        let vendor = self.vendor;
        let mut out = String::new();
        for (alias, j) in &self.q.alias_map {
            let a = self.alias(alias);
            match j.join_type {
                JoinType::Base => {
                    out.push_str(&q_name(vendor, &j.table));
                    if a != j.table {
                        out.push(' ');
                        out.push_str(&a);
                    }
                }
                jt => {
                    if j.refcount == 0 && jt == JoinType::Inner {
                        continue;
                    }
                    let parent = self.quote_alias(j.parent_alias.as_deref().unwrap_or(""));
                    let own = self.quote_alias(alias);
                    let on: Vec<String> = j
                        .join_cols
                        .iter()
                        .map(|(lc, rc)| format!("{parent}.{} = {own}.{}", q_name(vendor, lc), q_name(vendor, rc)))
                        .collect();
                    let kind = if jt == JoinType::LeftOuter { "LEFT OUTER JOIN" } else { "INNER JOIN" };
                    out.push_str(&format!(" {kind} {}", q_name(vendor, &j.table)));
                    if a != j.table {
                        out.push(' ');
                        out.push_str(&a);
                    }
                    out.push_str(&format!(" ON ({})", on.join(" AND ")));
                }
            }
        }
        out
    }

    fn combined_sql(&mut self, comb: &crate::models::query::Combinator) -> Result<Option<(String, Vec<Value>, SelectPlan)>> {
        let mut parts = Vec::new();
        let mut params = Vec::new();
        let mut plan = None;
        let mut first = self.q.clone();
        first.combinator = None;
        let all: Vec<Query> = std::iter::once(first).chain(comb.queries.clone()).collect();
        for mut sub in all {
            let mut c = Compiler { q: &mut sub, vendor: self.vendor, rename: HashMap::new(), outer: None, having_parts: Vec::new(), number_cols: true };
            let Some((sql, p, pl)) = c.select_sql()? else {
                if comb.kind == "INTERSECT" {
                    return Ok(None);
                }
                continue;
            };
            plan.get_or_insert(pl);
            parts.push(if self.vendor == Vendor::Postgresql { format!("({sql})") } else { sql });
            params.extend(p);
        }
        if parts.is_empty() {
            return Ok(None);
        }
        let op = format!(" {}{} ", comb.kind, if comb.all { " ALL" } else { "" });
        let (order_sql, op_params, _) = self.order_by_sql()?;
        let mut sql = parts.join(&op);
        if !order_sql.is_empty() {
            sql.push_str(&format!(" ORDER BY {order_sql}"));
            params.extend(op_params);
        }
        sql.push_str(&self.limit_sql());
        Ok(Some((sql, params, plan.unwrap_or_default())))
    }

    fn exists_sql_inner(&mut self) -> Result<(String, Vec<Value>)> {
        let mut q = self.q.clone();
        q.order_by.clear();
        q.default_ordering = false;
        q.select = Some(vec![]);
        q.annotations.clear();
        q.values_names = None;
        q.distinct = false;
        q.low_mark = 0;
        q.high_mark = Some(1);
        let prefix = self.q.alias_prefix.clone();
        q.base_alias();
        let mut c = Compiler { q: &mut q, vendor: self.vendor, rename: HashMap::new(), outer: self.outer, having_parts: Vec::new(), number_cols: false };
        if q_is_subquery(c.q) {
            c.rename = c.q.alias_map.keys().enumerate().map(|(i, a)| (a.clone(), format!("{prefix}{i}"))).collect();
        }
        let where_node = c.q.where_.clone();
        let (wsql, wparams) = match c.where_sql(&where_node)? {
            Sql::Nothing => return Ok((format!("SELECT 1 AS {} WHERE 0=1", q_name(self.vendor, "a")), vec![])),
            Sql::Everything => (String::new(), vec![]),
            Sql::Text(s, p) => (s, p),
        };
        let from = c.from_sql();
        let mut sql = format!("SELECT %s AS {} FROM {from}", q_name(self.vendor, "a"));
        if !wsql.is_empty() {
            sql.push_str(&format!(" WHERE {wsql}"));
        }
        sql.push_str(" LIMIT 1");
        let mut params = vec![Value::Int(1)];
        params.extend(wparams);
        Ok((sql, params))
    }
}

fn q_is_subquery(q: &Query) -> bool {
    q.subquery
}

/// Coerce a user value to the field's type for comparison parameters.
fn coerce_for_field(field: &Field, v: Value) -> Value {
    match (field.ty, v) {
        (t, Value::Str(s)) if t.is_integer() || matches!(t, FieldType::ForeignKey | FieldType::OneToOne) => s.parse::<i64>().map(Value::Int).unwrap_or(Value::Str(s)),
        (FieldType::Boolean, Value::Int(i)) => Value::Bool(i != 0),
        (_, v) => v,
    }
}

/// `"-name"` → ordering expression.
pub fn parse_ordering(s: &str) -> Expr {
    if let Some(rest) = s.strip_prefix('-') {
        Expr::F(rest.to_owned()).desc()
    } else if let Some(rest) = s.strip_prefix('+') {
        Expr::F(rest.to_owned()).asc()
    } else {
        Expr::F(s.to_owned()).asc()
    }
}

pub fn contains_agg(e: &Expr) -> bool {
    match e {
        Expr::Agg { .. } => true,
        Expr::Func { args, .. } => args.iter().any(contains_agg),
        Expr::Combined(a, _, b) => contains_agg(a) || contains_agg(b),
        Expr::Neg(a) | Expr::Cast(a, _) => contains_agg(a),
        Expr::Case { whens, default } => whens.iter().any(|(_, e)| contains_agg(e)) || default.as_deref().is_some_and(contains_agg),
        Expr::Ordered { expr, .. } => contains_agg(expr),
        _ => false,
    }
}

// ---------------------------------------------------------------------- statement builders

impl Query {
    /// `SELECT ...` for this query (with the row layout). `None` means the query can never match.
    pub fn compile_select(&self, vendor: Vendor) -> Result<Option<(Compiled, SelectPlan)>> {
        let mut q = self.clone();
        let mut c = Compiler::new(&mut q, vendor);
        Ok(c.select_sql()?.map(|(sql, params, plan)| (Compiled { sql, params }, plan)))
    }

    /// `COUNT(*)` for this query.
    pub fn compile_count(&self, vendor: Vendor) -> Result<Option<Compiled>> {
        let mut q = self.clone();
        let needs_subquery = q.distinct || q.high_mark.is_some() || q.low_mark > 0 || q.combinator.is_some() || q.annotations.values().any(contains_agg) || matches!(q.group_by, GroupBy::All);
        if needs_subquery {
            q.order_by.clear();
            q.default_ordering = false;
            let mut c = Compiler::new(&mut q, vendor);
            let Some((inner, params, _)) = c.select_sql()? else { return Ok(None) };
            return Ok(Some(Compiled { sql: format!("SELECT COUNT(*) FROM ({inner}) subquery"), params }));
        }
        q.order_by.clear();
        q.default_ordering = false;
        q.select = Some(vec![]);
        q.annotations.clear();
        q.values_names = None;
        q.select_related = SelectRelated::None;
        q.annotation_mask = None;
        q.annotations.insert("__count".into(), Expr::Agg { kind: AggKind::Count, expr: Box::new(Expr::Star), distinct: false, filter: None });
        let mut c = Compiler::new(&mut q, vendor);
        Ok(c.select_sql()?.map(|(sql, params, _)| Compiled { sql, params }))
    }

    /// `SELECT 1 AS "a" ... LIMIT 1` (`exists()`).
    pub fn compile_exists(&self, vendor: Vendor) -> Result<Option<Compiled>> {
        let mut q = self.clone();
        let mut c = Compiler::new(&mut q, vendor);
        let (sql, params) = c.exists_sql_inner()?;
        if sql.ends_with("WHERE 0=1") {
            return Ok(None);
        }
        Ok(Some(Compiled { sql, params }))
    }

    /// `DELETE FROM table WHERE ...` for a simple (join-free) filter; otherwise `pk IN (subquery)`.
    pub fn compile_delete(&self, vendor: Vendor) -> Result<Option<Compiled>> {
        let model = self.model.clone();
        let mut q = self.clone();
        q.order_by.clear();
        q.default_ordering = false;
        let mut c = Compiler::new(&mut q, vendor);
        let base = c.q.base_alias();
        let where_node = c.q.where_.clone();
        let (wsql, wparams) = match c.where_sql(&where_node)? {
            Sql::Nothing => return Ok(None),
            Sql::Everything => (String::new(), vec![]),
            Sql::Text(s, p) => (s, p),
        };
        let joins = c.q.alias_map.values().filter(|j| j.join_type != JoinType::Base && j.refcount > 0).count();
        let table = q_name(vendor, &model.db_table);
        if joins == 0 {
            // Un-alias the base table in the WHERE clause (DELETE FROM "t" WHERE "t"."x" = ...).
            let _ = base;
            let sql = if wsql.is_empty() { format!("DELETE FROM {table}") } else { format!("DELETE FROM {table} WHERE {wsql}") };
            return Ok(Some(Compiled { sql, params: wparams }));
        }
        let mut pk_q = self.clone();
        pk_q.select = None;
        pk_q.values_names = Some(vec![model.pk().name.clone()]);
        pk_q.subquery = true;
        pk_q.order_by.clear();
        pk_q.default_ordering = false;
        let mut pc = Compiler::new(&mut pk_q, vendor);
        let Some((inner, params, _)) = pc.select_sql()? else { return Ok(None) };
        Ok(Some(Compiled { sql: format!("DELETE FROM {table} WHERE {} IN ({inner})", q_name(vendor, &model.pk().column())), params }))
    }

    /// `UPDATE table SET ... WHERE ...`.
    pub fn compile_update(&self, vendor: Vendor, values: &[(String, UpdateValue)]) -> Result<Option<Compiled>> {
        let model = self.model.clone();
        let mut q = self.clone();
        q.order_by.clear();
        q.default_ordering = false;
        let mut c = Compiler::new(&mut q, vendor);
        let mut sets = Vec::new();
        let mut set_params = Vec::new();
        for (name, v) in values {
            let f = model.get_field(name).filter(|f| f.concrete()).ok_or_else(|| Error::field_error(format!("Cannot resolve keyword '{name}' into field.")))?.clone();
            match v {
                UpdateValue::Val(val) => {
                    set_params.push(prep_param(vendor, &f, val.clone()));
                    sets.push(format!("{} = %s", q_name(vendor, &f.column())));
                }
                UpdateValue::Expr(e) => {
                    let mut p = Vec::new();
                    let (s, _) = c.expr_sql_unaliased(e, &mut p)?;
                    set_params.extend(p);
                    sets.push(format!("{} = {s}", q_name(vendor, &f.column())));
                }
            }
        }
        let where_node = c.q.where_.clone();
        let (wsql, wparams) = match c.where_sql(&where_node)? {
            Sql::Nothing => return Ok(None),
            Sql::Everything => (String::new(), vec![]),
            Sql::Text(s, p) => (s, p),
        };
        let joins = c.q.alias_map.values().filter(|j| j.join_type != JoinType::Base && j.refcount > 0).count();
        let table = q_name(vendor, &model.db_table);
        let (wsql, wparams) = if joins > 0 {
            let mut pk_q = self.clone();
            pk_q.values_names = Some(vec![model.pk().name.clone()]);
            pk_q.subquery = true;
            pk_q.order_by.clear();
            pk_q.default_ordering = false;
            let mut pc = Compiler::new(&mut pk_q, vendor);
            let Some((inner, params, _)) = pc.select_sql()? else { return Ok(None) };
            (format!("{} IN ({inner})", q_name(vendor, &model.pk().column())), params)
        } else {
            (wsql, wparams)
        };
        let mut params = set_params;
        params.extend(wparams);
        let sql = if wsql.is_empty() { format!("UPDATE {table} SET {}", sets.join(", ")) } else { format!("UPDATE {table} SET {} WHERE {wsql}", sets.join(", ")) };
        Ok(Some(Compiled { sql, params }))
    }
}

/// Right-hand side of an `update()` assignment.
#[derive(Clone, Debug)]
pub enum UpdateValue {
    Val(Value),
    Expr(Expr),
}

impl From<Expr> for UpdateValue {
    fn from(e: Expr) -> Self {
        UpdateValue::Expr(e)
    }
}

impl<T: ToValue> From<T> for UpdateValue {
    fn from(v: T) -> Self {
        UpdateValue::Val(v.to_value())
    }
}

impl<'a> Compiler<'a> {
    /// Expression SQL where column references are written without the table alias qualifier
    /// issues for UPDATE SET (they keep the table-qualified form, which both backends accept).
    fn expr_sql_unaliased(&mut self, e: &Expr, params: &mut Vec<Value>) -> Result<(String, Option<Field>)> {
        self.expr_sql(e, params)
    }
}

/// Used by QuerySet to build INSERT statements.
pub fn insert_sql(vendor: Vendor, model: &ModelMeta, fields: &[&Field], rows: usize, returning_pk: bool) -> String {
    let table = q_name(vendor, &model.db_table);
    if fields.is_empty() {
        let mut sql = if vendor == Vendor::Sqlite { format!("INSERT INTO {table} DEFAULT VALUES") } else { format!("INSERT INTO {table} DEFAULT VALUES") };
        if returning_pk && vendor == Vendor::Postgresql {
            sql.push_str(&format!(" RETURNING {}", q_name(vendor, &model.pk().column())));
        }
        return sql;
    }
    let cols: Vec<String> = fields.iter().map(|f| q_name(vendor, &f.column())).collect();
    let one = format!("({})", vec!["%s"; fields.len()].join(", "));
    let values = vec![one; rows].join(", ");
    let mut sql = format!("INSERT INTO {table} ({}) VALUES {values}", cols.join(", "));
    if returning_pk && vendor == Vendor::Postgresql {
        sql.push_str(&format!(" RETURNING {}", q_name(vendor, &model.pk().column())));
    }
    sql
}

/// Hide a used-but-unreferenced helper from warnings in release builds.
#[allow(dead_code)]
fn _unused(_: HashSet<String>) {}
