# Relations

Models point at each other with `ForeignKey<T>`, `OneToOne<T>` and `ManyToMany<T>` fields. This page covers declaring
them, the `on_delete` choices, naming the reverse side, filtering across relations, loading related rows without one
query per row (`select_related`, `prefetch_related`), managing many-to-many links and what happens to related rows on
delete.

The examples use these models (the `Question`/`Choice` pair from [Models](models.md), plus authors and books):

```rust
use rango::db::models::{ForeignKey, ManyToMany, Model, OneToOne};

#[derive(Clone, Debug, Default, Model)]
#[model(str = "question_text")]
pub struct Question {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub question_text: String,
}

#[derive(Clone, Debug, Default, Model)]
pub struct Choice {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    pub question: ForeignKey<Question>,
    #[field(max_length = 200)]
    pub choice_text: String,
    #[field(default = 0)]
    pub votes: i32,
}

#[derive(Clone, Debug, Default, Model)]
pub struct Author {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100)]
    pub name: String,
    pub age: Option<i32>,
}

#[derive(Clone, Debug, Default, Model)]
pub struct Book {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub title: String,
    pub authors: ManyToMany<Author>,
}
```

## Declaring a relation

The Rust type of the field decides the kind:

| Field type | Kind | Column |
|---|---|---|
| `ForeignKey<T>` | many rows of this model point at one `T` | `<field>_id`, same type as `T`'s primary key, indexed |
| `Option<ForeignKey<T>>` | the same, but the link may be empty | nullable `<field>_id` |
| `OneToOne<T>` | one row points at one `T`, and no two rows share a `T` | `<field>_id` with a `UNIQUE` constraint |
| `ManyToMany<T>` | any number of rows on each side | none; a link table is created |

`OneToOne<T>` is another name for `ForeignKey<T>`; the derive picks the kind from the name you write. A foreign key
column mirrors the target's key type without the identity part, so a key declared `auto` gives an `integer` column
and `big_auto` gives `bigint`. On both backends the column carries a real foreign-key constraint that is checked at
commit (`DEFERRABLE INITIALLY DEFERRED`); SQLite enforces it because every connection runs with `foreign_keys = ON`.

The target type is the Rust type in the angle brackets, not a string, so the compiler checks that it exists. The
referenced model has to be a registered model (any `#[derive(Model)]` struct linked into the program); otherwise the
model registry panics on first use with
`Field polls.Choice.question references model '...' which is not installed, or is abstract.`

### Relation options

Options go in `#[field(...)]` next to the usual field options:

| Option | Value | Effect |
|---|---|---|
| `on_delete` | `"cascade"` (default), `"protect"`, `"restrict"`, `"set_null"`, `"set_default"`, `"do_nothing"` | What deleting the target does to this row. See [below](#deleting-related-rows). Any other string silently means `"cascade"`. |
| `related_name` | `"choices"` | Name of the reverse relation, in lookups and as the generated accessor. A name ending in `+` hides the reverse side entirely. |
| `related_query_name` | `"choice"` | Name of the reverse relation in lookups only. |
| `to_field` | `"slug"` | Target column to join on instead of the primary key. See [limits](#not-supported-yet). |
| `through` | `"polls.Membership"` | Label of an explicit link model for a `ManyToMany` field. |
| `null`, `blank`, `unique`, `db_index`, `db_column`, `default`, `verbose_name`, `help_text`, `editable` | | As for any other field. `db_column` renames the `<field>_id` column. |

## Working with a foreign key value

A `ForeignKey<T>` holds the target's primary key and, when the row was loaded with `select_related` or
`prefetch_related`, the target instance as well.

```rust
use rango::db::models::{ForeignKey, Model};
use rango::db::Db;

let db = Db::default_db();

let mut q = Question { question_text: "What's new?".into(), ..Question::default() };
q.save(&db).await?;

// By instance: stores the key and keeps a copy of the instance (needs `Question: Clone`).
let a = Choice { question: ForeignKey::to(&q), choice_text: "Not much".into(), ..Choice::default() };
// By key value.
let b = Choice { question: ForeignKey::new(q.id.unwrap()), choice_text: "The sky".into(), ..Choice::default() };
Choice::objects().create(&db, a).await?;
Choice::objects().create(&db, b).await?;
```

| Item | Meaning |
|---|---|
| `ForeignKey::new(id)` | Link by key value; nothing cached. |
| `ForeignKey::to(&instance)` | Link to an instance and cache a clone. `T` must be `Clone`. |
| `ForeignKey::null()` | An empty link (also `Default`). Saving it into a non-null column fails in the database. |
| `fk.id() -> &Value` | The stored key. |
| `fk.is_null()` | Whether the key is `NULL`. |
| `fk.cached() -> Option<&T>` | The loaded target, without a query. `None` when it was not loaded. |
| `fk.get(&db).await` | The cached target if there is one, otherwise a query by primary key. Fails with `Error::ObjectDoesNotExist` when the key is null or the row is gone. |

The target is never loaded behind your back: reading `choice.question` is just a struct field, and `cached()` returns
`None` unless you asked for the join. This keeps every query visible in the code that runs it.

For a nullable link the field type is `Option<ForeignKey<T>>`; read it with
`choice.parent.as_ref().and_then(|fk| fk.cached())`.

## Naming the reverse side

Every `ForeignKey` and `OneToOne` produces a reverse relation on the target. It has two names that matter:

- the **accessor**, a method generated on the target type that returns a `QuerySet` of the referencing rows;
- the **query name**, used in lookups such as `choice__votes__gt`.

| Declaration | Accessor | Query name |
|---|---|---|
| no options on `Choice.question` | `question.choice_set()` | `choice` |
| `related_name = "choices"` | `question.choices()` | `choices` |
| `related_query_name = "option"` | `question.choice_set()` | `option` |
| `related_name = "choices", related_query_name = "option"` | `question.choices()` | `option` |
| `related_name = "choices+"` | none | none |
| model option `default_related_name = "choices"` | `question.choice_set()` | `choices` |

```rust
let q = Question::objects().filter_kw("pk", 1).get(&db).await?;
let choices = q.choice_set().order_by(&["id"]).fetch(&db).await?;   // a QuerySet<Choice>
let n = q.choice_set().count(&db).await?;
```

Things to know:

- The accessor returns a plain queryset filtered to that instance, so every queryset method works on it
  (`filter_kw`, `order_by`, `count`, `fetch`, ...).
- For a `OneToOne` the accessor is still named `<model>_set` and still returns a queryset; call `.first(&db)` or
  `.get(&db)` on it to get the single row.
- Two relation fields on the same model that point at the same target would generate two methods with the same name
  and fail to compile. Give at least one of them a `related_name`.
- The accessor is an inherent method on the target type, so the target must live in the same crate as the model that
  points at it. For a target in another crate, set `related_name = "something+"` and query from the other side.
- A name ending in `+` removes the reverse relation from lookups and `prefetch_related`. It also removes it from the
  list of relations that `delete` consults, so deleting the target no longer cascades to, protects against or
  nulls those rows; the database constraint then rejects the delete while rows still point at the target.
- `default_related_name` changes the query name only; it does not rename the generated method.

`choice_set().create(...)` does not exist: create the child with the `ForeignKey` filled in, as shown above.

## Filtering across relations

A lookup path walks relations by field name, forward or backward, with `__` between the steps.

```rust
use rango::db::filter;

// Forward: from Choice to its Question.
Choice::objects().filter(filter!(question__question_text__startswith = "What"));
Choice::objects().filter_kw("question__question_text__startswith", "What");
Choice::objects().filter(filter!(question = 2));              // compares the key; no join
Choice::objects().filter_kw("question__id", 2);               // same, also no join
Choice::objects().filter_kw("question_id__in", vec![1, 2]);   // list and null checks go through the `_id` column
Choice::objects().filter(|f| f.question_id().gte(2));

// Backward: from Question to its choices, by query name.
Question::objects().filter_kw("choice__votes__gt", 2).distinct();
Question::objects().filter(|f| f.reverse::<ChoiceFields>("choice").votes().gt(2)).distinct();

// Many-to-many, from either side.
Book::objects().filter(filter!(authors__name = "Ann"));
Author::objects().filter_kw("book__title", "Rust");
Author::objects().filter_kw("book", 7);                       // authors of the book with key 7

// Other paths.
Choice::objects().order_by(&["question__question_text", "-votes"]);
Question::objects().values(&["id", "choice__choice_text"]).values_rows(&db).await?;
```

`filter!` and the typed closures only know forward relations (their field handles are generated from the struct).
Reverse relations are reached with `f.reverse::<TargetFields>("query_name")` in a closure or with a string path.
`TargetFields` is the `<Model>Fields` type the derive generates next to the model.

Forward joins are `INNER` for non-null links and `LEFT OUTER` for nullable ones. Reverse and many-to-many joins are
`LEFT OUTER` and can return the parent once per matching child, so add `.distinct()` when you only want parents.

### One filter call versus two

For a relation with many rows on the other side, `filter(a).filter(b)` and `filter(a & b)` are different questions:

```rust
use rango::db::models::{Q, Model};

// Two calls: some choice has more than 2 votes, and some choice (maybe another) says "a".
Question::objects()
    .filter_kw("choice__votes__gt", 2)
    .filter_kw("choice__choice_text", "a");

// One call: one and the same choice has more than 2 votes and says "a".
Question::objects().filter_q(Q::lookup("choice__votes__gt", 2) & Q::lookup("choice__choice_text", "a"));
```

Conditions inside one `filter_q` / `Q` (or one `filter!` over a forward path) share a join; each separate `filter` call starts a new join for many-valued
relations. Forward foreign keys have only one row on the other side, so the difference does not arise there.

### Exclude and to-many relations

`exclude` across a many-valued relation applies the negated condition to the joined rows. It is not rewritten into
"no related row matches". Use a subquery when you need that meaning:

```rust
use rango::db::models::expressions::{Exists, OuterRef};
use rango::db::models::Q;

let without_popular = Question::objects().exclude_q(Q::expr(Exists(
    Choice::objects().filter_expr("question", OuterRef("pk")).filter_kw("votes__gt", 2).into_query(),
)));
```

### Errors

A relation compared with a list or a null test must use its `_id` column (`question_id__in`, `parent_id__isnull`);
`question__in` and `question__isnull` are read as a join to `Question` followed by an unknown field and fail with a
`FieldError`. The relation handle's `is_in` and `isnull` methods in typed closures build exactly those paths, so use
`f.question_id().is_in(...)` instead.

An unknown name fails when the query runs, not when it is built:
`Error::FieldError("Cannot resolve keyword 'nope' into field. Choices are: choice, id, question_text")`. The list
includes reverse query names and annotation names.

## Loading related rows

Without help, using the rows of a relation takes one query per parent. Two builder methods fetch them up front.

### `select_related` and `cached()`

`select_related` joins foreign key (and one-to-one) targets into the same query and attaches them to each instance.

```rust
let choices = Choice::objects()
    .select_related(&["question"])
    .order_by(&["id"])
    .fetch(&db)
    .await?;

for c in &choices {
    let question = c.question.cached().expect("loaded by select_related");
    println!("{} -> {}", question.question_text, c.choice_text);
}
```

- Paths may chain: `select_related(&["question__category"])` loads both levels.
- An empty list, `select_related(&[])`, follows every **non-null** foreign key one level deep.
- Only forward `ForeignKey` / `OneToOne` fields can be named. Anything else fails with `Error::FieldError`
  (`Invalid field name(s) given in select_related: ...`).
- A nullable link whose value is `NULL` is read as `None`; otherwise the join is `LEFT OUTER` so the child row is kept.
- `select_related` replaces the earlier setting; pass every path in one call.

### `prefetch_related` and `prefetched()`

`prefetch_related` runs one extra query per lookup (`WHERE pk IN (...)`) and attaches the results afterwards. It works
for many-to-many fields and also for foreign keys.

```rust
let books = Book::objects().prefetch_related(&["authors"]).fetch(&db).await?;
for book in &books {
    let authors = book.authors.prefetched().expect("prefetched");   // &[Author]
    println!("{}: {}", book.title, authors.len());
}

// A foreign key can be prefetched too; it is read with `cached()` like select_related.
let choices = Choice::objects().prefetch_related(&["question"]).fetch(&db).await?;
```

Lookups can be nested with `__` (`"authors__publisher"`); each level adds one query.

| You want | Use | Read with |
|---|---|---|
| The target of a foreign key / one-to-one, one query | `select_related(&["field"])` | `field.cached()` |
| The same, in a separate query | `prefetch_related(&["field"])` | `field.cached()` |
| All rows of a many-to-many field | `prefetch_related(&["field"])` | `field.prefetched()` |
| All children of a parent (reverse foreign key) | one query for the children, see below | |

`ManyToMany::prefetched()` returns `None` when the field was not prefetched. A model loaded any other way has an
empty many-to-many field; use the manager (below) to read it.

**Do not list reverse relations in `prefetch_related`.** A derived struct has nowhere to keep the children of a
reverse foreign key, and the lookup is not usable with typed models. Fetch the children yourself with a single `IN`
query and group them:

```rust
use std::collections::HashMap;

let questions = Question::objects().order_by(&["id"]).fetch(&db).await?;
let ids: Vec<i64> = questions.iter().filter_map(|q| q.id).collect();
let choices = Choice::objects().filter_q(Q::is_in("question_id", ids)).fetch(&db).await?;

let mut by_question: HashMap<i64, Vec<&Choice>> = HashMap::new();
for c in &choices {
    if let Some(id) = c.question.id().as_i64() {
        by_question.entry(id).or_default().push(c);
    }
}
```

An unknown lookup fails the whole `fetch` with
`Cannot find 'x' on Question object, 'x' is an invalid parameter to prefetch_related()`.

## Many-to-many links

A `ManyToMany<T>` field has no column. Rango creates a link table named `<model table>_<field>` (`polls_book_authors`)
with an id, one foreign key column per side and a unique constraint on the pair. The columns are named after the
lower-cased model names: `book_id` and `author_id`. When both sides are the same model the columns are
`from_<model>_id` and `to_<model>_id`. Deleting either side removes its link rows.

The struct field itself only carries prefetched results. To change or read the links, use the manager the derive
generates, `<field>_manager()`, on a **saved** instance:

```rust
let ann = Author::objects().create(&db, Author { name: "Ann".into(), ..Author::default() }).await?;
let bob = Author::objects().create(&db, Author { name: "Bob".into(), ..Author::default() }).await?;
let book = Book::objects()
    .create(&db, Book { title: "Rust".into(), ..Book::default() })
    .await?;

let authors = book.authors_manager();
authors.add(&db, &[&ann, &bob]).await?;      // Result<usize>: links actually created
authors.add(&db, &[&ann]).await?;            // already linked: skipped, returns 0
authors.remove(&db, &[&bob]).await?;         // Result<u64>: links deleted
authors.set(&db, &[&bob]).await?;            // make the links exactly {bob}
let n = authors.all().count(&db).await?;     // a QuerySet<Author>
authors.clear(&db).await?;                   // Result<u64>
```

| Method | Returns | Behaviour |
|---|---|---|
| `all()` | `QuerySet<T>` | The linked rows; filter, order and count as usual. |
| `add(&db, &[&T])` | `Result<usize>` | Inserts missing links one by one. Already existing pairs are skipped. |
| `remove(&db, &[&T])` | `Result<u64>` | Deletes the listed links. An empty slice does nothing. |
| `set(&db, &[&T])` | `Result<()>` | Reads the current links, removes the extras and adds the missing ones. |
| `clear(&db)` | `Result<u64>` | Deletes every link of this instance. |

Pass a `Transaction` instead of the `Db` to make a group of changes atomic; see [Transactions](transactions.md).

### Working with keys

The functions in `rango::db::models::related` do the same on raw keys, which is handy when you only have ids (a
form's selected values, a fixture). They address the field by the owner's model label and field name:

```rust
use rango::db::models::related;
use rango::db::Value;

related::add(&db, "polls.Book", "authors", &Value::Int(1), &[Value::Int(5), Value::Int(6)]).await?;
related::remove(&db, "polls.Book", "authors", &Value::Int(1), &[Value::Int(6)]).await?;
related::set(&db, "polls.Book", "authors", &Value::Int(1), &[Value::Int(5), Value::Int(7)]).await?;
let ids: Vec<Value> = related::current_ids(&db, "polls.Book", "authors", &Value::Int(1)).await?;
related::clear(&db, "polls.Book", "authors", &Value::Int(1)).await?;
```

| Function | Returns |
|---|---|
| `add(db, label, field, owner_pk, targets)` | `Result<usize>` links created |
| `remove(db, label, field, owner_pk, targets)` | `Result<u64>` links deleted |
| `set(db, label, field, owner_pk, targets)` | `Result<()>` |
| `clear(db, label, field, owner_pk)` | `Result<u64>` links deleted |
| `current_ids(db, label, field, owner_pk)` | `Result<Vec<Value>>`, target keys in ascending order |

`label` is `app_label.ModelName` of the model that declares the field. A field that is not a many-to-many field fails
with `Error::FieldError("polls.Book has no many-to-many field 'x'")`.

`add`, `remove` and `clear` send the `M2M_CHANGED` signal (`rango::db::models::signals`) with the action
`PreAdd`/`PostAdd`, `PreRemove`/`PostRemove` or `PreClear`/`PostClear`; the sender is the owner's label.

### Reading the reverse side

The reverse of a many-to-many field has no manager. Query it with a lookup: the authors of a book are
`Author::objects().filter_kw("book", book_id)`, and the books of an author are
`Book::objects().filter_kw("authors", author_id)`. The reverse query name is the lower-cased owner model name unless
`related_name` or `related_query_name` is set on the many-to-many field.

### Explicit link models

`#[field(through = "polls.Membership")]` names a model you declare yourself, which lets the link carry data. That
model needs one foreign key to each side. Rango then creates no link table of its own. `add` only writes the two key
columns, so extra columns on the link model need defaults in the database, or you create the link rows yourself with
`Membership::objects().create(...)`.

## Deleting related rows

Deleting a row (`instance.delete(&db)` or `queryset.delete(&db)`) first finds the rows that reference it and applies
each relation's `on_delete`:

| `on_delete` | Effect on the referencing rows |
|---|---|
| `"cascade"` | Deleted too, recursively. This is the default. |
| `"protect"` | Nothing is deleted; the call fails with an `Error::Other` wrapping `ProtectedError`. |
| `"restrict"` | Same as `protect` here: the call fails with `RestrictedError`. |
| `"set_null"` | The column is set to `NULL`. The field must be an `Option<ForeignKey<T>>`. |
| `"set_default"` | The column is set to the field's `default = ...`; with no default, to `NULL`. |
| `"do_nothing"` | Left alone. The database constraint then rejects the delete at commit if rows still point at it. |

```rust
let (total, per_model) = question.delete(&db).await?;
// total == 3, per_model == {"polls.Choice": 2, "polls.Question": 1}
```

The result is the total number of rows removed and a count per model label. Link-table rows of many-to-many fields are
removed by the same mechanism and show up under the link model's label (`polls.Book_authors`).

A protected delete looks like this:

```rust
use rango::Error;

match author.delete(&db).await {
    Ok(_) => {}
    Err(Error::Other(e)) => eprintln!("{e}"),
    // Cannot delete some instances of model 'Author' because they are referenced through protected foreign keys: 'Book.author'
    Err(e) => return Err(e),
}
```

Deleting walks the relations in the program, not the database's `ON DELETE` rules, so foreign keys are created
without one. A bulk `queryset.delete(&db)` takes the same path when other models reference the model or when
`PRE_DELETE` / `POST_DELETE` have receivers for it; otherwise it is a single `DELETE ... WHERE`. Run a delete that
touches several tables inside [a transaction](transactions.md) so it is all or nothing.

## One-to-one relations

`OneToOne<T>` behaves like a foreign key with a unique column. Use it for an extension row such as a profile:

```rust
#[derive(Clone, Debug, Default, Model)]
pub struct Profile {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(on_delete = "cascade")]
    pub author: OneToOne<Author>,
    #[field(blank)]
    pub bio: String,
}

let profile = author.profile_set().first(&db).await?;          // Option<Profile>
let ann = Author::objects().filter_kw("profile__bio__icontains", "rust").first(&db).await?;
```

## Relations to the same model

A model may point at itself. Give the relation a `related_name`, because the default names would be confusing:

```rust
#[derive(Clone, Debug, Default, Model)]
pub struct Category {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100)]
    pub name: String,
    #[field(related_name = "children", on_delete = "cascade")]
    pub parent: Option<ForeignKey<Category>>,
}

let kids = root.children().fetch(&db).await?;
```

A self-referencing many-to-many field (`friends: ManyToMany<Author>` on `Author`) uses the
`from_<model>_id` / `to_<model>_id` link columns.

## Not supported yet

- Reverse relations in `prefetch_related` for typed models.
- A subquery form of `exclude` across to-many relations.
- `to_field` other than the primary key works for joins and for the column definition, but cascading deletes,
  `set_null` and `prefetch_related` compare the referencing column with the **primary key** of the target, so do not
  rely on them for such relations.
- Generic relations.
- A generated reverse accessor for a foreign key whose target is declared in another crate (use `related_name = "name+"`,
  which also disables cascading from that target).
- `limit_choices_to` on relation fields.
- A creating or scoped manager on reverse relations (`question.choice_set().create(...)`).
- Reverse accessors for many-to-many fields; query the reverse side with lookups.

## See also

- [Models](models.md)
- [Making queries](queries.md)
- [Aggregation and expressions](aggregation.md)
- [Transactions](transactions.md)
- [Migrations](migrations.md)
- [Model field reference](../../ref/model-fields.md)
- [QuerySet API reference](../../ref/queryset-api.md)
- [Signals](../signals.md)
