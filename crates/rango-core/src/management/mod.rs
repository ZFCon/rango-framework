//! `core.management`: command discovery, `call_command`, and the
//! `rango-admin` / project entry point.

mod base;
pub mod commands;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub use base::{
    CheckOptions, Command, CommandContext, CommandError, CommandRegistration, Output, Sink, Style,
    run_system_checks,
};

use crate::apps;

/// Framework version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn is_available(reg: &CommandRegistration) -> bool {
    reg.app.starts_with("rango.")
        || apps::apps()
            .map(|a| a.is_installed(reg.app))
            .unwrap_or(false)
}

/// `get_commands()`: `{name: app}` for every command available in this process.
pub fn get_commands() -> BTreeMap<&'static str, &'static str> {
    inventory::iter::<CommandRegistration>
        .into_iter()
        .filter(|r| is_available(r))
        .map(|r| (r.name, r.app))
        .collect()
}

fn find(name: &str) -> Option<&'static CommandRegistration> {
    inventory::iter::<CommandRegistration>
        .into_iter()
        .find(|r| r.name == name && is_available(r))
}

fn base_args(cmd: clap::Command, skip_checks: bool) -> clap::Command {
    use clap::{Arg, ArgAction};
    let cmd = cmd
        .no_binary_name(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::SetTrue)
                .help("Show program's version number and exit."),
        )
        .arg(
            Arg::new("verbosity")
                .short('v')
                .long("verbosity")
                .value_parser(["0", "1", "2", "3"])
                .default_value("1")
                .help("Verbosity level; 0=minimal output, 1=normal output, 2=verbose output, 3=very verbose output"),
        )
        .arg(Arg::new("traceback").long("traceback").action(ArgAction::SetTrue).help("Display error details."))
        .arg(Arg::new("no_color").long("no-color").action(ArgAction::SetTrue).help("Don't colorize the command output."))
        .arg(Arg::new("force_color").long("force-color").action(ArgAction::SetTrue).help("Force colorization of the command output."));
    if skip_checks {
        cmd.arg(
            Arg::new("skip_checks")
                .long("skip-checks")
                .action(ArgAction::SetTrue)
                .help("Skip system checks."),
        )
    } else {
        cmd
    }
}

/// Run one command by name with `args` (everything after the command name).
pub async fn execute(
    name: &str,
    args: &[String],
    stdout: Output,
    stderr: Output,
) -> Result<(), CommandError> {
    let reg = find(name).ok_or_else(|| CommandError::new(format!("Unknown command: '{name}'")))?;
    let command = (reg.build)();
    let parser = command.add_arguments(base_args(
        clap::Command::new(reg.name).about(command.help().to_owned()),
        command.requires_system_checks(),
    ));
    let matches = match parser.try_get_matches_from(args) {
        Ok(m) => m,
        Err(e) if e.use_stderr() => {
            return Err(CommandError {
                message: e.to_string(),
                returncode: 2,
                is_system_check: false,
            });
        }
        Err(e) => {
            stdout.write(&e.to_string());
            return Ok(());
        }
    };
    if matches.get_flag("version") {
        stdout.write(VERSION);
        return Ok(());
    }
    let color = !matches.get_flag("no_color")
        && (matches.get_flag("force_color")
            || (std::env::var_os("NO_COLOR").is_none() && is_tty()));
    let ctx = CommandContext {
        stdout,
        stderr,
        style: Style { color },
        verbosity: matches
            .get_one::<String>("verbosity")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    };
    if command.requires_system_checks() && !matches.get_flag("skip_checks") {
        run_system_checks(&ctx, &CheckOptions::default()).await?;
    }
    if let Some(output) = command.handle(&ctx, &matches).await? {
        ctx.stdout.write(&output);
    }
    Ok(())
}

fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// `call_command(name, *args)`: run a command programmatically and return what it printed.
pub async fn call_command(name: &str, args: &[&str]) -> Result<String, CommandError> {
    let buf = Arc::new(Mutex::new(String::new()));
    let out = Output::new(Sink::Buffer(Arc::clone(&buf)));
    execute(
        name,
        &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
        out.clone(),
        out,
    )
    .await?;
    let text = buf.lock().expect("buffer lock poisoned").clone();
    Ok(text)
}

/// `ManagementUtility.main_help_text()`.
pub fn main_help_text(prog_name: &str) -> String {
    let mut by_app: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, app) in get_commands() {
        let group = app.strip_prefix("rango.").map_or(app, |_| "rango");
        by_app.entry(group).or_default().push(name);
    }
    let mut out = format!(
        "\nType '{prog_name} help <subcommand>' for help on a specific subcommand.\n\nAvailable subcommands:\n"
    );
    for (app, cmds) in by_app {
        out.push_str(&format!("\n[{app}]\n"));
        for c in cmds {
            out.push_str(&format!("    {c}\n"));
        }
    }
    out
}

/// `execute_from_command_line(argv)`: the body of a project's `main`. Returns the process exit code.
pub async fn execute_from_command_line(argv: impl IntoIterator<Item = String>) -> i32 {
    let argv: Vec<String> = argv.into_iter().collect();
    let prog = argv
        .first()
        .and_then(|p| std::path::Path::new(p).file_name())
        .map_or_else(|| "manage".to_owned(), |n| n.to_string_lossy().into_owned());
    let (stdout, stderr) = (Output::new(Sink::Stdout), Output::new(Sink::Stderr));
    let sub = argv.get(1).map(String::as_str);
    match sub {
        None | Some("help" | "--help" | "-h") if argv.get(2).is_none() => {
            stdout.write(&main_help_text(&prog));
            0
        }
        Some("--version") => {
            stdout.write(VERSION);
            0
        }
        Some("help") => run(&argv[2], &["--help".to_owned()], stdout, stderr).await,
        Some(name) => run(name, &argv[2..], stdout, stderr).await,
        None => 0,
    }
}

async fn run(name: &str, args: &[String], stdout: Output, stderr: Output) -> i32 {
    if find(name).is_none() {
        stderr.write(&format!(
            "Unknown command: '{name}'\nType 'manage help' for usage."
        ));
        return 1;
    }
    match execute(name, args, stdout, stderr.clone()).await {
        Ok(()) => 0,
        Err(e) if e.is_system_check => {
            stderr.write(&e.message);
            e.returncode
        }
        Err(e) if e.returncode == 2 => {
            stderr.write(&e.message);
            2
        }
        Err(e) => {
            stderr.write(&format!("CommandError: {}", e.message));
            e.returncode
        }
    }
}
