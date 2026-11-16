# Content types

`rango::contrib::contenttypes` keeps one row per installed model, so other tables can point at "any model" by storing a content-type id.

## Enable

```rust
use rango::conf::Settings;

let settings = Settings {
    installed_apps: vec!["rango.contrib.contenttypes".into()],
    ..Settings::default()
};
```

Apps such as auth and admin depend on it. After each app's migrations, a `post_migrate` hook creates the missing rows for that app.

## The model

`ContentType` (table `rango_content_type`, unique on `(app_label, model)`):

| Field | Type | Meaning |
|---|---|---|
| `id` | `Option<i64>` | Primary key. |
| `app_label` | `String` | The owning app, lower case, for example `polls`. |
| `model` | `String` | The model name in lower case, for example `question`. |

Methods on a row:

- `name() -> String`: the model's verbose name, or the bare model name if the model is not registered.
- `natural_key() -> (String, String)`: `(app_label, model)`.
- `model_meta() -> Option<Arc<ModelMeta>>`: the registered metadata of the model, if it is installed.

## Look rows up

All lookups take the connection explicitly.

```rust
use rango::contrib::contenttypes::{get_by_natural_key, get_for_id, get_for_model};
use rango::db::Db;

let db = Db::default_db();
let ct = get_for_model(&db, "polls.Question").await?;       // case-insensitive label
let same = get_for_id(&db, ct.id.unwrap()).await?;
let by_key = get_by_natural_key(&db, "polls", "question").await?;
```

| Function | Behaviour |
|---|---|
| `get_for_model(db, "app.Model")` | Returns the row, creating it if missing. Results are cached in the process. A label without a `.` returns a `LookupError`. |
| `get_for_id(db, id)` | Fetches by primary key; `ObjectDoesNotExist` if absent. |
| `get_by_natural_key(db, app_label, model)` | Exact match on both columns; `ObjectDoesNotExist` if absent. |
| `create_contenttypes(db, app_label)` | Creates a row for every concrete, non-proxy model of the app that has none. Returns the number created. |
| `clear_cache()` | Empties the `get_for_model` cache. Call it after deleting rows. |

The cache is process-wide and keyed by lower-cased `(app, model)`. If you delete content-type rows (or reset the database) while the process runs, call `clear_cache()`, otherwise `get_for_model` can return an id that no longer exists.

## Use it for a generic reference

Store the content-type id and the object's primary key in your own model, and resolve the target with `model_meta()` and your own query.

```rust
#[derive(Clone, Debug, Default, Model)]
pub struct Note {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    pub content_type_id: i64,
    pub object_id: i64,
    #[field(max_length = 200)]
    pub text: String,
}
```

## Not supported yet

- There is no `GenericForeignKey` or `GenericRelation` field; the pattern above is manual.
- There is no `remove_stale_contenttypes` command; stale rows stay until you delete them.
- `get_for_model` takes a label string, not a model type.

## See also

- [Serialization](serialization.md)
- [Models](db/models.md)
- [Sites, flat pages and redirects](sites-flatpages-redirects.md)
