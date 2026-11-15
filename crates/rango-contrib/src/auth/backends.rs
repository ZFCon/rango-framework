//! `contrib.auth.backends`.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_db::models::meta::registry;
use rango_db::{Db, Executor, Value as DbValue};

use super::models::AbstractBaseUser;
use super::get_user_model;

/// `authenticate(**credentials)` arguments.
#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Credentials {
    pub fn new(username: &str, password: &str) -> Self {
        Self { username: Some(username.to_owned()), password: Some(password.to_owned()) }
    }
}

/// An authentication backend.
#[async_trait]
pub trait AuthBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn authenticate(&self, db: &Db, credentials: &Credentials) -> Result<Option<Box<dyn AbstractBaseUser>>>;
    async fn get_user(&self, db: &Db, pk: &DbValue) -> Result<Option<Box<dyn AbstractBaseUser>>>;
    async fn has_perm(&self, _db: &Db, _user: &dyn AbstractBaseUser, _perm: &str) -> Result<bool> {
        Ok(false)
    }
    async fn has_module_perms(&self, _db: &Db, _user: &dyn AbstractBaseUser, _app_label: &str) -> Result<bool> {
        Ok(false)
    }
}

/// `ModelBackend`: users and permissions from the database.
pub struct ModelBackend {
    /// `AllowAllUsersModelBackend` also accepts inactive users.
    pub allow_inactive: bool,
}

impl ModelBackend {
    fn name_for(&self) -> &'static str {
        if self.allow_inactive { "rango.contrib.auth.backends.AllowAllUsersModelBackend" } else { "rango.contrib.auth.backends.ModelBackend" }
    }

    /// All `app_label.codename` permissions of `user` (own and through groups).
    pub async fn get_all_permissions(&self, db: &dyn Executor, user: &dyn AbstractBaseUser) -> Result<BTreeSet<String>> {
        if !user.is_active() {
            return Ok(BTreeSet::new());
        }
        let reg = registry();
        let meta = reg.get(&user.model_label()).ok_or_else(|| Error::improperly_configured("unknown user model"))?;
        let (table, col) = (meta.db_table.clone(), format!("{}_id", meta.object_name.to_lowercase()));
        let v = db.vendor();
        let q = |s: &str| v.quote(s);
        let perm_select = "SELECT ct.app_label, p.codename FROM auth_permission p INNER JOIN rango_content_type ct ON p.content_type_id = ct.id";
        let direct = format!("{perm_select} INNER JOIN {up} up ON up.permission_id = p.id WHERE up.{c} = %s", up = q(&format!("{table}_user_permissions")), c = q(&col));
        let via_groups = format!(
            "{perm_select} INNER JOIN auth_group_permissions gp ON gp.permission_id = p.id INNER JOIN {ug} ug ON ug.group_id = gp.group_id WHERE ug.{c} = %s",
            ug = q(&format!("{table}_groups")),
            c = q(&col)
        );
        let pk = user.pk();
        let mut out = BTreeSet::new();
        for sql in [direct, via_groups] {
            for row in db.query(&sql, &[pk.clone()]).await? {
                if let [DbValue::Str(app), DbValue::Str(code)] = row.values.as_slice() {
                    out.insert(format!("{app}.{code}"));
                }
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl AuthBackend for ModelBackend {
    fn name(&self) -> &'static str {
        self.name_for()
    }

    async fn authenticate(&self, db: &Db, c: &Credentials) -> Result<Option<Box<dyn AbstractBaseUser>>> {
        let (Some(username), Some(password)) = (&c.username, &c.password) else { return Ok(None) };
        let um = get_user_model()?;
        let Some(mut user) = um.get_by_natural_key(db, username).await? else {
            // Run the default hasher once to reduce the timing difference between an existing and a nonexistent user.
            let _ = super::hashers::make_password(Some(password), None, "default");
            return Ok(None);
        };
        if user.check_password(password) && (self.allow_inactive || user.is_active()) {
            // The password may have been re-hashed with a stronger algorithm.
            user.save_fields(db, &["password"]).await?;
            return Ok(Some(user));
        }
        Ok(None)
    }

    async fn get_user(&self, db: &Db, pk: &DbValue) -> Result<Option<Box<dyn AbstractBaseUser>>> {
        Ok(get_user_model()?.get_by_pk(db, pk).await?.filter(|u| self.allow_inactive || u.is_active()))
    }

    async fn has_perm(&self, db: &Db, user: &dyn AbstractBaseUser, perm: &str) -> Result<bool> {
        Ok(self.get_all_permissions(db, user).await?.contains(perm))
    }

    async fn has_module_perms(&self, db: &Db, user: &dyn AbstractBaseUser, app_label: &str) -> Result<bool> {
        Ok(self.get_all_permissions(db, user).await?.iter().any(|p| p.split('.').next() == Some(app_label)))
    }
}

/// `get_backends()` from `AUTHENTICATION_BACKENDS`.
pub fn get_backends() -> Result<Vec<Arc<dyn AuthBackend>>> {
    settings()
        .authentication_backends
        .iter()
        .map(|n| -> Result<Arc<dyn AuthBackend>> {
            match n.as_str() {
                "rango.contrib.auth.backends.ModelBackend" => Ok(Arc::new(ModelBackend { allow_inactive: false })),
                "rango.contrib.auth.backends.AllowAllUsersModelBackend" => Ok(Arc::new(ModelBackend { allow_inactive: true })),
                other => Err(Error::improperly_configured(format!("Authentication backend '{other}' is not available."))),
            }
        })
        .collect()
}
