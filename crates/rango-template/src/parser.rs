//! Lexer and parser (`template.base`): tokens, filter expressions, `{% if %}` expressions.

use std::sync::Arc;

use crate::engine::{Library, TemplateError};
use crate::nodes::Node;
use crate::value::Value;

#[derive(Clone, Debug)]
pub enum TokenKind {
    Text,
    Var,
    Block,
    Comment,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub contents: String,
    pub line: usize,
}

/// Split template source into tokens. Tags must close on the same line.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut line = 1usize;
    let mut text_line = 1usize;
    let b = src.as_bytes();
    let mut i = 0;
    let flush = |out: &mut Vec<Token>, text: &mut String, line: usize| {
        if !text.is_empty() {
            out.push(Token { kind: TokenKind::Text, contents: std::mem::take(text), line });
        }
    };
    while i < b.len() {
        if b[i] == b'{' && i + 1 < b.len() && matches!(b[i + 1], b'%' | b'{' | b'#') {
            let (close, kind): (&str, TokenKind) = match b[i + 1] {
                b'%' => ("%}", TokenKind::Block),
                b'{' => ("}}", TokenKind::Var),
                _ => ("#}", TokenKind::Comment),
            };
            let rest = &src[i + 2..];
            let line_end = rest.find('\n').unwrap_or(rest.len());
            if let Some(pos) = rest[..line_end].find(close) {
                flush(&mut out, &mut text, text_line);
                out.push(Token { kind, contents: rest[..pos].trim().to_owned(), line });
                i += 2 + pos + 2;
                text_line = line;
                continue;
            }
        }
        let ch = src[i..].chars().next().expect("char");
        if ch == '\n' {
            line += 1;
        }
        if text.is_empty() {
            text_line = line;
        }
        text.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut out, &mut text, text_line);
    out
}

/// A variable reference or literal.
#[derive(Clone, Debug)]
pub enum Operand {
    Literal(Value),
    Var(String),
}

#[derive(Clone, Debug)]
pub struct FilterCall {
    pub name: String,
    pub arg: Option<Operand>,
}

/// `var|filter:arg|...`
#[derive(Clone, Debug)]
pub struct FilterExpr {
    pub var: Operand,
    pub filters: Vec<FilterCall>,
    pub source: String,
}

fn parse_operand(s: &str) -> Result<Operand, TemplateError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(TemplateError::Syntax("Empty variable".into()));
    }
    let first = s.chars().next().expect("non-empty");
    if (first == '"' || first == '\'') && s.len() >= 2 && s.ends_with(first) {
        let inner = &s[1..s.len() - 1];
        return Ok(Operand::Literal(Value::Safe(inner.replace(&format!("\\{first}"), &first.to_string()).replace("\\\\", "\\"))));
    }
    if let Some(inner) = s.strip_prefix("_(").and_then(|r| r.strip_suffix(')')) {
        if let Operand::Literal(Value::Safe(t)) = parse_operand(inner)? {
            return Ok(Operand::Literal(Value::Str(rango_core::utils::translation::gettext(&t))));
        }
    }
    if let Ok(i) = s.parse::<i64>() {
        return Ok(Operand::Literal(Value::Int(i)));
    }
    if s.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.') {
        if let Ok(f) = s.parse::<f64>() {
            if !s.contains("inf") && !s.contains("nan") {
                return Ok(Operand::Literal(Value::Float(f)));
            }
        }
    }
    if s.contains("._") || s.starts_with('_') {
        return Err(TemplateError::Syntax(format!("Variables and attributes may not begin with underscores: '{s}'")));
    }
    Ok(Operand::Var(s.to_owned()))
}

/// Split on `sep` outside of quotes.
fn split_outside_quotes(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some(_), '\\') => {
                cur.push(c);
                escaped = true;
            }
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, c) if c == sep => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

pub fn parse_filter_expr(s: &str) -> Result<FilterExpr, TemplateError> {
    let mut parts = split_outside_quotes(s, '|').into_iter();
    let var = parse_operand(&parts.next().unwrap_or_default())?;
    let mut filters = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            return Err(TemplateError::Syntax(format!("Could not parse some characters: {s}")));
        }
        let (name, arg) = match split_outside_quotes(p, ':').as_slice() {
            [n] => (n.trim().to_owned(), None),
            [n, rest @ ..] => (n.trim().to_owned(), Some(parse_operand(&rest.join(":"))?)),
            [] => unreachable!(),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(TemplateError::Syntax(format!("Could not parse the remainder: '{p}' from '{s}'")));
        }
        filters.push(FilterCall { name, arg });
    }
    Ok(FilterExpr { var, filters, source: s.to_owned() })
}

// ---------------------------------------------------------------------------- if expressions

#[derive(Clone, Debug)]
pub enum IfExpr {
    Operand(FilterExpr),
    Not(Box<IfExpr>),
    Bin(Box<IfExpr>, &'static str, Box<IfExpr>),
}

fn precedence(op: &str) -> u8 {
    match op {
        "or" => 6,
        "and" => 7,
        "not" => 8,
        "in" | "not in" => 9,
        "is" | "is not" | "==" | "!=" | ">" | ">=" | "<" | "<=" => 10,
        _ => 0,
    }
}

struct IfParser {
    toks: Vec<String>,
    pos: usize,
}

const OPS: &[&str] = &["or", "and", "not", "in", "is", "==", "!=", ">", ">=", "<", "<="];

impl IfParser {
    fn peek_op(&self) -> Option<&'static str> {
        let t = self.toks.get(self.pos)?;
        if t == "not" && self.toks.get(self.pos + 1).is_some_and(|n| n == "in") {
            return Some("not in");
        }
        if t == "is" && self.toks.get(self.pos + 1).is_some_and(|n| n == "not") {
            return Some("is not");
        }
        OPS.iter().find(|o| **o == t).copied()
    }

    fn expr(&mut self, rbp: u8) -> Result<IfExpr, TemplateError> {
        let tok = self.toks.get(self.pos).cloned().ok_or_else(|| TemplateError::Syntax("Unexpected end of expression in if tag.".into()))?;
        let mut left = if tok == "not" {
            self.pos += 1;
            IfExpr::Not(Box::new(self.expr(8)?))
        } else if OPS.contains(&tok.as_str()) && tok != "not" {
            return Err(TemplateError::Syntax(format!("Not expecting '{tok}' in this position in if tag.")));
        } else {
            self.pos += 1;
            IfExpr::Operand(parse_filter_expr(&tok)?)
        };
        while let Some(op) = self.peek_op() {
            let lbp = precedence(op);
            if rbp >= lbp {
                break;
            }
            self.pos += if matches!(op, "not in" | "is not") { 2 } else { 1 };
            let right = self.expr(lbp)?;
            left = IfExpr::Bin(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }
}

pub fn parse_if_expr(bits: &[String]) -> Result<IfExpr, TemplateError> {
    let mut p = IfParser { toks: bits.to_vec(), pos: 0 };
    let e = p.expr(0)?;
    if p.pos < p.toks.len() {
        return Err(TemplateError::Syntax(format!("Unused '{}' at end of if expression.", p.toks[p.pos])));
    }
    Ok(e)
}

// ---------------------------------------------------------------------------- parser

pub struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    pub libraries: Vec<Arc<Library>>,
    pub engine_libs: &'a dyn Fn(&str) -> Option<Arc<Library>>,
    pub node_counter: usize,
    /// Named `{% cycle ... as name %}` nodes: `(id, values, silent)`.
    pub named_cycles: std::collections::HashMap<String, (usize, Vec<FilterExpr>, bool)>,
}

impl<'a> Parser<'a> {
    pub fn new(src: &str, libraries: Vec<Arc<Library>>, engine_libs: &'a dyn Fn(&str) -> Option<Arc<Library>>) -> Self {
        Parser { tokens: tokenize(src), pos: 0, libraries, engine_libs, node_counter: 0, named_cycles: Default::default() }
    }

    pub fn next_id(&mut self) -> usize {
        self.node_counter += 1;
        self.node_counter
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    pub fn has_more(&self) -> bool {
        self.pos < self.tokens.len()
    }

    /// Parse nodes until a block tag whose first word is in `until` (not consumed unless returned).
    /// Returns the nodes and the terminating tag's contents (`None` at end of input).
    pub fn parse(&mut self, until: &[&str]) -> Result<(Vec<Node>, Option<String>), TemplateError> {
        let mut nodes = Vec::new();
        while let Some(tok) = self.tokens.get(self.pos).cloned() {
            self.pos += 1;
            match tok.kind {
                TokenKind::Text => nodes.push(Node::Text(tok.contents)),
                TokenKind::Comment => {}
                TokenKind::Var => {
                    if tok.contents.is_empty() {
                        return Err(TemplateError::Syntax(format!("Empty variable tag on line {}", tok.line)));
                    }
                    nodes.push(Node::Var(parse_filter_expr(&tok.contents).map_err(|e| e.at_line(tok.line))?));
                }
                TokenKind::Block => {
                    let bits = crate::tags::split_contents(&tok.contents);
                    let Some(cmd) = bits.first().cloned() else {
                        return Err(TemplateError::Syntax(format!("Empty block tag on line {}", tok.line)));
                    };
                    if until.contains(&cmd.as_str()) {
                        return Ok((nodes, Some(tok.contents)));
                    }
                    let node = crate::tags::compile_tag(self, &cmd, bits, &tok.contents).map_err(|e| e.at_line(tok.line))?;
                    if let Some(n) = node {
                        nodes.push(n);
                    }
                }
            }
        }
        if until.is_empty() {
            return Ok((nodes, None));
        }
        Err(TemplateError::Syntax(format!("Unclosed tag on line {}: '{}'. Looking for one of: {}.", self.tokens.last().map_or(0, |t| t.line), until[0], until.join(", "))))
    }

    /// Skip tokens until `end_tag` (used by `comment`).
    pub fn skip_past(&mut self, end_tag: &str) -> Result<(), TemplateError> {
        while let Some(t) = self.tokens.get(self.pos) {
            self.pos += 1;
            if matches!(t.kind, TokenKind::Block) && t.contents.split_whitespace().next() == Some(end_tag) {
                return Ok(());
            }
        }
        Err(TemplateError::Syntax(format!("Unclosed tag: looking for '{end_tag}'.")))
    }

    /// Collect raw source up to `end_tag` (used by `verbatim`).
    pub fn raw_until(&mut self, end_tag: &str) -> Result<String, TemplateError> {
        let mut out = String::new();
        while let Some(t) = self.tokens.get(self.pos).cloned() {
            self.pos += 1;
            match t.kind {
                TokenKind::Block if t.contents == end_tag => return Ok(out),
                TokenKind::Text => out.push_str(&t.contents),
                TokenKind::Var => out.push_str(&format!("{{{{ {} }}}}", t.contents)),
                TokenKind::Block => out.push_str(&format!("{{% {} %}}", t.contents)),
                TokenKind::Comment => out.push_str(&format!("{{# {} #}}", t.contents)),
            }
        }
        Err(TemplateError::Syntax(format!("Unclosed tag: looking for '{end_tag}'.")))
    }

    pub fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }
}
