//! `makemessages` and `compilemessages`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use clap::{Arg, ArgAction};

use crate::management::{Command, CommandContext, CommandError};
use crate::utils::translation::Catalog;

/// A message found in the sources.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub context: String,
    pub id: String,
    pub plural: Option<String>,
}

/// Extract string literals passed to `gettext`/`ngettext`/`pgettext`/`npgettext` (Rust sources) and
/// `{% trans %}`/`{% blocktrans %}` (templates).
pub fn extract(text: &str, is_template: bool) -> Vec<Found> {
    fn lit(s: &str) -> Option<(String, &str)> {
        let s = s.trim_start();
        let q = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let mut out = String::new();
        let mut it = s[1..].char_indices();
        while let Some((i, c)) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, o)) => out.push(o),
                    None => return None,
                }
            } else if c == q {
                return Some((out, &s[1 + i + 1..]));
            } else {
                out.push(c);
            }
        }
        None
    }
    let mut found = Vec::new();
    if is_template {
        let mut rest = text;
        while let Some(i) = rest.find("{%") {
            let Some(j) = rest[i..].find("%}") else { break };
            let tag = rest[i + 2..i + j].trim();
            if let Some(a) = tag.strip_prefix("trans ").or_else(|| tag.strip_prefix("translate ")) {
                if let Some((id, after)) = lit(a) {
                    let context = after.split_once("context").and_then(|(_, c)| lit(c)).map(|x| x.0).unwrap_or_default();
                    found.push(Found { context, id, plural: None });
                }
            }
            rest = &rest[i + j + 2..];
        }
        return found;
    }
    for (name, args) in [("ngettext", 2), ("npgettext", 3), ("pgettext", 2), ("gettext_lazy", 1), ("gettext", 1)] {
        let mut from = 0;
        while let Some(pos) = text[from..].find(&format!("{name}(")) {
            let start = from + pos;
            from = start + name.len() + 1;
            // Skip e.g. `ngettext` matching inside `npgettext`/`xgettext` names and function definitions.
            if text[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') || text[..start].trim_end().ends_with("fn") {
                continue;
            }
            let mut rest = &text[from..];
            let mut lits = Vec::new();
            for _ in 0..args {
                let Some((l, after)) = lit(rest) else { break };
                lits.push(l);
                rest = after.trim_start().strip_prefix(',').unwrap_or(after);
            }
            match (name, lits.as_slice()) {
                ("ngettext", [s, p]) => found.push(Found { context: String::new(), id: s.clone(), plural: Some(p.clone()) }),
                ("npgettext", [c, s, p]) => found.push(Found { context: c.clone(), id: s.clone(), plural: Some(p.clone()) }),
                ("pgettext", [c, s]) => found.push(Found { context: c.clone(), id: s.clone(), plural: None }),
                (_, [s]) if name.starts_with("gettext") => found.push(Found { context: String::new(), id: s.clone(), plural: None }),
                _ => {}
            }
        }
    }
    found
}

fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !matches!(name.as_str(), "target" | ".git" | "node_modules" | "locale") {
                walk(&p, exts, out);
            }
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| exts.contains(&x)) {
            out.push(p);
        }
    }
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

/// `makemessages -l LANG`: (re)write `locale/<lang>/LC_MESSAGES/rango.po`, keeping existing translations.
#[derive(Default)]
pub struct MakeMessages;

#[async_trait]
impl Command for MakeMessages {
    fn help(&self) -> &str {
        "Pulls out all strings marked for translation and creates or updates a message file."
    }
    fn requires_system_checks(&self) -> bool {
        false
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("locale").short('l').long("locale").action(ArgAction::Append).required(true)).arg(Arg::new("path").long("path").default_value("."))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let root = PathBuf::from(args.get_one::<String>("path").expect("default"));
        let mut files = Vec::new();
        walk(&root, &["rs", "html", "txt"], &mut files);
        files.sort();
        let mut msgs: BTreeMap<Found, Vec<String>> = BTreeMap::new();
        for f in &files {
            let Ok(text) = std::fs::read_to_string(f) else { continue };
            let is_tpl = f.extension().is_some_and(|e| e != "rs");
            for m in extract(&text, is_tpl) {
                msgs.entry(m).or_default().push(f.strip_prefix(&root).unwrap_or(f).display().to_string());
            }
        }
        for lang in args.get_many::<String>("locale").into_iter().flatten() {
            let dir = root.join("locale").join(lang).join("LC_MESSAGES");
            std::fs::create_dir_all(&dir).map_err(|e| CommandError::new(e.to_string()))?;
            let path = dir.join("rango.po");
            let existing = std::fs::read_to_string(&path).map(|t| Catalog::from_po(&t)).unwrap_or_default();
            let header = if existing.header.is_empty() { "Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=2; plural=(n != 1);\n".to_owned() } else { existing.header.clone() };
            let nplurals = header.split("nplurals=").nth(1).and_then(|s| s.split(';').next()).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(2);
            let old: BTreeMap<_, _> = existing.entries().into_iter().map(|((c, i, _), f)| ((c, i), f)).collect();
            let mut out = format!("msgid \"\"\nmsgstr \"\"\n{}\n", header.lines().map(|l| format!("\"{}\\n\"", l.replace('"', "\\\""))).collect::<Vec<_>>().join("\n"));
            for (m, places) in &msgs {
                out.push_str(&format!("\n#: {}\n", places.join(" ")));
                if !m.context.is_empty() {
                    out.push_str(&format!("msgctxt {}\n", q(&m.context)));
                }
                out.push_str(&format!("msgid {}\n", q(&m.id)));
                let prev = old.get(&(m.context.clone(), m.id.clone()));
                match &m.plural {
                    Some(p) => {
                        out.push_str(&format!("msgid_plural {}\n", q(p)));
                        for i in 0..nplurals {
                            out.push_str(&format!("msgstr[{i}] {}\n", q(prev.and_then(|f| f.get(i)).map_or("", String::as_str))));
                        }
                    }
                    None => out.push_str(&format!("msgstr {}\n", q(prev.and_then(|f| f.first()).map_or("", String::as_str)))),
                }
            }
            std::fs::write(&path, out).map_err(|e| CommandError::new(e.to_string()))?;
            ctx.stdout.write(&format!("processing locale {lang}"));
        }
        Ok(None)
    }
}

crate::register_command!("rango.core", "makemessages", MakeMessages);

/// `compilemessages`: every `po` under `locale/` becomes a `mo`.
#[derive(Default)]
pub struct CompileMessages;

#[async_trait]
impl Command for CompileMessages {
    fn help(&self) -> &str {
        "Compiles .po files to .mo files for use with builtin gettext support."
    }
    fn requires_system_checks(&self) -> bool {
        false
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("path").long("path").default_value("."))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let root = PathBuf::from(args.get_one::<String>("path").expect("default"));
        let mut pos = Vec::new();
        fn find(dir: &Path, out: &mut Vec<PathBuf>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() && e.file_name() != "target" && e.file_name() != ".git" {
                        find(&p, out);
                    } else if p.file_name().is_some_and(|n| n == "rango.po") {
                        out.push(p);
                    }
                }
            }
        }
        find(&root, &mut pos);
        for po in pos {
            let cat = Catalog::from_po(&std::fs::read_to_string(&po).map_err(|e| CommandError::new(e.to_string()))?);
            let mo = Catalog::to_mo(&cat.entries(), &cat.header);
            std::fs::write(po.with_extension("mo"), mo).map_err(|e| CommandError::new(e.to_string()))?;
            ctx.stdout.write(&format!("processing file {} in {}", po.file_name().and_then(|n| n.to_str()).unwrap_or(""), po.parent().map(|p| p.display().to_string()).unwrap_or_default()));
        }
        Ok(None)
    }
}

crate::register_command!("rango.core", "compilemessages", CompileMessages);

#[doc(hidden)]
pub fn __link() {
    let _ = (MakeMessages::default as fn() -> _, CompileMessages::default as fn() -> _);
}
