# Messages

The messages framework stores a short notice during one request and shows it on the next, typically after a redirect
("Profile saved"). `rango::contrib::messages` provides the middleware, the level helpers, the template context
processor and three storage backends.

## Enabling messages

Messages need the app, the middleware and, to use `{{ messages }}` in templates, the context processor. Session
storage and the default fallback storage also need the sessions middleware first.

```rust
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        installed_apps: [
            "rango.contrib.contenttypes",
            "rango.contrib.sessions",
            "rango.contrib.messages",
        ]
        .map(String::from)
        .to_vec(),
        middleware: [
            "rango.contrib.sessions.middleware.SessionMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.contrib.messages.middleware.MessageMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        templates: vec![rango::json!({
            "BACKEND": "rango.template.backends.rango.RangoTemplates",
            "DIRS": ["templates"],
            "APP_DIRS": true,
            "OPTIONS": {
                "context_processors": [
                    "rango.template.context_processors.request",
                    "rango.contrib.messages.context_processors.messages",
                ],
            },
        })],
        ..Settings::default()
    }
}
```

## Adding messages

One function per level, each taking the request and the text. They return `Result<()>` and fail when
`MessageMiddleware` is not installed.

```rust
use rango::contrib::messages;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::redirect;
use rango::Result;

pub async fn save_profile(req: HttpRequest) -> Result<HttpResponse> {
    // ... validate and save ...
    messages::success(&req, "Profile saved.")?;
    redirect("/profile/")
}
```

The functions are `messages::debug`, `info`, `success`, `warning` and `error`. For extra CSS tags or to ignore a
missing middleware, call `add_message` directly:

```rust
use rango::contrib::messages::{self, WARNING};

fn notify(req: &rango::http::HttpRequest) -> rango::Result<()> {
    // request, level, text, extra tags, fail_silently
    messages::add_message(req, WARNING, "Disk almost full", "sticky", true)
}
```

Empty messages are ignored.

## Levels and tags

| Constant | Value | Tag |
| --- | --- | --- |
| `DEBUG` | 10 | `debug` |
| `INFO` | 20 | `info` |
| `SUCCESS` | 25 | `success` |
| `WARNING` | 30 | `warning` |
| `ERROR` | 40 | `error` |

Messages below the effective level are dropped when added. The effective level is, in order: the level set on this
request with `messages::set_level(&req, level)` (returns `false` without the middleware), the `message_level`
setting, then `INFO`. So by default `debug` messages are discarded. `messages::get_level(&req)` returns the current
value.

A message's `tags` string is its extra tags followed by its level tag, separated by a space. Override or add level
tags with `message_tags`, a JSON object mapping the level number (as a string) to a tag:

```rust
use rango::conf::Settings;

let settings = Settings {
    message_tags: Some(rango::json!({ "40": "danger", "25": "ok" })),
    ..Settings::default()
};
```

Levels that have no tag and no override produce an empty tag, so you can define your own levels as plain integers.

## Reading messages

In a view, `get_messages(&req)` returns `Option<Messages>`. `take()` returns the messages and marks them as used, so
they are not stored again:

```rust
use rango::contrib::messages;

fn pending(req: &rango::http::HttpRequest) -> Vec<String> {
    messages::get_messages(req)
        .map(|m| m.take().into_iter().map(|x| format!("{}: {}", x.level_tag(), x.message)).collect())
        .unwrap_or_default()
}
```

`Message` has public fields `level`, `message`, `extra_tags` and `safe`, and methods `level_tag()` and `tags()`.
`Messages` also has `len()`, `is_empty()`, `level()`, `set_level()` and `add(level, text, extra_tags)`.

### In templates

With the context processor installed, templates get `messages` and `DEFAULT_MESSAGE_LEVELS` (a map with `DEBUG`,
`INFO`, `SUCCESS`, `WARNING`, `ERROR`):

```html
{% if messages %}
<ul class="messages">
  {% for message in messages %}
  <li class="{{ message.tags }}">{{ message }}</li>
  {% endfor %}
</ul>
{% endif %}
```

Each message exposes `message`, `level`, `tags`, `level_tag` and `extra_tags`. Printing `{{ message }}` shows the
text, escaped like any other value. Looping over `messages` consumes them. Compare levels with the constants:
`{% if message.level == DEFAULT_MESSAGE_LEVELS.ERROR %}`.

## Storage backends

`message_storage` selects where unread messages wait between requests. Selection is by the end of the dotted name:

| `message_storage` | Behaviour |
| --- | --- |
| `rango.contrib.messages.storage.fallback.FallbackStorage` (default) | Cookie first; messages that do not fit go to the session. |
| `rango.contrib.messages.storage.cookie.CookieStorage` | Signed cookie `messages` only. Messages beyond the size limit are dropped. |
| `rango.contrib.messages.storage.session.SessionStorage` | Session only; requires the sessions middleware. |

Details:

- The cookie holds JSON signed with `secret_key`. The size limit is 2048 bytes of signed value.
- When the fallback overflows, the cookie ends with an internal marker so the next request also reads the session.
- The cookie uses `session_cookie_domain`, `session_cookie_secure`, `session_cookie_httponly` and
  `session_cookie_samesite`; the session copy is stored under the session key `_messages`.
- The cookie is deleted once there is nothing left to store.
- Any value not ending in `SessionStorage` or `CookieStorage` behaves like the fallback storage; no value is rejected.

## Messages in the admin

The admin adds a success message after each add, change or delete and renders them above the page content
(see [admin](admin.md)).

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `message_level` | `None` (acts as `INFO`) | Minimum level that is stored. |
| `message_tags` | `None` | Map of level number to tag overrides. |
| `message_storage` | `...fallback.FallbackStorage` | Backend, see above. |

## Not supported yet

- No view mixins for success messages.
- No helpers beyond the five level functions, `add_message`, `get_level` and `set_level`.
- A message cannot be marked safe (unescaped) through the public helpers; text is always escaped in templates.

## See also

- [Sessions](sessions.md)
- [Admin](admin.md)
- [Settings](settings.md)
- [Templates language](templates/language.md)
