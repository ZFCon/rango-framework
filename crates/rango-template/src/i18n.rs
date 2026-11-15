//! `{% load i18n %}`: `trans`/`translate`, `blocktrans`/`blocktranslate`, `language`,
//! `get_current_language`, `get_available_languages`, `get_current_language_bidi`.

use std::sync::Arc;

use rango_core::utils::translation::{self as tr, get_language};

use crate::context::Context;
use crate::engine::{Engine, Library, TemplateError};
use crate::nodes::{CustomNode, Node, render_nodes, resolve_expr};
use crate::parser::{FilterExpr, Operand, parse_filter_expr};
use crate::value::Value;

fn syntax(m: impl Into<String>) -> TemplateError {
    TemplateError::Syntax(m.into())
}

#[derive(Debug)]
struct TransNode {
    expr: FilterExpr,
    literal: bool,
    noop: bool,
    context: Option<FilterExpr>,
    asvar: Option<String>,
}

impl CustomNode for TransNode {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError> {
        let msg = resolve_expr(&self.expr, ctx, engine)?.to_str();
        let pctx = match &self.context {
            Some(c) => resolve_expr(c, ctx, engine)?.to_str(),
            None => String::new(),
        };
        let out = if self.noop { msg } else { tr::pgettext(&pctx, &msg) };
        match &self.asvar {
            Some(v) => {
                ctx.set(v, Value::Str(out));
                Ok(String::new())
            }
            None if self.literal => Ok(out),
            None => Ok(if ctx.autoescape { rango_core::utils::html::escape(&out).into_string() } else { out }),
        }
    }
}

#[derive(Debug)]
struct BlockTransNode {
    singular: Vec<Node>,
    plural: Option<Vec<Node>>,
    with: Vec<(String, FilterExpr)>,
    count: Option<(String, FilterExpr)>,
    context: Option<FilterExpr>,
    trimmed: bool,
    asvar: Option<String>,
}

/// `(msgid, variable names)` of a block body: `{{ name }}` becomes `%(name)s`, `%` becomes `%%`.
fn msgid_of(nodes: &[Node], trimmed: bool) -> Result<(String, Vec<(String, FilterExpr)>), TemplateError> {
    let (mut id, mut vars) = (String::new(), Vec::new());
    for n in nodes {
        match n {
            Node::Text(t) => id.push_str(&t.replace('%', "%%")),
            Node::Var(e) => {
                if !e.filters.is_empty() {
                    return Err(syntax("Filters are not allowed inside blocktranslate; use `with` to bind the value first."));
                }
                let Operand::Var(name) = &e.var else { return Err(syntax("Only variables are allowed inside blocktranslate.")) };
                if name.contains('.') {
                    return Err(syntax(format!("'blocktranslate' doesn't allow dotted variable '{name}'; use `with` to bind it first.")));
                }
                id.push_str(&format!("%({name})s"));
                if !vars.iter().any(|(n, _)| n == name) {
                    vars.push((name.clone(), e.clone()));
                }
            }
            _ => return Err(syntax("Only text and variables are allowed inside blocktranslate.")),
        }
    }
    if trimmed {
        id = id.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    }
    Ok((id, vars))
}

impl CustomNode for BlockTransNode {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError> {
        ctx.push();
        let mut result = (|| -> Result<String, TemplateError> {
            for (k, e) in &self.with {
                let v = resolve_expr(e, ctx, engine)?;
                ctx.set(k, v);
            }
            let mut count = None;
            if let Some((name, e)) = &self.count {
                let v = resolve_expr(e, ctx, engine)?;
                count = Some(v.as_i64().unwrap_or(0).unsigned_abs());
                ctx.set(name, v);
            }
            let (singular, mut vars) = msgid_of(&self.singular, self.trimmed)?;
            let plural = match &self.plural {
                Some(p) => {
                    let (id, v) = msgid_of(p, self.trimmed)?;
                    vars.extend(v);
                    Some(id)
                }
                None => None,
            };
            let pctx = match &self.context {
                Some(c) => resolve_expr(c, ctx, engine)?.to_str(),
                None => String::new(),
            };
            let mut text = match (&plural, count) {
                (Some(p), Some(n)) => tr::npgettext(&pctx, &singular, p, n),
                _ => tr::pgettext(&pctx, &singular),
            };
            // `%(name)s` -> the (escaped) value; `%%` -> `%`.
            for (name, e) in &vars {
                let v = resolve_expr(e, ctx, engine)?;
                let s = v.to_str();
                let s = if ctx.autoescape && !v.is_safe() { rango_core::utils::html::escape(&s).into_string() } else { s };
                text = text.replace(&format!("%({name})s"), &s);
            }
            Ok(text.replace("%%", "%"))
        })();
        ctx.pop();
        if let (Ok(t), Some(v)) = (&result, &self.asvar) {
            ctx.set(v, Value::Safe(t.clone()));
            result = Ok(String::new());
        }
        result
    }
}

#[derive(Debug)]
struct LanguageNode {
    lang: FilterExpr,
    nodes: Vec<Node>,
}

impl CustomNode for LanguageNode {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError> {
        let lang = resolve_expr(&self.lang, ctx, engine)?.to_str();
        let previous = get_language();
        tr::activate(&lang);
        let mut out = String::new();
        let r = render_nodes(&self.nodes, ctx, engine, &mut out);
        tr::activate(&previous);
        r.map(|()| out)
    }
}

fn unquote(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    (b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0]).then(|| &s[1..s.len() - 1])
}

pub(crate) fn library() -> Library {
    let mut l = Library::new();
    for name in ["trans", "translate"] {
        l.tag(
            name,
            Arc::new(|_p, bits| {
                if bits.len() < 2 {
                    return Err(syntax(format!("'{}' takes at least one argument", bits[0])));
                }
                let (mut noop, mut context, mut asvar) = (false, None, None);
                let mut i = 2;
                while i < bits.len() {
                    match bits[i].as_str() {
                        "noop" => noop = true,
                        "context" => {
                            i += 1;
                            context = Some(parse_filter_expr(bits.get(i).ok_or_else(|| syntax("No argument provided to the 'context' option"))?)?);
                        }
                        "as" => {
                            i += 1;
                            asvar = Some(bits.get(i).cloned().ok_or_else(|| syntax("No argument provided to the 'as' option"))?);
                        }
                        other => return Err(syntax(format!("Unknown argument for '{}' tag: '{other}'.", bits[0]))),
                    }
                    i += 1;
                }
                let literal = unquote(&bits[1]).is_some();
                Ok(Node::Custom(Arc::new(TransNode { expr: parse_filter_expr(&bits[1])?, literal, noop, context, asvar })))
            }),
        );
    }
    for (open, ends) in [("blocktrans", ["endblocktrans", "endblocktranslate"]), ("blocktranslate", ["endblocktrans", "endblocktranslate"])] {
        l.tag(
            open,
            Arc::new(move |p, bits| {
                let (mut with, mut count, mut context, mut trimmed, mut asvar) = (vec![], None, None, false, None);
                let mut i = 1;
                while i < bits.len() {
                    match bits[i].as_str() {
                        "with" => {
                            i += 1;
                            while i < bits.len() && bits[i].contains('=') && !matches!(bits[i].as_str(), "count" | "context") {
                                let (k, v) = bits[i].split_once('=').expect("contains =");
                                with.push((k.to_owned(), parse_filter_expr(v)?));
                                i += 1;
                            }
                            continue;
                        }
                        "count" => {
                            i += 1;
                            let (k, v) = bits.get(i).and_then(|b| b.split_once('=')).ok_or_else(|| syntax("'count' in 'blocktranslate' needs name=value"))?;
                            count = Some((k.to_owned(), parse_filter_expr(v)?));
                        }
                        "context" => {
                            i += 1;
                            context = Some(parse_filter_expr(bits.get(i).ok_or_else(|| syntax("No argument provided to the 'context' option"))?)?);
                        }
                        "trimmed" => trimmed = true,
                        "asvar" => {
                            i += 1;
                            asvar = Some(bits.get(i).cloned().ok_or_else(|| syntax("No argument provided to the 'asvar' option"))?);
                        }
                        other if other.contains('=') => {
                            let (k, v) = other.split_once('=').expect("contains =");
                            with.push((k.to_owned(), parse_filter_expr(v)?));
                        }
                        other => return Err(syntax(format!("Unknown argument for 'blocktranslate' tag: '{other}'."))),
                    }
                    i += 1;
                }
                let (singular, term) = p.parse(&["plural", ends[0], ends[1]])?;
                let plural = if term.as_deref().is_some_and(|t| t.starts_with("plural")) {
                    let (pl, end) = p.parse(&[ends[0], ends[1]])?;
                    end.ok_or_else(|| syntax("'blocktranslate' doesn't have an end tag"))?;
                    Some(pl)
                } else {
                    term.ok_or_else(|| syntax("'blocktranslate' doesn't have an end tag"))?;
                    None
                };
                if count.is_some() && plural.is_none() {
                    return Err(syntax("'count' requires a '{% plural %}' section in blocktranslate"));
                }
                Ok(Node::Custom(Arc::new(BlockTransNode { singular, plural, with, count, context, trimmed, asvar })))
            }),
        );
    }
    l.tag(
        "language",
        Arc::new(|p, bits| {
            if bits.len() != 2 {
                return Err(syntax("'language' takes one argument (language)"));
            }
            let (nodes, end) = p.parse(&["endlanguage"])?;
            end.ok_or_else(|| syntax("'language' doesn't have an end tag"))?;
            Ok(Node::Custom(Arc::new(LanguageNode { lang: parse_filter_expr(&bits[1])?, nodes })))
        }),
    );
    l.simple_tag("get_current_language", Arc::new(|_c, _a, _k| Ok(Value::Str(get_language()))));
    l.simple_tag("get_current_language_bidi", Arc::new(|_c, _a, _k| Ok(Value::Bool(tr::get_language_bidi()))));
    l.simple_tag("get_available_languages", Arc::new(|_c, _a, _k| {
        let langs = rango_core::conf::try_settings().map(|s| s.languages.clone()).unwrap_or_default();
        Ok(Value::List(langs.into_iter().map(|(c, n)| Value::List(vec![Value::Str(c), Value::Str(n)])).collect()))
    }));
    l
}

#[allow(unused)]
fn _use(_: &Operand) {}
