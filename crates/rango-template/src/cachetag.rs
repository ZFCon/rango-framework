//! `{% load cache %}` / `{% cache timeout name vary... %}...{% endcache %}`.

use std::sync::Arc;

use md5::{Digest, Md5};
use rango_core::cache::{Timeout, caches};

use crate::context::Context;
use crate::engine::{Engine, Library, TemplateError};
use crate::nodes::{CustomNode, Node, render_nodes, resolve_expr};
use crate::parser::{FilterExpr, parse_filter_expr};

#[derive(Debug)]
struct CacheNode {
    nodes: Vec<Node>,
    expire: FilterExpr,
    name: String,
    vary: Vec<FilterExpr>,
    using: Option<String>,
}

/// `make_template_fragment_key(name, vary_on)`.
pub fn make_template_fragment_key(name: &str, vary_on: &[String]) -> String {
    let mut h = Md5::new();
    for a in vary_on {
        h.update(a.as_bytes());
        h.update(b":");
    }
    let digest: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("template.cache.{name}.{digest}")
}

impl CustomNode for CacheNode {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError> {
        let expire = resolve_expr(&self.expire, ctx, engine)?;
        let timeout = match expire {
            crate::value::Value::None => Timeout::Never,
            v => Timeout::Secs(v.as_f64().ok_or_else(|| TemplateError::Render(format!("'cache' tag got a non-integer timeout value: {}", v.repr())))?),
        };
        let vary: Vec<String> = self.vary.iter().map(|e| resolve_expr(e, ctx, engine).map(|v| v.to_str())).collect::<Result<_, _>>()?;
        let key = make_template_fragment_key(&self.name, &vary);
        let cache = caches(self.using.as_deref().unwrap_or("default")).map_err(|e| TemplateError::Render(e.to_string()))?;
        if let Ok(Some(hit)) = cache.get_blocking::<String>(&key) {
            return Ok(hit);
        }
        let mut out = String::new();
        render_nodes(&self.nodes, ctx, engine, &mut out)?;
        cache.set_blocking(&key, &out, timeout).map_err(|e| TemplateError::Render(e.to_string()))?;
        Ok(out)
    }
}

pub(crate) fn library() -> Library {
    let mut l = Library::new();
    l.tag(
        "cache",
        Arc::new(|p, bits| {
            if bits.len() < 3 {
                return Err(TemplateError::Syntax(format!("'{}' tag requires at least 2 arguments.", bits[0])));
            }
            let (nodes, end) = p.parse(&["endcache"])?;
            end.ok_or_else(|| TemplateError::Syntax("'cache' doesn't have an end tag".into()))?;
            let mut rest: Vec<String> = bits[3..].to_vec();
            let using = rest.last().and_then(|l| l.strip_prefix("using=")).map(|u| u.trim_matches(|c| c == '"' || c == '\'').to_owned());
            if using.is_some() {
                rest.pop();
            }
            Ok(Node::Custom(Arc::new(CacheNode { nodes, expire: parse_filter_expr(&bits[1])?, name: bits[2].clone(), vary: rest.iter().map(|b| parse_filter_expr(b)).collect::<Result<_, _>>()?, using })))
        }),
    );
    l
}
