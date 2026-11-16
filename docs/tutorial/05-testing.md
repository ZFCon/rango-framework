# Part 5: testing

This part writes an end-to-end test for the polls app: it starts an in-memory database, sends requests through the
real middleware stack, and checks pages, redirects, votes and the CSRF check. It continues from
[Part 4](04-forms-generic-views.md).

## Goal

`cargo test` runs one test that exercises the index, detail, vote and results pages against a fresh database.

## How tests are set up

Tests are ordinary Rust tests, placed in the `tests.rs` file the app generator created (`mod tests;` in
`src/polls/mod.rs` is compiled only for `cargo test`).

Three tools do the work:

- `rango::test::Client` is an in-process HTTP client. It builds a request, runs it through the full middleware chain
  and URL resolver, and returns a `TestResponse` with `status_code`, `content`, `text()`, `header(name)` and cookies.
  No socket is opened. It keeps cookies between calls, like a browser.
- `assert_contains`, `assert_not_contains` and `assert_redirects` check a response.
- A database that exists only for the test: an in-memory SQLite database, built by running your migrations.

Settings are installed once per process, so a test binary calls `rango::conf::configure` once. Keep the whole story
in one `#[tokio::test]` function (as the framework's own end-to-end tests do) rather than several functions that
each try to configure.

## The test

Replace the test from Part 2 with this one.

### `src/polls/tests.rs`

```rust
use rango::conf::{self, DatabaseConfig, Settings};
use rango::db::Db;
use rango::db::migrations::{MigrationLoader, Target, executor};
use rango::db::models::{ForeignKey, Model};
use rango::test::{Client, assert_contains, assert_redirects};

use super::models::{Choice, Question};

#[tokio::test]
async fn polls_end_to_end() {
    // Project settings, but with an in-memory database and the test client's host name allowed.
    conf::configure(Settings {
        allowed_hosts: vec!["testserver".into()],
        databases: [(
            "default".to_owned(),
            DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() },
        )]
        .into(),
        ..crate::settings::settings()
    })
    .unwrap();
    rango::setup().await.unwrap();

    let db = Db::default_db();
    executor::migrate(&db, &MigrationLoader::load(None).unwrap(), &Target::All).await.unwrap();

    let client = Client::new().unwrap();

    // No questions yet.
    assert_contains(&client.get("/polls/").await.unwrap(), "No polls are available.", 200);

    // One question with two choices.
    let question = Question::objects()
        .create(&db, Question { question_text: "Favourite colour?".into(), ..Question::default() })
        .await
        .unwrap();
    let qid = question.id.unwrap();
    for text in ["Red", "Blue"] {
        Choice::objects()
            .create(&db, Choice { question: ForeignKey::new(qid), choice_text: text.into(), ..Choice::default() })
            .await
            .unwrap();
    }

    // The list and the detail page show it.
    assert_contains(&client.get("/polls/").await.unwrap(), "Favourite colour?", 200);
    let detail = client.get(&format!("/polls/{qid}/")).await.unwrap();
    assert_contains(&detail, "Red", 200);
    assert_contains(&detail, "name=\"csrfmiddlewaretoken\"", 200);

    // A question that does not exist is a 404.
    assert_eq!(client.get("/polls/99/").await.unwrap().status_code, 404);

    // Voting counts the vote and redirects to the results.
    let vote_url = format!("/polls/{qid}/vote/");
    let r = client.post(&vote_url, &[("choice", "1")]).await.unwrap();
    assert_redirects(&r, &format!("/polls/{qid}/results/"), 302);
    let red = Choice::objects().filter_kw("pk", 1).get(&db).await.unwrap();
    assert_eq!(red.votes, 1);
    assert_contains(&client.get(&format!("/polls/{qid}/results/")).await.unwrap(), "Red -- 1 vote", 200);

    // Voting without a valid choice shows the form again with a message (the apostrophe is HTML-escaped).
    let r = client.post(&vote_url, &[]).await.unwrap();
    assert_contains(&r, "You didn&#x27;t select a choice.", 200);

    // With CSRF checking on, a POST without a token is refused.
    let strict = Client::new().unwrap().enforce_csrf_checks(true);
    assert_contains(&strict.post(&vote_url, &[("choice", "2")]).await.unwrap(), "CSRF verification failed", 403);
}
```

Run it:

```console
$ cargo test
```

What each part checks:

- `conf::configure(Settings { ..., ..crate::settings::settings() })` reuses your project settings and replaces only
  what the test needs. `allowed_hosts` must include `testserver`, the host name the client sends.
- `executor::migrate(...)` applies every migration compiled into the binary to the empty in-memory database. This also
  proves the migration file you generated in Part 2 matches the models.
- `Client::get` and `Client::post` return `Result<TestResponse>`; `post` takes the form fields as `(name, value)`
  pairs. By default the client does not enforce CSRF, so ordinary tests do not need a token;
  `enforce_csrf_checks(true)` switches it on, as in the last assertion.
- `assert_redirects(&response, url, status)` checks the status code and the `Location` header.
- To follow a redirect automatically, build the client with `Client::new()?.follow_redirects(true)`; the response then
  lists the hops in `redirect_chain`.

Because requests go through the real stack, a template error, a missing route name or a wrong middleware setting
fails the test, not just a bug in one function.

## Other things worth testing

- `RequestFactory` builds a bare `HttpRequest` (`get`, `post`, `generic`) when you want to call a view function
  directly without the middleware.
- `Client::default_header("Accept", "application/json")` sets a header on every request, which the API tests in
  [Part 8](08-rest-api.md) use.
- `rango::conf::override_settings(|s| s.debug = true, async { ... }).await` changes a setting for the duration of
  one future, for example to check the technical 404 page.

## Recap

- One `#[tokio::test]` configures settings, builds an in-memory database from your migrations, and drives the app
  with `Client`.
- `assert_contains` and `assert_redirects` keep checks short; the framework's own tests use the same tools.

Next: [Part 6: static files](06-static-files.md).

## See also

- [Testing](../topics/testing.md)
- [Settings](../topics/settings.md)
- [Migrations](../topics/db/migrations.md)
