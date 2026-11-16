# Writing management commands

A management command is a task you run as `cargo run -- <name>`: importing data, closing old records, sending a
digest. This page shows how to write one, register it, report errors, and test it. The commands Rango ships with are
listed in the [commands reference](../ref/commands.md).

## A first command

A command is a type that implements `rango::core::management::Command`, registered with `register_command!`. Two
extra crates are needed in your `Cargo.toml` because the trait is asynchronous and arguments are declared with
`clap`:

```toml
[dependencies]
rangoframework = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
async-trait = "0.1"
clap = "4"
```

This is the `greet` command from `crates/rango/examples/manage.rs`:

```rust
use rango::core::management::{Command, CommandContext, CommandError};
use rango::register_command;

#[derive(Default)]
struct Greet;

#[async_trait::async_trait]
impl Command for Greet {
    fn help(&self) -> &str {
        "Say hello."
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(clap::Arg::new("name").default_value("world"))
    }

    async fn handle(
        &self,
        _: &CommandContext,
        args: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        Ok(Some(format!(
            "Hello, {}!",
            args.get_one::<String>("name").expect("has default")
        )))
    }
}

register_command!("demo", "greet", Greet);
```

```console
$ cargo run -- greet
Hello, world!
$ cargo run -- greet Ada
Hello, Ada!
$ cargo run -- help greet
```

`register_command!(app, name, Type)` takes three arguments:

| Argument | Meaning |
|---|---|
| `app` | The app the command belongs to: the full app name, such as `"polls"`. The command is available only when that app is in `installed_apps` (see [Applications](apps.md)). |
| `name` | The word typed after `cargo run --`. |
| `Type` | A type that implements `Command` and `Default`; Rango builds a fresh value each time the command runs. |

Like `register_app!`, the macro runs at link time, so the module holding it must be compiled into your binary (a
`mod` line in `main.rs`). Do not reuse the name of a built-in command; which one wins is unspecified.

## The Command trait

```rust
#[async_trait]
pub trait Command: Send + Sync {
    fn help(&self) -> &str { "" }
    fn requires_system_checks(&self) -> bool { true }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command { cmd }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches)
        -> Result<Option<String>, CommandError>;
}
```

| Method | Purpose |
|---|---|
| `help` | One line shown in `cargo run -- help <name>`. |
| `requires_system_checks` | When `true` (the default), the [system checks](checks.md) run before `handle` and a failing check stops the command. Return `false` for commands that must work in a broken project, such as ones that fix configuration. |
| `add_arguments` | Declare positional arguments and options by adding `clap::Arg`s to the command you are handed and returning it. |
| `handle` | Do the work. Only this method is required. |

The text returned as `Ok(Some(text))` is written to stdout, with a newline added unless it already ends with one.
`Ok(None)` prints nothing more.

## Arguments

`add_arguments` uses the `clap` 4 builder API. Rango has already added these to every command, so do not reuse them:
`--version`, `-v`/`--verbosity`, `--traceback`, `--no-color`, `--force-color`, and (when the command requires system
checks) `--skip-checks`.

```rust
use clap::{Arg, ArgAction};

fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
    cmd.arg(Arg::new("poll_id").num_args(1..).required(true).help("Polls to close"))
        .arg(Arg::new("delete").long("delete").action(ArgAction::SetTrue).help("Delete instead of closing"))
        .arg(Arg::new("database").long("database").default_value("default"))
}
```

Read them in `handle` with the `ArgMatches` you receive:

```rust
let ids: Vec<String> = args.get_many::<String>("poll_id").into_iter().flatten().cloned().collect();
let delete = args.get_flag("delete");
let alias = args.get_one::<String>("database").expect("has default");
```

Values arrive as strings unless you add a `value_parser`, for example `Arg::new("days").value_parser(clap::value_parser!(u32))`
and `args.get_one::<u32>("days")`. A missing required argument or an unknown option is a usage error: the message goes to
stderr and the exit status is 2, before `handle` runs.

## CommandContext: output, style, verbosity

`handle` receives a `&CommandContext`:

| Field | Type | Meaning |
|---|---|---|
| `stdout` | `Output` | The standard output stream. |
| `stderr` | `Output` | The standard error stream. |
| `style` | `Style` | Colouring helpers; plain text when colour is off. |
| `verbosity` | `u8` | The `-v` level, 0 to 3 (default 1). |

`Output::write(&str)` writes a line (adding a newline if missing) and `Output::write_raw(&str)` writes the text as
is. `Style` has `error`, `warning`, `success`, `notice`, `migrate_heading` and `migrate_label`, each taking a `&str` and
returning a `String`.

```rust
async fn handle(
    &self,
    ctx: &CommandContext,
    args: &clap::ArgMatches,
) -> Result<Option<String>, CommandError> {
    let ids: Vec<String> = args.get_many::<String>("poll_id").into_iter().flatten().cloned().collect();
    for id in &ids {
        if ctx.verbosity >= 2 {
            ctx.stdout.write(&format!("closing poll {id}"));
        }
    }
    ctx.stdout.write(&ctx.style.success(&format!("Closed {} polls.", ids.len())));
    Ok(None)
}
```

Use `ctx.stderr` for diagnostics, and the returned `Some(text)` or `ctx.stdout` for results that another program may
read. The same command in a test captures both.

## Errors

`handle` returns `Result<_, CommandError>`. `CommandError` has public fields:

| Field | Meaning |
|---|---|
| `message` | The text shown to the user. |
| `returncode` | The process exit status; `1` from `CommandError::new`. |
| `is_system_check` | `true` only for failures of the system checks. |

`CommandError::new("message")` is the usual way to fail. The runner prints `CommandError: message` to stderr and exits
with the status. A `rango::Error` converts with `?` (the message is the error's text), so database and other
framework calls need no mapping:

```rust
use rango::core::management::CommandError;

async fn handle(&self, _: &CommandContext, args: &clap::ArgMatches)
    -> Result<Option<String>, CommandError>
{
    let days: u32 = args
        .get_one::<String>("days")
        .expect("required")
        .parse()
        .map_err(|_| CommandError::new("days must be a whole number"))?;
    if days == 0 {
        let mut e = CommandError::new("days must be positive");
        e.returncode = 3;
        return Err(e);
    }
    Ok(None)
}
```

## Commands that use the database

Commands receive no connection: open one explicitly like any other code. The database commands that ship with Rango
do it with the `--database` option and `Db::using(alias)`:

```rust
use rango::db::Db;

async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches)
    -> Result<Option<String>, CommandError>
{
    let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
    // run queries with `&db` ...
    Ok(None)
}
```

## Running and listing commands

The project's `main` finishes with `execute_from_command_line`, which parses `argv[1]` as the command name:

```rust
std::process::exit(rango::core::management::execute_from_command_line(std::env::args()).await);
```

It returns the exit status. `help` or no argument prints `main_help_text`, a list of commands grouped by app;
`help <name>` prints that command's usage. Other helpers in `rango::core::management`:

| Item | Purpose |
|---|---|
| `get_commands() -> BTreeMap<&str, &str>` | `{command name: app}` for every command available in this process. |
| `main_help_text(prog_name: &str) -> String` | The text `help` prints. |
| `execute(name, &[String], stdout: Output, stderr: Output) -> Result<(), CommandError>` | Run a command with explicit streams. The slice holds the arguments after the command name. |
| `call_command(name, &[&str]) -> Result<String, CommandError>` | Run a command and return everything it printed (stdout and stderr together). |
| `run_system_checks(ctx, &CheckOptions)` | Run the checks and print the report, as commands do. |
| `Output::new(Sink)`, `Sink::{Stdout, Stderr, Buffer(Arc<Mutex<String>>)}` | Build streams, including in-memory ones. |

`call_command` also works inside your own code, for example a view that triggers a maintenance task or a command
that delegates to another:

```rust
use rango::core::management::call_command;

let report = call_command("check", &["--deploy"]).await?;
```

Commands that require system checks run them first when invoked through `call_command` as well; pass `--skip-checks`
to avoid it.

## Testing a command

Configure settings, populate the registry, then use `call_command` (output only) or `execute` (separate streams and
the exact error). This is adapted from `crates/rango/tests/manage_cli.rs`:

```rust
use rango::apps::AppConfig;
use rango::conf::{self, Settings};
use rango::core::management::{call_command, execute, Output, Sink};
use rango::{register_app, register_command};

register_app!(AppConfig::new("polls"));
register_command!("polls", "greet", Greet);

#[tokio::test]
async fn greet_command() {
    conf::configure(Settings {
        debug: true,
        secret_key: "k".into(),
        installed_apps: vec!["polls".into()],
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();

    assert_eq!(call_command("greet", &["Ada"]).await.unwrap().trim(), "Hello, Ada!");

    // A usage error has status 2 and never reaches `handle`.
    let (out, err) = (Sink::Buffer(Default::default()), Sink::Buffer(Default::default()));
    let result = execute("greet", &["--nope".to_owned()], Output::new(out), Output::new(err)).await;
    assert_eq!(result.unwrap_err().returncode, 2);
}
```

Each integration-test file is its own process, so settings are configured once per file. To change settings for one
test, wrap the call in `rango::conf::override_settings`. The app and command are visible to `call_command` only
because the app is in `installed_apps`; a command registered under an app that is not installed fails with
`Unknown command`.

## Not supported yet

- Subcommands as separate registrations: model them with `clap` subcommands inside one command.
- Interactive prompts; read answers from arguments or the environment.
- Automatic `--traceback` output (the flag is accepted and ignored).
- Commands written in a synchronous style: `handle` is always `async`.

## See also

- [Management commands reference](../ref/commands.md)
- [Applications](apps.md)
- [System checks](checks.md)
- [Testing](testing.md)
