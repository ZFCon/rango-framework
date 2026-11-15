//! Template nodes and rendering (`template.defaulttags`, `loader_tags`, `base`).

use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::utils::html::{escape, strip_spaces_between_tags};

use crate::context::{BlockData, Context};
use crate::dateformat::{Parts, default_format};
use crate::engine::{Engine, SimpleTag, TemplateError};
use crate::parser::{FilterExpr, IfExpr, Operand};
use crate::value::Value;

/// A user-defined node (from `Library::tag`).
pub trait CustomNode: Send + Sync + std::fmt::Debug {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError>;
}

#[derive(Clone)]
pub struct SimpleTagNode {
    pub name: String,
    pub args: Vec<FilterExpr>,
    pub kwargs: Vec<(String, FilterExpr)>,
    pub asvar: Option<String>,
    pub func: SimpleTag,
}

impl std::fmt::Debug for SimpleTagNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SimpleTagNode({})", self.name)
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Text(String),
    Var(FilterExpr),
    If(Vec<(Option<IfExpr>, Vec<Node>)>),
    For { vars: Vec<String>, seq: FilterExpr, reversed: bool, body: Vec<Node>, empty: Vec<Node> },
    Block(Arc<BlockData>),
    Extends { parent: FilterExpr, nodes: Vec<Node> },
    Include { template: FilterExpr, extra: Vec<(String, FilterExpr)>, only: bool },
    With { extra: Vec<(String, FilterExpr)>, nodes: Vec<Node> },
    Url { name: FilterExpr, args: Vec<FilterExpr>, kwargs: Vec<(String, FilterExpr)>, asvar: Option<String> },
    CsrfToken,
    Static { path: FilterExpr, asvar: Option<String> },
    Cycle { id: usize, values: Vec<FilterExpr>, name: Option<String>, silent: bool },
    Firstof { vars: Vec<FilterExpr>, asvar: Option<String> },
    Filter { filters: FilterExpr, nodes: Vec<Node> },
    IfChanged { id: usize, vars: Vec<FilterExpr>, nodes: Vec<Node>, else_nodes: Vec<Node> },
    Lorem { count: FilterExpr, method: String, random: bool },
    Now { format: String, asvar: Option<String> },
    Regroup { target: FilterExpr, expr: String, var_name: String },
    Spaceless(Vec<Node>),
    TemplateTag(&'static str),
    WidthRatio { value: FilterExpr, max: FilterExpr, width: FilterExpr, asvar: Option<String> },
    Autoescape { on: bool, nodes: Vec<Node> },
    Debug,
    Simple(Arc<SimpleTagNode>),
    Inclusion { tag: Arc<crate::tags::InclusionNode> },
    Custom(Arc<dyn CustomNode>),
}

fn err(s: impl Into<String>) -> TemplateError {
    TemplateError::Render(s.into())
}

// ----------------------------------------------------------------------------- variable resolution

/// `Variable._resolve_lookup` on an already-fetched root value.
fn lookup_bits(mut current: Value, bits: &[&str]) -> Option<Value> {
    for bit in bits {
        current = match &current {
            Value::Map(m) => match m.get(*bit) {
                Some(v) => v.clone(),
                None => match *bit {
                    "items" => Value::List(m.iter().map(|(k, v)| Value::List(vec![Value::Str(k.clone()), v.clone()])).collect()),
                    "keys" => Value::List(m.keys().map(|k| Value::Str(k.clone())).collect()),
                    "values" => Value::List(m.values().cloned().collect()),
                    _ => return None,
                },
            },
            Value::List(l) => l.get(bit.parse::<usize>().ok()?)?.clone(),
            Value::Str(s) | Value::Safe(s) => Value::Str(s.chars().nth(bit.parse::<usize>().ok()?)?.to_string()),
            Value::Object(o) => match o.0.get(bit) {
                Some(v) => v,
                None => o.0.iterate()?.get(bit.parse::<usize>().ok()?)?.clone(),
            },
            _ => return None,
        };
        if let Value::Callable(f) = &current {
            current = f();
        }
    }
    Some(current)
}

fn resolve_var(name: &str, ctx: &Context) -> Option<Value> {
    let mut bits = name.split('.');
    let first = bits.next()?;
    let root = ctx.get(first)?.clone();
    let root = if let Value::Callable(f) = &root { f() } else { root };
    lookup_bits(root, &bits.collect::<Vec<_>>())
}

/// Resolve a [`FilterExpr`] (variable + filter chain). Missing variables become `string_if_invalid`.
pub fn resolve_expr(e: &FilterExpr, ctx: &mut Context, engine: &Engine) -> Result<Value, TemplateError> {
    resolve_expr_opts(e, ctx, engine, false)
}

pub fn resolve_expr_opts(e: &FilterExpr, ctx: &mut Context, engine: &Engine, ignore_failures: bool) -> Result<Value, TemplateError> {
    let mut obj = match &e.var {
        Operand::Literal(v) => v.clone(),
        Operand::Var(name) => {
            let special = name == "block.super";
            if special {
                Value::Safe(render_super(ctx, engine)?)
            } else {
                match resolve_var(name, ctx) {
                    Some(v) => v,
                    None if ignore_failures => Value::None,
                    None => {
                        let inv = &engine.string_if_invalid;
                        if !inv.is_empty() {
                            if inv.contains("%s") {
                                return Ok(Value::Str(inv.replace("%s", name)));
                            }
                            return Ok(Value::Str(inv.clone()));
                        }
                        Value::Str(String::new())
                    }
                }
            }
        }
    };
    for call in &e.filters {
        let filter = engine
            .builtins()
            .iter()
            .find_map(|l| l.filters.get(&call.name).copied())
            .or_else(|| engine.extra_libraries.iter().find_map(|(_, l)| l.filters.get(&call.name).copied()))
            .or_else(|| crate::tags::loaded_filter(&call.name, engine))
            .ok_or_else(|| err(format!("Invalid filter: '{}'", call.name)))?;
        let arg = match &call.arg {
            None => None,
            Some(Operand::Literal(v)) => Some(v.clone()),
            Some(Operand::Var(n)) => Some(resolve_var(n, ctx).unwrap_or(Value::Str(engine.string_if_invalid.clone()))),
        };
        let new = (filter.f)(&obj, arg.as_ref(), ctx.autoescape)?;
        obj = if filter.is_safe && obj.is_safe() { to_safe(new) } else { new };
    }
    Ok(obj)
}

fn to_safe(v: Value) -> Value {
    match v {
        Value::Str(s) => Value::Safe(s),
        other => other,
    }
}

fn tz_parts(v: &Value) -> Option<Parts> {
    match v {
        Value::Date(d) => Some(Parts { date: Some(*d), time: None, offset: None }),
        Value::Time(t) => Some(Parts { date: None, time: Some(*t), offset: None }),
        Value::DateTime(d, off) => Some(Parts { date: Some(d.date()), time: Some(d.time()), offset: *off }),
        _ => None,
    }
}

/// `render_value_in_context`: localize, stringify, escape.
pub fn render_value(v: &Value, ctx: &Context) -> String {
    let s = match tz_parts(v) {
        Some(p) => default_format(&p),
        None => v.to_str(),
    };
    if ctx.autoescape && !v.is_safe() { escape(&s).into_string() } else { s }
}

// ----------------------------------------------------------------------------- if

fn eval_if(e: &IfExpr, ctx: &mut Context, engine: &Engine) -> Result<bool, TemplateError> {
    Ok(eval_if_value(e, ctx, engine)?.truthy())
}

fn eval_if_value(e: &IfExpr, ctx: &mut Context, engine: &Engine) -> Result<Value, TemplateError> {
    Ok(match e {
        IfExpr::Operand(f) => resolve_expr_opts(f, ctx, engine, true)?,
        IfExpr::Not(inner) => Value::Bool(!eval_if_value(inner, ctx, engine)?.truthy()),
        IfExpr::Bin(l, op, r) => {
            if *op == "and" || *op == "or" {
                let lv = eval_if_value(l, ctx, engine)?;
                return Ok(Value::Bool(match *op {
                    "and" => lv.truthy() && eval_if_value(r, ctx, engine)?.truthy(),
                    _ => lv.truthy() || eval_if_value(r, ctx, engine)?.truthy(),
                }));
            }
            let (a, b) = (eval_if_value(l, ctx, engine)?, eval_if_value(r, ctx, engine)?);
            Value::Bool(match *op {
                "==" | "is" => a.eq_py(&b),
                "!=" | "is not" => !a.eq_py(&b),
                "in" => b.contains(&a),
                "not in" => !b.contains(&a),
                ">" => a.cmp_py(&b).is_some_and(|o| o.is_gt()),
                ">=" => a.cmp_py(&b).is_some_and(|o| o.is_ge()),
                "<" => a.cmp_py(&b).is_some_and(|o| o.is_lt()),
                "<=" => a.cmp_py(&b).is_some_and(|o| o.is_le()),
                _ => false,
            })
        }
    })
}

// ----------------------------------------------------------------------------- rendering

fn render_super(ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError> {
    let Some((name, chain)) = ctx.render.super_chain.last().cloned() else {
        return Err(TemplateError::Render("'BlockNode' object has no attribute 'context'".into()));
    };
    let Some(parent) = chain.last().cloned() else { return Ok(String::new()) };
    let mut remaining = chain.clone();
    remaining.pop();
    ctx.render.super_chain.push((name, remaining));
    let mut out = String::new();
    let r = render_nodes(&parent.nodes, ctx, engine, &mut out);
    ctx.render.super_chain.pop();
    r?;
    Ok(out)
}

fn render_block(b: &Arc<BlockData>, ctx: &mut Context, engine: &Engine, out: &mut String) -> Result<(), TemplateError> {
    // `BlockContext.pop(name)`: the most derived override wins; the rest feeds `block.super`.
    let mut chain = ctx.render.blocks.get(&b.name).cloned().unwrap_or_default();
    let chosen = chain.pop().unwrap_or_else(|| Arc::clone(b));
    ctx.render.super_chain.push((b.name.clone(), chain));
    ctx.push();
    let r = render_nodes(&chosen.nodes, ctx, engine, out);
    ctx.pop();
    ctx.render.super_chain.pop();
    r
}

fn collect_blocks(nodes: &[Node], into: &mut Vec<Arc<BlockData>>) {
    for n in nodes {
        match n {
            Node::Block(b) => {
                into.push(Arc::clone(b));
                collect_blocks(&b.nodes, into);
            }
            Node::If(branches) => branches.iter().for_each(|(_, ns)| collect_blocks(ns, into)),
            Node::For { body, empty, .. } => {
                collect_blocks(body, into);
                collect_blocks(empty, into);
            }
            Node::With { nodes, .. } | Node::Spaceless(nodes) | Node::Autoescape { nodes, .. } | Node::Filter { nodes, .. } => collect_blocks(nodes, into),
            Node::IfChanged { nodes, else_nodes, .. } => {
                collect_blocks(nodes, into);
                collect_blocks(else_nodes, into);
            }
            _ => {}
        }
    }
}

pub fn render_nodes(nodes: &[Node], ctx: &mut Context, engine: &Engine, out: &mut String) -> Result<(), TemplateError> {
    for n in nodes {
        render_node(n, ctx, engine, out)?;
    }
    Ok(())
}

fn args_values(args: &[FilterExpr], ctx: &mut Context, engine: &Engine) -> Result<Vec<Value>, TemplateError> {
    args.iter().map(|a| resolve_expr(a, ctx, engine)).collect()
}

fn render_node(n: &Node, ctx: &mut Context, engine: &Engine, out: &mut String) -> Result<(), TemplateError> {
    match n {
        Node::Text(t) => out.push_str(t),
        Node::Var(e) => {
            let v = resolve_expr(e, ctx, engine)?;
            out.push_str(&render_value(&v, ctx));
        }
        Node::If(branches) => {
            for (cond, body) in branches {
                let take = match cond {
                    Some(c) => eval_if(c, ctx, engine)?,
                    None => true,
                };
                if take {
                    return render_nodes(body, ctx, engine, out);
                }
            }
        }
        Node::For { vars, seq, reversed, body, empty } => {
            let value = resolve_expr_opts(seq, ctx, engine, false)?;
            let mut items: Vec<Value> = value.iterate().unwrap_or_default();
            if *reversed {
                items.reverse();
            }
            if items.is_empty() {
                return render_nodes(empty, ctx, engine, out);
            }
            let parent = ctx.get("forloop").cloned();
            let total = items.len();
            ctx.push();
            for (i, item) in items.into_iter().enumerate() {
                let mut fl: IndexMap<String, Value> = IndexMap::new();
                fl.insert("counter0".into(), Value::Int(i as i64));
                fl.insert("counter".into(), Value::Int(i as i64 + 1));
                fl.insert("revcounter".into(), Value::Int((total - i) as i64));
                fl.insert("revcounter0".into(), Value::Int((total - i - 1) as i64));
                fl.insert("first".into(), Value::Bool(i == 0));
                fl.insert("last".into(), Value::Bool(i + 1 == total));
                fl.insert("parentloop".into(), parent.clone().unwrap_or(Value::Map(IndexMap::new())));
                ctx.set("forloop", Value::Map(fl));
                if vars.len() == 1 {
                    ctx.set(&vars[0], item);
                } else {
                    let parts = match item {
                        Value::List(l) => l,
                        other => vec![other],
                    };
                    if parts.len() != vars.len() {
                        ctx.pop();
                        return Err(err(format!("Need {} values to unpack in for loop; got {}.", vars.len(), parts.len())));
                    }
                    for (v, val) in vars.iter().zip(parts) {
                        ctx.set(v, val);
                    }
                }
                if let Err(e) = render_nodes(body, ctx, engine, out) {
                    ctx.pop();
                    return Err(e);
                }
            }
            ctx.pop();
        }
        Node::Block(b) => render_block(b, ctx, engine, out)?,
        Node::Extends { parent, nodes } => {
            let parent_t = crate::engine::resolve_template_arg(engine, ctx, parent)?;
            // Child blocks override; the root's blocks are registered last (outermost).
            let mut child_blocks = Vec::new();
            collect_blocks(nodes, &mut child_blocks);
            let mut parent_blocks = Vec::new();
            collect_blocks(&parent_t.nodes, &mut parent_blocks);
            let is_root = !parent_t.nodes.iter().any(|n| matches!(n, Node::Extends { .. }));
            for b in child_blocks {
                // `add_blocks` inserts at the front: ancestors end up first, the most derived last.
                ctx.render.blocks.entry(b.name.clone()).or_default().insert(0, b);
            }
            if is_root {
                for b in parent_blocks {
                    ctx.render.blocks.entry(b.name.clone()).or_default().insert(0, b);
                }
            }
            ctx.render.used_templates.push(parent_t.name.clone());
            render_nodes(&parent_t.nodes, ctx, engine, out)?;
        }
        Node::Include { template, extra, only } => {
            let name = resolve_expr(template, ctx, engine)?;
            let tpl = engine.get_template(&name.to_str())?;
            let mut vals = Vec::new();
            for (k, e) in extra {
                vals.push((k.clone(), resolve_expr(e, ctx, engine)?));
            }
            let saved = if *only { Some(ctx.isolate()) } else { None };
            ctx.push();
            for (k, v) in vals {
                ctx.set(&k, v);
            }
            ctx.render.used_templates.push(tpl.name.clone());
            ctx.render.include_depth += 1;
            let blocks_backup = std::mem::take(&mut ctx.render.blocks);
            let r = render_nodes(&tpl.nodes, ctx, engine, out);
            ctx.render.blocks = blocks_backup;
            ctx.render.include_depth -= 1;
            ctx.pop();
            if let Some(s) = saved {
                ctx.restore(s);
            }
            r?;
        }
        Node::With { extra, nodes } => {
            let mut vals = Vec::new();
            for (k, e) in extra {
                vals.push((k.clone(), resolve_expr(e, ctx, engine)?));
            }
            ctx.push();
            for (k, v) in vals {
                ctx.set(&k, v);
            }
            let r = render_nodes(nodes, ctx, engine, out);
            ctx.pop();
            r?;
        }
        Node::Url { name, args, kwargs, asvar } => {
            let view = resolve_expr(name, ctx, engine)?.to_str();
            let mut r = rango_urls::Reverse::new();
            let argv = args_values(args, ctx, engine)?;
            r.args = argv.iter().map(Value::to_str).collect();
            for (k, e) in kwargs {
                r.kwargs.insert(k.clone(), resolve_expr(e, ctx, engine)?.to_str());
            }
            match rango_urls::reverse(&view, r) {
                Ok(url) => match asvar {
                    Some(v) => ctx.set(v, Value::Str(url)),
                    None => out.push_str(&if ctx.autoescape { escape(&url).into_string() } else { url }),
                },
                Err(e) => {
                    if let Some(v) = asvar {
                        ctx.set(v, Value::Str(String::new()));
                    } else {
                        return Err(err(e.to_string()));
                    }
                }
            }
        }
        Node::CsrfToken => {
            if let Some(t) = ctx.get("csrf_token") {
                let t = if let Value::Callable(f) = t { f() } else { t.clone() }.to_str();
                if !t.is_empty() && t != "NOTPROVIDED" {
                    out.push_str(&format!("<input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{}\">", escape(&t)));
                }
            }
        }
        Node::Static { path, asvar } => {
            let p = resolve_expr(path, ctx, engine)?.to_str();
            let base = rango_core::conf::try_settings().ok().and_then(|s| s.static_url.clone()).unwrap_or_default();
            let url = format!("{}{}", base, rango_core::utils::http::quote(&p, "/"));
            match asvar {
                Some(v) => ctx.set(v, Value::Str(url)),
                None => out.push_str(&escape(&url)),
            }
        }
        Node::Cycle { id, values, name, silent } => {
            let i = *ctx.render.cycles.entry(*id).or_insert(0);
            ctx.render.cycles.insert(*id, i + 1);
            let v = resolve_expr(&values[i % values.len()], ctx, engine)?;
            if let Some(n) = name {
                ctx.set(n, v.clone());
            }
            if !*silent {
                out.push_str(&render_value(&v, ctx));
            }
        }
        Node::Firstof { vars, asvar } => {
            let mut result = Value::Str(String::new());
            for v in vars {
                let r = resolve_expr_opts(v, ctx, engine, true)?;
                if r.truthy() {
                    result = r;
                    break;
                }
            }
            match asvar {
                Some(n) => ctx.set(n, result),
                None => out.push_str(&render_value(&result, ctx)),
            }
        }
        Node::Filter { filters, nodes } => {
            let mut inner = String::new();
            let saved = ctx.autoescape;
            ctx.autoescape = true;
            let r = render_nodes(nodes, ctx, engine, &mut inner);
            ctx.autoescape = saved;
            r?;
            ctx.push();
            ctx.set("var", Value::Safe(inner));
            let mut expr = filters.clone();
            expr.var = Operand::Var("var".into());
            let res = resolve_expr(&expr, ctx, engine);
            ctx.pop();
            out.push_str(&render_value(&res?, ctx));
        }
        Node::IfChanged { id, vars, nodes, else_nodes } => {
            let current: Vec<Value> = if vars.is_empty() {
                let mut s = String::new();
                render_nodes(nodes, ctx, engine, &mut s)?;
                vec![Value::Str(s)]
            } else {
                vars.iter().map(|v| resolve_expr(v, ctx, engine)).collect::<Result<_, _>>()?
            };
            let changed = ctx.render.ifchanged.get(id).is_none_or(|last| last.len() != current.len() || last.iter().zip(&current).any(|(a, b)| !a.eq_py(b)));
            if changed {
                ctx.render.ifchanged.insert(*id, current);
                render_nodes(nodes, ctx, engine, out)?;
            } else {
                render_nodes(else_nodes, ctx, engine, out)?;
            }
        }
        Node::Lorem { count, method, random } => {
            let c = resolve_expr(count, ctx, engine)?.as_i64().unwrap_or(1).max(0) as usize;
            out.push_str(&crate::tags::lorem(c, method, *random));
        }
        Node::Now { format, asvar } => {
            let now = rango_core::utils::timezone::now();
            let local = rango_core::utils::timezone::localtime(now, None).map(|d| (d.naive_local(), chrono::Offset::fix(d.offset()).local_minus_utc())).unwrap_or((now.naive_utc(), 0));
            let s = crate::dateformat::format(&Parts { date: Some(local.0.date()), time: Some(local.0.time()), offset: Some(local.1) }, format).map_err(err)?;
            match asvar {
                Some(v) => ctx.set(v, Value::Str(s)),
                None => out.push_str(&if ctx.autoescape { escape(&s).into_string() } else { s }),
            }
        }
        Node::Regroup { target, expr, var_name } => {
            let list = resolve_expr(target, ctx, engine)?.iterate().unwrap_or_default();
            let mut groups: Vec<Value> = Vec::new();
            let mut last: Option<Value> = None;
            for item in list {
                let key = lookup_bits(item.clone(), &expr.split('.').collect::<Vec<_>>()).unwrap_or(Value::Str(String::new()));
                if last.as_ref().is_some_and(|l| l.eq_py(&key)) {
                    if let Some(Value::Map(m)) = groups.last_mut() {
                        if let Some(Value::List(l)) = m.get_mut("list") {
                            l.push(item);
                        }
                    }
                } else {
                    let mut m = IndexMap::new();
                    m.insert("grouper".to_owned(), key.clone());
                    m.insert("list".to_owned(), Value::List(vec![item]));
                    groups.push(Value::Map(m));
                    last = Some(key);
                }
            }
            ctx.set(var_name, Value::List(groups));
        }
        Node::Spaceless(nodes) => {
            let mut s = String::new();
            render_nodes(nodes, ctx, engine, &mut s)?;
            out.push_str(&strip_spaces_between_tags(s.trim()));
        }
        Node::TemplateTag(t) => out.push_str(t),
        Node::WidthRatio { value, max, width, asvar } => {
            let (v, m, w) = (resolve_expr(value, ctx, engine)?.as_f64(), resolve_expr(max, ctx, engine)?.as_f64(), resolve_expr(width, ctx, engine)?.as_f64());
            let s = match (v, m, w) {
                (Some(v), Some(m), Some(w)) if m != 0.0 => {
                    let r = v / m * w;
                    if r.is_finite() { format!("{}", r.round_ties_even() as i64) } else { String::new() }
                }
                _ => "0".to_owned(),
            };
            match asvar {
                Some(n) => ctx.set(n, Value::Str(s)),
                None => out.push_str(&s),
            }
        }
        Node::Autoescape { on, nodes } => {
            let saved = ctx.autoescape;
            ctx.autoescape = *on;
            let r = render_nodes(nodes, ctx, engine, out);
            ctx.autoescape = saved;
            r?;
        }
        Node::Debug => out.push_str("<pre>{}</pre>"),
        Node::Simple(s) => {
            let argv = args_values(&s.args, ctx, engine)?;
            let mut kw = IndexMap::new();
            for (k, e) in &s.kwargs {
                kw.insert(k.clone(), resolve_expr(e, ctx, engine)?);
            }
            let v = (s.func)(ctx, argv, kw)?;
            match &s.asvar {
                Some(n) => ctx.set(n, v),
                None => out.push_str(&render_value(&v, ctx)),
            }
        }
        Node::Inclusion { tag } => tag.render(ctx, engine, out)?,
        Node::Custom(c) => out.push_str(&c.render(ctx, engine)?),
    }
    Ok(())
}
