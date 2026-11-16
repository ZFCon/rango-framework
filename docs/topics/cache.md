# Caching

Rango ships a cache framework with three backends (local memory, file based, dummy), a low-level async API, a
per-site cache middleware, a per-view wrapper and a `{% cache %}` template tag. This page covers each layer and its
limits.

## Importing the cache API

The middleware, the view wrappers and the template tag are reachable from the `rango` facade. The low-level API
(`caches`, `cache`, `Cache`, `Timeout`, `CacheBackend`) lives in the core crate, which the facade does not re-export
at the moment. Add the core crate next to `rangoframework`, at the same version:

```toml
[dependencies]
rangoframework = "0.1"
rangoframework-core = "0.1"   # library name: rango_core
```

```rust
use rango_core::cache::{cache, caches, Timeout};        // low-level API
use rango::middleware::cache::cache_page;               // per-view caching
use rango::views::decorators::{cache_control, never_cache, vary_on_headers};
use rango::http::cache_utils::{patch_cache_control, patch_vary_headers, Cc};
```

Both crates share one settings object and one cache registry, so a cache configured through `rango::conf` is the
one `rango_core::cache::caches` returns.

## Configuring caches

The `caches` setting is a JSON object: one entry per alias. `default` is the alias used when nothing else is named.

```rust
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        caches: rango::json!({
            "default": {
                "BACKEND": "rango.core.cache.backends.locmem.LocMemCache",
                "LOCATION": "main",
                "TIMEOUT": 300,
                "KEY_PREFIX": "mysite",
                "OPTIONS": { "MAX_ENTRIES": 1000 },
            },
            "disk": {
                "BACKEND": "rango.core.cache.backends.filebased.FileBasedCache",
                "LOCATION": "/var/tmp/mysite-cache",
                "TIMEOUT": null,
            },
        }),
        ..Settings::default()
    }
}
```

The default value of the setting is one local-memory cache:
`{"default": {"BACKEND": "rango.core.cache.backends.locmem.LocMemCache"}}`.

### Keys of a cache entry

| Key | Default | Meaning |
|---|---|---|
| `BACKEND` | `rango.core.cache.backends.locmem.LocMemCache` | Dotted name of a registered backend. An unknown name fails with `ImproperlyConfigured: Could not find backend '...'`. |
| `LOCATION` | `""` | Backend specific: a store name (local memory) or a directory (file based). An array of strings is joined with `,`. |
| `TIMEOUT` | `300` | Default lifetime in seconds for `Timeout::Default`. `null` means entries never expire. |
| `KEY_PREFIX` | `""` | Prepended to every key (see [Key format and versions](#key-format-and-versions)). |
| `VERSION` | `1` | Default key version. |
| `OPTIONS` | none | Backend options: `MAX_ENTRIES` (default `300`) and `CULL_FREQUENCY` (default `3`). |

`KEY_FUNCTION` is not read from the setting. A custom key function can only be attached to a `Cache` you build
yourself with `Cache::new(backend, params).with_key_func(...)`.

## Backends

### Local memory

`rango.core.cache.backends.locmem.LocMemCache` keeps entries in the process. Caches with the same `LOCATION` share
one store, so two aliases with different locations are isolated from each other.

- Entries are lost on restart and are not shared between processes. With several server processes each one has its
  own cache.
- When a new key arrives and the store already holds `MAX_ENTRIES` entries, expired entries are dropped first. If the
  store is still full, the least recently used `len / CULL_FREQUENCY` entries are removed. `CULL_FREQUENCY: 0` empties
  the whole store instead.

### File based

`rango.core.cache.backends.filebased.FileBasedCache` writes one file per entry into the `LOCATION` directory. The
directory is created on first write. Each file is named `<md5 of the full key>.djcache`; its first line is the expiry
time as seconds since the epoch (`-1` for "never"), followed by the JSON value.

- Before each write, if the directory holds `MAX_ENTRIES` or more cache files, every `CULL_FREQUENCY`-th file (in
  name order) is deleted; `CULL_FREQUENCY: 0` deletes all of them.
- Expired files are removed when they are read.
- File I/O is synchronous. Point `LOCATION` at a local disk, and give each cache its own directory: `clear()` deletes
  every `.djcache` file in it.

### Dummy

`rango.core.cache.backends.dummy.DummyCache` implements the interface and stores nothing: `get` always returns
`None`, `add` returns `true`, `delete` returns `false`. Use it to switch caching off in development without touching
the calling code.

```rust
caches: rango::json!({ "default": { "BACKEND": "rango.core.cache.backends.dummy.DummyCache" } }),
```

## The low-level API

`caches(alias)` returns the `Arc<Cache>` for an alias; `cache()` is `caches("default")`. Both return an error when
settings are not configured or the alias does not exist (`The connection '<alias>' doesn't exist.`). The set of
caches is rebuilt whenever the `caches` setting changes, which makes `override_settings` work in tests.

Every operation is `async` and returns `rango::Result`.

```rust
use rango_core::cache::{cache, caches, Timeout};

async fn demo() -> rango::Result<()> {
    let c = cache()?;

    c.set("greeting", "hello", Timeout::Default).await?;
    let v: Option<String> = c.get("greeting").await?;
    assert_eq!(v.as_deref(), Some("hello"));

    // add() stores only when the key is absent and reports whether it stored.
    assert!(!c.add("greeting", "other", Timeout::Default).await?);

    // Counters.
    c.set("hits", &1, Timeout::Default).await?;
    assert_eq!(c.incr("hits", 4).await?, 5);

    // Compute once, reuse afterwards.
    let total: i64 = c.get_or_set("total", || 7, Timeout::Secs(60.0)).await?;
    assert_eq!(total, 7);

    // A second cache by alias.
    let disk = caches("disk")?;
    disk.set("report", &vec![1, 2, 3], Timeout::Never).await?;
    assert_eq!(disk.get::<Vec<i32>>("report").await?, Some(vec![1, 2, 3]));
    Ok(())
}
```

### Timeouts

The `timeout` argument is a `Timeout`:

| Value | Meaning |
|---|---|
| `Timeout::Default` | The cache's `TIMEOUT` setting. |
| `Timeout::Never` | The entry does not expire. |
| `Timeout::Secs(f64)` | Lifetime in seconds. `Timeout::from(60u64)` is the same as `Timeout::Secs(60.0)`. |

A timeout of zero or less means "do not cache": `set` deletes the key, `add` stores nothing and returns `false`,
`touch` deletes the key.

### Methods of `Cache`

| Method | Returns | Notes |
|---|---|---|
| `get::<T>(key)` | `Option<T>` | `None` when missing or expired. Fails when the stored JSON does not deserialize into `T`. |
| `get_versioned::<T>(key, version)` | `Option<T>` | Reads the key under another version. |
| `set(key, &value, timeout)` | `()` | Overwrites. |
| `add(key, &value, timeout)` | `bool` | Stores only when absent; `true` when it stored. |
| `get_or_set(key, default_fn, timeout)` | `T` | Returns the cached value, or calls the closure, stores the result with `add` and returns the stored value. |
| `get_many::<T>(&keys)` | `IndexMap<String, T>` | Only the keys that exist, in the order asked. All values must share the type `T`. |
| `set_many(&[(key, value)], timeout)` | `Vec<String>` | The keys that failed; always empty with the built-in backends. |
| `delete(key)` | `bool` | Whether a live entry was removed. |
| `delete_many(&keys)` | `()` | |
| `has_key(key)` | `bool` | |
| `touch(key, timeout)` | `bool` | Sets a new expiry; `false` when the key is missing. |
| `incr(key, delta)` / `decr(key, delta)` | `i64` | Error `Key '<key>' not found` when missing. The value must be an integer. |
| `clear()` | `()` | Removes every entry of the backend store, not only keys with this cache's prefix. |
| `close()` | `()` | Lets the backend release resources; a no-op for the built-in backends. |
| `make_key(key, version)` | `String` | The full key the backend sees. |

Two details that matter in practice:

- `incr` and `decr` read the value and write it back. They are not atomic across concurrent tasks, and the write uses
  the cache's default timeout, so the entry's expiry restarts on each increment.
- A full key longer than 250 characters, or containing spaces or control characters, logs a warning on the
  `rango.cache` target. The built-in backends still accept it.

### Blocking variants

`get_blocking` and `set_blocking` exist for synchronous code such as template rendering. They only work with
backends whose futures complete immediately, which is the case for the three built-in ones. With a backend that
really suspends they return the error `cache backend needs an async context`.

## Value format

Values are serialized with `serde_json` when stored and deserialized when read. Anything that implements
`Serialize` can be stored and anything that implements `DeserializeOwned` can be read back:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct Summary { count: u32, latest: String }

c.set("summary", &Summary { count: 3, latest: "today".into() }, Timeout::Default).await?;
let s: Option<Summary> = c.get("summary").await?;
```

Consequences of JSON storage: types must round-trip through JSON (map keys are strings, binary data needs an
encoding such as base64), and reading a key as a different type than it was written with is an error rather than a
silent `None`. Model instances can be cached when the struct derives `Serialize` and `Deserialize`.

## Key format and versions

The key a backend sees is `<KEY_PREFIX>:<VERSION>:<key>`:

```rust
// "default" configured with KEY_PREFIX "p" and the default VERSION 1
assert_eq!(c.make_key("a", None), "p:1:a");
assert_eq!(c.make_key("a", Some(2)), "p:2:a");
```

- `KEY_PREFIX` lets several sites share one store without collisions.
- Raising `VERSION` in the setting makes every existing entry invisible in one step, which is a cheap way to
  invalidate everything after a change in the shape of cached data.
- `get_versioned(key, version)` reads a key written under another version. There is no versioned `set`, and no
  `incr_version`/`decr_version`.

## The per-site cache

Two middleware cache whole pages. `UpdateCacheMiddleware` stores responses on the way out and
`FetchFromCacheMiddleware` answers from the cache on the way in. List the update half first and the fetch half last,
so that every other middleware runs between them:

```rust
middleware: [
    "rango.middleware.cache.UpdateCacheMiddleware",
    "rango.middleware.common.CommonMiddleware",
    // ... the rest of your middleware ...
    "rango.middleware.cache.FetchFromCacheMiddleware",
]
.map(String::from)
.to_vec(),
cache_middleware_seconds: 600,
```

`rango.middleware.cache.CacheMiddleware` does both jobs in a single entry.

| Setting | Default | Meaning |
|---|---|---|
| `cache_middleware_alias` | `"default"` | Cache alias pages are stored in. |
| `cache_middleware_seconds` | `600` | Lifetime of a cached page when the response has no `max-age` of its own. |
| `cache_middleware_key_prefix` | `""` | Extra prefix inside the page keys, for sites sharing one cache. |

What is cached and what is not:

- Only `GET` and `HEAD` requests are looked up. Other methods always reach the view.
- Only non-streaming responses with status `200` or `304` are stored.
- A response that sets cookies and has `Vary: Cookie` is not stored.
- A response whose `Cache-Control` contains `private` is not stored.
- A `max-age` in the response's `Cache-Control` overrides `cache_middleware_seconds`; `max-age=0` disables storing.
- The response to a `HEAD` request is not stored.
- The key contains the absolute URL including the query string, so `/c/` and `/c/?x=1` are separate entries, and the
  values of the request headers named in the response's `Vary` header.

Stored responses get `Expires` and `Cache-Control: max-age=<seconds>` headers, so browsers and proxies cache them
for the same period. A cached page is stored as JSON holding the status, the headers and the base64 body.

## Per-view caching

`cache_page(seconds, view)` wraps one view with the same logic as the per-site cache. It returns a `View`, so it
goes straight into `path(...)`:

```rust
use rango::http::{HttpRequest, HttpResponse};
use rango::middleware::cache::cache_page;
use rango::urls::path;
use rango::{register_urlconf, urlpatterns, Result};

async fn report(_: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new("expensive page"))
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("report/", cache_page(60 * 15, report)).name("report"),
]);
```

`cache_page` uses the alias and key prefix from `cache_middleware_alias` and `cache_middleware_key_prefix`; it has no
arguments to pick another cache or prefix per view.

## Template fragment caching

`{% cache %}` caches the output of a block. Load the `cache` library first. The arguments are the timeout in
seconds, a fragment name, and any number of values the fragment varies on:

```html
{% load cache %}
{% cache 500 sidebar request.user.username %}
  ... expensive sidebar ...
{% endcache %}
```

- A timeout of `None` caches forever. A non-numeric timeout is a render error.
- `using="alias"` as the last argument selects another cache: `{% cache 300 footer using="disk" %}`.
- The key is `template.cache.<name>.<md5 of the vary values>`. Compute it in Rust to invalidate a fragment:

```rust
use rango::template::cachetag::make_template_fragment_key;
use rango_core::cache::cache;

let key = make_template_fragment_key("sidebar", &["alice".to_owned()]);
cache()?.delete(&key).await?;
```

The tag runs during synchronous rendering and uses the blocking cache calls, so it works with the local-memory,
file-based and dummy backends only.

## Controlling downstream caches

Browsers and proxies decide what to cache from response headers. These wrappers and helpers set them.

```rust
use rango::http::cache_utils::Cc;
use rango::views::decorators::{cache_control, never_cache, vary_on_cookie, vary_on_headers};

urlpatterns![
    // Cache-Control: max-age=3600, public
    path("logo/", cache_control(&[("max_age", Cc::Value("3600".into())), ("public", Cc::Flag)], logo)),
    // Expires: <now>, Cache-Control: max-age=0, no-cache, no-store, must-revalidate, private
    path("account/", never_cache(account)),
    // Vary: User-Agent
    path("feed/", vary_on_headers(&["User-Agent"], feed)),
    // Vary: Cookie
    path("home/", vary_on_cookie(home)),
]
```

Inside a view, work on the response directly with `rango::http::cache_utils`:

| Function | Effect |
|---|---|
| `patch_cache_control(&mut response, &[(name, Cc)])` | Merges directives into `Cache-Control`. Underscores in names become hyphens. An existing smaller `max-age` wins; `public` and `private` replace each other. |
| `patch_vary_headers(&mut response, &["Cookie"])` | Adds header names to `Vary` without duplicates. |
| `has_vary_header(&response, "Cookie")` | Whether `Vary` lists the header. |
| `patch_response_headers(&mut response, Some(seconds))` | Sets `Expires` (if absent) and `max-age`. `None` uses `cache_middleware_seconds`. |
| `add_never_cache_headers(&mut response)` | What `never_cache` applies. |
| `get_max_age(&response)` | The `max-age` value, if any. |
| `get_conditional_response(&request, etag, last_modified, response)` | Applies `If-Match`, `If-None-Match`, `If-Modified-Since` and `If-Unmodified-Since`. |

For conditional requests, `rango::views::decorators` also has `etag`, `last_modified` and `condition`, and the
`rango.middleware.http.ConditionalGetMiddleware` middleware adds an `ETag` to responses and answers `304` when it
matches.

## Writing a cache backend

A backend implements `CacheBackend` and registers a factory under the name used in `BACKEND`. Keys arrive already
prefixed and versioned; values arrive as JSON bytes; `ttl` is `None` for "never expires".

```rust
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rango_core::cache::{CacheBackend, CacheParams};
use rango_core::Result;

pub struct NullCache;

#[async_trait]
impl CacheBackend for NullCache {
    async fn add(&self, _key: &str, _value: Vec<u8>, _ttl: Option<Duration>) -> Result<bool> { Ok(true) }
    async fn get(&self, _key: &str) -> Result<Option<Vec<u8>>> { Ok(None) }
    async fn set(&self, _key: &str, _value: Vec<u8>, _ttl: Option<Duration>) -> Result<()> { Ok(()) }
    async fn touch(&self, _key: &str, _ttl: Option<Duration>) -> Result<bool> { Ok(false) }
    async fn delete(&self, _key: &str) -> Result<bool> { Ok(false) }
    async fn clear(&self) -> Result<()> { Ok(()) }
}

rango_core::register_cache_backend!("mysite.cache.NullCache", |_params: &CacheParams| Ok(Arc::new(NullCache)));
```

`has_key` and `close` have default implementations. The factory receives `CacheParams` with `location`,
`default_timeout`, `key_prefix`, `version` and the raw `options` value. Registration happens at link time, so the
module containing the macro call must be part of the binary. The example needs `async-trait` in `Cargo.toml`.

## Not supported yet

- Database, Redis and Memcached backends, and the `createcachetable` command.
- `KEY_FUNCTION` in the `caches` setting.
- Writing under a non-default version, `incr_version` and `decr_version`.
- Choosing the cache alias or key prefix per `cache_page` call.
- `{% cache %}` and other synchronous callers with a backend that performs real asynchronous I/O.
- Atomic `incr`/`decr`.

## See also

- [Settings](settings.md) and the [settings reference](../ref/settings.md)
- [Middleware](http/middleware.md) and the [middleware reference](../ref/middleware.md)
- [Views](http/views.md) for view wrappers
- [Template tags reference](../ref/template-tags.md)
- [Sessions](sessions.md), which can store session data in a cache
- [Deployment](deployment.md)
