# Serialization and fixtures

`rango::db::serializers` turns table rows into a JSON document and back. The `dumpdata` and `loaddata` commands use it to export data and to load fixtures.

## The document shape

A dump is a list of objects, one per row. Each object has the model label in lower case, the primary key, and a `fields` map with every other concrete column.

```json
[
{"model": "polls.question", "pk": 1, "fields": {"question_text": "What's up?", "pub_date": "2024-01-05T12:00:00Z"}},
{"model": "polls.choice", "pk": 1, "fields": {"question": 1, "choice_text": "Not much", "votes": 3}},
{"model": "polls.book", "pk": 1, "fields": {"title": "Rust", "published": "2024-05-06", "price": "12.50", "authors": [1]}}
]
```

(These shapes are taken from `crates/rango-db/tests/serializers.rs`.)

Rules:

- The primary key is not repeated inside `fields`.
- A foreign key or one-to-one field is its target's primary key (`"question": 1`).
- A many-to-many field is a list of target primary keys (`"authors": [1]`), listed after the plain fields. Auto-created join tables are not dumped as their own models.
- Rows are ordered by primary key within each model. Models are sorted by app label, then by name.

Value encoding:

| Field type | JSON |
|---|---|
| boolean, integers, floats | number or boolean |
| decimal | string, rescaled to the field's `decimal_places` (`"12.50"`) |
| text, UUID | string |
| date | `"2024-05-06"` |
| time | `"12:30:00"`, with `.ffffff` microseconds only when non-zero |
| datetime | `"2024-01-05T12:00:00Z"` for aware values; no `Z` for naive values; microseconds only when non-zero |
| duration | duration string |
| JSON field | the JSON value itself |
| null | `null` |
| bytes | text decoded lossily from UTF-8 (binary data does not round-trip) |

## Export with dumpdata

```console
$ cargo run -- dumpdata polls --indent 2 -o polls.json
$ cargo run -- dumpdata polls.Question --format jsonl
$ cargo run -- dumpdata --exclude auth --exclude sessions
```

`dumpdata [app_label[.Model] ...]`:

| Option | Default | Meaning |
|---|---|---|
| positional `app_label` or `app_label.Model` | all installed apps | Models to dump. An unknown label is an error (`No installed app with label '...'` or `Unknown model: ...`). Model names are case-insensitive. |
| `--format` | `json` | `json` or `jsonl`. Anything else fails with `Unknown serialization format`. |
| `--indent N` | none | Pretty-print JSON with N spaces. Ignored for `jsonl`. |
| `-o`, `--output FILE` | stdout | Write to a file instead of printing. |
| `--database ALIAS` | `default` | Which configured database to read. |
| `-e`, `--exclude X` | none | Repeatable. Drops an app label or an `app.Model`. |

With no labels, abstract and unmanaged models are skipped. `jsonl` writes one object per line.

## Import with loaddata

```console
$ cargo run -- loaddata polls.json
$ cargo run -- loaddata initial_data
```

`loaddata fixture [fixture ...] [--database ALIAS]`. A name containing a dot is a file name; a name without one is tried as `NAME.json`, then `NAME.jsonl`. Files are searched in the directories of the `fixture_dirs` setting, the current directory, and each installed app's `fixtures/` directory, and, for names with a dot, as a literal path. The format comes from the extension: `.jsonl` is line-delimited, everything else is read as one JSON array.

```rust
use rango::conf::Settings;

let settings = Settings { fixture_dirs: vec!["fixtures".into()], ..Settings::default() };
```

Behaviour:

- Each object is inserted, or updated in place when a row with that primary key exists. Loading the same fixture twice is safe, and gives the same data.
- An object without `pk` (or with `null`) is inserted and gets a new key.
- Fields missing from `fields` are left out of the insert, so database defaults apply (and an update keeps the current value).
- Many-to-many lists replace the existing links of that row.
- Errors name the file: `Problem installing fixture '<path>': ...`. Bad entries report `Invalid model identifier`, `Fixture entry without "model"`, `Fixture entry without "fields"`, or `Fixture must be a JSON array`.
- It prints `Installed N object(s) from M fixture(s)`.
- Rows are written one statement at a time, with no wrapping transaction and no constraint deferral. Order objects so foreign-key targets come first (a `dumpdata` output is only safe when its model order satisfies that), or load into a database that does not enforce the constraint yet.

## From code

The functions behind the commands are public.

```rust
use rango::db::serializers::{dump, format_objects, load_objects, parse_objects, select_models};
use rango::db::Db;

let db = Db::default_db();
let models = select_models(&["polls".to_owned()])?;      // empty slice means all installed apps
let objects = dump(&db, &models).await?;                  // Vec<serde_json::Value>
let text = format_objects(&objects, "json", Some(2))?;    // or "jsonl", indent None

let parsed = parse_objects(&text, "json")?;
let saved = load_objects(&db, &parsed).await?;            // number of objects saved
```

`to_json(&Field, &Value)` and `from_json(&Field, &serde_json::Value)` convert single values.

## Fixtures in tests

Keep a fixture next to the app, in `fixtures/`, and load it from a test with `parse_objects` and `load_objects` after migrating the in-memory database (see [Testing](testing.md)):

```rust
let text = std::fs::read_to_string("fixtures/polls.json")?;
load_objects(&db, &parse_objects(&text, "json")?).await?;
```

## Limits and not supported yet

- Only `json` and `jsonl`. There is no XML or YAML format.
- No natural keys, no `--natural-foreign`, no `--pks`, no `--all`/`--use-base-manager`, and no `--ignorenonexistent`.
- `loaddata` has no `--app` option and no compressed fixtures.
- Unknown field names in a fixture are ignored silently.
- Binary columns do not round-trip.
- There is no public serializer for a single queryset or object; use the REST serializers for API output ([REST serializers](rest/serializers.md)).

## See also

- [Models](db/models.md)
- [Migrations](db/migrations.md)
- [Testing](testing.md)
- [Content types](contenttypes.md)
