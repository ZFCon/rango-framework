//! `contrib.auth.models`: `Permission`, `Group`, `User`.

use std::any::Any;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rango_core::exceptions::{Error, Result};
use rango_db::models::{ForeignKey, ManyToMany, Model};
use rango_db::{Db, Executor, Value as DbValue};
use rango_template::Value as T;

use super::hashers::{check_password, make_password};
use crate::contenttypes::ContentType;

/// `Permission`.
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "auth", db_table = "auth_permission", ordering = ["content_type__app_label", "content_type__model", "codename"], unique_together = [["content_type", "codename"]], verbose_name = "permission", verbose_name_plural = "permissions")]
pub struct Permission {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 255, verbose_name = "name")]
    pub name: String,
    pub content_type: ForeignKey<ContentType>,
    #[field(max_length = 100, verbose_name = "codename")]
    pub codename: String,
}

/// `Group`.
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "auth", db_table = "auth_group", verbose_name = "group", verbose_name_plural = "groups", str = "name")]
pub struct Group {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 150, unique, verbose_name = "name")]
    pub name: String,
    #[field(blank, verbose_name = "permissions")]
    pub permissions: ManyToMany<Permission>,
}

/// The default `User` (`AUTH_USER_MODEL = "auth.User"`).
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "auth", db_table = "auth_user", verbose_name = "user", verbose_name_plural = "users", str = "username")]
pub struct User {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 128, verbose_name = "password")]
    pub password: String,
    #[field(null, blank, verbose_name = "last login")]
    pub last_login: Option<DateTime<Utc>>,
    #[field(default = false, verbose_name = "superuser status", help_text = "Designates that this user has all permissions without explicitly assigning them.")]
    pub is_superuser: bool,
    #[field(max_length = 150, unique, verbose_name = "username", help_text = "Required. 150 characters or fewer. Letters, digits and @/./+/-/_ only.")]
    pub username: String,
    #[field(max_length = 150, blank, verbose_name = "first name")]
    pub first_name: String,
    #[field(max_length = 150, blank, verbose_name = "last name")]
    pub last_name: String,
    #[field(max_length = 254, blank, verbose_name = "email address")]
    pub email: String,
    #[field(default = false, verbose_name = "staff status", help_text = "Designates whether the user can log into this admin site.")]
    pub is_staff: bool,
    #[field(default = true, verbose_name = "active", help_text = "Designates whether this user should be treated as active. Unselect this instead of deleting accounts.")]
    pub is_active: bool,
    #[field(auto_now_add, verbose_name = "date joined")]
    pub date_joined: DateTime<Utc>,
    #[field(blank, verbose_name = "groups")]
    pub groups: ManyToMany<Group>,
    #[field(blank, verbose_name = "user permissions")]
    pub user_permissions: ManyToMany<Permission>,
}

/// What `request.user` and the auth backends need from a user model (`AbstractBaseUser` + `PermissionsMixin`).
///
/// A custom `AUTH_USER_MODEL` implements this (and [`UserModel`]) and calls [`register_user_model`](super::register_user_model).
#[async_trait]
pub trait AbstractBaseUser: Send + Sync {
    fn pk(&self) -> DbValue;
    fn get_username(&self) -> String;
    fn password(&self) -> String;
    fn set_password_hash(&mut self, encoded: String);
    fn last_login(&self) -> Option<DateTime<Utc>>;
    fn set_last_login(&mut self, when: DateTime<Utc>);
    fn is_active(&self) -> bool {
        true
    }
    fn is_staff(&self) -> bool {
        false
    }
    fn is_superuser(&self) -> bool {
        false
    }
    fn email(&self) -> String {
        String::new()
    }
    fn get_full_name(&self) -> String {
        self.get_username()
    }
    fn get_short_name(&self) -> String {
        self.get_username()
    }
    /// `"app_label.ModelName"` of the user model.
    fn model_label(&self) -> String;
    fn as_any(&self) -> &dyn Any;
    fn to_template(&self) -> T;
    /// Persist the named fields (`save(update_fields=...)`).
    async fn save_fields(&mut self, db: &dyn Executor, fields: &[&str]) -> Result<()>;

    // ---- provided

    fn is_authenticated(&self) -> bool {
        true
    }

    /// `set_password(raw)`.
    fn set_password(&mut self, raw: &str) -> Result<()> {
        let enc = make_password(Some(raw), None, "default")?;
        self.set_password_hash(enc);
        Ok(())
    }

    fn set_unusable_password(&mut self) {
        self.set_password_hash(make_password(None, None, "default").unwrap_or_default());
    }

    fn has_usable_password(&self) -> bool {
        super::hashers::is_password_usable(Some(&self.password()))
    }

    /// `check_password(raw)` (re-hashes with the preferred hasher when needed; call [`save_fields`](Self::save_fields) after).
    fn check_password(&mut self, raw: &str) -> bool {
        let (ok, update) = check_password(Some(raw), &self.password());
        if ok && update {
            let _ = self.set_password(raw);
        }
        ok
    }

    /// `get_session_auth_hash()`: ties a session to the current password.
    fn get_session_auth_hash(&self) -> String {
        let key_salt = "rango.contrib.auth.models.AbstractBaseUser.get_session_auth_hash";
        let secret = rango_core::conf::try_settings().and_then(|s| s.secret_key().map(str::to_owned)).unwrap_or_default();
        let mac = rango_core::utils::crypto::salted_hmac(key_salt.as_bytes(), self.password().as_bytes(), secret.as_bytes(), rango_core::utils::crypto::HashAlgorithm::Sha256);
        mac.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The manager side of a user model (`UserManager`): lookups used by authentication and the session.
#[async_trait]
pub trait UserModel: Send + Sync {
    fn label(&self) -> String;
    fn username_field(&self) -> String {
        "username".into()
    }
    async fn get_by_pk(&self, db: &dyn Executor, pk: &DbValue) -> Result<Option<Box<dyn AbstractBaseUser>>>;
    async fn get_by_natural_key(&self, db: &dyn Executor, username: &str) -> Result<Option<Box<dyn AbstractBaseUser>>>;
    async fn create_user(&self, db: &dyn Executor, username: &str, email: &str, password: Option<&str>) -> Result<Box<dyn AbstractBaseUser>>;
    async fn create_superuser(&self, db: &dyn Executor, username: &str, email: &str, password: Option<&str>) -> Result<Box<dyn AbstractBaseUser>>;
}

#[async_trait]
impl AbstractBaseUser for User {
    fn pk(&self) -> DbValue {
        self.pk_value()
    }
    fn get_username(&self) -> String {
        self.username.clone()
    }
    fn password(&self) -> String {
        self.password.clone()
    }
    fn set_password_hash(&mut self, encoded: String) {
        self.password = encoded;
    }
    fn last_login(&self) -> Option<DateTime<Utc>> {
        self.last_login
    }
    fn set_last_login(&mut self, when: DateTime<Utc>) {
        self.last_login = Some(when);
    }
    fn is_active(&self) -> bool {
        self.is_active
    }
    fn is_staff(&self) -> bool {
        self.is_staff
    }
    fn is_superuser(&self) -> bool {
        self.is_superuser
    }
    fn email(&self) -> String {
        self.email.clone()
    }
    fn get_full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name).trim().to_owned()
    }
    fn get_short_name(&self) -> String {
        self.first_name.clone()
    }
    fn model_label(&self) -> String {
        "auth.User".into()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn to_template(&self) -> T {
        use rango_template::{Object, TemplateObject};
        struct U(User);
        impl TemplateObject for U {
            fn get(&self, key: &str) -> Option<T> {
                let u = &self.0;
                Some(match key {
                    "pk" | "id" => u.id.map_or(T::None, T::Int),
                    "username" => T::Str(u.username.clone()),
                    "first_name" => T::Str(u.first_name.clone()),
                    "last_name" => T::Str(u.last_name.clone()),
                    "email" => T::Str(u.email.clone()),
                    "is_staff" => T::Bool(u.is_staff),
                    "is_active" => T::Bool(u.is_active),
                    "is_superuser" => T::Bool(u.is_superuser),
                    "is_authenticated" => T::Bool(true),
                    "is_anonymous" => T::Bool(false),
                    "date_joined" => T::from(u.date_joined),
                    "last_login" => u.last_login.map_or(T::None, T::from),
                    "get_username" => T::Str(u.username.clone()),
                    "get_full_name" => T::Str(AbstractBaseUser::get_full_name(u)),
                    "get_short_name" => T::Str(u.first_name.clone()),
                    _ => return None,
                })
            }
            fn display(&self) -> T {
                T::Str(self.0.username.clone())
            }
        }
        Object::new(U(self.clone()))
    }
    async fn save_fields(&mut self, db: &dyn Executor, fields: &[&str]) -> Result<()> {
        self.save_with(db, false, Some(fields)).await
    }
}

/// `User.objects` for the default model.
pub struct DefaultUserModel;

fn boxed(u: User) -> Box<dyn AbstractBaseUser> {
    Box::new(u)
}

#[async_trait]
impl UserModel for DefaultUserModel {
    fn label(&self) -> String {
        "auth.User".into()
    }

    async fn get_by_pk(&self, db: &dyn Executor, pk: &DbValue) -> Result<Option<Box<dyn AbstractBaseUser>>> {
        match User::objects().filter_kw("pk", pk.clone()).get(db).await {
            Ok(u) => Ok(Some(boxed(u))),
            Err(Error::ObjectDoesNotExist { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn get_by_natural_key(&self, db: &dyn Executor, username: &str) -> Result<Option<Box<dyn AbstractBaseUser>>> {
        match User::objects().filter_kw("username", username).get(db).await {
            Ok(u) => Ok(Some(boxed(u))),
            Err(Error::ObjectDoesNotExist { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn create_user(&self, db: &dyn Executor, username: &str, email: &str, password: Option<&str>) -> Result<Box<dyn AbstractBaseUser>> {
        if username.is_empty() {
            return Err(Error::Other("The given username must be set".into()));
        }
        let mut u = User { username: username.to_owned(), email: email.trim().to_owned(), is_active: true, ..User::default() };
        match password {
            Some(p) => u.set_password(p)?,
            None => u.set_unusable_password(),
        }
        u.save(db).await?;
        Ok(boxed(u))
    }

    async fn create_superuser(&self, db: &dyn Executor, username: &str, email: &str, password: Option<&str>) -> Result<Box<dyn AbstractBaseUser>> {
        if username.is_empty() {
            return Err(Error::Other("The given username must be set".into()));
        }
        let mut u = User { username: username.to_owned(), email: email.trim().to_owned(), is_active: true, is_staff: true, is_superuser: true, ..User::default() };
        match password {
            Some(p) => u.set_password(p)?,
            None => u.set_unusable_password(),
        }
        u.save(db).await?;
        Ok(boxed(u))
    }
}

/// `Permission.__str__`: `app | model | name`.
pub async fn permission_str(db: &Db, p: &Permission) -> Result<String> {
    let ct = crate::contenttypes::get_for_id(db, p.content_type.id().as_i64().unwrap_or(0)).await?;
    Ok(format!("{} | {} | {}", ct.app_label, ct.name(), p.name))
}
