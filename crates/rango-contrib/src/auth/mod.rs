//! `contrib.auth`: users, groups, permissions, authentication and login state.

pub mod backends;
pub mod commands;
pub mod decorators;
pub mod forms;
pub mod hashers;
pub mod middleware;
pub mod models;
pub mod tokens;
pub mod views;
pub mod password_validation;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_core::utils::crypto::constant_time_compare;
use rango_db::{Db, Value as DbValue};
use rango_http::HttpRequest;
use rango_template::{Object, TemplateObject, Value as T};

pub use backends::{AuthBackend, Credentials, ModelBackend};
pub use models::{AbstractBaseUser, DefaultUserModel, Group, Permission, User, UserModel};

use crate::sessions::RequestSessionExt;

pub const SESSION_KEY: &str = "_auth_user_id";
pub const BACKEND_SESSION_KEY: &str = "_auth_user_backend";
pub const HASH_SESSION_KEY: &str = "_auth_user_hash";
pub const REDIRECT_FIELD_NAME: &str = "next";

static USER_MODELS: LazyLock<RwLock<HashMap<String, Arc<dyn UserModel>>>> = LazyLock::new(|| RwLock::new(HashMap::from([("auth.User".to_owned(), Arc::new(DefaultUserModel) as Arc<dyn UserModel>)])));

/// Register a custom `AUTH_USER_MODEL` implementation.
pub fn register_user_model(m: Arc<dyn UserModel>) {
    USER_MODELS.write().expect("user models").insert(m.label(), m);
}

/// `get_user_model()`.
pub fn get_user_model() -> Result<Arc<dyn UserModel>> {
    let label = settings().auth_user_model.clone();
    USER_MODELS.read().expect("user models").get(&label).cloned().ok_or_else(|| Error::improperly_configured(format!("AUTH_USER_MODEL refers to model '{label}' that has not been installed")))
}

/// A user handle shared by the request (`request.user` when authenticated).
pub type UserHandle = Arc<tokio::sync::Mutex<Box<dyn AbstractBaseUser>>>;

/// `request.user`: an authenticated user or `AnonymousUser`.
#[derive(Clone, Default)]
pub struct CurrentUser(pub Option<UserHandle>);

impl CurrentUser {
    pub fn is_authenticated(&self) -> bool {
        self.0.is_some()
    }
    pub fn is_anonymous(&self) -> bool {
        self.0.is_none()
    }
    /// A snapshot of the user fields.
    pub async fn snapshot<R>(&self, f: impl FnOnce(&dyn AbstractBaseUser) -> R) -> Option<R> {
        match &self.0 {
            Some(u) => Some(f(&**u.lock().await)),
            None => None,
        }
    }
    pub async fn pk(&self) -> Option<DbValue> {
        self.snapshot(|u| u.pk()).await
    }
    pub async fn username(&self) -> String {
        self.snapshot(|u| u.get_username()).await.unwrap_or_default()
    }
    pub async fn is_staff(&self) -> bool {
        self.snapshot(|u| u.is_staff() && u.is_active()).await.unwrap_or(false)
    }
    pub async fn is_superuser(&self) -> bool {
        self.snapshot(|u| u.is_superuser()).await.unwrap_or(false)
    }
    /// `user.has_perm("app_label.codename")` through the configured backends.
    pub async fn has_perm(&self, db: &Db, perm: &str) -> Result<bool> {
        let Some(u) = &self.0 else { return Ok(false) };
        let u = u.lock().await;
        if u.is_active() && u.is_superuser() {
            return Ok(true);
        }
        for b in backends::get_backends()? {
            if b.has_perm(db, &**u, perm).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub async fn has_perms(&self, db: &Db, perms: &[&str]) -> Result<bool> {
        for p in perms {
            if !self.has_perm(db, p).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub async fn has_module_perms(&self, db: &Db, app_label: &str) -> Result<bool> {
        let Some(u) = &self.0 else { return Ok(false) };
        let u = u.lock().await;
        if u.is_active() && u.is_superuser() {
            return Ok(true);
        }
        for b in backends::get_backends()? {
            if b.has_module_perms(db, &**u, app_label).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Template value for `{{ user }}` (a snapshot; anonymous users are falsy-authenticated).
    pub fn to_template(&self) -> T {
        match &self.0 {
            Some(u) => match u.try_lock() {
                Ok(g) => g.to_template(),
                Err(_) => T::None,
            },
            None => Object::new(Anonymous),
        }
    }
}

struct Anonymous;

impl TemplateObject for Anonymous {
    fn get(&self, key: &str) -> Option<T> {
        Some(match key {
            "is_authenticated" | "is_staff" | "is_superuser" | "is_active" => T::Bool(false),
            "is_anonymous" => T::Bool(true),
            "username" | "get_username" => T::Str(String::new()),
            "pk" | "id" => T::None,
            _ => return None,
        })
    }
    fn display(&self) -> T {
        T::Str("AnonymousUser".into())
    }
}

/// `request.user`.
pub trait RequestUserExt {
    fn user(&self) -> CurrentUser;
    fn set_user(&self, user: CurrentUser);
}

impl RequestUserExt for HttpRequest {
    fn user(&self) -> CurrentUser {
        self.ext::<CurrentUser>().map_or_else(CurrentUser::default, |u| (*u).clone())
    }
    fn set_user(&self, user: CurrentUser) {
        self.set_ext(user);
    }
}

/// `authenticate(request, **credentials)`: the first backend that accepts the credentials.
pub async fn authenticate(db: &Db, credentials: &Credentials) -> Result<Option<(Box<dyn AbstractBaseUser>, String)>> {
    for b in backends::get_backends()? {
        if let Some(u) = b.authenticate(db, credentials).await? {
            return Ok(Some((u, b.name().to_owned())));
        }
    }
    Ok(None)
}

/// `login(request, user, backend=None)`: remember the user in the session.
pub async fn login(request: &HttpRequest, db: &Db, mut user: Box<dyn AbstractBaseUser>, backend: Option<&str>) -> Result<()> {
    let session = request.session();
    let hash = user.get_session_auth_hash();
    let pk = user.pk();
    let same_user = session.get(SESSION_KEY).is_some_and(|v| value_matches(&v, &pk));
    if session.contains(SESSION_KEY) {
        if !same_user || session.get(HASH_SESSION_KEY).and_then(|h| h.as_str().map(str::to_owned)).as_deref() != Some(hash.as_str()) {
            session.flush().await?;
        }
    } else {
        session.cycle_key().await?;
    }
    let backend = match backend {
        Some(b) => b.to_owned(),
        None => backends::get_backends()?.first().map(|b| b.name().to_owned()).ok_or_else(|| Error::Other("You have multiple authentication backends configured and therefore must provide the `backend` argument or set the `backend` attribute on the user.".into()))?,
    };
    session.set(SESSION_KEY, db_to_json(&pk));
    session.set(BACKEND_SESSION_KEY, backend);
    session.set(HASH_SESSION_KEY, hash);
    rango_middleware::middleware::csrf::rotate_token(request);
    user.set_last_login(rango_core::utils::timezone::now());
    user.save_fields(db, &["last_login"]).await?;
    request.set_user(CurrentUser(Some(Arc::new(tokio::sync::Mutex::new(user)))));
    Ok(())
}

/// `logout(request)`: flush the session.
pub async fn logout(request: &HttpRequest) -> Result<()> {
    request.session().flush().await?;
    request.set_user(CurrentUser(None));
    Ok(())
}

/// `update_session_auth_hash(request, user)`: keep the session valid after a password change.
pub async fn update_session_auth_hash(request: &HttpRequest) -> Result<()> {
    let session = request.session();
    session.cycle_key().await?;
    if let Some(u) = request.user().0 {
        session.set(HASH_SESSION_KEY, u.lock().await.get_session_auth_hash());
    }
    Ok(())
}

/// `get_user(request)`: the user stored in the session, if its auth hash still matches.
pub async fn get_user(request: &HttpRequest, db: &Db) -> Result<CurrentUser> {
    let Some(session) = request.try_session() else { return Ok(CurrentUser(None)) };
    let Some(pk) = session.get(SESSION_KEY) else { return Ok(CurrentUser(None)) };
    let backend_name = session.get(BACKEND_SESSION_KEY).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    let Some(backend) = backends::get_backends()?.into_iter().find(|b| b.name() == backend_name) else { return Ok(CurrentUser(None)) };
    let Some(user) = backend.get_user(db, &json_to_db(&pk)).await? else { return Ok(CurrentUser(None)) };
    let expected = user.get_session_auth_hash();
    let stored = session.get(HASH_SESSION_KEY).and_then(|v| v.as_str().map(str::to_owned));
    match stored {
        Some(s) if constant_time_compare(&s, &expected) => Ok(CurrentUser(Some(Arc::new(tokio::sync::Mutex::new(user))))),
        _ => {
            session.flush().await?;
            Ok(CurrentUser(None))
        }
    }
}

fn db_to_json(v: &DbValue) -> serde_json::Value {
    match v {
        DbValue::Int(i) => (*i).into(),
        DbValue::Str(s) => s.clone().into(),
        DbValue::Uuid(u) => u.to_string().into(),
        other => format!("{other:?}").into(),
    }
}

fn json_to_db(v: &serde_json::Value) -> DbValue {
    match v {
        serde_json::Value::Number(n) => n.as_i64().map_or(DbValue::Null, DbValue::Int),
        serde_json::Value::String(s) => DbValue::Str(s.clone()),
        _ => DbValue::Null,
    }
}

fn value_matches(j: &serde_json::Value, pk: &DbValue) -> bool {
    db_to_json(pk) == *j
}

/// `redirect_to_login(next, login_url, redirect_field_name)`.
pub fn redirect_to_login(next: &str, login_url: Option<&str>, field: &str) -> Result<rango_http::HttpResponse> {
    let login = login_url.map_or_else(|| settings().login_url.clone(), str::to_owned);
    let resolved = rango_views::shortcuts::resolve_url(&login, rango_urls::Reverse::new())?;
    let mut parts = rango_core::utils::http::urlsplit(&resolved).map_err(|e| Error::Other(e.into()))?;
    let q = parts.query.clone();
    let mut qd: Vec<String> = if q.is_empty() { vec![] } else { vec![q] };
    qd.push(format!("{field}={}", rango_core::utils::http::quote(next, "/")));
    parts.query = qd.join("&");
    rango_http::HttpResponse::redirect(&parts.unsplit())
}

/// `context_processors.auth`: `user` (and `perms` limited to the superuser shortcut).
fn auth_context_processor(request: &HttpRequest) -> indexmap::IndexMap<String, T> {
    let mut m = indexmap::IndexMap::new();
    m.insert("user".to_owned(), request.user().to_template());
    m
}

rango_template::register_context_processor!("rango.contrib.auth.context_processors.auth", auth_context_processor);
fn ready() -> futures_util::future::BoxFuture<'static, Result<()>> {
    Box::pin(async {
        rango_db::migrations::executor::POST_MIGRATE.connect(
            |d| async move {
                if let Err(e) = crate::admin::create_permissions(&Db::using(&d.args.using), &d.args.app_label).await {
                    tracing::warn!(target: "rango.contrib.auth", "create_permissions failed: {e}");
                }
            },
            rango_core::dispatch::ConnectOptions::default().uid("rango.contrib.auth.management.create_permissions"),
        );
        Ok(())
    })
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.auth").ready(ready));
rango_db::register_initial_migration!("auth", [("contenttypes", "0001_initial")]);
