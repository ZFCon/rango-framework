//! `startproject` and `startapp` (`core.management.templates.TemplateCommand`).

use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use clap::Arg;
use rango_core::management::{Command, CommandContext, CommandError};
use rango_core::utils::crypto::get_random_string;

/// (path inside the generated tree, template text). `-tpl` suffixes are dropped, `gitignore` becomes `.gitignore`.
const PROJECT_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../templates/project/Cargo.toml-tpl")),
    (".gitignore", include_str!("../templates/project/gitignore-tpl")),
    ("src/main.rs", include_str!("../templates/project/main.rs-tpl")),
    ("src/settings.rs", include_str!("../templates/project/settings.rs-tpl")),
    ("src/urls.rs", include_str!("../templates/project/urls.rs-tpl")),
    ("templates/.gitkeep", ""),
    ("static/.gitkeep", ""),
];

const APP_FILES: &[(&str, &str)] = &[
    ("mod.rs", include_str!("../templates/app/mod.rs-tpl")),
    ("models.rs", include_str!("../templates/app/models.rs-tpl")),
    ("views.rs", include_str!("../templates/app/views.rs-tpl")),
    ("urls.rs", include_str!("../templates/app/urls.rs-tpl")),
    ("admin.rs", include_str!("../templates/app/admin.rs-tpl")),
    ("tests.rs", include_str!("../templates/app/tests.rs-tpl")),
    ("migrations/mod.rs", include_str!("../templates/app/migrations/mod.rs-tpl")),
];

const RESERVED: &[&str] = &[
    "rango", "std", "core", "alloc", "test", "proc_macro", "tokio", "as", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
];

/// Substitute `{{ name }}` placeholders.
fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{ {k} }}}}"), v);
    }
    out
}

fn validate_name(kind: &str, name: &str) -> Result<(), CommandError> {
    let valid = !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(CommandError::new(format!(
            "'{name}' is not a valid {kind} name. Please make sure the name is a valid identifier (letters, digits and underscores, not starting with a digit)."
        )));
    }
    if RESERVED.contains(&name) {
        return Err(CommandError::new(format!(
            "'{name}' conflicts with the name of an existing Rust module or keyword and will cause an ImportError. Please try another name for your {kind}."
        )));
    }
    Ok(())
}

fn write_tree(root: &Path, files: &[(&str, &str)], vars: &[(&str, &str)]) -> Result<(), CommandError> {
    for (rel, text) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CommandError::new(format!("{}: {e}", parent.display())))?;
        }
        fs::write(&path, render(text, vars)).map_err(|e| CommandError::new(format!("{}: {e}", path.display())))?;
    }
    Ok(())
}

#[derive(Default)]
pub struct StartProject;

#[async_trait]
impl Command for StartProject {
    fn help(&self) -> &str {
        "Creates a Rango project directory structure for the given project name in the current directory or optionally in the given directory."
    }

    fn requires_system_checks(&self) -> bool {
        false
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("name").required(true).help("Name of the project."))
            .arg(Arg::new("directory").help("Optional destination directory"))
            .arg(Arg::new("rango_path").long("rango-path").help("Depend on a local checkout of rango instead of the published crate."))
    }

    async fn handle(&self, _: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let name = args.get_one::<String>("name").expect("required");
        validate_name("project", name)?;
        let target: PathBuf = args.get_one::<String>("directory").map_or_else(|| PathBuf::from(name), PathBuf::from);
        if target.exists() && fs::read_dir(&target).map_or(true, |mut d| d.next().is_some()) {
            return Err(CommandError::new(format!("'{}' already exists", target.display())));
        }
        let dependency = match args.get_one::<String>("rango_path") {
            Some(p) => format!("{{ path = {:?} }}", fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)).join("crates/rango")),
            None => format!("\"{}\"", rango_core::management::VERSION),
        };
        let secret = format!("rango-insecure-{}", get_random_string(50, "abcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*(-_=+)"));
        write_tree(&target, PROJECT_FILES, &[("project_name", name), ("secret_key", &secret), ("rango_dependency", &dependency)])?;
        Ok(None)
    }
}

#[derive(Default)]
pub struct StartApp;

#[async_trait]
impl Command for StartApp {
    fn help(&self) -> &str {
        "Creates a Rango app directory structure for the given app name in the current directory or optionally in the given directory."
    }

    fn requires_system_checks(&self) -> bool {
        false
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("name").required(true).help("Name of the application."))
            .arg(Arg::new("directory").help("Optional destination directory"))
    }

    async fn handle(&self, _: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let name = args.get_one::<String>("name").expect("required");
        validate_name("app", name)?;
        let in_project = Path::new("src/main.rs").exists();
        let target: PathBuf = args
            .get_one::<String>("directory")
            .map_or_else(|| if in_project { Path::new("src").join(name) } else { PathBuf::from(name) }, PathBuf::from);
        if target.exists() {
            return Err(CommandError::new(format!("'{}' already exists, overlaying an app into an existing directory won't replace conflicting files.", target.display())));
        }
        write_tree(&target, APP_FILES, &[("app_name", name)])?;
        // Link the app into the binary so its `register_app!` runs.
        if in_project {
            let main = fs::read_to_string("src/main.rs").map_err(|e| CommandError::new(e.to_string()))?;
            let decl = format!("mod {name};");
            if !main.lines().any(|l| l.trim() == decl) {
                let mut lines: Vec<&str> = main.lines().collect();
                let at = lines.iter().rposition(|l| l.starts_with("mod ")).map_or(0, |i| i + 1);
                lines.insert(at, &decl);
                fs::write("src/main.rs", lines.join("\n") + "\n").map_err(|e| CommandError::new(e.to_string()))?;
            }
        }
        Ok(None)
    }
}

rango_core::register_command!("rango.core", "startproject", StartProject);
rango_core::register_command!("rango.core", "startapp", StartApp);

/// Referenced by the facade crate so the linker keeps this crate's registrations.
#[doc(hidden)]
pub fn __link() {}
