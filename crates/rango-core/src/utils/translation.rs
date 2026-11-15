//! `utils.translation`: gettext catalogs (`.po` and `.mo`), plural forms, per-task language.
//!
//! Catalogs are looked up in `LOCALE_PATHS` and in every installed app's `locale/` directory:
//! `<dir>/<lang>/LC_MESSAGES/rango.{mo,po}`. Call the functions when the message is *produced*
//! so the active language of the current request is honoured.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use crate::conf::try_settings;

tokio::task_local! {
    static CURRENT: String;
}

thread_local! {
    static ACTIVATED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// `activate(language)` for the current thread (synchronous code and tests).
pub fn activate(language: &str) {
    ACTIVATED.with(|a| *a.borrow_mut() = Some(language.to_owned()));
}

/// `deactivate()`.
pub fn deactivate() {
    ACTIVATED.with(|a| *a.borrow_mut() = None);
}

/// `translation.override(language)` for async code: tasks spawned inside do not inherit it.
pub async fn override_language<F: std::future::Future>(language: &str, fut: F) -> F::Output {
    CURRENT.scope(language.to_owned(), fut).await
}

/// `get_language()`: the activated language, else `LANGUAGE_CODE`.
pub fn get_language() -> String {
    if let Some(l) = ACTIVATED.with(|a| a.borrow().clone()) {
        return l;
    }
    if let Ok(l) = CURRENT.try_with(Clone::clone) {
        return l;
    }
    try_settings().map_or_else(|_| "en-us".to_owned(), |s| s.language_code.clone())
}

/// `get_language_bidi()`.
pub fn get_language_bidi() -> bool {
    let l = get_language();
    let base = l.split(['-', '_']).next().unwrap_or("");
    matches!(base, "he" | "ar" | "ar-dz" | "ckb" | "fa" | "ur")
}

/// `to_locale("en-us")` -> `"en_US"`.
pub fn to_locale(language: &str) -> String {
    match language.to_lowercase().split_once('-') {
        Some((a, b)) => {
            let b = if b.contains('-') || b.len() > 2 { b.split('-').map(capitalize).collect::<Vec<_>>().join("_") } else { b.to_uppercase() };
            format!("{a}_{b}")
        }
        None => language.to_lowercase(),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

// ----------------------------------------------------------------------------- plural forms

#[derive(Debug, Clone)]
enum Expr {
    N,
    Num(i64),
    Un(char, Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
}

fn tokenize(s: &str) -> Vec<String> {
    let cs: Vec<char> = s.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let st = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            out.push(cs[st..i].iter().collect());
        } else if i + 1 < cs.len() && ["==", "!=", "<=", ">=", "&&", "||"].contains(&&*cs[i..i + 2].iter().collect::<String>()) {
            out.push(cs[i..i + 2].iter().collect());
            i += 2;
        } else {
            out.push(c.to_string());
            i += 1;
        }
    }
    out
}

struct P {
    t: Vec<String>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&str> {
        self.t.get(self.i).map(String::as_str)
    }
    fn cond(&mut self) -> Option<Expr> {
        let c = self.bin(0)?;
        if self.peek() == Some("?") {
            self.i += 1;
            let a = self.cond()?;
            if self.peek() != Some(":") {
                return None;
            }
            self.i += 1;
            let b = self.cond()?;
            return Some(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Some(c)
    }
    fn bin(&mut self, min: u8) -> Option<Expr> {
        const OPS: [(&str, u8); 13] = [("||", 1), ("&&", 2), ("==", 3), ("!=", 3), ("<", 4), (">", 4), ("<=", 4), (">=", 4), ("+", 5), ("-", 5), ("*", 6), ("/", 6), ("%", 6)];
        let mut l = self.un()?;
        while let Some(&(op, prec)) = self.peek().and_then(|t| OPS.iter().find(|(o, _)| *o == t)) {
            if prec < min {
                break;
            }
            self.i += 1;
            let r = self.bin(prec + 1)?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Some(l)
    }
    fn un(&mut self) -> Option<Expr> {
        match self.peek()? {
            "!" => {
                self.i += 1;
                Some(Expr::Un('!', Box::new(self.un()?)))
            }
            "(" => {
                self.i += 1;
                let e = self.cond()?;
                (self.peek() == Some(")")).then(|| self.i += 1)?;
                Some(e)
            }
            "n" => {
                self.i += 1;
                Some(Expr::N)
            }
            t => {
                let n = t.parse().ok()?;
                self.i += 1;
                Some(Expr::Num(n))
            }
        }
    }
}

fn eval(e: &Expr, n: i64) -> i64 {
    match e {
        Expr::N => n,
        Expr::Num(v) => *v,
        Expr::Un(_, x) => i64::from(eval(x, n) == 0),
        Expr::Cond(c, a, b) => if eval(c, n) != 0 { eval(a, n) } else { eval(b, n) },
        Expr::Bin(op, a, b) => {
            let (x, y) = (eval(a, n), eval(b, n));
            match *op {
                "||" => i64::from(x != 0 || y != 0),
                "&&" => i64::from(x != 0 && y != 0),
                "==" => i64::from(x == y),
                "!=" => i64::from(x != y),
                "<" => i64::from(x < y),
                ">" => i64::from(x > y),
                "<=" => i64::from(x <= y),
                ">=" => i64::from(x >= y),
                "+" => x + y,
                "-" => x - y,
                "*" => x * y,
                "/" => if y == 0 { 0 } else { x / y },
                _ => if y == 0 { 0 } else { x % y },
            }
        }
    }
}

/// A message catalog for one language.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    /// `(context, msgid)` -> msgstr forms.
    messages: HashMap<(String, String), Vec<String>>,
    /// `msgid_plural` of the plural entries.
    plural_ids: HashMap<(String, String), String>,
    plural: Option<(usize, Expr)>,
    /// The header entry (`msgid ""`), kept for `.mo` output.
    pub header: String,
}

impl Catalog {
    fn plural_index(&self, n: u64) -> usize {
        match &self.plural {
            Some((np, e)) => (eval(e, n as i64).max(0) as usize).min(np.saturating_sub(1)),
            None => usize::from(n != 1),
        }
    }

    fn parse_header(&mut self, header: &str) {
        self.header = header.to_owned();
        for line in header.lines() {
            if let Some(v) = line.strip_prefix("Plural-Forms:") {
                let np = v.split("nplurals=").nth(1).and_then(|s| s.split(';').next()).and_then(|s| s.trim().parse().ok()).unwrap_or(2);
                if let Some(expr) = v.split("plural=").nth(1).map(|s| s.trim().trim_end_matches(';')) {
                    if let Some(e) = (P { t: tokenize(expr), i: 0 }).cond() {
                        self.plural = Some((np, e));
                    }
                }
            }
        }
    }

    /// Parse GNU `.po` text (fuzzy entries are skipped).
    pub fn from_po(text: &str) -> Catalog {
        let mut c = Catalog::default();
        #[derive(Default)]
        struct Entry {
            ctx: Option<String>,
            id: String,
            id_plural: Option<String>,
            strs: Vec<String>,
            fuzzy: bool,
            field: u8,
        }
        let mut cur = Entry::default();
        let mut flush = |e: &mut Entry, c: &mut Catalog| {
            let e = std::mem::take(e);
            if e.id.is_empty() && !e.strs.is_empty() {
                c.parse_header(&e.strs[0]);
            } else if !e.id.is_empty() && !e.fuzzy && e.strs.iter().any(|s| !s.is_empty()) {
                let key = (e.ctx.unwrap_or_default(), e.id.clone());
                if let Some(p) = e.id_plural {
                    c.plural_ids.insert(key.clone(), p);
                }
                c.messages.insert(key, e.strs);
            }
        };
        let unquote = |s: &str| -> String {
            let s = s.trim();
            let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
            let mut out = String::new();
            let mut it = inner.chars();
            while let Some(ch) = it.next() {
                if ch == '\\' {
                    match it.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some(o) => {
                            out.push('\\');
                            out.push(o);
                        }
                        None => out.push('\\'),
                    }
                } else {
                    out.push(ch);
                }
            }
            out
        };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("#,") {
                if cur.field != 0 && (!cur.id.is_empty() || !cur.strs.is_empty()) {
                    flush(&mut cur, &mut c);
                }
                cur.fuzzy |= line.contains("fuzzy");
            } else if line.starts_with('#') || line.is_empty() {
                if line.is_empty() && cur.field != 0 {
                    flush(&mut cur, &mut c);
                }
            } else if let Some(r) = line.strip_prefix("msgctxt") {
                if matches!(cur.field, 3 | 4) {
                    flush(&mut cur, &mut c);
                }
                cur.ctx = Some(unquote(r));
                cur.field = 5;
            } else if let Some(r) = line.strip_prefix("msgid_plural") {
                cur.id_plural = Some(unquote(r));
                cur.field = 2;
            } else if let Some(r) = line.strip_prefix("msgid") {
                if matches!(cur.field, 3 | 4) {
                    flush(&mut cur, &mut c);
                }
                cur.id = unquote(r);
                cur.field = 1;
            } else if let Some(r) = line.strip_prefix("msgstr") {
                let idx = r.strip_prefix('[').and_then(|s| s.split(']').next()).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                let val = unquote(r.split_once(' ').map_or("", |x| x.1));
                while cur.strs.len() <= idx {
                    cur.strs.push(String::new());
                }
                cur.strs[idx] = val;
                cur.field = 3 + idx.min(1) as u8;
            } else if line.starts_with('"') {
                let s = unquote(line);
                match cur.field {
                    1 => cur.id.push_str(&s),
                    2 => cur.id_plural.get_or_insert_with(String::new).push_str(&s),
                    5 => cur.ctx.get_or_insert_with(String::new).push_str(&s),
                    _ => {
                        if let Some(l) = cur.strs.last_mut() {
                            l.push_str(&s);
                        }
                    }
                }
            }
        }
        flush(&mut cur, &mut c);
        c
    }

    /// Parse a GNU `.mo` file.
    pub fn from_mo(data: &[u8]) -> Option<Catalog> {
        let rd = |off: usize, le: bool| -> Option<u32> {
            let b: [u8; 4] = data.get(off..off + 4)?.try_into().ok()?;
            Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
        };
        let le = match rd(0, true)? {
            0x9504_12de => true,
            0xde12_0495 => false,
            _ => return None,
        };
        let (n, o_orig, o_trans) = (rd(8, le)? as usize, rd(12, le)? as usize, rd(16, le)? as usize);
        let mut c = Catalog::default();
        for i in 0..n {
            let (l1, p1) = (rd(o_orig + i * 8, le)? as usize, rd(o_orig + i * 8 + 4, le)? as usize);
            let (l2, p2) = (rd(o_trans + i * 8, le)? as usize, rd(o_trans + i * 8 + 4, le)? as usize);
            let orig = String::from_utf8_lossy(data.get(p1..p1 + l1)?).into_owned();
            let trans = String::from_utf8_lossy(data.get(p2..p2 + l2)?).into_owned();
            if orig.is_empty() {
                c.parse_header(&trans);
                continue;
            }
            let (ctx, id) = match orig.split_once('\u{4}') {
                Some((cx, i)) => (cx.to_owned(), i.to_owned()),
                None => (String::new(), orig),
            };
            let mut ids = id.split('\0');
            let id = ids.next().unwrap_or("").to_owned();
            if let Some(p) = ids.next() {
                c.plural_ids.insert((ctx.clone(), id.clone()), p.to_owned());
            }
            c.messages.insert((ctx, id), trans.split('\0').map(str::to_owned).collect());
        }
        Some(c)
    }

    fn merge_under(&mut self, other: &Catalog) {
        for (k, v) in &other.messages {
            self.messages.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if self.plural.is_none() {
            self.plural = other.plural.clone();
        }
    }

    /// All entries as `((context, msgid, msgid_plural), msgstr forms)` (for [`Catalog::to_mo`]).
    pub fn entries(&self) -> Vec<((String, String, Option<String>), Vec<String>)> {
        let mut v: Vec<_> = self.messages.iter().map(|((c, i), f)| ((c.clone(), i.clone(), self.plural_ids.get(&(c.clone(), i.clone())).cloned()), f.clone())).collect();
        v.sort();
        v
    }

    /// Serialize as a `.mo` file (`compilemessages`).
    pub fn to_mo(entries: &[((String, String, Option<String>), Vec<String>)], header: &str) -> Vec<u8> {
        let mut items: Vec<(Vec<u8>, Vec<u8>)> = vec![(Vec::new(), header.as_bytes().to_vec())];
        for ((ctx, id, plural), forms) in entries {
            let mut orig = String::new();
            if !ctx.is_empty() {
                orig.push_str(ctx);
                orig.push('\u{4}');
            }
            orig.push_str(id);
            if let Some(p) = plural {
                orig.push('\0');
                orig.push_str(p);
            }
            items.push((orig.into_bytes(), forms.join("\0").into_bytes()));
        }
        items.sort();
        let n = items.len() as u32;
        let (tbl_o, tbl_t) = (28u32, 28 + n * 8);
        let mut data_off = 28 + n * 16;
        let mut keys = Vec::new();
        let mut vals = Vec::new();
        let mut kdata = Vec::new();
        for (k, _) in &items {
            keys.push((k.len() as u32, data_off + kdata.len() as u32));
            kdata.extend_from_slice(k);
            kdata.push(0);
        }
        data_off += kdata.len() as u32;
        let mut vdata = Vec::new();
        for (_, v) in &items {
            vals.push((v.len() as u32, data_off + vdata.len() as u32));
            vdata.extend_from_slice(v);
            vdata.push(0);
        }
        let mut out = Vec::new();
        for w in [0x9504_12de_u32, 0, n, tbl_o, tbl_t, 0, 0] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for (l, o) in keys.iter().chain(vals.iter()) {
            out.extend_from_slice(&l.to_le_bytes());
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend(kdata);
        out.extend(vdata);
        out
    }
}

static CATALOGS: LazyLock<RwLock<HashMap<String, Arc<Catalog>>>> = LazyLock::new(|| RwLock::new(HashMap::new()));
static EXTRA_DIRS: LazyLock<RwLock<Vec<PathBuf>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Add a locale directory (`<dir>/<lang>/LC_MESSAGES/rango.{mo,po}`), e.g. from an app.
pub fn add_locale_dir(dir: impl Into<PathBuf>) {
    EXTRA_DIRS.write().expect("locale dirs").push(dir.into());
    CATALOGS.write().expect("catalogs").clear();
}

/// Forget cached catalogs (after changing `LOCALE_PATHS` or catalog files).
pub fn reset_catalogs() {
    CATALOGS.write().expect("catalogs").clear();
}

fn locale_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = try_settings().map(|s| s.locale_paths.iter().map(PathBuf::from).collect()).unwrap_or_default();
    if let Ok(apps) = crate::apps::apps() {
        dirs.extend(apps.get_app_configs().filter_map(|c| c.path.as_ref().map(|p| p.join("locale"))));
    }
    dirs.extend(EXTRA_DIRS.read().expect("locale dirs").iter().cloned());
    dirs
}

fn load_file(dir: &Path, locale: &str) -> Option<Catalog> {
    let base = dir.join(locale).join("LC_MESSAGES");
    if let Ok(b) = std::fs::read(base.join("rango.mo")) {
        if let Some(c) = Catalog::from_mo(&b) {
            return Some(c);
        }
    }
    std::fs::read_to_string(base.join("rango.po")).ok().map(|t| Catalog::from_po(&t))
}

fn catalog_for(language: &str) -> Arc<Catalog> {
    if let Some(c) = CATALOGS.read().expect("catalogs").get(language) {
        return Arc::clone(c);
    }
    let mut merged = Catalog::default();
    // `en-us` -> `en_US`, then `en`.
    let mut locales = vec![to_locale(language)];
    if let Some((base, _)) = language.split_once(['-', '_']) {
        locales.push(base.to_lowercase());
    }
    for dir in locale_dirs() {
        for l in &locales {
            if let Some(c) = load_file(&dir, l) {
                merged.merge_under(&c);
            }
        }
    }
    let arc = Arc::new(merged);
    CATALOGS.write().expect("catalogs").insert(language.to_owned(), Arc::clone(&arc));
    arc
}

fn lookup(ctx: &str, id: &str) -> Option<Vec<String>> {
    catalog_for(&get_language()).messages.get(&(ctx.to_owned(), id.to_owned())).cloned()
}

pub fn gettext(message: &str) -> String {
    pgettext("", message)
}

pub fn ngettext(singular: &str, plural: &str, n: u64) -> String {
    npgettext("", singular, plural, n)
}

pub fn pgettext(context: &str, message: &str) -> String {
    lookup(context, message).and_then(|v| v.into_iter().next()).filter(|s| !s.is_empty()).unwrap_or_else(|| message.to_owned())
}

pub fn npgettext(context: &str, singular: &str, plural: &str, n: u64) -> String {
    if let Some(forms) = lookup(context, singular) {
        let cat = catalog_for(&get_language());
        if let Some(f) = forms.get(cat.plural_index(n)).filter(|s| !s.is_empty()) {
            return f.clone();
        }
    }
    if n == 1 { singular } else { plural }.to_owned()
}

/// Languages that have a catalog available (plus the default language).
pub fn available_languages() -> Vec<String> {
    let mut v: Vec<String> = try_settings().map(|s| vec![s.language_code.clone()]).unwrap_or_default();
    for d in locale_dirs() {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten().filter(|e| e.path().join("LC_MESSAGES").is_dir()) {
                let l = e.file_name().to_string_lossy().replace('_', "-").to_lowercase();
                if !v.contains(&l) {
                    v.push(l);
                }
            }
        }
    }
    v
}
