# Signals

A signal lets one part of a program announce that something happened, and other parts react without the announcer
knowing about them. This page covers the `Signal` type, connecting and disconnecting receivers, sending, the
signals that Rango sends itself, and how to define your own.

## The model in a nutshell

- A `Signal<A, R>` is a value, normally a `static`. `A` is the type of the arguments a sender passes; `R` is what each
  receiver returns (`()` by default).
- A receiver is a function or async closure connected to the signal. It gets an `Arc<Dispatch<A>>` holding the
  sender label and the arguments.
- A sender is identified by a label string (a model's `"polls.Question"`, an app label, any text you choose), or by
  `None` for "no particular sender".
- `send` calls the matching receivers one after another, in the order they were connected, and collects what they
  return.

Receivers are held until you disconnect them; there are no weak references. Receivers run on the sender's task, so a
slow receiver slows the code that sent the signal.

## Reacting to a signal

Connect receivers once at start-up, for example in an app's `ready` hook (see [Applications](apps.md)):

```rust
use rango::apps::AppConfig;
use rango::db::models::signals::POST_SAVE;
use rango::dispatch::ConnectOptions;
use rango::register_app;

register_app!(
    AppConfig::new("polls").ready(|| Box::pin(async {
        POST_SAVE.connect(
            |event| async move {
                if event.args.created {
                    println!("new {} with pk {:?}", event.args.label, event.args.pk);
                }
            },
            ConnectOptions::sender("polls.Question").uid("polls.announce_new_question"),
        );
        Ok(())
    }))
);
```

`rango::dispatch` exports `Signal`, `Dispatch`, `ConnectOptions` and `ReceiverId`.

### Connecting

| Method | Receiver | Returns |
|---|---|---|
| `connect(receiver, options)` | `Fn(Arc<Dispatch<A>>) -> impl Future<Output = R> + Send + 'static`, i.e. an async closure body | `ReceiverId` |
| `connect_sync(receiver, options)` | `Fn(&Dispatch<A>) -> R`, a plain function | `ReceiverId` |

Both receivers must be `Send + Sync + 'static`. The `Dispatch<A>` a receiver sees has two public fields:

| Field | Type | Meaning |
|---|---|---|
| `sender` | `Option<String>` | The label the sender passed. |
| `args` | `A` | The arguments. |

A synchronous receiver is the shortest form when nothing needs awaiting:

```rust
use rango::dispatch::{ConnectOptions, Signal};

static GREETED: Signal<String, String> = Signal::new();

GREETED.connect_sync(|d| format!("hello {}", d.args), ConnectOptions::default());
```

### ConnectOptions

`ConnectOptions` has two public fields, both `Option<String>`, and `Default`:

| Constructor / method | Effect |
|---|---|
| `ConnectOptions::default()` | Receive the signal from any sender. |
| `ConnectOptions::sender("polls.Question")` | Receive only signals sent with exactly that sender label. |
| `.uid("some.id")` | A `dispatch_uid`: connecting again with the same uid and the same sender does not register a second receiver; the first receiver's id is returned. |

Use a `uid` for any receiver that might be connected more than once (a hook that can run again in a test process,
or the same code reached by two paths).

A receiver connected without a sender gets every send. A receiver connected for a sender gets only sends carrying
that sender. A send with sender `None` therefore reaches only the receivers connected without a sender.

### Disconnecting

| Method | Effect |
|---|---|
| `disconnect(id: ReceiverId) -> bool` | Remove the receiver with that id. `true` if one was removed. |
| `disconnect_uid(uid: &str, sender: Option<&str>) -> bool` | Remove the receiver registered with that uid and sender. |
| `has_listeners(sender: Option<&str>) -> bool` | Whether any receiver would be called for that sender. |

Receivers are identified by id or uid because closures cannot be compared.

## Sending a signal

| Method | Behaviour |
|---|---|
| `send(sender: Option<&str>, args: A) -> Vec<(ReceiverId, R)>` | Call every matching receiver in order and return their results. A panic in a receiver propagates to the caller. |
| `send_robust(sender, args) -> Vec<(ReceiverId, Result<R, String>)>` | Like `send`, but a receiver that panics yields `Err(message)` and the remaining receivers still run. |

```rust
let all = GREETED.send(Some("polls.Question"), "x".into()).await;
for (id, reply) in all {
    println!("receiver {id} said {reply}");
}

let results = GREETED.send_robust(None, "y".into()).await;
```

The list of receivers is copied before any is called, so a receiver may connect or disconnect receivers of the same
signal without deadlock; the change takes effect on the next send.

## Defining your own signal

Make the signal a `static` with `Signal::new()` (a `const fn`), define a type for its arguments, and send it where the
event occurs:

```rust
use rango::dispatch::{ConnectOptions, Signal};

#[derive(Debug)]
pub struct OrderPaid {
    pub order_id: i64,
    pub amount_cents: i64,
}

pub static ORDER_PAID: Signal<OrderPaid> = Signal::new();

pub async fn mark_paid(order_id: i64, amount_cents: i64) {
    // ... update the database ...
    ORDER_PAID
        .send(Some("shop.Order"), OrderPaid { order_id, amount_cents })
        .await;
}

// elsewhere, at start-up:
ORDER_PAID.connect(
    |d| async move { println!("paid: {:?}", d.args) },
    ConnectOptions::sender("shop.Order").uid("shop.email_receipt"),
);
```

The arguments type `A` must be `Send + Sync + 'static`, the receiver result `R` `Send + 'static`.

## Signals Rango sends

### Request signals

In `rango::core::signals`. All carry no data except the last.

| Signal | Type | When | Sender |
|---|---|---|---|
| `REQUEST_STARTED` | `Signal<()>` | A request begins. | `"rango.handlers"` |
| `REQUEST_FINISHED` | `Signal<()>` | The response has been handled. | `"rango.handlers"` |
| `GOT_REQUEST_EXCEPTION` | `Signal<String>` | A request ended in an unhandled error; the argument is the error text. | `None` |

### Model signals

In `rango::db::models::signals`. The sender is the model label, `app_label.ModelName` (`"polls.Question"`). Instances
are not passed: receivers get a snapshot as a `ModelEvent`, because typed model values cannot cross the signal
boundary.

| Signal | Type | When |
|---|---|---|
| `PRE_SAVE` | `Signal<ModelEvent>` | Before a model instance is inserted or updated. |
| `POST_SAVE` | `Signal<ModelEvent>` | After it was saved. |
| `PRE_DELETE` | `Signal<ModelEvent>` | Before an instance (or each row of a queryset delete) is removed. |
| `POST_DELETE` | `Signal<ModelEvent>` | After removal. |
| `M2M_CHANGED` | `Signal<M2mEvent>` | Around adding to, removing from or clearing a many-to-many relation. |

`ModelEvent` fields:

| Field | Type | Meaning |
|---|---|---|
| `label` | `String` | The model label. |
| `pk` | `Value` | The primary key (`rango::db::value::Value`). |
| `created` | `bool` | Whether the save inserted a new row. In `PRE_SAVE` it is the best guess made before the write. Always `false` for delete signals. |
| `update_fields` | `Option<Vec<String>>` | The fields passed to a partial save, if any. |
| `values` | `IndexMap<String, Value>` | Concrete field values by attribute name. Empty for delete signals. |
| `using` | `String` | The database alias. |

`M2mEvent` fields: `action: M2mAction`, `label: String` (the model that owns the field), `field: String`,
`pk: Value` (of the owner) and `pk_set: Vec<Value>` (the related keys involved). `M2mAction` is one of `PreAdd`,
`PostAdd`, `PreRemove`, `PostRemove`, `PreClear`, `PostClear`.

When nothing is connected for a model label the delete signals are skipped entirely, so bulk deletes stay cheap.

```rust
use rango::db::models::signals::{M2mAction, M2M_CHANGED};
use rango::dispatch::ConnectOptions;

M2M_CHANGED.connect_sync(
    |d| {
        if d.args.action == M2mAction::PostAdd {
            println!("{}.{} gained {:?}", d.args.label, d.args.field, d.args.pk_set);
        }
    },
    ConnectOptions::sender("polls.Question").uid("polls.audit_m2m"),
);
```

### Migration signals

In `rango::db::migrations::executor`. The sender is the app label and the argument is a `MigrateEvent` with
`app_label` and `using` (the database alias).

| Signal | When |
|---|---|
| `POST_MIGRATE` | Sent by `migrate` once for every app after all migrations are applied, and by `flush` after it empties the tables. Rango's own apps use it to create content types, permissions and the default site. |
| `PRE_MIGRATE` | Defined, but never sent: connecting to it has no effect today. |

Use `POST_MIGRATE` to create reference data that must exist after every migration. Make the receiver idempotent: it
runs on every `migrate` and `flush`.

## Testing with signals

Signals are process-wide statics, so connect with a `uid` and disconnect when a test must leave no trace. This is
adapted from `crates/rango/tests/manage_cli.rs`:

```rust
use rango::dispatch::{ConnectOptions, Signal};

static GREETED: Signal<String, String> = Signal::new();

#[tokio::test]
async fn ordering_and_filters() {
    GREETED.connect_sync(|d| format!("a:{}", d.args), ConnectOptions::default());
    GREETED.connect_sync(
        |d| format!("b:{}", d.args),
        ConnectOptions::sender("polls.Question").uid("once"),
    );
    // Same uid and sender: not registered a second time.
    GREETED.connect_sync(|_| String::from("dupe"), ConnectOptions::sender("polls.Question").uid("once"));

    let replies: Vec<_> = GREETED
        .send(Some("polls.Question"), "x".into())
        .await
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    assert_eq!(replies, ["a:x", "b:x"]);

    // No sender: only receivers connected without one.
    let anon: Vec<_> = GREETED.send(None, "y".into()).await.into_iter().map(|(_, r)| r).collect();
    assert_eq!(anon, ["a:y"]);
}
```

## Not supported yet

- Matching senders by anything but an exact label string; there is no sender type or sender pattern.
- `PRE_MIGRATE` is defined but not sent; there are no signals for model class preparation or request-level failures beyond those listed.
- Receivers that stop automatically when their owner is dropped; disconnect explicitly.
- Sending from synchronous code: `send` is `async`.

## See also

- [Applications](apps.md) (`ready` hook)
- [Writing management commands](commands.md)
- [Migrations](db/migrations.md)
- [Models](db/models.md)
