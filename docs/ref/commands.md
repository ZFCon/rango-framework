# Management commands reference

Every command that ships with Rango: what it does, its arguments, and an example. To write your own command, see
[Writing management commands](../topics/commands.md).

## Running commands

A project runs its commands through its own binary, because the project binary is what links your apps, models and
migrations:

```console
$ cargo run -- <command> [arguments]
$ cargo run -- help
$ cargo run -- help migrate
```

`help` with no command lists every available command grouped by app. `help <command>` shows that command's usage
(it is the same as `<command> --help`). The project's `src/main.rs` calls
`rango::core::management::execute_from_command_line`, which does the dispatching.

`rango-admin` (installed with `cargo install rangoframework-cli`) has no project behind it. Use it for
`startproject` and `startapp`; run everything else through the project binary.

A command is available when it is registered by a `rango.*` app (all the commands on this page) or by an app that is
listed in `installed_apps`. Asking for any other command prints `Unknown command: '<name>'` and exits with status 1.

### Options every command accepts

| Option | Meaning |
|---|---|
| `--version` | Print the Rango version and exit. |
| `-v`, `--verbosity <0\|1\|2\|3>` | Output level: 0 minimal, 1 normal (default), 2 verbose, 3 very verbose. Only some commands print less at 0; `migrate` prints nothing about its progress below 1. |
| `--traceback` | Accepted for compatibility; has no effect. |
| `--no-color` | Do not colorize output. |
| `--force-color` | Colorize output even when stdout is not a terminal. |
| `--skip-checks` | Skip the [system checks](../topics/checks.md) that run before the command. Only offered by commands that run checks. |

Colour is on only when stdout is a terminal, `NO_COLOR` is not set and `--no-color` is not given (or
`--force-color` is given).

### System checks before a command

Commands marked "runs checks" below run all non-deployment system checks before they start and stop with the
report if any check is at `ERROR` level or higher. Commands marked "no checks" skip them and do not accept
`--skip-checks` (`runserver` runs them itself and accepts `--skip-checks`).

### Exit status

| Status | Cause |
|---|---|
| 0 | Success. |
| 1 | The command failed (`CommandError: <message>` on stderr), a system check failed, or the command is unknown. |
| 2 | Bad command-line usage (missing or unknown argument); the usage message is on stderr. |

## Summary

| Command | Purpose | Checks |
|---|---|---|
| [`changepassword`](#changepassword) | Set a user's password. | runs |
| [`check`](#check) | Run the system checks. | no |
| [`collectstatic`](#collectstatic) | Copy static files into `static_root`. | runs |
| [`compilemessages`](#compilemessages) | Compile `.po` catalogs to `.mo`. | no |
| [`createsuperuser`](#createsuperuser) | Create an administrator account. | runs |
| [`dbshell`](#dbshell) | Open the database's command-line client. | runs |
| [`diffsettings`](#diffsettings) | Show settings that differ from the defaults. | no |
| [`dumpdata`](#dumpdata) | Write the database contents as a fixture. | runs |
| [`findstatic`](#findstatic) | Locate static files. | runs |
| [`flush`](#flush) | Delete all data. | runs |
| [`inspectdb`](#inspectdb) | Generate model code from existing tables. | runs |
| [`loaddata`](#loaddata) | Load fixtures. | runs |
| [`makemessages`](#makemessages) | Extract translatable strings into a `.po` file. | no |
| [`makemigrations`](#makemigrations) | Write new migrations. | runs |
| [`migrate`](#migrate) | Apply or unapply migrations. | runs |
| [`runserver`](#runserver) | Start the development server. | own (see below) |
| [`showmigrations`](#showmigrations) | List migrations and their state. | runs |
| [`spectacular`](#spectacular) | Generate the OpenAPI document. | runs |
| [`sqlflush`](#sqlflush) | Print the SQL `flush` would run. | runs |
| [`sqlmigrate`](#sqlmigrate) | Print the SQL of a migration. | runs |
| [`startapp`](#startapp) | Create an app skeleton. | no |
| [`startproject`](#startproject) | Create a project skeleton. | no |
| [`version`](#version) | Print the Rango version. | no |

Commands that talk to the database take `--database <alias>` (default `default`), naming a key of
`Settings::databases`.

## changepassword

```console
$ cargo run -- changepassword <username> --password <password> [--database <alias>]
```

Sets the password of an existing user (found by the user model's username) and saves only the password column.
Both `username` and `--password` are required; the command never prompts. Fails with `user '<name>' does not exist`
for an unknown user.

```console
$ cargo run -- changepassword alice --password 'a long passphrase'
Password changed successfully for user 'alice'
```

Part of `rango.contrib.auth`. A password on the command line is visible in the process list and shell history.

## check

```console
$ cargo run -- check [app_label ...] [-t <tag>]... [--list-tags] [--deploy]
                     [--fail-level <level>] [--database <alias>]...
```

Runs the registered system checks and prints a report. No checks run before it (it is the check).

| Argument | Meaning |
|---|---|
| `app_label ...` | Restrict model-related checks to these app labels. |
| `-t`, `--tag <tag>` | Run only checks carrying this tag. Repeatable. |
| `--list-tags` | Print the available tags, one per line, and exit. With `--deploy` the deployment checks' tags are included. |
| `--deploy` | Also run the deployment checks (the [security checks](../topics/checks.md#built-in-security-checks)). |
| `--fail-level <level>` | Lowest level that makes the command exit non-zero: `CRITICAL`, `ERROR` (default), `WARNING`, `INFO` or `DEBUG`. |
| `--database <alias>` | Allow database checks to use this connection. Repeatable. |

The report groups messages as CRITICALS, ERRORS, WARNINGS, INFOS and DEBUGS and ends with a count that includes how
many messages were silenced through `silenced_system_checks`:

```console
$ cargo run -- check
System check identified no issues (0 silenced).
$ cargo run -- check --deploy --fail-level WARNING
```

If a message at or above `--fail-level` is not silenced, the report goes to stderr and the exit status is 1.

## collectstatic

```console
$ cargo run -- collectstatic [--noinput] [-c|--clear] [-n|--dry-run] [-i|--ignore <pattern>]...
```

Copies every static file found by the finders (each `staticfiles_dirs` entry, then each installed app's `static`
directory) into `static_root`. The first location that holds a path wins. Requires `static_root`; without it the
command fails with an `ImproperlyConfigured` message.

| Argument | Meaning |
|---|---|
| `--noinput`, `--no-input` | Accepted for scripts; the command never asks for confirmation. |
| `-c`, `--clear` | Delete `static_root` before copying (skipped with `--dry-run`). |
| `-n`, `--dry-run` | Report what would be copied without writing anything. |
| `-i`, `--ignore <pattern>` | Glob matched against file and directory names to skip. Repeatable. The patterns `CVS`, `.*` and `*~` are always ignored. |

Files whose destination already has identical content count as unmodified and are not rewritten. If the
`staticfiles` entry of `storages` has a backend ending in `ManifestStaticFilesStorage`, files are written under
content-hashed names (`app.1a2b3c4d5e6f.css`) and a `staticfiles.json` manifest is written to `static_root`.

```console
$ cargo run -- collectstatic --noinput

12 static files copied to '/srv/mysite/static', 3 unmodified.
```

## compilemessages

```console
$ cargo run -- compilemessages [--path <dir>]
```

Finds every file named `rango.po` below `--path` (default `.`, skipping `target` and `.git`) and writes a `rango.mo`
beside it. Prints `processing file rango.po in <dir>` for each. See [Internationalization](../topics/i18n.md).

## createsuperuser

```console
$ cargo run -- createsuperuser [--username <name>] [--email <address>] [--noinput] [--database <alias>]
```

Creates an active staff user with every permission. The command is non-interactive: it never prompts. The username
comes from `--username` or the `RANGO_SUPERUSER_USERNAME` environment variable and is required; the email comes from
`--email` or `RANGO_SUPERUSER_EMAIL` (empty if neither is set); the password comes only from
`RANGO_SUPERUSER_PASSWORD`. Without a password the account gets an unusable password, which you can fix with
`changepassword`.

`--noinput` is accepted but changes nothing. An existing username fails with `Error: That username is already taken.`

```console
$ RANGO_SUPERUSER_PASSWORD='a long passphrase' cargo run -- createsuperuser --username admin --email admin@example.com
Superuser created successfully.
```

Part of `rango.contrib.auth`.

## dbshell

```console
$ cargo run -- dbshell [--database <alias>]
```

Starts the database's own command-line client against the configured connection: `sqlite3 <name>` for SQLite, or
`psql` with `-U`, `-h`, `-p` taken from `user`, `host` and `port` and the password passed in `PGPASSWORD` for
PostgreSQL. The client must be on `PATH`; otherwise the command fails with "You appear not to have the database
client installed". A non-zero client exit status is reported as an error.

## diffsettings

```console
$ cargo run -- diffsettings [--all]
```

Prints every setting whose value differs from `Settings::default()` as `NAME = <JSON value>`, sorted by name.

| Argument | Meaning |
|---|---|
| `--all` | Print every setting; those equal to their default are prefixed with `### `. |

```console
$ cargo run -- diffsettings
DEBUG = true
INSTALLED_APPS = ["polls"]
SECRET_KEY = "insecure-demo-key"
```

## dumpdata

```console
$ cargo run -- dumpdata [app_label | app_label.Model ...] [--format json|jsonl] [--indent <n>]
                        [-o|--output <file>] [--database <alias>] [-e|--exclude <name>]...
```

Writes the rows of the selected models as a fixture. With no positional arguments every managed model of every
installed app is dumped. Rows are ordered by primary key; models are ordered by app label and model name; many-to-many
relations are written as lists of primary keys inside their owning object (auto-created through tables are not
dumped separately).

| Argument | Meaning |
|---|---|
| `app_label`, `app_label.Model` | What to dump. An unknown label fails with `No installed app with label '<label>'.`; an unknown model with `Unknown model: <label>`. |
| `--format <format>` | `json` (default, one array) or `jsonl` (one object per line). Any other value fails with `Unknown serialization format`. XML and YAML are not implemented. |
| `--indent <n>` | Pretty-print JSON with `n` spaces. Without it the output is compact. |
| `-o`, `--output <file>` | Write to a file instead of stdout. |
| `-e`, `--exclude <name>` | Skip an app label or an `app_label.Model`. Repeatable. |

Each object has the fixture shape `{"model": "polls.question", "pk": 1, "fields": {...}}`.

```console
$ cargo run -- dumpdata polls --indent 2 -o polls.json
$ cargo run -- dumpdata --exclude auth.permission --format jsonl > everything.jsonl
```

## findstatic

```console
$ cargo run -- findstatic <path> [<path> ...]
```

For each path (relative to a static directory, such as `css/site.css`) prints every file the finders locate, in
search order, or `No matching file found for '<path>'.`

```console
$ cargo run -- findstatic css/site.css
Found 'css/site.css' here:
  /home/me/mysite/static/css/site.css
```

## flush

```console
$ cargo run -- flush --noinput [--database <alias>]
```

Deletes every row of every managed model table of the installed apps, inside one transaction, then sends the
`post_migrate` signal for each app so that built-in data (content types, permissions) is created again. Without
`--noinput` it refuses to run (`flush deletes all data; pass --noinput to confirm.`); there is no interactive
prompt. See [`sqlflush`](#sqlflush) for the exact statements.

## inspectdb

```console
$ cargo run -- inspectdb [table ...] [--database <alias>] [--app-label <label>]
```

Reads the schema of the database and prints Rust model definitions. With no table names every table is included.

| Argument | Meaning |
|---|---|
| `table ...` | Only these tables. |
| `--app-label <label>` | Value written to each model's `app_label` attribute. Default `app`. |

Each table becomes a `#[derive(Model)]` struct with `managed = false`. Column types map to Rust types (integers to
`i16`/`i32`/`i64`, text to `String` with `max_length` for `varchar(n)`, numerics to `Decimal`, dates and times to
`chrono` types, `uuid`, JSON), nullable columns to `Option<_>` with `null` and `blank`, primary keys get
`primary_key`, and SQLite foreign keys become `ForeignKey<Target>` fields. Unknown column types fall back to
`String`. The header comment says to review the result; always do, especially relationships on PostgreSQL (foreign
keys are only detected on SQLite).

## loaddata

```console
$ cargo run -- loaddata <fixture> [<fixture> ...] [--database <alias>]
```

Loads fixtures written by [`dumpdata`](#dumpdata) (JSON array or JSON Lines). A `fixture` that has no dot is a name:
the command looks for `<name>.json` then `<name>.jsonl` in each of the `fixture_dirs` entries, the current
directory, and each installed app's `fixtures` directory. A name with a dot (`polls.json`) is looked for in those
same directories and then as a path. The first file found is used; none found fails with `No fixture named '<name>'
found.`

An object whose primary key already exists is updated, otherwise it is inserted; many-to-many lists are replaced.
Objects are saved one at a time without a surrounding transaction, and a bad object stops the load part-way.

```console
$ cargo run -- loaddata initial_polls
Installed 14 object(s) from 1 fixture(s)
```

## makemessages

```console
$ cargo run -- makemessages -l <lang> [-l <lang>]... [--path <dir>]
```

Scans `--path` (default `.`) for `.rs`, `.html` and `.txt` files, skipping `target`, `.git`, `node_modules` and
`locale`, and writes `locale/<lang>/LC_MESSAGES/rango.po` for each `-l`. At least one `-l` is required.

It extracts the first string literal arguments of `gettext(`, `gettext_lazy(`, `ngettext(`, `pgettext(` and
`npgettext(` in Rust sources, and the literal of `{% trans %}` / `{% translate %}` (with an optional `context`) in
templates. Existing translations in the `.po` file are kept for messages that are still present; messages no longer
found are dropped. Each message gets a `#:` comment listing the files that use it. Plural entries get `nplurals`
slots from the file's `Plural-Forms` header (default 2). Prints `processing locale <lang>`.

`{% blocktrans %}` bodies are not extracted.

## Translation workflow

```console
$ cargo run -- makemessages -l de
$ # translate locale/de/LC_MESSAGES/rango.po
$ cargo run -- compilemessages
```

## makemigrations

```console
$ cargo run -- makemigrations [app_label ...] [--dry-run] [-n|--name <name>] [--check]
```

Compares the models of the installed apps with the state described by the existing migrations and writes a new
migration for every app that changed.

| Argument | Meaning |
|---|---|
| `app_label ...` | Only these apps. An app that is not installed fails with `App '<label>' could not be found. Is it in INSTALLED_APPS?` |
| `--dry-run` | Print the planned operations; write nothing. |
| `-n`, `--name <name>` | Name part of the file (the number prefix is added). Default: a name suggested from the operations, or `initial` for an app's first migration. |
| `--check` | Write nothing and exit with status 1 if any model change lacks a migration. Prints `No changes detected` and exits 0 otherwise. |

Migration files are Rust modules named `m<number>_<name>.rs` inside the app's `migrations` directory (found at
`src/<app>/migrations` or `./<app>/migrations`); the command appends the matching `pub mod` line to the directory's
`mod.rs`. The new migration depends on the app's previous one and on the latest migration of any app it references.
Rebuild the project afterwards: the new file is compiled in, then `migrate` can see it.

```console
$ cargo run -- makemigrations polls
Migrations for 'polls':
  src/polls/migrations/m0001_initial.rs
    + Create model Question
Rebuild the project (cargo build) for the new migration to be picked up.
```

See [Migrations](../topics/db/migrations.md).

## migrate

```console
$ cargo run -- migrate [app_label [migration_name]] [--database <alias>] [--noinput]
```

Applies unapplied migrations, or moves an app back to an earlier migration.

| Argument | Meaning |
|---|---|
| `app_label` | Migrate only this app (and what it depends on). |
| `migration_name` | Bring the app to the state right after this migration; later ones are unapplied. `zero` unapplies all of the app's migrations. |
| `--database <alias>` | Database to migrate. |
| `--noinput`, `--no-input` | Accepted; the command never prompts. |

At verbosity 1 and above it prints `Operations to perform:`, `Running migrations:` and one line per migration
applied or unapplied. It sends the `post_migrate` signal once per app when it finishes (see [Signals](../topics/signals.md)).

```console
$ cargo run -- migrate
$ cargo run -- migrate polls 0001
$ cargo run -- migrate polls zero
```

There is no `--fake`, `--plan` or `--run-syncdb`.

## runserver

```console
$ cargo run -- runserver [[addr:]port] [-6|--ipv6] [--nothreading] [--noreload] [--skip-checks]
```

Starts the development server. The default address is `127.0.0.1:8000` (`[::1]:8000` with `-6`). `addr:port` accepts an
IPv4 address, a bracketed IPv6 address or a host name; `0` as address means `0.0.0.0`. An argument that is not a port
or `addr:port` fails with `"<arg>" is not a valid port number or address:port pair.`

The command refuses to start with `You must set settings.ALLOWED_HOSTS if DEBUG is False.` when `debug` is off and
`allowed_hosts` is empty. Unless `--skip-checks` is given it prints `Performing system checks...` and runs the checks
first. It then prints the date, the Rango version and `Starting development server at http://<addr>:<port>/`, and
serves until you press Ctrl-C.

`--nothreading` and `--noreload` are accepted for scripts but ignored: the server is always asynchronous and never
reloads. Use an external watcher such as `cargo watch -x run` while developing:

```console
$ cargo run -- runserver 0.0.0.0:8080
$ cargo watch -x 'run -- runserver'
```

The development server is not meant for production. See [Deployment](../topics/deployment.md).

## showmigrations

```console
$ cargo run -- showmigrations [app_label ...] [--database <alias>]
```

Lists each app that has migrations (or only the given apps) and its migrations in order, with `[X]` for applied and
`[ ]` for not applied in the chosen database:

```console
$ cargo run -- showmigrations
polls
 [X] 0001_initial
 [ ] 0002_question_pub_date
```

## spectacular

```console
$ cargo run -- spectacular [--file <path>]
```

Generates the OpenAPI document of the project's REST API as pretty-printed JSON, on stdout or into `--file`. Part
of `rango.rest`. Its options come from `spectacular_settings`; see [OpenAPI](../topics/rest/openapi.md).

## sqlflush

```console
$ cargo run -- sqlflush [--database <alias>]
```

Prints the SQL that `flush` would run. SQLite: a `DELETE FROM` per table plus a statement resetting
`sqlite_sequence`. PostgreSQL: one `TRUNCATE ... RESTART IDENTITY CASCADE;`. Nothing is executed.

## sqlmigrate

```console
$ cargo run -- sqlmigrate <app_label> <migration_name> [--database <alias>] [--backwards]
```

Prints the SQL of one migration, wrapped in `BEGIN;` and `COMMIT;`, with a comment per operation. `migration_name`
is matched as a prefix, so `0001` is enough; no match fails with `Cannot find a migration matching '<name>' from
app '<app>'.` `--backwards` prints the SQL that undoes it. The SQL is for the dialect of the chosen database.

## startapp

```console
$ rango-admin startapp <name> [directory]
$ cargo run -- startapp <name> [directory]
```

Creates an app skeleton. The name must be a valid Rust identifier (letters, digits, underscores; no leading digit)
and must not be a keyword or one of `rango`, `std`, `core`, `alloc`, `test`, `proc_macro`, `tokio`.

When run from a project root (where `src/main.rs` exists) the app goes in `src/<name>` and `mod <name>;` is added
after the last `mod` line of `src/main.rs`, so that its `register_app!` is linked. Elsewhere it goes in `./<name>`;
`directory` overrides the location. An existing target directory is an error.

The skeleton contains `mod.rs` (with `register_app!`), `models.rs`, `views.rs`, `urls.rs`, `admin.rs`, `tests.rs`
and `migrations/mod.rs`. The command does not edit your settings: add the app's name to `installed_apps` yourself.

## startproject

```console
$ rango-admin startproject <name> [directory] [--rango-path <path>]
```

Creates a project: `Cargo.toml`, `.gitignore`, `src/main.rs`, `src/settings.rs`, `src/urls.rs`, and empty
`templates/` and `static/` directories. The settings get a freshly generated `secret_key` starting with
`rango-insecure-`.

| Argument | Meaning |
|---|---|
| `name` | Project (and crate) name; same rules as `startapp`. |
| `directory` | Destination; default `./<name>`. It must not exist or must be empty. |
| `--rango-path <path>` | Depend on a local checkout of Rango (`<path>/crates/rango`) instead of the published crate. |

## version

```console
$ cargo run -- version
```

Prints the Rango version. `--version` on any command does the same.

## Not supported

- Interactive prompts: `createsuperuser`, `flush`, `migrate` and `collectstatic` never ask questions.
- `shell`, `test`, `testserver`, `sendtestemail`, `squashmigrations`, `optimizemigrations`, `showmigrations --plan`,
  `createcachetable`, `clearsessions` and `remove_stale_contenttypes`. Run tests with `cargo test`.
- `dumpdata` and `loaddata` in XML or YAML.

## See also

- [Writing management commands](../topics/commands.md)
- [System checks](../topics/checks.md)
- [Settings reference](settings.md)
- [Migrations](../topics/db/migrations.md)
- [Static files](../topics/static-files.md)
