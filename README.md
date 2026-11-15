<p align="center"><img src="crates/rango-views/assets/rango-logo.svg" width="128" alt="Rango"></p>

# Rango

A batteries-included web framework for Rust: ORM and migrations, templates, forms, admin, auth, REST API with
OpenAPI docs, and CORS, behind one crate.

```rust
#[derive(Model)]
pub struct Question {
    #[field(primary_key, auto)] pub id: i64,
    #[field(max_length = 200)]  pub question_text: String,
}
```

## Install

```console
cargo install rangoframework-cli        # the rango-admin tool
rango-admin startproject mysite
cd mysite && cargo run -- migrate && cargo run -- runserver
```

In an existing project, add `rangoframework = "0.1"` to `Cargo.toml`; the library is imported as `rango`
(`use rango::db::models::Model;`). On crates.io every package is prefixed `rangoframework` (`rangoframework-core`,
`rangoframework-cli`, ...); `rangoframework` re-exports them all.

Repository: <https://github.com/ZFCon/rango-framework>

- `docs/tutorial.md` — build a polls app step by step.
- `docs/guide.md` — module map, rules of thumb, design notes.
- `docs/features.md` — feature-by-feature checklist, including what is partial or not started.

**Included:** core, HTTP/URLs/views, ORM + migrations (SQLite, PostgreSQL), templates, forms, generic views, contrib
(auth, sessions, messages, contenttypes, staticfiles, sites, redirects, flatpages, sitemaps, syndication, admin), cache,
mail, files, serializers, gettext i18n and locale formats, REST framework (serializers, viewsets, routers), OpenAPI docs
(Swagger UI, Redoc) and CORS — the last three are enabled in new projects.
**Not started:** MySQL, Oracle, `contrib.gis`, `contrib.postgres`.
