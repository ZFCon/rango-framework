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

## Performance

Requests per second on the same 16-core machine, same endpoints, same default middleware (security, sessions, CSRF,
auth, messages), debug off. Rango was a release build. Load: `oha -z 10s -c 64`, run on the same box.

| Endpoint | Rango | Python web framework (gunicorn, 16 workers) | NestJS (Express, 1 process) |
|---|---|---|---|
| `/hello` plain text | **31,280** | 17,046 (Rango 1.8x) | 7,860 (Rango 4.0x) |
| `/json` small JSON | **31,048** | 16,660 (1.9x) | 7,344 (4.2x) |
| `/db` 20 rows from SQLite as JSON | **21,942** | 7,229 (3.0x) | 4,946 (4.4x) |

Median latency on `/db`: Rango 2.9 ms, the Python framework 8.7 ms, NestJS 12.2 ms.

A small, single-run test, not a rigorous benchmark. NestJS ran as one Node process; a cluster of processes would score
several times higher. Real applications with templates, forms and heavier queries will see smaller gaps.

**Included:** core, HTTP/URLs/views, ORM + migrations (SQLite, PostgreSQL), templates, forms, generic views, contrib
(auth, sessions, messages, contenttypes, staticfiles, sites, redirects, flatpages, sitemaps, syndication, admin), cache,
mail, files, serializers, gettext i18n and locale formats, REST framework (serializers, viewsets, routers), OpenAPI docs
(Swagger UI, Redoc) and CORS — the last three are enabled in new projects.
**Not started:** MySQL, Oracle, `contrib.gis`, `contrib.postgres`.
