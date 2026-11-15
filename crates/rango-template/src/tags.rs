//! Built-in tags (`template.defaulttags`, `loader_tags`) and the custom-tag helpers.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use rand::RngExt;

use crate::context::{BlockData, Context};
use crate::engine::{Engine, Filter, Library, SimpleTag, TemplateError};
use crate::nodes::{Node, SimpleTagNode, render_nodes};
use crate::parser::{FilterExpr, Parser, parse_filter_expr, parse_if_expr};
use crate::value::Value;

fn syntax(m: impl Into<String>) -> TemplateError {
    TemplateError::Syntax(m.into())
}

/// `Token.split_contents()`: split on whitespace keeping quoted phrases together.
pub fn split_contents(contents: &str) -> Vec<String> {
    rango_core::utils::text::smart_split(contents).into_iter().map(str::to_owned).collect()
}

/// `token_kwargs`: leading `key=value` bits, removed from `bits`.
fn token_kwargs(bits: &mut Vec<String>) -> Result<Vec<(String, FilterExpr)>, TemplateError> {
    let mut out = Vec::new();
    while let Some(b) = bits.first() {
        let Some((k, v)) = b.split_once('=') else { break };
        if k.is_empty() || !k.chars().all(|c| c.is_alphanumeric() || c == '_') || v.is_empty() {
            break;
        }
        out.push((k.to_owned(), parse_filter_expr(v)?));
        bits.remove(0);
    }
    Ok(out)
}

pub fn default_tags() -> Library {
    Library::new()
}

pub(crate) fn builtin_extra_library(name: &str) -> Option<Library> {
    match name {
        "static" => {
            let mut l = Library::new();
            l.tag(
                "static",
                Arc::new(|_p, bits| {
                    if bits.len() < 2 {
                        return Err(syntax("'static' takes at least one argument (path to file)"));
                    }
                    let asvar = (bits.len() >= 4 && bits[bits.len() - 2] == "as").then(|| bits[bits.len() - 1].clone());
                    Ok(Node::Static { path: parse_filter_expr(&bits[1])?, asvar })
                }),
            );
            l.simple_tag(
                "get_static_prefix",
                Arc::new(|_c, _a, _k| Ok(Value::Str(rango_core::conf::try_settings().ok().and_then(|s| s.static_url.clone()).unwrap_or_default()))),
            );
            Some(l)
        }
        "i18n" => Some(crate::i18n::library()),
        "cache" => Some(crate::cachetag::library()),
        _ => None,
    }
}

static ALL_LIB_FILTERS: LazyLock<HashMap<String, Filter>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for r in inventory::iter::<crate::engine::LibraryRegistration> {
        let lib = (r.build)();
        m.extend(lib.filters);
    }
    for n in ["static", "i18n", "humanize", "cache"] {
        if let Some(l) = builtin_extra_library(n).or_else(|| crate::filters::extra_library(n)) {
            m.extend(l.filters);
        }
    }
    m
});

/// Filters from `{% load %}`-able libraries (looked up by name at render time).
pub(crate) fn loaded_filter(name: &str, engine: &Engine) -> Option<Filter> {
    let _ = engine;
    ALL_LIB_FILTERS.get(name).copied()
}

/// Compile one block tag. `Ok(None)` for tags that produce no node (`comment`, `load`).
pub fn compile_tag(p: &mut Parser<'_>, cmd: &str, bits: Vec<String>, contents: &str) -> Result<Option<Node>, TemplateError> {
    let _ = contents;
    Ok(Some(match cmd {
        "if" => {
            let mut branches = Vec::new();
            let mut cond = Some(parse_if_expr(&bits[1..])?);
            loop {
                let (nodes, term) = p.parse(&["elif", "else", "endif"])?;
                branches.push((cond.take(), nodes));
                let term = term.ok_or_else(|| syntax("Unclosed tag 'if'"))?;
                let tbits = split_contents(&term);
                match tbits[0].as_str() {
                    "elif" => cond = Some(parse_if_expr(&tbits[1..])?),
                    "else" => {
                        let (nodes, _) = p.parse(&["endif"])?;
                        branches.push((None, nodes));
                        break;
                    }
                    _ => break,
                }
            }
            Node::If(branches)
        }
        "for" => {
            let mut b = bits[1..].to_vec();
            let reversed = b.last().is_some_and(|x| x == "reversed");
            if reversed {
                b.pop();
            }
            let in_pos = b.iter().position(|x| x == "in").ok_or_else(|| syntax("'for' statements should use the format 'for x in y'"))?;
            let vars: Vec<String> = b[..in_pos].join(" ").split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            if vars.is_empty() || vars.iter().any(|v| v.contains(' ')) {
                return Err(syntax(format!("'for' tag received an invalid argument: {}", bits.join(" "))));
            }
            let seq = parse_filter_expr(&b[in_pos + 1..].join(" "))?;
            let (body, term) = p.parse(&["empty", "endfor"])?;
            let empty = if term.as_deref().map(|t| t.split_whitespace().next()) == Some(Some("empty")) { p.parse(&["endfor"])?.0 } else { vec![] };
            Node::For { vars, seq, reversed, body, empty }
        }
        "block" => {
            if bits.len() != 2 {
                return Err(syntax("'block' tag takes only one argument"));
            }
            let (nodes, _) = p.parse(&["endblock"])?;
            Node::Block(Arc::new(BlockData { name: bits[1].clone(), nodes }))
        }
        "extends" => {
            if bits.len() != 2 {
                return Err(syntax("'extends' takes one argument"));
            }
            let parent = parse_filter_expr(&bits[1])?;
            let (nodes, _) = p.parse(&[])?;
            Node::Extends { parent, nodes }
        }
        "include" => {
            let mut b = bits[1..].to_vec();
            if b.is_empty() {
                return Err(syntax("'include' tag takes at least one argument: the name of the template to be included."));
            }
            let template = parse_filter_expr(&b.remove(0))?;
            let mut extra = Vec::new();
            let mut only = false;
            while !b.is_empty() {
                match b[0].as_str() {
                    "with" => {
                        b.remove(0);
                        extra.extend(token_kwargs(&mut b)?);
                    }
                    "only" => {
                        b.remove(0);
                        only = true;
                    }
                    other => return Err(syntax(format!("Unknown argument for 'include' tag: '{other}'."))),
                }
            }
            Node::Include { template, extra, only }
        }
        "with" => {
            let mut b = bits[1..].to_vec();
            let extra = if b.len() == 3 && b[1] == "as" {
                vec![(b[2].clone(), parse_filter_expr(&b[0])?)]
            } else {
                token_kwargs(&mut b)?
            };
            if extra.is_empty() {
                return Err(syntax("'with' expected at least one variable assignment"));
            }
            let (nodes, _) = p.parse(&["endwith"])?;
            Node::With { extra, nodes }
        }
        "url" => {
            if bits.len() < 2 {
                return Err(syntax("'url' takes at least one argument, a URL pattern name."));
            }
            let name = parse_filter_expr(&bits[1])?;
            let mut rest = bits[2..].to_vec();
            let asvar = if rest.len() >= 2 && rest[rest.len() - 2] == "as" {
                let v = rest.pop();
                rest.pop();
                v
            } else {
                None
            };
            let kwargs = token_kwargs(&mut rest)?;
            let args: Vec<FilterExpr> = rest.iter().map(|a| parse_filter_expr(a)).collect::<Result<Vec<_>, _>>()?;
            Node::Url { name, args, kwargs, asvar }
        }
        "csrf_token" => Node::CsrfToken,
        "comment" => {
            p.skip_past("endcomment")?;
            return Ok(None);
        }
        "verbatim" => {
            let end = if bits.len() > 1 { format!("endverbatim {}", bits[1]) } else { "endverbatim".to_owned() };
            Node::Text(p.raw_until(&end)?)
        }
        "load" => {
            let mut names: Vec<String> = bits[1..].to_vec();
            if names.len() >= 3 && names[names.len() - 2] == "from" {
                let lib = names.pop().expect("lib");
                names.pop();
                let l = (p.engine_libs)(&lib).ok_or_else(|| syntax(format!("'{lib}' is not a registered tag library.")))?;
                let mut sel = Library::new();
                for n in names {
                    if let Some(t) = l.tags.get(&n) {
                        sel.tags.insert(n.clone(), t.clone());
                    } else if let Some(f) = l.filters.get(&n) {
                        sel.filters.insert(n.clone(), *f);
                    } else {
                        return Err(syntax(format!("'{n}' is not a valid tag or filter in tag library '{lib}'")));
                    }
                }
                p.libraries.push(Arc::new(sel));
            } else {
                for n in names {
                    let l = (p.engine_libs)(&n).ok_or_else(|| syntax(format!("'{n}' is not a registered tag library. Must be one of:\n")))?;
                    p.libraries.push(l);
                }
            }
            return Ok(None);
        }
        "cycle" => {
            let mut b = bits[1..].to_vec();
            if b.is_empty() {
                return Err(syntax("'cycle' tag requires at least one argument"));
            }
            let id = p.next_id();
            let mut name = None;
            let mut silent = false;
            if b.len() >= 3 && b[b.len() - 2] == "as" {
                name = b.pop();
                b.pop();
            } else if b.len() >= 4 && b[b.len() - 3] == "as" && b[b.len() - 1] == "silent" {
                silent = true;
                b.pop();
                name = b.pop();
                b.pop();
            }
            if b.len() == 1 && name.is_none() {
                if let Some((nid, vals, sil)) = p.named_cycles.get(&b[0]).cloned() {
                    // `{% cycle name %}`: advance the existing named cycle.
                    return Ok(Some(Node::Cycle { id: nid, values: vals, name: Some(b[0].clone()), silent: sil }));
                }
                if !b[0].contains('"') && !b[0].contains('\'') && !b[0].contains('.') {
                    return Err(syntax(format!("Named cycle '{}' does not exist", b[0])));
                }
            }
            let values: Vec<FilterExpr> = b.iter().map(|v| parse_filter_expr(v)).collect::<Result<_, _>>()?;
            if let Some(n) = &name {
                p.named_cycles.insert(n.clone(), (id, values.clone(), silent));
            }
            Node::Cycle { id, values, name, silent }
        }
        "firstof" => {
            let mut b = bits[1..].to_vec();
            let asvar = if b.len() >= 2 && b[b.len() - 2] == "as" {
                let v = b.pop();
                b.pop();
                v
            } else {
                None
            };
            if b.is_empty() {
                return Err(syntax("'firstof' statement requires at least one argument"));
            }
            Node::Firstof { vars: b.iter().map(|v| parse_filter_expr(v)).collect::<Result<_, _>>()?, asvar }
        }
        "filter" => {
            let f = parse_filter_expr(&format!("var|{}", bits[1..].join(" ")))?;
            if f.filters.iter().any(|c| c.name == "escape" || c.name == "safe") {
                return Err(syntax("\"filter\" tag does not accept 'escape' or 'safe' filters"));
            }
            let (nodes, _) = p.parse(&["endfilter"])?;
            Node::Filter { filters: f, nodes }
        }
        "ifchanged" => {
            let vars: Vec<FilterExpr> = bits[1..].iter().map(|v| parse_filter_expr(v)).collect::<Result<_, _>>()?;
            let id = p.next_id();
            let (nodes, term) = p.parse(&["else", "endifchanged"])?;
            let else_nodes = if term.as_deref() == Some("else") { p.parse(&["endifchanged"])?.0 } else { vec![] };
            Node::IfChanged { id, vars, nodes, else_nodes }
        }
        "lorem" => {
            let mut b = bits[1..].to_vec();
            let mut common = true;
            if b.last().is_some_and(|x| x == "random") {
                common = false;
                b.pop();
            }
            let method = if b.last().is_some_and(|x| matches!(x.as_str(), "w" | "p" | "b")) { b.pop().expect("method") } else { "b".to_owned() };
            let count = parse_filter_expr(b.first().map_or("1", String::as_str))?;
            Node::Lorem { count, method, random: !common }
        }
        "now" => {
            let mut b = bits[1..].to_vec();
            let asvar = if b.len() >= 2 && b[b.len() - 2] == "as" {
                let v = b.pop();
                b.pop();
                v
            } else {
                None
            };
            if b.len() != 1 {
                return Err(syntax("'now' statement takes one argument"));
            }
            let fmt = match crate::parser::parse_filter_expr(&b[0])?.var {
                crate::parser::Operand::Literal(v) => v.to_str(),
                _ => return Err(syntax("'now' format must be a string literal")),
            };
            Node::Now { format: fmt, asvar }
        }
        "regroup" => {
            if bits.len() != 6 || bits[2] != "by" || bits[4] != "as" {
                return Err(syntax("'regroup' tag takes five arguments: regroup <list> by <attr> as <name>"));
            }
            Node::Regroup { target: parse_filter_expr(&bits[1])?, expr: bits[3].clone(), var_name: bits[5].clone() }
        }
        "spaceless" => Node::Spaceless(p.parse(&["endspaceless"])?.0),
        "templatetag" => {
            if bits.len() != 2 {
                return Err(syntax("'templatetag' statement takes one argument"));
            }
            Node::TemplateTag(match bits[1].as_str() {
                "openblock" => "{%",
                "closeblock" => "%}",
                "openvariable" => "{{",
                "closevariable" => "}}",
                "openbrace" => "{",
                "closebrace" => "}",
                "opencomment" => "{#",
                "closecomment" => "#}",
                other => return Err(syntax(format!("Invalid templatetag argument: '{other}'. Must be one of: openblock, closeblock, openvariable, closevariable, openbrace, closebrace, opencomment, closecomment"))),
            })
        }
        "widthratio" => {
            let mut b = bits[1..].to_vec();
            let asvar = if b.len() >= 2 && b[b.len() - 2] == "as" {
                let v = b.pop();
                b.pop();
                v
            } else {
                None
            };
            if b.len() != 3 {
                return Err(syntax("widthratio takes at least three arguments"));
            }
            Node::WidthRatio { value: parse_filter_expr(&b[0])?, max: parse_filter_expr(&b[1])?, width: parse_filter_expr(&b[2])?, asvar }
        }
        "autoescape" => {
            let on = match bits.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => return Err(syntax("'autoescape' argument should be 'on' or 'off'")),
            };
            Node::Autoescape { on, nodes: p.parse(&["endautoescape"])?.0 }
        }
        "debug" => Node::Debug,
        other => {
            // Custom tags from loaded libraries.
            let compile = p.libraries.iter().rev().find_map(|l| l.tags.get(other).cloned());
            match compile {
                Some(c) => c(p, bits)?,
                None => return Err(syntax(format!("Invalid block tag: '{other}'"))),
            }
        }
    }))
}

// ---------------------------------------------------------------------------- custom tags

/// Compile `{% name args kw=val as var %}` for `simple_tag`.
pub fn compile_simple_tag(name: &str, bits: Vec<String>, func: SimpleTag) -> Result<Node, TemplateError> {
    let mut b = bits[1..].to_vec();
    let asvar = if b.len() >= 2 && b[b.len() - 2] == "as" {
        let v = b.pop();
        b.pop();
        v
    } else {
        None
    };
    let mut args = Vec::new();
    let mut kwargs = Vec::new();
    for bit in b {
        match bit.split_once('=') {
            Some((k, v)) if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') => kwargs.push((k.to_owned(), parse_filter_expr(v)?)),
            _ => args.push(parse_filter_expr(&bit)?),
        }
    }
    Ok(Node::Simple(Arc::new(SimpleTagNode { name: name.to_owned(), args, kwargs, asvar, func })))
}

type InclusionFn = Arc<dyn Fn(&mut Context, Vec<Value>, IndexMap<String, Value>) -> Result<IndexMap<String, Value>, TemplateError> + Send + Sync>;

pub struct InclusionNode {
    pub template: String,
    pub args: Vec<FilterExpr>,
    pub kwargs: Vec<(String, FilterExpr)>,
    pub func: InclusionFn,
}

impl std::fmt::Debug for InclusionNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InclusionNode({})", self.template)
    }
}

impl InclusionNode {
    pub fn render(&self, ctx: &mut Context, engine: &Engine, out: &mut String) -> Result<(), TemplateError> {
        let args: Vec<Value> = self.args.iter().map(|a| crate::nodes::resolve_expr(a, ctx, engine)).collect::<Result<_, _>>()?;
        let mut kw = IndexMap::new();
        for (k, e) in &self.kwargs {
            kw.insert(k.clone(), crate::nodes::resolve_expr(e, ctx, engine)?);
        }
        let values = (self.func)(ctx, args, kw)?;
        let tpl = engine.get_template(&self.template)?;
        let mut inner = Context::new();
        inner.autoescape = ctx.autoescape;
        if let Some(t) = ctx.get("csrf_token") {
            inner.set("csrf_token", t.clone());
        }
        for (k, v) in values {
            inner.set(&k, v);
        }
        render_nodes(&tpl.nodes, &mut inner, engine, out)
    }
}

pub fn compile_inclusion_tag(name: &str, bits: Vec<String>, template: &str, func: InclusionFn) -> Result<Node, TemplateError> {
    let _ = name;
    let mut args = Vec::new();
    let mut kwargs = Vec::new();
    for bit in &bits[1..] {
        match bit.split_once('=') {
            Some((k, v)) if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') => kwargs.push((k.to_owned(), parse_filter_expr(v)?)),
            _ => args.push(parse_filter_expr(bit)?),
        }
    }
    Ok(Node::Inclusion { tag: Arc::new(InclusionNode { template: template.to_owned(), args, kwargs, func }) })
}

// ---------------------------------------------------------------------------- lorem

static LOREM_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| include_str!("../data/lorem_words.txt").lines().collect());
const COMMON_WORDS: [&str; 19] = [
    "lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipisicing", "elit", "sed", "do", "eiusmod", "tempor", "incididunt", "ut", "labore", "et", "dolore", "magna", "aliqua",
];

fn sentence(rng: &mut impl rand::Rng) -> String {
    let n = rng.random_range(1..=5);
    let sections: Vec<String> = (0..n)
        .map(|_| {
            let k = rng.random_range(3..=12);
            (0..k).map(|_| LOREM_WORDS[rng.random_range(0..LOREM_WORDS.len())]).collect::<Vec<_>>().join(" ")
        })
        .collect();
    let s = sections.join(", ");
    let mut c = s.chars();
    let first = c.next().map(|f| f.to_uppercase().to_string()).unwrap_or_default();
    format!("{first}{}{}", c.as_str(), if rng.random_bool(0.5) { '?' } else { '.' })
}

fn paragraph(rng: &mut impl rand::Rng) -> String {
    let n = rng.random_range(1..=4);
    (0..n).map(|_| sentence(rng)).collect::<Vec<_>>().join(" ")
}

/// `{% lorem count method random %}`.
pub fn lorem(count: usize, method: &str, random: bool) -> String {
    let mut rng = rand::rng();
    let common = !random;
    if method == "w" {
        let mut words: Vec<String> = if common { COMMON_WORDS.iter().map(|s| (*s).to_owned()).collect() } else { vec![] };
        if count > words.len() {
            let mut left = count - words.len();
            while left > 0 {
                let c = left.min(LOREM_WORDS.len());
                left -= c;
                for _ in 0..c {
                    words.push(LOREM_WORDS[rng.random_range(0..LOREM_WORDS.len())].to_owned());
                }
            }
        } else {
            words.truncate(count);
        }
        return words.join(" ");
    }
    let common_p = include_str!("../data/lorem_common_p.txt").trim_end();
    let paras: Vec<String> = (0..count).map(|i| if common && i == 0 { common_p.to_owned() } else { paragraph(&mut rng) }).collect();
    let paras: Vec<String> = if method == "p" { paras.into_iter().map(|p| format!("<p>{p}</p>")).collect() } else { paras };
    paras.join("\n\n")
}
