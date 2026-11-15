//! `core.management.base` and `core.management.color`.

use std::io::Write;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::checks::{self, CheckContext, CheckMessage};
use crate::exceptions::Error;

/// Where an [`Output`] writes.
#[derive(Clone)]
pub enum Sink {
    Stdout,
    Stderr,
    /// Captured text (used by `call_command` and tests).
    Buffer(Arc<Mutex<String>>),
}

/// `OutputWrapper`: writes lines, appending a newline unless the text already ends with one.
#[derive(Clone)]
pub struct Output(Sink);

impl Output {
    pub fn new(sink: Sink) -> Self {
        Self(sink)
    }

    pub fn write(&self, msg: &str) {
        let line = if msg.ends_with('\n') {
            msg.to_owned()
        } else {
            format!("{msg}\n")
        };
        match &self.0 {
            Sink::Stdout => {
                let _ = std::io::stdout().write_all(line.as_bytes());
            }
            Sink::Stderr => {
                let _ = std::io::stderr().write_all(line.as_bytes());
            }
            Sink::Buffer(b) => b.lock().expect("buffer lock poisoned").push_str(&line),
        }
    }

    /// Write without the implicit trailing newline.
    pub fn write_raw(&self, msg: &str) {
        match &self.0 {
            Sink::Stdout => {
                let _ = std::io::stdout().write_all(msg.as_bytes());
            }
            Sink::Stderr => {
                let _ = std::io::stderr().write_all(msg.as_bytes());
            }
            Sink::Buffer(b) => b.lock().expect("buffer lock poisoned").push_str(msg),
        }
    }
}

/// `color_style()` / `no_style()`: ANSI styling for command output.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub color: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
    pub fn error(&self, t: &str) -> String {
        self.paint("31;1", t)
    }
    pub fn warning(&self, t: &str) -> String {
        self.paint("33;1", t)
    }
    pub fn success(&self, t: &str) -> String {
        self.paint("32;1", t)
    }
    pub fn notice(&self, t: &str) -> String {
        self.paint("31", t)
    }
    pub fn migrate_heading(&self, t: &str) -> String {
        self.paint("1", t)
    }
    pub fn migrate_label(&self, t: &str) -> String {
        self.paint("1", t)
    }
}

/// Everything a running command needs: its streams, style and verbosity.
#[derive(Clone)]
pub struct CommandContext {
    pub stdout: Output,
    pub stderr: Output,
    pub style: Style,
    /// `-v/--verbosity` (0 minimal, 1 normal, 2 verbose, 3 very verbose).
    pub verbosity: u8,
}

/// `CommandError` / `SystemCheckError`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    pub message: String,
    pub returncode: i32,
    pub is_system_check: bool,
}

impl CommandError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            returncode: 1,
            is_system_check: false,
        }
    }
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        Self::new(e.to_string())
    }
}

/// A management command (`BaseCommand`).
#[async_trait]
pub trait Command: Send + Sync {
    /// One-line description shown in `help`.
    fn help(&self) -> &str {
        ""
    }

    /// Run system checks before `handle` (unless `--skip-checks`).
    fn requires_system_checks(&self) -> bool {
        true
    }

    /// Declare the command's own arguments.
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd
    }

    /// Do the work; returned text is written to stdout.
    async fn handle(
        &self,
        ctx: &CommandContext,
        args: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError>;
}

/// Link-time registration of a command. Create with [`register_command!`].
pub struct CommandRegistration {
    /// App the command belongs to; commands of `rango.*` are always available, others only
    /// when the app is in `INSTALLED_APPS`.
    pub app: &'static str,
    pub name: &'static str,
    pub build: fn() -> Box<dyn Command>,
}

inventory::collect!(CommandRegistration);

/// `register_command!("polls", "closepoll", ClosePoll)` (needs `ClosePoll: Default`).
#[macro_export]
macro_rules! register_command {
    ($app:expr, $name:expr, $ty:ty) => {
        $crate::inventory::submit! {
            $crate::management::CommandRegistration {
                app: $app,
                name: $name,
                build: || ::std::boxed::Box::new(<$ty as ::std::default::Default>::default()),
            }
        }
    };
}

/// Options of `BaseCommand.check()`.
#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub context: CheckContext,
    pub tags: Option<Vec<String>>,
    pub display_num_errors: bool,
    pub include_deployment_checks: bool,
    pub fail_level: u32,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            context: CheckContext::default(),
            tags: None,
            display_num_errors: false,
            include_deployment_checks: false,
            fail_level: checks::ERROR,
        }
    }
}

/// `BaseCommand.check()`: run the checks, print the report, fail on serious issues.
pub async fn run_system_checks(
    ctx: &CommandContext,
    opts: &CheckOptions,
) -> Result<(), CommandError> {
    let all = checks::run_checks(
        &opts.context,
        opts.tags.as_deref(),
        opts.include_deployment_checks,
    )
    .await?;
    let visible: Vec<&CheckMessage> = all.iter().filter(|e| !e.is_silenced()).collect();

    let groups: [(&str, Vec<&CheckMessage>); 5] = [
        (
            "CRITICALS",
            visible
                .iter()
                .copied()
                .filter(|e| e.level >= checks::CRITICAL)
                .collect(),
        ),
        (
            "ERRORS",
            visible
                .iter()
                .copied()
                .filter(|e| (checks::ERROR..checks::CRITICAL).contains(&e.level))
                .collect(),
        ),
        (
            "WARNINGS",
            visible
                .iter()
                .copied()
                .filter(|e| (checks::WARNING..checks::ERROR).contains(&e.level))
                .collect(),
        ),
        (
            "INFOS",
            visible
                .iter()
                .copied()
                .filter(|e| (checks::INFO..checks::WARNING).contains(&e.level))
                .collect(),
        ),
        (
            "DEBUGS",
            visible
                .iter()
                .copied()
                .filter(|e| e.level < checks::INFO)
                .collect(),
        ),
    ];

    let (mut header, mut body, mut footer) = (String::new(), String::new(), String::new());
    for (name, issues) in &groups {
        if issues.is_empty() {
            continue;
        }
        let mut formatted: Vec<String> = issues
            .iter()
            .map(|e| {
                if e.is_serious(checks::ERROR) {
                    ctx.style.error(&e.to_string())
                } else {
                    ctx.style.warning(&e.to_string())
                }
            })
            .collect();
        formatted.sort();
        body.push_str(&format!("\n{name}:\n{}\n", formatted.join("\n")));
    }
    let visible_count = visible.len();
    if visible_count > 0 {
        header = "System check identified some issues:\n".to_owned();
    }
    if opts.display_num_errors {
        if visible_count > 0 {
            footer.push('\n');
        }
        let n = match visible_count {
            0 => "no issues".to_owned(),
            1 => "1 issue".to_owned(),
            n => format!("{n} issues"),
        };
        footer.push_str(&format!(
            "System check identified {n} ({} silenced).",
            all.len() - visible_count
        ));
    }

    if all
        .iter()
        .any(|e| e.is_serious(opts.fail_level) && !e.is_silenced())
    {
        return Err(CommandError {
            message: format!(
                "{}{body}{footer}",
                ctx.style.error(&format!("SystemCheckError: {header}"))
            ),
            returncode: 1,
            is_system_check: true,
        });
    }
    let msg = format!("{header}{body}{footer}");
    if !msg.is_empty() {
        if visible_count > 0 {
            ctx.stderr.write(&msg)
        } else {
            ctx.stdout.write(&msg)
        }
    }
    Ok(())
}
