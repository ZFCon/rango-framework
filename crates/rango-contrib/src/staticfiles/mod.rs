//! `contrib.staticfiles`: finders, storage, `collectstatic`, `findstatic`, `serve`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use clap::{Arg, ArgAction};
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_core::management::{Command, CommandContext, CommandError};
use rango_http::{HttpRequest, HttpResponse};

/// `find(path)` / `list()` over `STATICFILES_DIRS` and every app's `static/` directory.
pub struct Finders {
    roots: Vec<(Option<String>, PathBuf)>,
}

impl Finders {
    /// `FileSystemFinder` (with `("prefix", dir)` support) then `AppDirectoriesFinder`.
    pub fn from_settings() -> Self {
        let s = settings();
        let mut roots: Vec<(Option<String>, PathBuf)> = s
            .staticfiles_dirs
            .iter()
            .map(|d| match d.split_once('\u{0}') {
                Some((p, dir)) => (Some(p.to_owned()), PathBuf::from(dir)),
                None => (None, PathBuf::from(d)),
            })
            .collect();
        if let Ok(apps) = rango_core::apps::apps() {
            roots.extend(apps.get_app_configs().filter_map(|c| c.path.as_ref().map(|p| (None, p.join("static")))).filter(|(_, p)| p.is_dir()));
        }
        Finders { roots }
    }

    pub fn with_roots(roots: Vec<(Option<String>, PathBuf)>) -> Self {
        Finders { roots }
    }

    /// `find(path, all=False)`: the first match.
    pub fn find(&self, path: &str) -> Option<PathBuf> {
        self.find_all(path).into_iter().next()
    }

    pub fn find_all(&self, path: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for (prefix, root) in &self.roots {
            let rel = match prefix {
                Some(p) => match path.strip_prefix(&format!("{}/", p.trim_end_matches('/'))) {
                    Some(r) => r,
                    None => continue,
                },
                None => path,
            };
            if let Ok(p) = rango_core::utils::os::safe_join(root, &[rel]) {
                if p.is_file() {
                    out.push(p);
                }
            }
        }
        out
    }

    /// `list()`: `(relative path, absolute path)` of every file, first root wins on duplicates.
    pub fn list(&self, ignore: &[String]) -> BTreeMap<String, PathBuf> {
        fn walk(dir: &Path, base: &Path, ignore: &[String], out: &mut Vec<(String, PathBuf)>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let name = e.file_name().to_string_lossy().into_owned();
                if ignore.iter().any(|pat| glob_match(pat, &name)) {
                    continue;
                }
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, ignore, out);
                } else if let Ok(rel) = p.strip_prefix(base) {
                    out.push((rel.to_string_lossy().replace('\\', "/"), p));
                }
            }
        }
        let mut out = BTreeMap::new();
        for (prefix, root) in &self.roots {
            let mut found = Vec::new();
            walk(root, root, ignore, &mut found);
            for (rel, abs) in found {
                let key = prefix.as_ref().map_or(rel.clone(), |p| format!("{}/{rel}", p.trim_end_matches('/')));
                out.entry(key).or_insert(abs);
            }
        }
        out
    }
}

/// `fnmatch` for `*` and `?` (what `--ignore` takes).
fn glob_match(pat: &str, s: &str) -> bool {
    fn m(p: &[char], s: &[char]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some('*'), _) => m(&p[1..], s) || (!s.is_empty() && m(p, &s[1..])),
            (Some('?'), Some(_)) => m(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => m(&p[1..], &s[1..]),
            _ => false,
        }
    }
    m(&pat.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>())
}

/// `StaticFilesStorage` / `ManifestStaticFilesStorage`: where collected files go and how URLs are made.
pub struct StaticStorage {
    pub root: PathBuf,
    pub base_url: String,
    /// Hashed file names recorded in `staticfiles.json`.
    pub manifest: bool,
}

impl StaticStorage {
    pub fn from_settings() -> Result<Self> {
        let s = settings();
        let root = s.static_root.clone().ok_or_else(|| Error::improperly_configured("You're using the staticfiles app without having set the STATIC_ROOT setting to a filesystem path."))?;
        let manifest = s.storages.get("staticfiles").and_then(|v| v.get("BACKEND")).and_then(|b| b.as_str()).is_some_and(|b| b.ends_with("ManifestStaticFilesStorage"));
        Ok(Self { root: PathBuf::from(root), base_url: s.static_url.clone().unwrap_or_default(), manifest })
    }

    fn hashed(name: &str, content: &[u8]) -> String {
        use md5::{Digest, Md5};
        let h: String = Md5::digest(content).iter().map(|b| format!("{b:02x}")).collect::<String>().chars().take(12).collect();
        match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.ends_with('/') => format!("{stem}.{h}.{ext}"),
            _ => format!("{name}.{h}"),
        }
    }

    /// `url(name)`: through the manifest when there is one.
    pub fn url(&self, name: &str) -> Result<String> {
        let mut n = name.to_owned();
        if self.manifest {
            if let Ok(raw) = std::fs::read_to_string(self.root.join("staticfiles.json")) {
                let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| Error::Other(Box::new(e)))?;
                n = v["paths"].get(name).and_then(|x| x.as_str()).map(str::to_owned).ok_or_else(|| Error::Other(format!("Missing staticfiles manifest entry for '{name}'").into()))?;
            }
        }
        Ok(format!("{}{}", self.base_url, rango_core::utils::http::quote(&n, "/")))
    }
}

/// `collectstatic`.
#[derive(Default)]
pub struct CollectStatic;

#[async_trait]
impl Command for CollectStatic {
    fn help(&self) -> &str {
        "Collect static files in a single location."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("noinput").long("noinput").visible_alias("no-input").action(ArgAction::SetTrue))
            .arg(Arg::new("clear").short('c').long("clear").action(ArgAction::SetTrue))
            .arg(Arg::new("dry_run").short('n').long("dry-run").action(ArgAction::SetTrue))
            .arg(Arg::new("ignore").short('i').long("ignore").action(ArgAction::Append))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let st = StaticStorage::from_settings().map_err(|e| CommandError::new(e.to_string()))?;
        let mut ignore: Vec<String> = ["CVS", ".*", "*~"].map(String::from).to_vec();
        ignore.extend(args.get_many::<String>("ignore").into_iter().flatten().cloned());
        let (dry, clear) = (args.get_flag("dry_run"), args.get_flag("clear"));
        if clear && !dry {
            let _ = std::fs::remove_dir_all(&st.root);
        }
        let files = Finders::from_settings().list(&ignore);
        let mut manifest: BTreeMap<String, String> = BTreeMap::new();
        let (mut copied, mut unmodified) = (0, 0);
        for (rel, src) in &files {
            let content = std::fs::read(src).map_err(|e| CommandError::new(format!("{}: {e}", src.display())))?;
            let dest_rel = if st.manifest { StaticStorage::hashed(rel, &content) } else { rel.clone() };
            let dest = st.root.join(&dest_rel);
            if dest.exists() && std::fs::read(&dest).ok().as_deref() == Some(&content) {
                unmodified += 1;
            } else {
                copied += 1;
                if !dry {
                    if let Some(d) = dest.parent() {
                        std::fs::create_dir_all(d).map_err(|e| CommandError::new(e.to_string()))?;
                    }
                    std::fs::write(&dest, &content).map_err(|e| CommandError::new(e.to_string()))?;
                }
            }
            manifest.insert(rel.clone(), dest_rel);
        }
        if st.manifest && !dry {
            let json = serde_json::json!({"paths": manifest, "version": "1.1", "hash": ""});
            std::fs::write(st.root.join("staticfiles.json"), serde_json::to_string(&json).expect("json")).map_err(|e| CommandError::new(e.to_string()))?;
        }
        ctx.stdout.write(&format!("\n{copied} static file{} copied to '{}'{}.", if copied == 1 { "" } else { "s" }, st.root.display(), if unmodified > 0 { format!(", {unmodified} unmodified") } else { String::new() }));
        Ok(None)
    }
}

rango_core::register_command!("rango.contrib.staticfiles", "collectstatic", CollectStatic);

/// `findstatic <path>`.
#[derive(Default)]
pub struct FindStatic;

#[async_trait]
impl Command for FindStatic {
    fn help(&self) -> &str {
        "Finds the absolute paths for the given static file(s)."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("path").num_args(1..).required(true))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let f = Finders::from_settings();
        for p in args.get_many::<String>("path").into_iter().flatten() {
            let found = f.find_all(p);
            if found.is_empty() {
                ctx.stdout.write(&format!("No matching file found for '{p}'."));
            } else {
                ctx.stdout.write(&format!("Found '{p}' here:\n  {}", found.iter().map(|x| x.display().to_string()).collect::<Vec<_>>().join("\n  ")));
            }
        }
        Ok(None)
    }
}

rango_core::register_command!("rango.contrib.staticfiles", "findstatic", FindStatic);

/// `staticfiles.views.serve(request, path)`: development-only serving through the finders.
pub async fn serve(_request: &HttpRequest, path: &str) -> Result<HttpResponse> {
    let found = Finders::from_settings().find(path.trim_start_matches('/')).ok_or_else(|| Error::Http404(format!("'{path}' could not be found")))?;
    let (dir, name) = (found.parent().unwrap_or(Path::new(".")).to_owned(), found.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let _ = (dir, name);
    let bytes = std::fs::read(&found).map_err(|e| Error::Other(Box::new(e)))?;
    let mime = mime_guess::from_path(&found).first_or_octet_stream();
    Ok(HttpResponse::new(bytes).content_type(mime.as_ref()))
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.staticfiles"));
