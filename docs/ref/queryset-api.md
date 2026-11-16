# QuerySet API reference

Every public method of `QuerySet<M>`, the lookups and transforms a filter path accepts, the `Q` object, the typed
field handles and the expression functions. For explanations and examples see
[Making queries](../topics/db/queries.md), [Relations](../topics/db/relations.md) and
[Aggregation and expressions](../topics/db/aggregation.md).

```rust
use rango::db::models::{F, Model, Q, QuerySet};
use rango::db::models::expressions::{Avg, Count, Exists, Max, Min, OuterRef, Sum, V};
use rango::db::{Db, Executor, Value, filter};
```

`M` is the model type. `db` is `&dyn Executor` (a `Db` or a `Transaction`).

## Getting a queryset

| Call | Result |
|---|---|
| `M::objects()` | `QuerySet<M>` matching every row. |
| `QuerySet::<M>::new()`, `QuerySet::default()` | The same. |
| `QuerySet::<M>::for_label("app.Model")` | A queryset for the model with this label; an unknown label is reported by the terminal call as `Error::LookupError`. Mostly useful for `Dynamic` models. |

A `QuerySet` is `Clone`. Builder methods consume it and return a new one; terminal methods borrow it.

## Builder methods

Nothing here touches the database.

### Filtering

| Method | Meaning |
|---|---|
| `all(self) -> Self` | Returns the queryset unchanged. |
| `filter(self, cond: impl FnOnce(&M::Fields) -> Q) -> Self` | Keep rows matching the typed condition (`filter!(...)` or a closure). |
| `exclude(self, cond: impl FnOnce(&M::Fields) -> Q) -> Self` | Drop rows matching it. |
| `filter_kw(self, path: &str, value: impl ToValue) -> Self` | Keep rows where `path` (which may end in a lookup) matches `value`. |
| `exclude_kw(self, path: &str, value: impl ToValue) -> Self` | Drop them. |
| `filter_q(self, q: Q) -> Self` | Keep rows matching a `Q`. |
| `exclude_q(self, q: Q) -> Self` | Drop rows matching a `Q`. |
| `filter_path(self, path: &str, value: impl ToValue) -> Result<Self>` | `filter_kw` returning a `Result` (always `Ok`; errors surface later). |
| `filter_expr(self, path: &str, e: Expr) -> Self` | Compare `path` with an expression (`F`, arithmetic, `OuterRef`, ...). |
| `none(self) -> Self` | A queryset that matches nothing and never queries. |

Chained filters are combined with `AND`. `exclude*` keeps rows where the column is `NULL`.

### Ordering and slicing

| Method | Meaning |
|---|---|
| `order_by(self, fields: &[&str]) -> Self` | Replace the ordering. `"-name"` is descending, `"?"` is random, `&[]` clears all ordering, including the model's default. |
| `order_by_expr(self, exprs: Vec<Expr>) -> Self` | Order by expressions (`F("x").desc().nulls_last()`). |
| `reverse(self) -> Self` | Flip every direction (starting from the model's ordering if none is set). |
| `slice(self, low: usize, high: Option<usize>) -> Self` | `OFFSET low`, `LIMIT high - low`; relative to an earlier slice. |
| `distinct(self) -> Self` | `SELECT DISTINCT`. |

### Shaping the result

| Method | Meaning |
|---|---|
| `values(self, fields: &[&str]) -> Self` | Select named columns (all columns for `&[]`); run with `values_rows`, `values_list` or `values_flat`. Names may cross relations and name annotations. |
| `annotate(self, name: &str, expr: Expr) -> Self` | Add a computed column; an aggregate groups the result. |
| `only(self, fields: &[&str]) -> Self` | Select only these columns (and the key). |
| `defer(self, fields: &[&str]) -> Self` | Select every column except these. |
| `select_related(self, fields: &[&str]) -> Self` | Join foreign keys and one-to-ones; read with `ForeignKey::cached()`. `&[]` follows every non-null one. |
| `prefetch_related(self, lookups: &[&str]) -> Self` | Load many-to-many fields and foreign keys in extra queries; read with `ManyToMany::prefetched()` / `ForeignKey::cached()`. |
| `select_for_update(self, mode: &str) -> Self` | Append `FOR UPDATE <mode>` on PostgreSQL (`""`, `"NOWAIT"`, `"SKIP LOCKED"`). No effect on SQLite. |

### Combining

| Method | Meaning |
|---|---|
| `union(self, other: QuerySet<M>, all: bool) -> Self` | `UNION`, or `UNION ALL` when `all` is true. |
| `intersection(self, other: QuerySet<M>) -> Self` | `INTERSECT`. |
| `difference(self, other: QuerySet<M>) -> Self` | `EXCEPT`. |

## Inspection

| Method | Meaning |
|---|---|
| `ordered(&self) -> bool` | Whether the query has an ordering (explicit or the model's default). |
| `sql(&self, vendor: Vendor) -> Result<Option<Compiled>>` | The statement without running it: `Compiled { sql, params }`. `None` when the queryset can never match. Placeholders are `%s`. |
| `query(&self) -> &Query`, `into_query(self) -> Query` | The underlying query, for use in `Exists`, `Subquery` and `Rhs::Query`. |
| `explain(&self, db) -> Result<String>` | The database's plan (`EXPLAIN ...`) as text. |

## Terminal methods

All are `async`, borrow the queryset (`&self`) and take the connection first. Each call runs a new query.

### Reading

| Method | Returns | Notes |
|---|---|---|
| `fetch(&self, db)` | `Result<Vec<M>>` | All matching rows, with `prefetch_related` applied. |
| `get(&self, db)` | `Result<M>` | Exactly one row. `ObjectDoesNotExist` for none, `MultipleObjectsReturned` for more (looks at up to 21 rows). |
| `first(&self, db)` | `Result<Option<M>>` | First row; orders by primary key when unordered. |
| `last(&self, db)` | `Result<Option<M>>` | Last row, the same way. |
| `earliest(&self, db, field: &str)` | `Result<M>` | Row with the smallest `field`; `ObjectDoesNotExist` when empty. |
| `latest(&self, db, field: &str)` | `Result<M>` | Row with the largest `field`. |
| `count(&self, db)` | `Result<usize>` | `SELECT COUNT(*)`, or a counted subquery when the query is sliced, distinct, combined or grouped. |
| `exists(&self, db)` | `Result<bool>` | Whether any row matches. |
| `values_rows(&self, db)` | `Result<Vec<IndexMap<String, Value>>>` | One map per row of a `values` queryset. |
| `values_list(&self, db)` | `Result<Vec<Vec<Value>>>` | One list per row. |
| `values_flat::<T: FromValue>(&self, db)` | `Result<Vec<T>>` | First selected column of each row. |
| `aggregate(&self, db, aggs: Vec<(&str, Expr)>)` | `Result<IndexMap<String, Value>>` | One row of aggregate values; ignores annotations and ordering. |
| `in_bulk(&self, db, ids: Vec<Value>)` | `Result<IndexMap<String, M>>` | Rows by primary key; the map key is the debug form of the key value (`"Int(1)"`). Empty `ids` returns an empty map without a query. |
| `QuerySet::<M>::raw(db, sql: &str, params: &[Value])` | `Result<Vec<M>>` | Run SQL and map result columns to fields by column or attribute name; other columns are dropped. Associated function, no `self`. |

### Writing

| Method | Returns | Notes |
|---|---|---|
| `create(&self, db, obj: M)` | `Result<M>` | `INSERT`, even if the key is set. Sends save signals. |
| `bulk_create(&self, db, objs: Vec<M>, batch_size: Option<usize>)` | `Result<usize>` | Multi-row inserts; rows inserted. No signals; generated keys are not reported. Default batch: `999 / columns` rows. |
| `bulk_update(&self, db, objs: &mut [M], fields: &[&str])` | `Result<usize>` | Saves the listed fields of each object, one `UPDATE` each. |
| `update(&self, db, values: Vec<(&str, UpdateValue)>)` | `Result<u64>` | One `UPDATE` over the matching rows; rows matched. Values are plain values or expressions (`.into()`). Own columns only; no signals; `auto_now` untouched. |
| `delete(&self, db)` | `Result<(usize, IndexMap<String, usize>)>` | Delete with cascading; total and per-model counts. |
| `get_or_create(&self, db, lookup: Q, create: impl FnOnce() -> M + Send)` | `Result<(M, bool)>` | The row and whether it was created. |
| `update_or_create(&self, db, lookup: Q, update: impl FnOnce(&mut M) + Send, create: impl FnOnce() -> M + Send)` | `Result<(M, bool)>` | Update the existing row with `update` and save it, or insert `create()`. |

`UpdateValue` (in `rango::db::models::sql`) is `Val(Value)` or `Expr(Expr)`; any `ToValue` and any `Expr` convert
into it with `.into()`.

### Errors

| Error | When |
|---|---|
| `Error::ObjectDoesNotExist { model, message }` | `get`, `earliest`, `latest` found no row; `ForeignKey::get` on a null or missing target. |
| `Error::MultipleObjectsReturned { model, message }` | `get` found several rows. |
| `Error::FieldError(msg)` | Unknown field, unsupported lookup or transform, bad `select_related` path, unknown column in `update`. The message lists the valid names. |
| `Error::LookupError(msg)` | The model label is not registered. |
| `Error::ImproperlyConfigured(msg)` | Unknown database alias or engine. |
| `Error::Other(e)` | Driver and SQL errors, protected deletes (`ProtectedError`, `RestrictedError`), `prefetch_related` on an unknown name, failed value conversion. |

Mistakes in string paths and unsupported lookups are recorded when the queryset is built and returned when it runs.
Compile-checked filters (`filter!`, closures) avoid most of them. `get`, `first`, `last`, `earliest`, `latest`,
`in_bulk`, `get_or_create` and `update_or_create` run on a copy of the queryset, and a copy does not carry a recorded
build error: write the filters in front of those calls with `filter!` or a closure, or check them first with `count`
or `exists`, so that a mistyped path cannot be dropped silently.

## Lookups

A path ends in one of these names; without one the lookup is `exact`.

| Lookup | Value | SQL (SQLite / PostgreSQL) |
|---|---|---|
| `exact` | any; `None` becomes `IS NULL`; `true`/`false` on a boolean field test the column itself | `= v` |
| `iexact` | text | `LIKE v` / `UPPER(col) = UPPER(v)` |
| `contains`, `icontains` | text | `LIKE '%v%'` (`UPPER` for `i`) |
| `startswith`, `istartswith` | text | `LIKE 'v%'` |
| `endswith`, `iendswith` | text | `LIKE '%v'` |
| `gt`, `gte`, `lt`, `lte` | any ordered value | `>`, `>=`, `<`, `<=` |
| `in` | a list | `IN (...)`; an empty list matches nothing; `NULL`s in the list are dropped |
| `range` | two values | `BETWEEN a AND b` |
| `isnull` | `bool` | `IS NULL` / `IS NOT NULL` |
| `regex`, `iregex` | pattern | `REGEXP` (Rust `regex` syntax) / `~`, `~*` (POSIX) |

`%`, `_` and `\` in the value of a pattern lookup are escaped. The right-hand side can also be an expression
(`filter_expr`, `Q::compare_expr`) or a subquery (`Rhs::Query`). Filters on `year` compared with an integer become a
range over the column.

### Transforms

A path may apply transforms between the field and the lookup (`pub_date__year__gte`). Several can be chained.

| Transform | Applies to | Result |
|---|---|---|
| `year`, `iso_year`, `month`, `day`, `week`, `week_day`, `iso_week_day`, `quarter` | date, datetime | integer |
| `hour`, `minute`, `second` | datetime | integer |
| `date`, `time` | datetime | date, time (PostgreSQL only) |
| `lower`, `upper` | text | text |
| `length` | text | integer |

`week_day` is 1 (Sunday) to 7 (Saturday); `iso_week_day` is 1 (Monday) to 7 (Sunday); `week` is the ISO week.

### Relation paths

A path walks relations by field name (forward) or by the reverse query name, with `__` between steps; see
[Relations](../topics/db/relations.md#filtering-across-relations). `pk` stands for the primary key at any step.
`<field>_id` names the stored foreign key column. A path that ends on a relation compares its key; combine a relation
with `in` or `isnull` through the `_id` column.

## `Q` objects

`Q` (in `rango::db::models`) is a tree of conditions.

| Item | Meaning |
|---|---|
| `Q::new()` | Empty condition; the identity for `&` and `\|`. |
| `Q::lookup(path, value)` | `path` (with optional lookup) against a value. |
| `Q::is_in(path, values)` | `path__in` against an iterator of values. |
| `Q::compare_expr(path, expr)` | `path` against an expression. |
| `Q::expr(expr)` | A boolean expression, such as `Exists(...)`. |
| `Q::lookup_rhs(path, rhs)` | General form; `rhs` is `Rhs::Val`, `Rhs::List`, `Rhs::Range`, `Rhs::Expr`, `Rhs::Query`. |
| `q & r`, `q \| r`, `!q` | And, or, not (also `.and(r)`, `.or(r)`, `.not()`). |
| `q.is_empty()` | Whether it has no conditions. |

## Typed field handles

`#[derive(Model)]` on `Question` also generates `QuestionFields`, the argument of closure filters (`|f| ...`).

| Generated item | Meaning |
|---|---|
| `f.<field>()` | A `Path<T>` for a plain field. For a relation, the target model's `...Fields`, so paths chain: `f.question().question_text()`. |
| `f.<fk>_id()` | `Path<i64>` for the stored foreign key column. |
| `f.pk()` | `Path` of the primary key. |
| `f.eq(v)`, `f.is_in(vs)`, `f.isnull(b)` | Compare a relation handle itself (see the note on `_id` in [Relations](../topics/db/relations.md#filtering-across-relations)). |
| `f.reverse::<TargetFields>("query_name")` | Enter a reverse relation. |

`Path<T>` methods, each returning a `Q` unless noted:

| Method | Lookup |
|---|---|
| `eq(v)`, `exact(v)` | `exact` |
| `iexact(v)`, `contains(v)`, `icontains(v)`, `startswith(v)`, `istartswith(v)`, `endswith(v)`, `iendswith(v)` | the same-named lookups |
| `gt(v)`, `gte(v)`, `lt(v)`, `lte(v)` | comparisons |
| `regex(v)`, `iregex(v)` | pattern match |
| `isnull(b)` | null test |
| `is_in(values)` | `in` |
| `range(lo, hi)` | `range` |
| `cmp_expr(lookup, expr)` | `lookup` against an expression, e.g. `cmp_expr("gt", F("goal"))` |
| `year()`, `month()`, `day()`, `week_day()`, `quarter()`, `hour()`, `minute()`, `second()`, `length()` | transform, returns `Path<i64>` |
| `lower()`, `upper()` | transform, returns `Path<String>` |
| `asc()`, `desc()` | the string form for `order_by` (`"name"`, `"-name"`) |

### `filter!` and `q!`

```rust
Question::objects().filter(filter!(question_text__startswith = "What", id__in = [1, 2]));
Question::objects().filter(filter!((id = 1) | !(question_text = "Why?")));
```

`filter!(...)` (alias `q!`) expands to a closure over the field handles. Grammar: `key = value` conditions joined by
`,` or `&` (and), `|` (or), prefixed with `!` (not) and grouped with parentheses. The key is a forward field path
separated by `__`, optionally ending in one of `exact`, `iexact`, `contains`, `icontains`, `in`, `gt`, `gte`, `lt`,
`lte`, `startswith`, `istartswith`, `endswith`, `iendswith`, `isnull`, `regex`, `iregex`. A value ends at the next
top-level `,`, `&` or `|`. `range` and reverse relations cannot be written in the macro.

## Expressions

All in `rango::db::models::expressions` (`F` and `V` also in `rango::db::models`).

| Item | Meaning |
|---|---|
| `F("path")` | A column, or an annotation. |
| `V(value)` | A literal. Also `Expr::val(v)`, and `.into()` from any `ToValue`. |
| `OuterRef("path")` | A column of the enclosing queryset, inside `Exists` / `Subquery`. |
| `Count("path")`, `Count("*")`, `Sum`, `Avg`, `Min`, `Max` | Aggregates. |
| `expr.distinct()`, `expr.filter(q)` | Distinct and conditional aggregates. |
| `Lower`, `Upper`, `Length`, `Abs`, `Trim`, `LTrim`, `RTrim`, `Ceil`, `Floor`, `Sqrt` | One-argument functions. |
| `Coalesce(vec)`, `Greatest(vec)`, `Least(vec)`, `Concat(vec)` | Variadic functions. |
| `Substr(e, pos, Option<len>)` | Substring. |
| `Cast(e, FieldType)` | `CAST`. |
| `Case(vec![(Q, Expr)], Option<Expr>)` | `CASE WHEN`. |
| `Exists(Query)`, `Subquery(Query)` | Subquery expressions. |
| `Expr::func(name, args)`, `Expr::Raw(sql, params)` | Any function, or raw SQL. |
| `a + b`, `a - b`, `a * b`, `a / b`, `a % b`, `-a` | Arithmetic (also `.add`, `.sub`, `.mul`, `.div`, `.rem`). |
| `expr.asc()`, `.desc()`, `.nulls_first()`, `.nulls_last()` | Ordering expressions. |

`Expr` variants: `F`, `Val`, `OuterRef`, `Func`, `Combined`, `Neg`, `Case`, `Agg`, `Star`, `Subquery`, `Exists`,
`Cast`, `Ordered`, `Raw`. `AggKind`: `Count`, `Sum`, `Avg`, `Min`, `Max`, `StdDev`, `Variance`. `Op`: `Add`, `Sub`,
`Mul`, `Div`, `Mod`, `Pow`, `BitAnd`, `BitOr`, `BitLeftShift`, `BitRightShift`.

## Model methods

From the `Model` trait (derived):

| Method | Meaning |
|---|---|
| `M::objects() -> QuerySet<M>` | All rows. |
| `M::label() -> String` | `app_label.ModelName`. |
| `M::meta() -> Result<Arc<ModelMeta>>` | The model's metadata. |
| `save(&mut self, db)` | Insert or update; sends `PRE_SAVE` / `POST_SAVE`. |
| `save_with(&mut self, db, force_insert: bool, update_fields: Option<&[&str]>)` | `save` with options. |
| `delete(&self, db)` | Delete with cascading; `(total, per-model counts)`. |
| `refresh_from_db(&mut self, db)` | Reload every column by key. |
| `pk_value(&self) -> Value`, `set_pk(&mut self, Value)` | The key as a `Value`. |
| `to_data(&self)`, `M::from_data(RowData)` | Conversion to and from database values. |
| `pre_save(&mut self, created: bool)` | Assigns `auto_now` / `auto_now_add` fields. |

Signals are statics in `rango::db::models::signals`: `PRE_SAVE`, `POST_SAVE`, `PRE_DELETE`, `POST_DELETE` (payload
`ModelEvent { label, pk, created, update_fields, values, using }`) and `M2M_CHANGED` (payload
`M2mEvent { action, label, field, pk, pk_set }`). The sender is the model label. See [Signals](../topics/signals.md).

## Not supported yet

- Window functions and `DISTINCT ON`.
- Result caching and streaming; every terminal call loads the whole result.
- `select_for_update` on SQLite.
- Loading deferred columns after the fact.
- A subquery form of `exclude` across to-many relations.

## See also

- [Making queries](../topics/db/queries.md)
- [Relations](../topics/db/relations.md)
- [Aggregation and expressions](../topics/db/aggregation.md)
- [Transactions](../topics/db/transactions.md)
- [Model field reference](model-fields.md)
