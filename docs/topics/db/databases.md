# Databases

Rango talks to SQLite and PostgreSQL. This page covers configuring database aliases, what each backend does with a
connection and with your data, how to pick a database for a query, closing pools, and the commands that work on the
database directly (`dbshell`, `inspectdb`, `flush`, `dumpdata`, `loaddata`).

```rust
use rango::db::{Db, Executor};
```

## Configuring databases

The `databases` setting maps an alias to a `DatabaseConfig`. The alias `default` is the one every command and
`Db::default_db()` use unless told otherwise. The full field list is in [Settings](../settings.md#databases); the
fields the ORM reads are these:

| Field | SQLite | PostgreSQL |
|---|---|---|
| `engine` | `"rango.db.backends.sqlite3"` | `"rango.db.backends.postgresql"` |
| `name` | file path; empty or `":memory:"` for an in-memory database | database name |
| `user`, `password`, `host`, `port` | ignored | added to the connection when not empty |
| `options` | ignored | `{"max_connections": N}` sets the pool size (default 10) |

```rust
use std::collections::BTreeMap;
use rango::conf::{DatabaseConfig, Settings};

Settings {
    databases: BTreeMap::from([
        (
            "default".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.sqlite3".into(),
                name: "db.sqlite3".into(),
                ..DatabaseConfig::default()
            },
        ),
        (
            "reports".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.postgresql".into(),
                name: "reports".into(),
                user: "app".into(),
                password: std::env::var("REPORTS_DB_PASSWORD").unwrap_or_default(),
                host: "db.internal".into(),
                port: "5432".into(),
                options: rango::json!({ "max_connections": 20 }),
                ..DatabaseConfig::default()
            },
        ),
    ]),
    ..Settings::default()
}
```

The other `DatabaseConfig` fields (`atomic_requests`, `autocommit`, `conn_max_age`, `conn_health_checks`,
`disable_server_side_cursors`, `time_zone`, `test`) are accepted but not used yet, and so is the `database_routers`
setting.

Errors you can meet when an alias is first used:

| Situation | Error |
|---|---|
| The alias is not in `databases` | `ImproperlyConfigured`: `The connection 'x' doesn't exist.` |
| `engine` is anything else, including the default `rango.db.backends.dummy` | `ImproperlyConfigured`: `settings.DATABASES is improperly configured. Unsupported or unknown ENGINE "..."` |
| The server or file cannot be opened | `Error::Other` with the driver's message |

## Choosing a database in code

A `Db` is a small, cheap-to-clone handle that stores only an alias. There is no ambient "current database": every
query names its connection, so the database a statement uses is always visible in the code.

```rust
let db = Db::default_db();            // alias "default"
let reports = Db::using("reports");   // any alias from `databases`

let n = Question::objects().count(&db).await?;
let rows = Sale::objects().filter_kw("year", 2025).fetch(&reports).await?;
```

- A model is not tied to an alias; you decide per call. The same model can be read from one database and written to
  another.
- Relations cannot span databases. Joins, `select_related` and cascading deletes all run on the connection you pass.
- `db.vendor()` returns `rango::db::backends::Vendor::Sqlite` or `Vendor::Postgresql` when you need to branch on the
  backend; `db.alias()` returns the alias.
- Management commands that touch the database take `--database ALIAS` (`migrate`, `showmigrations`, `sqlmigrate`,
  `dbshell`, `inspectdb`, `flush`, `sqlflush`, `dumpdata`, `loaddata`).

For a group of statements that must succeed together use `db.atomic(...)`; see [Transactions](transactions.md).

### Connection pools

Each alias gets one pool, created the first time the alias is used and kept for the life of the process. Statements
borrow a connection and give it back. Changing `databases` afterwards has no effect on an alias that is already open
(tests that switch databases call `rango::db::connection::close_all().await` first, which forgets every pool so that
the next use opens a fresh one).

| | SQLite | PostgreSQL |
|---|---|---|
| Pool size | 8 connections for a file, exactly 1 for an in-memory database | `options.max_connections`, default 10 |
| When all are in use | The caller waits for one to be returned. | The same. |
| Connections are | opened on demand, kept idle for reuse | opened on demand, kept idle for reuse; closed ones are discarded |

Because callers wait, a transaction that holds a connection while it runs a statement through `db` instead of its own
`tx` can block itself; see [Transactions](transactions.md#pitfalls).

## SQLite

- `name` is a path, resolved against the process's working directory. `db.sqlite3` in the project root is the
  convention.
- Every connection is opened with `PRAGMA foreign_keys = ON` and `busy_timeout = 5000` (writers wait up to five
  seconds for a lock before failing with `database is locked`).
- A `REGEXP` function is registered, so `regex` / `iregex` lookups work, using the syntax of the Rust `regex` crate.
  Date-part lookups (`pub_date__month`) use helper functions registered on each connection.
- An in-memory database (`":memory:"` or empty `name`) lives only as long as its single connection, belongs to that
  pool alone, and cannot be shared between aliases. It suits tests; for anything else use a file.
- Statements run on a blocking thread (`spawn_blocking`) so they do not stall the async runtime.
- `LIKE` is case-insensitive for ASCII text, so `contains` behaves like `icontains` (see
  [Making queries](queries.md#lookups)). `select_for_update` does nothing.

### How SQLite stores values

SQLite has few native types, so the ORM converts. You only see this when reading the file with other tools.

| Rust type | Stored as |
|---|---|
| `bool` | integer 0 or 1 |
| `DateTime<Utc>` / `NaiveDateTime` | text `YYYY-MM-DD HH:MM:SS[.ffffff]`, in UTC |
| `NaiveDate`, `NaiveTime` | text `YYYY-MM-DD`, `HH:MM:SS[.ffffff]` |
| `Decimal` | text, rounded to `decimal_places` when it is set |
| `Uuid` | text, 32 hexadecimal characters without hyphens |
| `serde_json::Value` | text (the column also has a `JSON_VALID` check) |
| `TimeDelta` | integer, microseconds |
| `Vec<u8>` | blob |

## PostgreSQL

- The connection string is built as `dbname=... host=... port=... user=... password=...`, leaving out the fields that
  are empty. Values are placed in it as they are, so a password with spaces or quotes is not supported.
- Connections are made without TLS. Put the database on a private network or behind a local proxy that terminates
  TLS.
- Each statement is prepared on the connection that runs it; there is no client-side statement cache.
- Datetimes are `timestamp with time zone` and arrive as `DateTime<Utc>`. Durations are `interval`; months in an
  interval value are read as 30 days.
- `contains`, `startswith` and `endswith` are case-sensitive, and `select_for_update` appends `FOR UPDATE` to the
  query.
- Auto primary keys are identity columns (`GENERATED BY DEFAULT AS IDENTITY`).
- Column types that Rango does not map are read back as text.

The column types each backend uses for every field kind are listed in the
[model field reference](../../ref/model-fields.md#column-types).

## Raw access

Anything the ORM does not cover can go through the `Executor` methods of a `Db` or a `Transaction`: `query`,
`execute`, `execute_batch`. Use `%s` placeholders on both backends. See [Making queries](queries.md#raw-sql).

```rust
let rows = db.query("SELECT count(*) FROM polls_choice WHERE votes > %s", &[Value::Int(10)]).await?;
let total = rows[0].get(0).as_i64();
```

## Commands

Run these through your project's entry point (`cargo run -- <command>`).

### `dbshell [--database ALIAS]`

Starts the backend's own client with the connection details from settings: `sqlite3 <name>` for SQLite, or `psql`
with `-U`, `-h`, `-p` and `PGPASSWORD` taken from the config for PostgreSQL. The client must be installed; if it is
not, the command fails with `You appear not to have the database client installed (...)`. A non-zero exit of the
client is reported as an error.

### `inspectdb [table ...] [--database ALIAS] [--app-label LABEL]`

Reads the tables of an existing database and prints Rust model source for them, to be redirected into a file:

```console
$ cargo run -- inspectdb --app-label legacy customers orders > src/legacy/models.rs
```

```rust
// Generated by `inspectdb`. Review field types and relationships before use.
use rango::db::models::Model;

#[derive(Model)]
#[model(app_label = "legacy", db_table = "customers", managed = false)]
pub struct Customers {
    #[field(primary_key)] pub id: i32,
    #[field(max_length = 100)] pub name: String,
    #[field(max_length = 254, null, blank)] pub email: Option<String>,
}
```

Without table names every table is listed (including `rango_migrations`); `--app-label` defaults to `app`. Every
generated model is `managed = false`, so migrations leave the table alone. Struct names are the CamelCase form of the
table name. Treat the result as a first draft:

- column types are guessed from the declared SQL type (integers become `i32`/`i64`/`i16`, `varchar(n)` becomes
  `String` with `max_length = n`, `numeric` becomes `Decimal`, `timestamp`/`datetime` become `DateTime<Utc>`, and
  anything unknown becomes `String`);
- nullable columns become `Option<...>` with `null, blank`;
- the primary key is marked `primary_key` but not `auto`; add `auto` or `big_auto` yourself when the database
  generates it, and change the Rust type to `Option<i64>` if you want `None` to mean "not saved yet";
- foreign keys are detected only on SQLite, where they become `ForeignKey<Target>` fields named without the `_id`
  suffix; on PostgreSQL they come out as plain integers;
- unique constraints, indexes and defaults are not reproduced.

### `flush --noinput [--database ALIAS]`, `sqlflush [--database ALIAS]`

`sqlflush` prints the SQL that empties every managed model table (`DELETE FROM` plus a sequence reset on SQLite,
`TRUNCATE ... RESTART IDENTITY CASCADE` on PostgreSQL). `flush` runs it in one transaction, then sends
`POST_MIGRATE` for every app so that content types, permissions and the default site are created again. It refuses to
run without `--noinput`.

### `dumpdata` and `loaddata`

`dumpdata [app_label[.Model] ...] [--format json|jsonl] [--indent N] [-o FILE] [-e EXCLUDE] [--database ALIAS]`
writes rows as a fixture to standard output or a file. `loaddata FIXTURE ... [--database ALIAS]` installs fixtures
given as file paths, or as names looked up as `<name>.json` / `<name>.jsonl` in the `fixture_dirs` setting, in each
app's `fixtures` directory and in the current directory. See [Serialization](../serialization.md).

## Not supported yet

- MySQL and Oracle.
- Database routers (`database_routers` is not read) and read replicas.
- TLS to PostgreSQL, Unix-socket specific options, `options` other than `max_connections`.
- Per-request transactions (`atomic_requests`), connection lifetime settings (`conn_max_age`) and health checks.
- A test database that is created and destroyed for you; tests usually configure an in-memory SQLite alias and run
  `migrate` in their setup (see [Testing](../testing.md)).
- `inspectdb` detection of relations, uniqueness and indexes on PostgreSQL.

## See also

- [Settings](../settings.md#databases)
- [Making queries](queries.md)
- [Transactions](transactions.md)
- [Migrations](migrations.md)
- [Model field reference](../../ref/model-fields.md)
- [Serialization](../serialization.md)
- [Testing](../testing.md)
