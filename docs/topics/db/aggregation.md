# Aggregation and expressions

Expressions let the database compute values: add one to a column, count related rows, total a price, pick between two
values. This page covers `F` and the other expression builders, `annotate` and `aggregate`, filtering on aggregates
(`HAVING`), grouping, conditional aggregates and subqueries.

The examples use the `Question`/`Choice` and `Author`/`Book` models from [Relations](relations.md); `Choice` has a
`votes: i32` column and `Book` has a `price: Decimal` column.

```rust
use rango::db::models::expressions::{
    Abs, Avg, Case, Coalesce, Concat, Count, Exists, F, Lower, Max, Min, OuterRef, Sum, Upper, V,
};
use rango::db::models::{Model, Q};
use rango::db::Db;

let db = Db::default_db();
```

## Expressions

An expression is a value of the `Expr` enum. Build one with the functions below; everything that accepts an
expression takes an `Expr`, and anything with a `ToValue` implementation converts into one with `.into()` (a literal
becomes a bound parameter, never part of the SQL text).

### Column references: `F`

`F("votes")` refers to a column of the queryset's model. The path may cross relations: `F("question__question_text")`
or `F("choice__votes")`. It can also name an annotation.

```rust
use rango::db::models::{Model, F};

// Atomic increment: the database evaluates votes + 1, no read-modify-write race.
Choice::objects()
    .filter_kw("question", 1)
    .update(&db, vec![("votes", (F("votes") + 1).into())])
    .await?;

// Compare two columns.
let ahead = Choice::objects().filter_expr("votes__gt", F("question_id") * 5).fetch(&db).await?;
```

Arithmetic works with the operators `+ - * / %` and unary `-`, or with the methods `.add()`, `.sub()`, `.mul()`,
`.div()`, `.rem()`. The right-hand side can be another expression or a plain value:

```rust
let expr = (F("votes") + F("bonus")) * 2 - 1;
let neg = -F("votes");
```

`Op` also has `Pow` and bitwise variants (`BitAnd`, `BitOr`, `BitLeftShift`, `BitRightShift`) in
`rango::db::models::expressions`; there are no operator methods for them, so build the node with
`Expr::Combined(Box::new(left), Op::Pow, Box::new(right))` when you need one.

### Values: `V`

`V(value)` wraps a literal as an expression, for example to give `Case` a branch value or to put text between columns:

```rust
let label = Concat(vec![Upper(F("name")), V(" - "), Lower(F("name"))]);
```

### Functions

All take expressions (or anything that converts into one) and return an `Expr`:

| Function | SQL | Notes |
|---|---|---|
| `Lower(e)`, `Upper(e)` | `LOWER`, `UPPER` | |
| `Length(e)` | `LENGTH` | |
| `Trim(e)`, `LTrim(e)`, `RTrim(e)` | `TRIM`, `LTRIM`, `RTRIM` | |
| `Abs(e)` | `ABS` | |
| `Ceil(e)`, `Floor(e)`, `Sqrt(e)` | `CEILING`, `FLOOR`, `SQRT` | `CEILING` is written `CEIL` on SQLite; these need a SQLite build with the math functions enabled. |
| `Coalesce(vec![...])` | `COALESCE` | First non-null argument. |
| `Greatest(vec![...])`, `Least(vec![...])` | `GREATEST`, `LEAST` | Written `MAX(a, b)` / `MIN(a, b)` on SQLite. |
| `Concat(vec![...])` | `CONCAT` | Written `(a || b)` on SQLite. |
| `Substr(e, pos, Some(len))` | `SUBSTR` | `pos` is 1-based; `len` is optional. |
| `Cast(e, FieldType::X)` | `CAST(e AS ...)` | Targets: `Integer`, `BigInteger`, `Float`, `Decimal`, `Char`, `Text`, `Boolean`, `Date`; anything else casts to text. |
| `Case(vec![(Q, Expr), ...], Some(default))` | `CASE WHEN ... THEN ... ELSE ... END` | Conditions are `Q` objects. |
| `Expr::func("NAME", vec![...])` | `NAME(...)` | Any other function the backend has. |
| `Expr::Raw(sql, params)` | verbatim | Escape hatch; use `%s` for parameters. |

```rust
use rango::db::models::expressions::{Case, Coalesce, V};
use rango::db::models::Q;

let tier = Case(vec![(Q::lookup("votes__gte", 100), V("popular"))], Some(V("quiet")));
let age = Coalesce(vec![F("age"), V(0)]);
```

Expressions can also order rows: `.asc()`, `.desc()`, `.nulls_first()` and `.nulls_last()` return ordering
expressions for `order_by_expr` (see [Making queries](queries.md#ordering)).

## Annotations

`annotate(name, expr)` adds a computed value to every row of the queryset. The value can be used in later `filter`,
`order_by` and `values` calls under its name, and in other annotations.

```rust
// How many choices does each question have?
let rows = Question::objects()
    .annotate("n", Count("choice"))
    .order_by(&["-n", "id"])
    .values(&["id", "n"])
    .values_rows(&db)
    .await?;
assert_eq!(rows[0]["n"], rango::db::Value::Int(2));
```

Two rules shape how annotations are used:

1. **Read annotations through `values`.** A derived model struct has no field for them, so `fetch` returns the
   instances without the computed value. Name the annotation in `values(&[...])` and run `values_rows`, `values_list`
   or `values_flat`. The annotation must be listed explicitly; `values(&[])` selects only real columns.
2. **Annotations are not part of `get`, `first`, ...** Those return model instances. Use `.values(...)` first when you
   need the number.

Annotations that are not aggregates are evaluated per row and can be filtered in `WHERE`:

```rust
let rows = Choice::objects()
    .annotate("score", F("votes") * 2)
    .filter_kw("score__gt", 10)
    .order_by(&["-score"])
    .values(&["id", "score"])
    .values_rows(&db)
    .await?;
```

A name that equals a real field of the model is not treated as an annotation reference.

### Counting related rows

`Count("relation")` over a reverse relation counts the related rows; parents with none get `0` because the join is a
`LEFT OUTER` join.

```rust
let totals = Question::objects()
    .annotate("n_choices", Count("choice"))
    .annotate("n_votes", Sum("choice__votes"))
    .values(&["id", "n_choices", "n_votes"])
    .values_rows(&db)
    .await?;
```

`Sum("choice__votes")` is `NULL` for a parent with no choices; wrap it in `Coalesce(vec![Sum("choice__votes"), V(0)])`
to get zero.

Several aggregates over **different** many-valued relations in one queryset multiply each other's rows (each join
repeats the rows of the other). Count distinct keys in that case:

```rust
Question::objects().annotate("n", Count("choice").distinct());
```

### Grouping

Annotating an aggregate groups the result. Without `values`, rows are grouped by every selected column of the model
(on PostgreSQL by primary key only). With `values`, they are grouped by the columns you list. Annotate first, then
list both the group columns and the aggregates in `values`:

```rust
// Total votes per question.
let per_question = Choice::objects()
    .annotate("total", Sum("votes"))
    .values(&["question_id", "total"])
    .values_rows(&db)
    .await?;
```

When the queryset aggregates, the model's default `ordering` option is not applied; add an explicit `order_by`.

## Aggregates

| Function | Result |
|---|---|
| `Count("path")` | Number of non-null values of the column. `Count("*")` counts rows. |
| `Sum("path")` | Total. `NULL` when there are no rows. |
| `Avg("path")` | Mean. `NULL` when there are no rows. |
| `Min("path")`, `Max("path")` | Smallest and largest value. |

Two builders adjust an aggregate expression:

| Method | Effect |
|---|---|
| `.distinct()` | `COUNT(DISTINCT ...)`, `SUM(DISTINCT ...)`, and so on. Only affects aggregates. |
| `.filter(q)` | Only rows matching the `Q` contribute. `FILTER (WHERE ...)` on PostgreSQL, `CASE WHEN ... THEN ... END` inside the aggregate on SQLite. |

```rust
// Per question: all choices and the choices with at least 10 votes.
let rows = Choice::objects()
    .annotate("all", Count("*"))
    .annotate("popular", Count("*").filter(Q::lookup("votes__gte", 10)))
    .values(&["question_id", "all", "popular"])
    .values_rows(&db)
    .await?;
```

Keep the condition of a filtered aggregate (and of a `Case` branch) on columns of the queryset's own model or its
forward foreign keys. A condition that walks a reverse or many-to-many relation adds a second join to that relation
instead of reusing the one the aggregate counts.

`AggKind` also contains `StdDev` and `Variance` (`STDDEV_POP`, `VAR_POP`) with no constructor functions; build
`Expr::Agg { kind: AggKind::StdDev, expr: Box::new(F("x")), distinct: false, filter: None }` by hand. SQLite does not
have those functions.

### `aggregate`

`aggregate` collapses the whole queryset to one row and returns it as a map from name to value. It is a terminal call
and takes the connection.

```rust
let stats = Choice::objects()
    .filter_kw("question", 1)
    .aggregate(&db, vec![
        ("total", Sum("votes")),
        ("average", Avg("votes")),
        ("most", Max("votes")),
        ("rows", Count("*")),
    ])
    .await?;

let total = stats["total"].as_i64();       // Some(...) or None when NULL
```

The values are `rango::db::Value`s. `Count` is `Value::Int`. `Sum`/`Min`/`Max` keep the column's type (`Value::Decimal`
for a `Decimal` column, `Value::Date` for a date). `Avg` is `Value::Float` on SQLite; on PostgreSQL the average of an
integer column is `numeric` and arrives as `Value::Decimal`. An aggregate over no rows is `Value::Null`, except
`Count`, which is `0`.

Details:

- `aggregate` ignores any annotations already on the queryset and its ordering. It cannot refer to an annotation
  by name.
- Slicing and `distinct()` are not applied as a subquery: call `aggregate` on a filtered, unsliced queryset.
- A queryset made with `none()` returns every name mapped to `Value::Null` without querying.
- An unknown column fails with `Error::FieldError`.

## Filtering on aggregates (`HAVING`)

A condition on an annotation that contains an aggregate is moved into `HAVING` automatically. Filter with the
annotation's name:

```rust
let busy = Question::objects()
    .annotate("n", Count("choice"))
    .filter_kw("n__gt", 1)
    .fetch(&db)
    .await?;
```

Conditions on plain columns stay in `WHERE`, so the same queryset can restrict rows before grouping and groups after:

```rust
let rows = Choice::objects()
    .filter_kw("votes__gt", 0)                       // WHERE, applied to rows
    .annotate("total", Sum("votes"))
    .filter_kw("total__gte", 50)                     // HAVING, applied to groups
    .values(&["question_id", "total"])
    .values_rows(&db)
    .await?;
```

Limits: aggregate conditions are always combined with `AND` and cannot be negated or placed inside an `OR`. In
particular, `exclude_kw("n", 1)` on an aggregate annotation currently matches no rows at all; use the opposite
lookup (`n__gt`, `n__lt`, ...) with `filter_kw` instead.

## Comparing columns

`filter_expr(path, expr)` compares a lookup with an expression instead of a literal. The path may end in any of the
comparison lookups; without one it is `exact`.

```rust
Choice::objects().filter_expr("votes__gte", F("votes") * 2);
Choice::objects().filter_expr("votes__lt", F("question__id"));
```

The expression can also be a subquery, and `Q::compare_expr(path, expr)` builds the same condition for use inside a
larger `Q`:

```rust
let cond = Q::compare_expr("votes__gt", F("question_id")) | Q::lookup("votes", 0);
```

`contains`, `startswith` and the other pattern lookups accept an expression on the right: the database escapes the
`%`, `_` and `\` characters in it.

## Subqueries

`Exists(query)` is a boolean expression that is true when the inner query returns a row. Inside it, `OuterRef` points
at a column of the queryset it is used in. Pass the inner queryset as a `Query` (`into_query()`):

```rust
use rango::db::models::expressions::{Exists, OuterRef};

// Questions that have a choice with more than 2 votes.
let hot = Question::objects()
    .filter_q(Q::expr(Exists(
        Choice::objects()
            .filter_expr("question", OuterRef("pk"))
            .filter_kw("votes__gt", 2)
            .into_query(),
    )))
    .fetch(&db)
    .await?;
```

A queryset can also stand in for the value list of an `in` lookup. `Rhs::Query` holds the inner `Query`:

```rust
use rango::db::models::q::Rhs;

let ids = Choice::objects().filter_kw("votes__gt", 1).values(&["question_id"]).into_query();
let questions = Question::objects().filter_q(Q::lookup_rhs("id__in", Rhs::Query(Box::new(ids))));
```

`Subquery(query)` embeds a one-column query as a scalar expression (for example in `Coalesce` or arithmetic); the
inner query must select a single column with `values`.

## Backend differences

| Topic | SQLite | PostgreSQL |
|---|---|---|
| Filtered aggregate | `AGG(CASE WHEN ... THEN x ELSE NULL END)` | `AGG(x) FILTER (WHERE ...)` |
| `Concat` | `a \|\| b` | `CONCAT(a, b)` (nulls are skipped) |
| `Greatest` / `Least` | `MAX(a, b)` / `MIN(a, b)` | `GREATEST` / `LEAST` |
| `Avg` of integers | float | numeric (`Value::Decimal`) |
| Grouping a plain annotated queryset | by every selected column | by primary key |
| `Sqrt`, `Ceil`, `Floor` | needs math functions in the SQLite build | built in |

## Not supported yet

- Window functions (`Window`, `RowNumber`, ...).
- Date and text functions beyond the list above (`TruncDate`, `Extract` and similar); use the date transforms of
  lookups (`pub_date__year`) or `Expr::func`.
- Annotation values on model instances returned by `fetch`; read them with `values`.
- Negated or OR-ed aggregate filters.

## See also

- [Making queries](queries.md)
- [Relations](relations.md)
- [Transactions](transactions.md)
- [QuerySet API reference](../../ref/queryset-api.md)
