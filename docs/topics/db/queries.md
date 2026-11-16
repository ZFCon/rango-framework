# Making queries

A `QuerySet` describes a database query for one model: which rows, in what order, with which related data. This
page covers building querysets, the three ways to write a filter, every lookup, and the operations that read and
write rows. The complete method list is in the [QuerySet API reference](../../ref/queryset-api.md).

The examples use the `Question` and `Choice` models from [Models](models.md), and in a few places the `Author`
model (`name: String`, `age: Option<i32>`) from [Relations](relations.md).

```rust
use rango::db::models::{Model, Q, F};
use rango::db::{Db, Executor, Value, filter};

let db = Db::default_db();
```

## Querysets are lazy descriptions

`Question::objects()` returns a `QuerySet<Question>` that matches every row. Builder methods (`filter`, `exclude`,
`order_by`, `slice`, ...) take the queryset by value and return a new one. Nothing is sent to the database until a
*terminal* method is awaited with a connection:

```rust
let recent = Question::objects()
    .filter(filter!(question_text__startswith = "What"))
    .order_by(&["-pub_date"]);                 // no query yet

let rows: Vec<Question> = recent.fetch(&db).await?;   // SELECT ...
let n: usize = recent.count(&db).await?;              // SELECT COUNT(*) ...
```

Three consequences:

- **The connection is always explicit.** Terminal methods take `&dyn Executor`, which is a `Db` (each statement on
  its own pooled connection) or a `Transaction`. There is no ambient "current connection", so it is always visible
  which database and which transaction a query runs in.
- **Nothing is cached.** Terminal methods borrow the queryset (`&self`), so one queryset can be run many times, and
  every call is a new query. Keep the `Vec` if you need the rows twice.
- **Querysets are values.** `QuerySet` is `Clone`; clone it to branch a base query into several variants.

Mistakes in string paths (an unknown field, an unsupported lookup) are recorded while the queryset is built and
returned as an error by the terminal method, so the chain itself never needs `?`.

## Three ways to write a filter

### `filter!` — keyword syntax, checked at compile time

```rust
Question::objects().filter(filter!(question_text__startswith = "What"));
Question::objects().filter(filter!(id = 1, question_text__icontains = "new"));   // AND
Question::objects().filter(filter!((id = 1) | (question_text = "Why?")));        // OR
Question::objects().exclude(filter!(id__in = [1, 2]));
Question::objects().filter(filter!(!(id = 1)));                                  // NOT
Choice::objects().filter(filter!(question__question_text__startswith = "What")); // across a foreign key
Choice::objects().filter(filter!(question = 2));                                 // the foreign key itself
```

The key is a field path with segments separated by `__`, optionally ending in a lookup name. `filter!` turns it
into method calls on the model's generated `…Fields` type, so a misspelled field or a lookup that does not exist
for that path does not compile. Conditions are combined with `,` or `&` (AND), `|` (OR), `!` (NOT) and
parentheses. `q!` is another name for the same macro.

Limits of the macro: a value ends at the next top-level `,`, `&` or `|`, so wrap a value expression that contains
those operators in parentheses; `range` (which takes two values) and reverse relations are not expressible — use a
closure or a string path for those.

### Typed closures

`filter` and `exclude` take any closure from the model's field handles to a `Q`. `filter!` only writes that closure
for you; writing it by hand gives access to everything the handles offer:

```rust
Question::objects().filter(|f| f.pub_date().year().gte(2025) & !f.id().eq(1));
Choice::objects().filter(|f| f.votes().range(1, 10));
Choice::objects().filter(|f| f.question().question_text().startswith("What"));
Choice::objects().filter(|f| f.votes().cmp_expr("gt", F("question_id")));      // compare with another column
Question::objects().filter(|f| f.reverse::<ChoiceFields>("choice").votes().gt(2));  // reverse relation
```

Each field method returns a `Path<T>`; its methods are the lookups (`eq`, `exact`, `iexact`, `contains`,
`icontains`, `startswith`, `istartswith`, `endswith`, `iendswith`, `gt`, `gte`, `lt`, `lte`, `regex`, `iregex`,
`isnull`, `is_in`, `range`, `cmp_expr`) and the transforms (`year`, `month`, `day`, `week_day`, `quarter`, `hour`,
`minute`, `second`, `lower`, `upper`, `length`). A relation method returns the target model's handles, which also
offer `eq` for the relation itself and `pk()` (for `in` and `isnull` on a relation use the `_id` column, for example `question_id__in`).

### String paths

`filter_kw` and `exclude_kw` take the path as a string. They are checked when the query runs, not when it is
compiled, and are the way to filter on things the typed handles cannot name: annotations, reverse relations and
the transforms without a typed method.

```rust
Question::objects().filter_kw("pub_date__year", 2025);
Question::objects().filter_kw("choice__votes__gt", 2);          // reverse relation
Question::objects().exclude_kw("question_text", "Why?");
Question::objects().filter_kw("pk", 1);
```

A wrong path fails at the terminal call with `Error::FieldError`, whose message lists the valid names:
`Cannot resolve keyword 'nope' into field. Choices are: choice, id, pub_date, question_text`.

## `Q` objects

All three forms produce a `Q`, a tree of conditions. Build one directly when the condition is assembled at run
time:

```rust
let mut cond = Q::lookup("question_text__icontains", "rust");
if only_recent {
    cond = cond & Q::lookup("pub_date__year__gte", 2025);
}
let qs = Question::objects().filter_q(cond | Q::lookup("id", 1));
```

| Constructor | Meaning |
|---|---|
| `Q::lookup(path, value)` | `path` (which may end in a lookup) compared with a value |
| `Q::is_in(path, values)` | `path IN (values)` |
| `Q::compare_expr(path, expr)` | `path` compared with an expression such as `F("other")` |
| `Q::expr(expr)` | a boolean expression such as `Exists(...)` |
| `Q::lookup_rhs(path, rhs)` | the general form; `rhs` is a `rango::db::models::q::Rhs` (`Val`, `List`, `Range`, `Expr`, `Query`) |
| `Q::new()` | an empty condition, the identity for `&` and `\|` |

Combine with `&`, `|` and `!` (or `.and()`, `.or()`, `.not()`). `filter_q(q)` and `exclude_q(q)` apply a `Q` to a
queryset.

Chained filters are AND-ed: `qs.filter(a).filter(b)` matches rows satisfying both. The one case where chaining and
a single call differ is filtering across a to-many relation; see
[Relations](relations.md#filtering-across-relations).

`exclude` keeps rows where the column is `NULL`: `Author::objects().exclude_kw("age", 40)` returns authors whose
age is not 40 **and** authors with no age.

## Lookups

A path may end in one of these lookup names. With no lookup, `exact` is used.

| Lookup | Matches |
|---|---|
| `exact` | Equal. Comparing with a null value (`None`) becomes `IS NULL`. |
| `iexact` | Equal, ignoring case. |
| `contains`, `icontains` | Substring; `i` ignores case. |
| `startswith`, `istartswith` | Prefix. |
| `endswith`, `iendswith` | Suffix. |
| `gt`, `gte`, `lt`, `lte` | Ordering comparisons. |
| `in` | One of a list of values. An empty list matches nothing (no query is sent). Null values in the list are dropped. |
| `range` | Between two values, inclusive. |
| `isnull` | `true`: is `NULL`; `false`: is not `NULL`. |
| `regex`, `iregex` | Regular-expression match; `i` ignores case. |

`%`, `_` and `\` in the value of a pattern lookup are escaped, so `contains = "100%"` looks for the literal text.

Backend differences:

- SQLite's `LIKE` ignores ASCII case, so `contains`, `startswith` and `endswith` behave like their `i` variants
  there. On PostgreSQL they are case-sensitive.
- `regex` uses the Rust `regex` crate syntax on SQLite and POSIX regular expressions (`~`, `~*`) on PostgreSQL.

### Transforms

Between the field and the lookup a path may apply transforms:

| Transform | Applies to | Result |
|---|---|---|
| `year`, `iso_year`, `month`, `day`, `week`, `week_day`, `iso_week_day`, `quarter` | date and datetime columns | integer |
| `hour`, `minute`, `second` | datetime columns | integer |
| `date`, `time` | datetime columns | date / time |
| `lower`, `upper` | text columns | text |
| `length` | text columns | integer |

```rust
Question::objects().filter_kw("pub_date__year", 2025);
Question::objects().filter_kw("pub_date__month__gte", 6);
Question::objects().filter_kw("question_text__length__gt", 20);
Question::objects().filter(|f| f.question_text().lower().eq("why?"));
```

`week_day` counts from Sunday = 1 to Saturday = 7; `iso_week_day` from Monday = 1 to Sunday = 7.

`year` compared with an integer (`exact`, `gt`, `gte`, `lt`, `lte`) is compiled to a range over the column
(`BETWEEN '2025-01-01' AND '2025-12-31 23:59:59.999999'`) so an index on the column can be used; the bounds are
taken in the current time zone. The other date parts are extracted from the UTC value.

The `date` and `time` transforms currently produce SQL that only PostgreSQL accepts; do not use them on SQLite.

## Ordering

```rust
Question::objects().order_by(&["-pub_date", "id"]);        // descending date, then id
Choice::objects().order_by(&["question__question_text"]);  // by a related column
Question::objects().order_by(&["?"]);                      // random
Question::objects().order_by(&[]);                         // no ordering at all
Question::objects().order_by(&["-pub_date"]).reverse();    // flip every direction
```

Without `order_by`, the model's `ordering` option applies; with neither, the database decides the order.
`order_by` replaces any earlier ordering rather than adding to it. For control over `NULL` placement use
expressions:

```rust
Author::objects().order_by_expr(vec![F("age").desc().nulls_last(), F("name").asc()]);
```

`Path` handles have `asc()` and `desc()` that return the string form, for building `order_by` arguments from typed
handles.

## Slicing

`slice(low, high)` is `OFFSET low LIMIT high - low`; `None` for `high` means "to the end".

```rust
Question::objects().order_by(&["id"]).slice(0, Some(5));   // first five
Question::objects().order_by(&["id"]).slice(10, Some(20)); // rows 11–20
Question::objects().order_by(&["id"]).slice(5, None);      // everything after the fifth
```

A second `slice` is relative to the first. Always order a sliced queryset; otherwise which rows you get is up to
the database.

## Retrieving rows

| Call | Returns |
|---|---|
| `fetch(&db)` | `Vec<M>`, all matching rows |
| `get(&db)` | `M`, exactly one row |
| `first(&db)`, `last(&db)` | `Option<M>`; ordered by primary key when the queryset has no ordering |
| `earliest(&db, "field")`, `latest(&db, "field")` | `M`, the row with the smallest / largest value of the field |
| `exists(&db)` | `bool`, with a `LIMIT 1` query |
| `count(&db)` | `usize`, with `SELECT COUNT(*)` |
| `in_bulk(&db, ids)` | a map from primary key to instance |

```rust
let q = Question::objects().filter_kw("pk", 1).get(&db).await?;
let newest = Question::objects().order_by(&["-pub_date"]).first(&db).await?;   // Option<Question>
if Question::objects().filter(filter!(question_text__startswith = "What")).exists(&db).await? { /* ... */ }
```

### `get` and its errors

`get` returns the single matching row. It fails with:

- `Error::ObjectDoesNotExist { model, message }` when nothing matches (`Question matching query does not exist.`);
- `Error::MultipleObjectsReturned { model, message }` when more than one row matches
  (`get() returned more than one Question -- it returned 2!`).

```rust
use rango::Error;

match Choice::objects().filter_kw("pk", choice_id).get(&db).await {
    Ok(choice) => { /* ... */ }
    Err(Error::ObjectDoesNotExist { .. }) => { /* not found */ }
    Err(e) => return Err(e),
}
```

`earliest` and `latest` also return `ObjectDoesNotExist` on an empty result. In a view,
`rango::shortcuts::get_object_or_404(queryset, &db)` turns the "does not exist" case into a 404 response and
`get_list_or_404` does the same for an empty `fetch`.

`in_bulk` takes the ids as `Vec<Value>` and returns `IndexMap<String, M>`. The key is the debug form of the
primary-key value (`"Int(1)"`), not the bare number.

## Selecting columns instead of instances

`values` restricts the query to named columns (all columns when the list is empty). Run it with one of:

```rust
// One map per row: IndexMap<String, Value>
let rows = Question::objects().values(&["id", "question_text"]).values_rows(&db).await?;
let text = rows[0]["question_text"].as_str();

// One Vec<Value> per row
let lists = Question::objects().values(&["id", "question_text"]).values_list(&db).await?;

// A single column, converted to a Rust type
let texts: Vec<String> = Question::objects().values(&["question_text"]).order_by(&["id"]).values_flat(&db).await?;
```

Names may cross relations (`"question__question_text"`) and may name annotations. `values_flat::<T>` converts the
first selected column of each row with `FromValue`, which is implemented for the integer and float types, `bool`,
`String`, `Decimal`, the `chrono` date/time types, `Uuid`, `Vec<u8>`, `serde_json::Value`, `Value` itself and
`Option<T>` of those.

`distinct()` adds `SELECT DISTINCT`; it takes no column list. It is mostly needed after filtering across a to-many
relation, which can return the same row several times.

`only(&[...])` and `defer(&[...])` narrow the selected columns while still building instances. There is no later
loading of the columns that were left out: a left-out field is filled from a null, which only works for
`Option<T>` fields. For other shapes use `values`.

## Combining querysets

```rust
let a = Author::objects().filter_kw("name", "Ann");
let b = Author::objects().filter_kw("name", "Bob");

let either = a.clone().union(b.clone(), false).fetch(&db).await?;   // UNION; `true` for UNION ALL
let both   = a.clone().intersection(b.clone());                      // INTERSECT
let only_a = a.difference(b);                                        // EXCEPT
```

Both sides must be querysets of the same model. Finish filtering each side before combining: a filter added after
`union` applies to the left-hand queryset only.

`none()` turns any queryset into one that matches nothing and never queries: `fetch` returns an empty `Vec`,
`count` returns `0`.

## Creating rows

```rust
let q = Question::objects()
    .create(&db, Question { question_text: "What's new?".into(), ..Question::default() })
    .await?;                                              // always INSERTs; returns the saved instance

let n = Author::objects()
    .bulk_create(&db, authors, None)                      // Vec<Author> -> rows inserted
    .await?;
```

`bulk_create` inserts in multi-row statements (`batch_size` rows per statement; by default as many as fit in 999
parameters). It assigns `auto_now` fields but does not send save signals and does not report the generated primary
keys — the instances are consumed.

`get_or_create` and `update_or_create` look a row up by a `Q` and create it when it is missing. Both return the
instance and whether it was created:

```rust
let (author, created) = Author::objects()
    .get_or_create(&db, Q::lookup("name", "Cy"), || Author { name: "Cy".into(), ..Author::default() })
    .await?;

let (author, created) = Author::objects()
    .update_or_create(
        &db,
        Q::lookup("name", "Cy"),
        |a| a.age = Some(2),                                              // applied when the row exists
        || Author { name: "Cy".into(), age: Some(2), ..Author::default() }, // built when it does not
    )
    .await?;
```

The lookup and the insert are two statements. Under concurrent requests two tasks can both miss and both insert;
put a unique constraint on the lookup columns and run the call inside [a transaction](transactions.md) when that
matters. A lookup matching several rows returns `MultipleObjectsReturned`.

## Updating rows

`update` writes columns on every matching row in one statement and returns the number of rows matched. Each value
is a plain value or an expression, converted with `.into()`:

```rust
Choice::objects().filter_kw("question", 1)
    .update(&db, vec![("votes", (F("votes") + 1).into())])
    .await?;

Author::objects().filter_kw("name__startswith", "tmp")
    .update(&db, vec![("age", 0.into()), ("name", "anonymous".into())])
    .await?;
```

`F("votes") + 1` is evaluated by the database, so concurrent increments are not lost the way they can be with
read-modify-`save`. `update` only accepts columns of the model itself, sends no signals and does not touch
`auto_now` fields. When the filter crosses a relation the statement becomes `UPDATE ... WHERE pk IN (SELECT ...)`.

`bulk_update(&db, &mut objs, &["field", ...])` saves the listed fields of each instance, one `UPDATE` per
instance.

## Deleting rows

```rust
let (total, per_model) = Question::objects().filter_kw("pub_date__year__lt", 2020).delete(&db).await?;
```

`delete` returns the total number of rows removed and a count per model label. When other models reference the
model (or delete signals have receivers) the primary keys are read first and deletion cascades according to each
relation's `on_delete`; otherwise it is a single `DELETE ... WHERE`. See
[Relations](relations.md#deleting-related-rows).

## Raw SQL

Three levels are available when the query builder is not enough.

**Rows into model instances.** `QuerySet::<M>::raw` runs your SQL and maps result columns onto fields by column
name:

```rust
use rango::db::models::QuerySet;

let rows: Vec<Question> = QuerySet::<Question>::raw(
    &db,
    "SELECT * FROM polls_question WHERE question_text LIKE %s ORDER BY id",
    &[Value::Str("What%".into())],
)
.await?;
```

**Plain rows and statements.** Every `Executor` (`Db` and `Transaction`) has:

```rust
let rows = db.query("SELECT id, question_text FROM polls_question WHERE id > %s", &[Value::Int(10)]).await?;
for row in &rows {
    let id = row.get(0).as_i64();
    let text = row.by_name("question_text").and_then(Value::as_str);
}

let affected: u64 = db.execute("UPDATE polls_choice SET votes = 0 WHERE question_id = %s", &[Value::Int(1)]).await?;

db.execute_batch("CREATE INDEX IF NOT EXISTS q_text ON polls_question (question_text); ANALYZE;").await?;
```

Placeholders are always `%s`, on both backends; write a literal percent sign as `%%` in `query` and `execute`.
Parameters are `rango::db::Value`s. `execute_batch` runs several statements separated by `;` and takes no
parameters. Never build SQL by formatting user input into the string; pass it as a parameter.

**Looking at the generated SQL.** `sql(vendor)` compiles a queryset without running it, and `explain(&db)` asks
the database for its plan:

```rust
use rango::db::backends::Vendor;

if let Some(c) = Question::objects().filter_kw("id", 1).sql(Vendor::Sqlite)? {
    println!("{} {:?}", c.sql, c.params);
}
let plan: String = Question::objects().filter_kw("id", 1).explain(&db).await?;
```

`sql` returns `None` for a queryset that can never match (`none()`, an empty `in` list).

## Errors

| Error | When |
|---|---|
| `Error::ObjectDoesNotExist` | `get`, `earliest`, `latest`, `ForeignKey::get` found nothing |
| `Error::MultipleObjectsReturned` | `get` found more than one row |
| `Error::FieldError` | unknown field or unsupported lookup in a string path; unknown column in `update` |
| `Error::LookupError` | the model label is not registered |
| `Error::ImproperlyConfigured` | the database alias is not in `databases`, or its engine is unknown |
| `Error::Other` | driver and SQL errors (constraint violations, syntax errors, connection failures), protected deletes |

## Not supported yet

- Result caching or streaming: every terminal call loads the whole result into memory.
- `DISTINCT ON` a list of columns.
- Row locking on SQLite (`select_for_update` only adds `FOR UPDATE` on PostgreSQL).
- Loading deferred columns after the fact.
- Window functions.

## See also

- [Models](models.md)
- [Relations](relations.md)
- [Aggregation and expressions](aggregation.md)
- [Transactions](transactions.md)
- [QuerySet API reference](../../ref/queryset-api.md)
- [Shortcuts](../http/shortcuts.md)
- [Pagination](../pagination.md)
