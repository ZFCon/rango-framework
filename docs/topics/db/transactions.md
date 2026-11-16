# Transactions

A transaction makes a group of statements succeed or fail together. This page covers `Db::atomic`, nested
transactions (savepoints), `on_commit` callbacks, how the `Executor` trait lets the same code run inside or outside a
transaction, and the pitfalls of mixing connections.

```rust
use rango::db::models::{F, Model};
use rango::db::{Db, Executor, Transaction};
use rango::Result;
```

## Autocommit by default

`Db` is a handle to a database alias. Every statement you run through it takes a connection from the pool, runs, and
returns the connection, so each statement is its own committed transaction. Two statements issued through a `Db` can
be interleaved with other requests' statements, and if the second fails the first stays.

```rust
let db = Db::default_db();
Account::objects().filter_kw("id", 1).update(&db, vec![("balance", (F("balance") - 50).into())]).await?;
// If this fails, the first update is already committed.
Account::objects().filter_kw("id", 2).update(&db, vec![("balance", (F("balance") + 50).into())]).await?;
```

Single statements that must be indivisible (an increment with `F`, a bulk `update`, a `bulk_create`) do not need a
transaction. A read followed by a write, or several writes, do.

## `Db::atomic`

`atomic` opens a transaction on one dedicated connection, runs your closure, commits when the closure returns `Ok`
and rolls back when it returns `Err`. The closure receives a `&Transaction`; pass that wherever a `Db` is accepted.

```rust
let moved = db
    .atomic(|tx| {
        Box::pin(async move {
            let from = Account::objects().filter_kw("id", 1).get(tx).await?;
            if from.balance < 50 {
                return Err(rango::Error::Other("insufficient funds".into()));
            }
            Account::objects().filter_kw("id", 1).update(tx, vec![("balance", (F("balance") - 50).into())]).await?;
            Account::objects().filter_kw("id", 2).update(tx, vec![("balance", (F("balance") + 50).into())]).await?;
            Ok(50)
        })
    })
    .await?;
```

The signature is:

```rust
pub async fn atomic<T, F>(&self, f: F) -> Result<T>
where
    F: for<'a> FnOnce(&'a Transaction) -> BoxFuture<'a, Result<T>>;
```

- The closure returns a boxed future. `Box::pin(async move { ... })` is enough; `.boxed()` from
  `futures_util::FutureExt` works too.
- The error type is `rango::Error`, so `?` works on every ORM call. Anything that returns `Err` rolls the transaction
  back, and the same error is returned from `atomic`.
- The value in `Ok(...)` is returned from `atomic` after the commit.
- If `COMMIT` itself fails (for example a deferred foreign key constraint is violated), `atomic` returns that error.
  Foreign keys are created `DEFERRABLE INITIALLY DEFERRED`, so a dangling reference is reported at commit, not at the
  statement that caused it.
- `rollback` failures are ignored; the original error is what you get back.

Use `tx` for **every** query inside the closure. A query that uses `db` goes to a different connection and is neither
part of the transaction nor rolled back with it.

### Isolation and locking

Rango does not change the isolation level: you get the database's default (PostgreSQL: read committed; SQLite:
serialized writes, a deferred `BEGIN`). To lock rows for a read-modify-write on PostgreSQL, use `select_for_update`
inside the transaction:

```rust
db.atomic(|tx| {
    Box::pin(async move {
        let acct = Account::objects().filter_kw("id", 1).select_for_update("").get(tx).await?;
        // ... compute and save with `tx` ...
        Ok(())
    })
})
.await?;
```

The argument is appended to `FOR UPDATE`: `""` for a plain lock, `"NOWAIT"` or `"SKIP LOCKED"` for those modes. On
SQLite `select_for_update` has no effect.

## Nested transactions

`Transaction::atomic` takes the same closure and opens a **savepoint**. If the inner closure returns `Err`, the
changes since the savepoint are rolled back and the error is returned to the caller; the outer transaction is still
open. If the caller propagates the error with `?`, the outer closure fails too and the whole transaction rolls back.

```rust
db.atomic(|tx| {
    Box::pin(async move {
        Author::objects().create(tx, Author { name: "kept".into(), ..Author::default() }).await?;

        let inner: Result<()> = tx
            .atomic(|inner| {
                Box::pin(async move {
                    Author::objects().create(inner, Author { name: "dropped".into(), ..Author::default() }).await?;
                    Err(rango::Error::Other("boom".into()))
                })
            })
            .await;

        // Only "dropped" was rolled back. Carry on, or return `inner` to abort everything.
        let _ = inner;
        Ok(())
    })
})
.await?;
```

Savepoints are named `s1`, `s2`, ... by depth. They are meant for "try this part, fall back if it fails" logic and for
making a function usable both on its own and inside a caller's transaction.

## Code that works inside or outside a transaction

Both `Db` and `Transaction` implement the `Executor` trait, and every queryset method takes `&dyn Executor`. Write
helpers against the trait and the caller decides whether a transaction is involved:

```rust
async fn record_vote(db: &dyn Executor, choice_id: i64) -> Result<()> {
    Choice::objects()
        .filter_kw("pk", choice_id)
        .update(db, vec![("votes", (F("votes") + 1).into())])
        .await?;
    Ok(())
}

record_vote(&db, 3).await?;                                   // its own statement

db.atomic(|tx| Box::pin(async move {                           // part of a bigger unit
    record_vote(tx, 3).await?;
    record_vote(tx, 4).await
})).await?;
```

The trait:

| Method | Meaning |
|---|---|
| `alias() -> &str` | Database alias the executor belongs to. |
| `vendor() -> Vendor` | `Vendor::Sqlite` or `Vendor::Postgresql`. |
| `query(sql, &[Value]) -> Result<Vec<Row>>` | Run a statement and read rows. |
| `execute(sql, &[Value]) -> Result<u64>` | Run a statement, return rows affected. |
| `insert(sql, &[Value], returning) -> Result<Option<i64>>` | Insert and return the generated key. |
| `execute_batch(sql) -> Result<()>` | Run several `;`-separated statements without parameters. |

Placeholders are `%s` on both backends; see [Making queries](queries.md#raw-sql).

## Running code after commit

`Transaction::on_commit(f)` registers a plain `FnOnce() + Send + 'static` callback. Callbacks run, in registration
order, after the **outermost** transaction commits successfully, and are dropped if it rolls back.

```rust
db.atomic(|tx| {
    Box::pin(async move {
        let order = Order::objects().create(tx, new_order).await?;
        let id = order.id;
        tx.on_commit(move || println!("order {id:?} committed"));
        Ok(order)
    })
})
.await?;
```

The callback is synchronous. To start async work from it, spawn a task (`tokio::spawn`). Callbacks registered inside a
savepoint that is later rolled back still run if the outer transaction commits: they are only discarded when the
whole transaction rolls back.

## Signals inside transactions

Model signals (`PRE_SAVE`, `POST_SAVE`, `PRE_DELETE`, `POST_DELETE`, `M2M_CHANGED`) are sent at the moment of the
operation, not at commit. A receiver that reacts to `POST_SAVE` runs before the data is durable. Use `on_commit` for
side effects that must only happen once the data is committed (sending mail, queueing a job).

## What runs in a transaction for you

- `migrate` runs each migration, and the row that records it, in one transaction. A migration that fails leaves the
  database as it was. See [Migrations](migrations.md).
- `flush --noinput` empties the tables in one transaction.
- `delete` on an instance or a queryset issues its updates and deletes as separate statements. Wrap the call in
  `atomic` if a half-finished cascade is unacceptable.
- `save`, `create`, `bulk_create`, `get_or_create` and `update_or_create` are not transactional; `get_or_create` is
  a lookup followed by an insert, so run it in `atomic` and put a unique constraint on the lookup columns when two
  requests can race.

## Pitfalls

- **Using `db` inside the closure.** It runs on another connection. On a file SQLite database the second connection
  can wait for the transaction's write lock and fail with `database is locked` after five seconds. On an in-memory
  SQLite database the pool has exactly one connection, which the transaction holds, so the statement waits forever.
  Always use the `tx` you were given.
- **Cancelling an `atomic` future.** A transaction that is dropped without finishing (a `tokio::time::timeout`
  expiring, the task being aborted, a panic) is not rolled back by a destructor: the connection goes back to the pool
  with the transaction still open. Let the closure run to completion, and enforce time limits inside it (for example
  with `statement_timeout` in PostgreSQL).
- **Holding a transaction across slow work.** The connection is reserved for the whole closure. The pool is small
  (8 connections for a SQLite file, 10 for PostgreSQL by default), so avoid network calls inside a transaction.
- **Expecting a request-wide transaction.** The `atomic_requests` database setting is accepted but not read yet;
  wrap the view's work in `atomic` yourself.

## Not supported yet

- Isolation level selection, read-only transactions.
- Manual savepoint control (`savepoint`, `rollback_to`); use nested `atomic`.
- `atomic_requests` and `autocommit` database settings.
- Automatic rollback when an `atomic` future is dropped.

## See also

- [Making queries](queries.md)
- [Relations](relations.md)
- [Migrations](migrations.md)
- [Databases](databases.md)
- [Signals](../signals.md)
