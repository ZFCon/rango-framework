# Models

A model is a Rust struct that maps to one database table. This page covers declaring models with
`#[derive(Model)]`, the options the derive accepts, how Rust types become columns, and how instances are saved,
reloaded and deleted.

## A first model

```rust
use rango::db::chrono::{DateTime, Utc};
use rango::db::models::{ForeignKey, Model};

#[derive(Clone, Debug, Default, Model)]
#[model(str = "question_text", ordering = ["-pub_date"])]
pub struct Question {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub question_text: String,
    #[field(auto_now_add, verbose_name = "date published")]
    pub pub_date: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Model)]
#[model(str = "choice_text")]
pub struct Choice {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    pub question: ForeignKey<Question>,
    #[field(max_length = 200)]
    pub choice_text: String,
    #[field(default = 0)]
    pub votes: i32,
}
```

The struct must have named fields. `#[model(...)]` on the struct sets table-level options, `#[field(...)]` on a
field sets column options. Inside your own project the derive needs no `crate = ...` option; it expands to paths
under `::rango::db`.

`rango::db::chrono` and `rango::db::rust_decimal` re-export the `chrono` and `rust_decimal` crates the ORM is built
against, so a project does not have to add them itself. `uuid` and `serde_json` are not re-exported; add them to
`Cargo.toml` when you use `Uuid` or JSON columns.

`Clone`, `Debug` and `Default` are not required by the derive, but they are worth having:

- `Default` lets you write `Question { question_text: "…".into(), ..Question::default() }`.
- `ForeignKey::to(&instance)` needs the target model to be `Clone`.
- A struct that contains `ForeignKey<T>` can only derive `Clone`/`Debug` when `T` does.

## Where the model lives: app label and table name

Every model belongs to an app. The label is taken from the module path of the struct:

- if the struct is declared in a module named `models` (`mysite::polls::models`), the label is the parent module
  (`polls`);
- otherwise the label is the first path segment, which is the crate name.

So a model in `src/polls/models.rs` gets the label `polls`, but a model in `src/polls/models/extra.rs` or in
`src/polls/shapes.rs` gets the crate name. Set `#[model(app_label = "polls")]` in those cases.

The label matters in three places: the full model label is `app_label.StructName` (`polls.Question`), the default
table name is `<app_label>_<struct name in lower case>` (`polls_question`), and `makemigrations` only looks at
models whose label matches an app listed in `installed_apps`.

## Model options

All options go in `#[model(...)]` on the struct. An unknown option is a compile error.

| Option | Value | Effect |
|---|---|---|
| `app_label` | `"polls"` | App the model belongs to. Default: inferred from the module path (above). |
| `db_table` | `"legacy_questions"` | Table name. Default: `<app_label>_<lowercased struct name>`. |
| `ordering` | `["-pub_date", "id"]` or `"id"` | Default `ORDER BY` for querysets with no explicit ordering. A leading `-` means descending. |
| `verbose_name` | `"poll question"` | Human-readable name. Default: the struct name split into words. Used by the admin and forms. |
| `verbose_name_plural` | `"poll questions"` | Default: `verbose_name` plus `s`. |
| `str` | `"question_text"` | Field whose value is the text of an instance: `{{ question }}` in templates, option labels in model forms, the admin. Without it the text is `Question object (1)`. |
| `unique_together` | `[["question", "choice_text"]]` | Groups of fields that must be unique together. Always a list of lists; each group becomes a unique index (SQLite) or unique constraint (PostgreSQL). |
| `default_related_name` | `"choices"` | Name of the reverse relation in lookups for every relation field of this model that has no `related_name`. `%(model_name)s` and `%(app_label)s` are substituted. It does not rename the generated Rust accessor (see [Relations](relations.md)). |
| `get_latest_by` | `"pub_date"` or `["pub_date", "id"]` | Stored in the model metadata. `latest()` and `earliest()` take the field as an argument and do not read it. |
| `managed` | `false` | Excludes the model from migrations, `flush` and `dumpdata`. Use it for tables the project does not own. Default `true`. |
| `crate` | `"rango::db"` | Path to the ORM crate. Only needed when the derive is used where `::rango::db` does not resolve. |

Indexes spanning several columns, check constraints and model inheritance cannot be declared through the derive.

## Field types

The column type is inferred from the Rust type of the field. `Option<T>` makes the column nullable.

| Rust type | Field kind | Notes |
|---|---|---|
| `String` with `max_length = n` | Char | `varchar(n)` |
| `String` without `max_length` | Text | unbounded text |
| `i8`, `i16` | SmallInteger | |
| `i32` | Integer | |
| `i64`, `isize` | BigInteger | |
| `u8`, `u16` | PositiveSmallInteger | column has a `>= 0` check |
| `u32` | PositiveInteger | column has a `>= 0` check |
| `u64`, `usize` | PositiveBigInteger | column has a `>= 0` check |
| `bool` | Boolean | |
| `f32`, `f64` | Float | double precision |
| `Decimal` (`rust_decimal`) | Decimal | set `max_digits` and `decimal_places` |
| `DateTime<Utc>` | DateTime | use when `use_tz` is `true` (the default) |
| `NaiveDateTime` | DateTime | use when `use_tz` is `false` |
| `NaiveDate` | Date | |
| `NaiveTime` | Time | |
| `TimeDelta` (`chrono`) | Duration | the type must be written `TimeDelta` |
| `Uuid` | Uuid | |
| `Vec<u8>` | Binary | |
| `serde_json::Value` | Json | any type whose last path segment is `Value` |
| `ForeignKey<T>` | ForeignKey | see [Relations](relations.md) |
| `OneToOne<T>` | OneToOne | |
| `ManyToMany<T>` | ManyToMany | no column on this table |

The derive looks at the last segment of the type path, so `chrono::DateTime<Utc>` and `DateTime<Utc>` are the same
to it. A type it does not recognise is a compile error (`cannot infer a model field type for ...`).

Some kinds cannot be told apart by type; pick them with a flag:

```rust
#[field(text)]                         pub body: String,        // Text even though max_length may be set
#[field(email)]                        pub email: String,       // varchar(254) unless max_length is given
#[field(url)]                          pub homepage: String,    // varchar(200)
#[field(slug, max_length = 50, unique)] pub slug: String,       // varchar(50)
#[field(ip)]                           pub last_ip: String,     // inet on PostgreSQL, char(39) on SQLite
#[field(json)]                         pub extra: serde_json::Value,
#[field(binary)]                       pub blob: Vec<u8>,
#[field(file, upload_to = "docs")]     pub attachment: String,  // stores the file name
#[field(image, upload_to = "avatars")] pub avatar: String,      // stores the file name
```

The exact column types per backend are in the [model field reference](../../ref/model-fields.md).

### Date and time values

Datetimes are stored in UTC. A `DateTime` column is read back as `DateTime<Utc>` when the `use_tz` setting is
`true` and as `NaiveDateTime` when it is `false`, so the Rust type of the field has to match the setting: a
`NaiveDateTime` field fails to load under `use_tz: true`, and a `DateTime<Utc>` field fails under `use_tz: false`.

### File and image columns

`#[field(file)]` and `#[field(image)]` columns hold the stored file name as a string (`varchar(100)` unless
`max_length` is set). The model layer does not touch the storage; a `ModelForm` saves an uploaded file under
`upload_to` and writes the resulting name into the field. Image dimensions are not tracked. See
[Files](../files.md) and [Model forms](../forms/modelforms.md).

## Field options

Flags (written without a value):

| Flag | Effect |
|---|---|
| `primary_key` | The field is the primary key. |
| `auto` | Auto-incrementing 32-bit primary key (implies `primary_key`). |
| `big_auto` | Auto-incrementing 64-bit primary key (implies `primary_key`). |
| `null` | Column allows `NULL`. Implied by `Option<T>`; rarely needed on its own. |
| `blank` | The field may be left empty in forms. |
| `unique` | Adds a `UNIQUE` constraint. |
| `db_index` | Creates an index on the column. |
| `auto_now` | Set to the current time on every save. |
| `auto_now_add` | Set to the current time when the row is first inserted. |
| `text`, `email`, `url`, `slug`, `json`, `binary`, `ip`, `file`, `image` | Choose the field kind (see above). |

Options with a value:

| Option | Value | Effect |
|---|---|---|
| `max_length` | integer | Maximum length of a string column. |
| `max_digits`, `decimal_places` | integer | Precision of a `Decimal` column. On PostgreSQL the column is `numeric(max_digits, decimal_places)` (10, 2 when omitted). On SQLite values are stored rounded to `decimal_places`. |
| `default` | expression | Default value (see [Defaults](#defaults)). |
| `db_column` | `"name"` | Column name. Default: the field name, or `<field>_id` for a foreign key. |
| `verbose_name` | `"date published"` | Label used by forms and the admin. Default: the field name with underscores replaced by spaces. |
| `help_text` | `"..."` | Help text shown by forms. |
| `editable` | `false` | Leaves the field out of model forms. |
| `upload_to` | `"dir"` | Directory under the storage root for `file`/`image` fields. |
| `on_delete`, `related_name`, `related_query_name`, `to_field`, `through` | strings | Relation options, see [Relations](relations.md). |

An unknown option is a compile error (`unknown field option`). Choices, validators and column comments cannot be
declared through the derive.

Two field-level markers use the `model` attribute instead:

- `#[model(skip)]` — the field is not a column. It is not saved, and it is filled with `Default::default()` when an
  instance is loaded. Use it for transient data carried on the struct.
- `#[model(state)]` (or any field of type `ModelState`) — reserved bookkeeping slot; it is not a column and is reset
  to `ModelState::default()` on load.

## Primary keys

Every model must declare its primary key; the derive fails with `a model needs a primary key field` otherwise.
The usual form is:

```rust
#[field(primary_key, auto)]
pub id: Option<i64>,
```

`None` means "not saved yet"; after the first `save` it holds the generated id.

- `auto` creates a 32-bit identity column on PostgreSQL (`integer GENERATED BY DEFAULT AS IDENTITY`), `big_auto` a
  64-bit one (`bigint ...`). Foreign keys pointing at the model use the matching integer type. On SQLite both are
  `integer PRIMARY KEY AUTOINCREMENT`. Prefer `big_auto` for tables that can grow large on PostgreSQL.
- The Rust type may also be a plain integer (`pub id: i64`). In that case `0` is treated as "not saved yet".
- `#[field(primary_key)]` alone declares a key you assign yourself, for example a `String` code or a `Uuid`. Nothing
  generates the value. Create such rows with `objects().create(...)`, which always inserts; `save()` on a new
  instance first tries an `UPDATE`, finds no row, and then inserts.

`pk` is accepted wherever a field name is: `filter_kw("pk", 3)`, `order_by(&["pk"])`, `f.pk()` in typed filters.

## Defaults

```rust
#[field(default = 0)]        pub votes: i32,
#[field(default = false)]    pub done: bool,
#[field(default = "draft")]  pub status: String,
```

The expression is anything that converts to a database value. The default is model metadata, and it is used in
these places:

- `makemigrations` requires it when you add a non-nullable column to an existing table, and the generated
  `ALTER TABLE ... ADD COLUMN` carries it as the column `DEFAULT` so existing rows get a value;
- model forms use it as the field's initial value;
- `on_delete = "set_default"` writes it into the referencing rows.

It is **not** applied to the Rust struct. A struct literal must give every field a value, and
`..Default::default()` fills the rest with Rust's own defaults (`0`, `false`, `""`), which are not necessarily the
`default = ...` you declared. `CREATE TABLE` does not emit a column default either. If the Rust default and the
declared default must agree, implement `Default` by hand.

## `auto_now` and `auto_now_add`

```rust
#[field(auto_now_add)] pub created: DateTime<Utc>,
#[field(auto_now)]     pub updated: DateTime<Utc>,
```

Both work on `DateTime<Utc>`, `NaiveDateTime` and `NaiveDate` fields (and their `Option<...>` forms). The value is
assigned in memory just before the row is written, so the instance you hold is up to date after `save`:

- `auto_now_add` assigns on insert only;
- `auto_now` assigns on every `save`, `create` and `bulk_create`.

Both mark the field as not editable and blank, so model forms skip it. A queryset `update(...)` writes exactly the
columns you list and does not refresh `auto_now` fields.

## Creating and saving instances

Everything that talks to the database is `async` and takes the connection as an argument.

```rust
use rango::db::models::Model;
use rango::db::Db;

let db = Db::default_db();

// INSERT, then `id` is filled in.
let mut q = Question { question_text: "What's new?".into(), ..Question::default() };
q.save(&db).await?;
assert!(q.id.is_some());

// UPDATE of every non-key column.
q.question_text = "What's up?".into();
q.save(&db).await?;

// Same as building + saving, returns the saved instance.
let q2 = Question::objects()
    .create(&db, Question { question_text: "Why?".into(), ..Question::default() })
    .await?;
```

How `save` decides:

1. The primary key is unset: `INSERT`. For `auto`/`big_auto` keys the generated id is written back into the struct.
2. The primary key is set: `UPDATE ... WHERE pk = ...`. If that touches no row, the instance is inserted with the
   key it carries.

`save_with` exposes the two knobs:

```rust
// Force an INSERT even though the key is set.
q.save_with(&db, true, None).await?;

// UPDATE only some columns.
c.votes += 1;
c.save_with(&db, false, Some(&["votes"])).await?;
```

With `update_fields`, an unknown or many-to-many field name is an error, and so is updating a row that does not
exist (`Save with update_fields did not affect any rows.`).

`save` does not validate. Length limits, `blank` and choices are checked by forms and serializers; the database
enforces `NOT NULL`, `UNIQUE` and foreign keys (and, on PostgreSQL, `varchar` lengths).

`save` sends the `PRE_SAVE` and `POST_SAVE` signals; see [Signals](../signals.md).

## Reloading and deleting

```rust
q.refresh_from_db(&db).await?;      // re-read every column by primary key

let (total, per_model) = q.delete(&db).await?;
```

`delete` removes the row and everything that depends on it according to each relation's `on_delete`, and returns
the total number of rows deleted plus a count per model label (`{"polls.Choice": 2, "polls.Question": 1}`).
Deleting an instance whose primary key is unset is an error. Cascading is described in
[Relations](relations.md#deleting-related-rows).

## What the derive generates

For `struct Question` the derive produces:

- `impl Model for Question`. The trait provides:

  | Item | Purpose |
  |---|---|
  | `Question::objects()` | A `QuerySet<Question>` over all rows; see [Making queries](queries.md). |
  | `Question::label()` | `"polls.Question"`. |
  | `Question::meta()` | The model's `ModelMeta` (fields, table name, options). |
  | `save`, `save_with`, `delete`, `refresh_from_db` | Described above. |
  | `pk_value()`, `set_pk(value)` | The primary key as a `Value`. |
  | `to_data()`, `from_data(row)` | Conversion between the struct and database values. |
  | `pre_save(created)` | Assigns `auto_now`/`auto_now_add` fields. |

- A registration that adds the model to the global registry when the program starts. There is no list of models to
  maintain; a model exists as soon as it is compiled into the binary.

- `QuestionFields`, a struct of typed field handles used by `filter!` and by closure filters. It has one method per
  field returning a `Path<T>` (`f.question_text()`), plus `pk()`, `eq`, `is_in`, `isnull` and `reverse`. For a
  relation field the method returns the target model's `…Fields`, so paths chain:
  `f.question().question_text().startswith("What")`. A foreign key also gets `f.question_id()`.

- For each `ForeignKey`/`OneToOne` field, a reverse accessor on the **target** type:
  `question.choice_set()` returns `QuerySet<Choice>` filtered to that question.

- For each `ManyToMany` field `authors`, a method `book.authors_manager()` on the owning type that returns the
  relation manager (`add`, `remove`, `set`, `clear`, `all`).

## Models in templates

Model structs are not template values. Convert an instance with `model_value` before putting it in a context:

```rust
use rango::views::generic::model_value;
use rango::{context, shortcuts::render};

let latest = Question::objects().slice(0, Some(5)).fetch(&db).await?;
render(&req, "polls/index.html", context! {
    "latest_question_list" => latest.iter().map(model_value).collect::<Vec<_>>(),
})
```

In the template the value supports:

- `{{ question.question_text }}` — any column by field name;
- `{{ choice.question_id }}` (or `{{ choice.question }}`) — the stored foreign-key value;
- `{{ question.pk }}` — the primary key;
- `{{ question }}` — the `str` field, or `Question object (1)`.

It does not follow relations: there is no `{{ question.choice_set.all }}`, because template rendering cannot run
queries. Load related rows in the view and pass them in as their own context entries.

## Not supported yet

- Abstract, proxy and multi-table inheritance.
- Multi-column indexes and check constraints declared on the model.
- Per-field `choices`, validators and `unique_for_date` in the derive.
- Model-level validation before saving.
- Generated columns and file-path fields.

## See also

- [Making queries](queries.md)
- [Relations](relations.md)
- [Migrations](migrations.md)
- [Model field reference](../../ref/model-fields.md)
- [Model forms](../forms/modelforms.md)
- [Signals](../signals.md)
- [Tutorial part 2: models](../../tutorial/02-models.md)
