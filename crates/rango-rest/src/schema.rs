//! OpenAPI 3 schema generation and documentation views (`OpenAPI schema generation`).
//!
//! The generator walks the URLconf, finds the routes registered through [`crate::Router`] and
//! describes them from their serializers, pagination, filters and authentication.
//! The document is served as JSON (which is also valid YAML).

use std::collections::BTreeMap;

use async_trait::async_trait;
use clap::Arg;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_core::management::{Command, CommandContext, CommandError};
use rango_http::HttpResponse;
use rango_urls::{UrlEntry, View, get_resolver};
use serde_json::{Map, Value as Json, json};

use crate::routers::{Registered, lookup};
use crate::serializers::{Field, Kind, Serializer};

fn setting(key: &str) -> Option<Json> {
    settings().spectacular_settings.get(key).cloned()
}

/// `{name}` path from a route (`<int:pk>/`) or regex (`^x/(?P<pk>[^/.]+)/$`) pattern.
fn clean(pattern: &str) -> String {
    let p = pattern.strip_prefix('^').unwrap_or(pattern);
    let p = p.strip_suffix('$').unwrap_or(p);
    let (cs, mut out, mut i) = (p.chars().collect::<Vec<_>>(), String::new(), 0);
    while i < cs.len() {
        if cs[i] == '<' {
            let end = cs[i..].iter().position(|c| *c == '>').map_or(cs.len(), |e| i + e);
            let inner: String = cs[i + 1..end].iter().collect();
            out.push_str(&format!("{{{}}}", inner.rsplit(':').next().unwrap_or(&inner)));
            i = end + 1;
        } else if cs[i..].starts_with(&['(', '?', 'P', '<']) {
            let name_end = cs[i..].iter().position(|c| *c == '>').map_or(cs.len(), |e| i + e);
            let name: String = cs[i + 4..name_end].iter().collect();
            let (mut depth, mut j) = (0, i);
            while j < cs.len() {
                match cs[j] {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    '\\' => j += 1,
                    _ => {}
                }
                j += 1;
            }
            out.push_str(&format!("{{{name}}}"));
            i = j + 1;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    out
}

fn walk(entries: &[UrlEntry], prefix: &str, out: &mut Vec<(String, Registered)>) {
    for e in entries {
        match e {
            UrlEntry::Pattern(p) => {
                if let Some(r) = lookup(&p.callback.name) {
                    out.push((format!("{prefix}{}", clean(&p.pattern.describe)), r));
                }
            }
            UrlEntry::Resolver(r) => {
                if let Ok(children) = r.url_patterns() {
                    walk(children, &format!("{prefix}{}", clean(&r.pattern.describe)), out);
                }
            }
        }
    }
}

/// The OpenAPI schema of one serializer field.
fn field_schema(f: &Field) -> Json {
    let mut m = Map::new();
    let set = |m: &mut Map<String, Json>, k: &str, v: Json| {
        m.insert(k.to_owned(), v);
    };
    match &f.kind {
        Kind::Boolean => set(&mut m, "type", json!("boolean")),
        Kind::Char { max_length, min_length, format, .. } => {
            set(&mut m, "type", json!("string"));
            match format {
                Some("slug") => set(&mut m, "pattern", json!("^[-a-zA-Z0-9_]+$")),
                Some(fmt) => set(&mut m, "format", json!(fmt)),
                None => {}
            }
        if let Some(n) = min_length {
                set(&mut m, "minLength", json!(n));
            }
            if let Some(n) = max_length {
                set(&mut m, "maxLength", json!(n));
            }
        }
        Kind::Integer { min, max } => {
            set(&mut m, "type", json!("integer"));
            if let Some(v) = max {
                set(&mut m, "maximum", json!(v));
                if *v > i64::from(i32::MAX) {
                    set(&mut m, "format", json!("int64"));
                }
            }
            if let Some(v) = min {
                set(&mut m, "minimum", json!(v));
            }
        }
        Kind::Float { min, max } => {
            set(&mut m, "type", json!("number"));
            set(&mut m, "format", json!("double"));
            if let Some(v) = max {
                set(&mut m, "maximum", json!(v));
            }
            if let Some(v) = min {
                set(&mut m, "minimum", json!(v));
            }
        }
        Kind::Decimal { max_digits, decimal_places } => {
            set(&mut m, "type", json!("string"));
            set(&mut m, "format", json!("decimal"));
            if let (Some(d), Some(p)) = (max_digits, decimal_places) {
                set(&mut m, "pattern", json!(format!("^-?\\d{{0,{}}}(?:\\.\\d{{0,{p}}})?$", d - p)));
            }
        }
        Kind::DateTime => {
            set(&mut m, "type", json!("string"));
            set(&mut m, "format", json!("date-time"));
        }
        Kind::Date => {
            set(&mut m, "type", json!("string"));
            set(&mut m, "format", json!("date"));
        }
        Kind::Time => {
            set(&mut m, "type", json!("string"));
            set(&mut m, "format", json!("time"));
        }
        Kind::Duration => set(&mut m, "type", json!("string")),
        Kind::Choice(c) => {
            set(&mut m, "enum", Json::Array(c.iter().map(|(v, _)| v.clone()).collect()));
            set(&mut m, "type", json!(if c.iter().all(|(v, _)| v.is_i64()) { "integer" } else { "string" }));
        }
        Kind::Json | Kind::Method(_) => {}
        Kind::List(child) => {
            set(&mut m, "type", json!("array"));
            set(&mut m, "items", field_schema(child));
        }
        Kind::Related { many, .. } => {
            if *many {
                set(&mut m, "type", json!("array"));
                set(&mut m, "items", json!({"type": "integer"}));
            } else {
                set(&mut m, "type", json!("integer"));
            }
        }
        Kind::Nested(s, many) => {
            let r = json!({"$ref": format!("#/components/schemas/{}", s.name)});
            if *many {
                set(&mut m, "type", json!("array"));
                set(&mut m, "items", r);
            } else {
                return r;
            }
        }
    }
    if f.read_only {
        set(&mut m, "readOnly", json!(true));
    }
    if f.write_only {
        set(&mut m, "writeOnly", json!(true));
    }
    if f.allow_null {
        set(&mut m, "nullable", json!(true));
    }
    if let Some(d) = &f.default {
        set(&mut m, "default", d.clone());
    }
    if let Some(h) = &f.help_text {
        set(&mut m, "description", json!(h));
    }
    Json::Object(m)
}

/// The component schema of a serializer (`patched` drops `required`, as for PATCH bodies).
fn serializer_schema(s: &Serializer, patched: bool, components: &mut BTreeMap<String, Json>) -> Json {
    let mut props = Map::new();
    let mut required = Vec::new();
    for (name, f) in &s.fields {
        if let Kind::Nested(inner, _) = &f.kind {
            if !components.contains_key(&inner.name) {
                let sc = serializer_schema(inner, false, components);
                components.insert(inner.name.clone(), sc);
            }
        }
        props.insert(name.clone(), field_schema(f));
        if !patched && (f.read_only || f.is_required()) {
            required.push(json!(name));
        }
    }
    let mut m = Map::new();
    m.insert("type".into(), json!("object"));
    m.insert("properties".into(), Json::Object(props));
    if !required.is_empty() {
        m.insert("required".into(), Json::Array(required));
    }
    Json::Object(m)
}

fn reference(name: &str) -> Json {
    json!({"$ref": format!("#/components/schemas/{name}")})
}

/// `SchemaGenerator().get_schema()`: the OpenAPI document for the current URLconf.
pub fn generate() -> Result<Json> {
    let resolver = get_resolver(None)?;
    let mut found = Vec::new();
    walk(resolver.url_patterns()?, "/", &mut found);
    // Common prefix of all API paths (`SCHEMA_PATH_PREFIX`): left out of operation ids and tags.
    let segs: Vec<Vec<&str>> = found.iter().map(|(p, _)| p.split('/').filter(|s| !s.is_empty()).collect()).collect();
    let mut common = 0;
    while segs.iter().all(|s| s.len() > common + 1 && !s[common].starts_with('{') && s[common] == segs[0][common]) && !segs.is_empty() {
        common += 1;
    }
    let mut paths: BTreeMap<String, Map<String, Json>> = BTreeMap::new();
    let mut schemas: BTreeMap<String, Json> = BTreeMap::new();
    let mut security_schemes: BTreeMap<String, Json> = BTreeMap::new();
    for (i, (path, reg)) in found.iter().enumerate() {
        let info = reg.viewset.schema();
        let name = info.serializer.name.clone();
        let pk_name = if info.lookup_is_integer { "id" } else { "pk" };
        let path = path.replace("{pk}", &format!("{{{pk_name}}}"));
        let rest: Vec<String> = segs[i][common..].iter().map(|s| s.trim_matches(|c| c == '{' || c == '}').replace('-', "_")).filter(|s| *s != "pk").collect();
        let tag = segs[i].get(common).map_or_else(|| reg.prefix.clone(), |s| (*s).to_owned());
        let auths = reg.viewset.config().authenticators();
        let perms = reg.viewset.config().permission_list();
        for (method, action) in &reg.route.actions {
            let suffix = match (action.as_str(), *method) {
                ("list" | "create" | "retrieve" | "update" | "partial_update" | "destroy", _) => action.clone(),
                (_, "GET") => if reg.route.detail { "retrieve".into() } else { "list".into() },
                (_, "POST") => "create".into(),
                (_, "PUT") => "update".into(),
                (_, "PATCH") => "partial_update".into(),
                _ => "destroy".into(),
            };
            let standard = reg.route.url_path.is_none();
            let mut op = Map::new();
            op.insert("operationId".into(), json!(format!("{}_{suffix}", rest.join("_"))));
            if !info.description.is_empty() {
                op.insert("description".into(), json!(info.description));
            }
            let mut params: Vec<Json> = Vec::new();
            if action == "list" {
                for f in &info.filterset_fields {
                    let schema = info.serializer.fields.get(f).map_or(json!({"type": "string"}), |fl| {
                        let mut s = field_schema(fl);
                        if let Some(o) = s.as_object_mut() {
                            o.retain(|k, _| matches!(k.as_str(), "type" | "format" | "enum"));
                        }
                        s
                    });
                    params.push(json!({"in": "query", "name": f, "schema": schema}));
                }
                if info.ordering {
                    params.push(json!({"name": crate::settings::ordering_param(), "required": false, "in": "query", "description": "Which field to use when ordering the results.", "schema": {"type": "string"}}));
                }
                if let Some(p) = &info.pagination {
                    for (n, ty, desc) in p.schema_parameters() {
                        params.push(json!({"name": n, "required": false, "in": "query", "description": desc, "schema": {"type": ty}}));
                    }
                }
                if info.search {
                    params.push(json!({"name": crate::settings::search_param(), "required": false, "in": "query", "description": "A search term.", "schema": {"type": "string"}}));
                }
            }
            if reg.route.detail {
                let (ty, desc) = if info.lookup_is_integer { ("integer", format!("A unique integer value identifying this {}.", reg.viewset.display_name().to_lowercase())) } else { ("string", String::new()) };
                let mut p = json!({"in": "path", "name": pk_name, "schema": {"type": ty}, "required": true});
                if !desc.is_empty() {
                    p["description"] = json!(desc);
                }
                params.push(p);
            }
            params.sort_by_key(|p| (p["in"] == "path", p["name"].as_str().unwrap_or("").to_owned()));
            if !params.is_empty() {
                op.insert("parameters".into(), Json::Array(params));
            }
            op.insert("tags".into(), json!([tag]));
            // request body
            let writes = matches!(*method, "POST" | "PUT" | "PATCH");
            if writes && standard {
                let patched = *method == "PATCH";
                let comp = if patched { format!("Patched{name}") } else { name.clone() };
                if !schemas.contains_key(&comp) {
                    let sc = serializer_schema(&info.serializer, patched, &mut schemas);
                    schemas.insert(comp.clone(), sc);
                }
                let content: Map<String, Json> = ["application/json", "application/x-www-form-urlencoded", "multipart/form-data"].iter().map(|ct| ((*ct).to_owned(), json!({"schema": reference(&comp)}))).collect();
                let mut body = Map::new();
                body.insert("content".into(), Json::Object(content));
                if !patched && info.serializer.fields.values().any(|f| !f.read_only && f.is_required()) {
                    body.insert("required".into(), json!(true));
                }
                op.insert("requestBody".into(), Json::Object(body));
            }
            // security
            let safe = matches!(*method, "GET");
            let mut security: Vec<Json> = Vec::new();
            for a in &auths {
                if let Some((n, scheme)) = a.openapi() {
                    security_schemes.insert(n.clone(), scheme);
                    security.push(json!({n: []}));
                }
            }
            if perms.iter().all(|p| p.allows_anonymous(safe)) {
                security.push(json!({}));
            }
            if !security.is_empty() {
                op.insert("security".into(), Json::Array(security));
            }
            // responses
            if !schemas.contains_key(&name) {
                let sc = serializer_schema(&info.serializer, false, &mut schemas);
                schemas.insert(name.clone(), sc);
            }
            let ok = |schema: Json| json!({"content": {"application/json": {"schema": schema}}, "description": ""});
            let responses = match action.as_str() {
                "destroy" => json!({"204": {"description": "No response body"}}),
                "create" => json!({"201": ok(reference(&name))}),
                "list" => match &info.pagination {
                    Some(_) => {
                        let pname = format!("Paginated{name}List");
                        schemas.entry(pname.clone()).or_insert_with(|| {
                            json!({"type": "object", "required": ["count", "results"], "properties": {
                                "count": {"type": "integer", "example": 123},
                                "next": {"type": "string", "nullable": true, "format": "uri", "example": "http://api.example.org/accounts/?page=4"},
                                "previous": {"type": "string", "nullable": true, "format": "uri", "example": "http://api.example.org/accounts/?page=2"},
                                "results": {"type": "array", "items": reference(&name)}}})
                        });
                        json!({"200": ok(reference(&pname))})
                    }
                    None => json!({"200": ok(json!({"type": "array", "items": reference(&name)}))}),
                },
                _ if standard => json!({"200": ok(reference(&name))}),
                _ => json!({"200": {"description": "No response body"}}),
            };
            op.insert("responses".into(), responses);
            paths.entry(path.clone()).or_default().insert(method.to_lowercase(), Json::Object(op));
        }
    }
    let mut info = Map::new();
    info.insert("title".into(), setting("TITLE").unwrap_or(json!("")));
    info.insert("version".into(), setting("VERSION").unwrap_or(json!("0.0.0")));
    if let Some(d) = setting("DESCRIPTION").filter(|d| d.as_str().is_some_and(|s| !s.is_empty())) {
        info.insert("description".into(), d);
    }
    let mut doc = Map::new();
    doc.insert("openapi".into(), json!("3.0.3"));
    doc.insert("info".into(), Json::Object(info));
    doc.insert("paths".into(), json!(paths));
    let mut components = Map::new();
    components.insert("schemas".into(), json!(schemas));
    if !security_schemes.is_empty() {
        components.insert("securitySchemes".into(), json!(security_schemes));
    }
    doc.insert("components".into(), Json::Object(components));
    if let Some(s) = setting("SERVERS").filter(|s| s.as_array().is_some_and(|a| !a.is_empty())) {
        doc.insert("servers".into(), s);
    }
    Ok(Json::Object(doc))
}

fn page(html: String) -> View {
    View::new("rango.rest.schema.docs", move |_, _| {
        let html = html.clone();
        Box::pin(async move { Ok(HttpResponse::new(html)) })
    })
}

/// `SpectacularAPIView`: the OpenAPI document.
pub fn schema_view() -> View {
    View::new("rango.rest.schema", |_, _| {
        Box::pin(async {
            let doc = generate()?;
            Ok(HttpResponse::new(rango_core::utils::json::dumps_with(&doc, false, false).unwrap_or_default()).content_type("application/vnd.oai.openapi+json"))
        })
    })
}

/// `SpectacularSwaggerView(url=schema_url)`.
pub fn swagger_view(schema_url: &str) -> View {
    let title = setting("TITLE").and_then(|t| t.as_str().map(str::to_owned)).unwrap_or_default();
    page(include_str!("../templates/swagger_ui.html").replace("{{ title }}", &rango_core::utils::html::escape(&title)).replace("{{ schema_url }}", &rango_core::utils::html::escape(schema_url)))
}

/// `SpectacularRedocView(url=schema_url)`.
pub fn redoc_view(schema_url: &str) -> View {
    let title = setting("TITLE").and_then(|t| t.as_str().map(str::to_owned)).unwrap_or_default();
    page(include_str!("../templates/redoc.html").replace("{{ title }}", &rango_core::utils::html::escape(&title)).replace("{{ schema_url }}", &rango_core::utils::html::escape(schema_url)))
}

/// `manage spectacular --file schema.json`: write (or print) the OpenAPI document.
#[derive(Default)]
pub struct SpectacularCommand;

#[async_trait]
impl Command for SpectacularCommand {
    fn help(&self) -> &str {
        "Generate the OpenAPI schema for the project's API."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("file").long("file"))
    }
    async fn handle(&self, _ctx: &CommandContext, args: &clap::ArgMatches) -> std::result::Result<Option<String>, CommandError> {
        let doc = generate().map_err(|e| CommandError::new(e.to_string()))?;
        let text = serde_json::to_string_pretty(&doc).map_err(|e| CommandError::new(e.to_string()))?;
        match args.get_one::<String>("file") {
            Some(f) => {
                std::fs::write(f, text).map_err(|e| CommandError::new(e.to_string()))?;
                Ok(None)
            }
            None => Ok(Some(text)),
        }
    }
}

rango_core::register_command!("rango.rest", "spectacular", SpectacularCommand);
