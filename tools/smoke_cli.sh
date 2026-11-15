#!/usr/bin/env bash
# S1: rango-admin startproject -> startapp -> check -> runserver -> curl.
# Usage: tools/smoke_cli.sh [workdir]   (builds into the workspace target dir to save disk)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${1:-$(mktemp -d)}"
export CARGO_TARGET_DIR="$ROOT/target"
cargo build -q -p rangoframework-cli --manifest-path "$ROOT/Cargo.toml"
ADMIN="$ROOT/target/debug/rango-admin"
rm -rf "$WORK/mysite" && mkdir -p "$WORK" && cd "$WORK"
"$ADMIN" startproject mysite --rango-path "$ROOT"
cd mysite
"$ADMIN" startapp polls
grep -q "^mod polls;" src/main.rs
sed -i 's|installed_apps: vec!\[\],|installed_apps: vec!["polls".into()],|' src/settings.rs
sed -i 's|// path("polls/", rango::urls::include("polls.urls")),|rango::urls::path("polls/", rango::urls::include("polls.urls")),|' src/urls.rs
cat > src/polls/models.rs <<'RS'
//! Models for the polls app.

use chrono::{DateTime, Utc};
use rango::db::models::{ForeignKey, Model};

#[derive(Clone, Debug, Model)]
pub struct Question {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub question_text: String,
    #[field(auto_now_add)]
    pub pub_date: DateTime<Utc>,
}

#[derive(Clone, Debug, Model)]
pub struct Choice {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    pub question: ForeignKey<Question>,
    #[field(max_length = 200)]
    pub choice_text: String,
    #[field(default = 0)]
    pub votes: i32,
}
RS
grep -q chrono Cargo.toml || printf 'chrono = "0.4"\n' >> Cargo.toml
cargo build -q
M="$CARGO_TARGET_DIR/debug/mysite"
"$M" makemigrations | grep -q "Create model Question"
test -f src/polls/migrations/m0001_initial.rs
grep -q "pub mod m0001_initial;" src/polls/migrations/mod.rs
cargo build -q
"$M" migrate | grep -q "Applying polls.0001_initial... OK"
"$M" showmigrations | grep -q "\[X\] 0001_initial"
"$M" sqlmigrate polls 0001 | grep -q 'CREATE TABLE "polls_question"'
"$M" makemigrations | grep -q "No changes detected"
test "$("$M" check)" = "System check identified no issues (0 silenced)."
"$M" help | grep -q "\[polls\]" || "$M" help | grep -q "runserver"
PORT=$((20000 + RANDOM % 20000))
"$M" runserver "127.0.0.1:$PORT" --skip-checks >/dev/null 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$PORT/polls/" && break; sleep 0.1; done
BODY="$(curl -s "http://127.0.0.1:$PORT/polls/")"
echo "$BODY" | grep -q "Hello, world. You're at the polls index."
CODE="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/nope/")"
test "$CODE" = "404"
curl -s -H 'Accept: text/html' "http://127.0.0.1:$PORT/" | grep -q "Page not found"
curl -s "http://127.0.0.1:$PORT/api/schema/" | grep -q '"openapi": "3.0.3"'
curl -s "http://127.0.0.1:$PORT/api/docs/" | grep -q "swagger-ui"
curl -s -H 'Origin: https://x.example' -D - -o /dev/null "http://127.0.0.1:$PORT/polls/" | grep -qi "^vary:.*origin"
echo "S1 OK (port $PORT)"
