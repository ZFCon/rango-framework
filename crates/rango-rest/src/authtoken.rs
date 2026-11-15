//! The `Token` model and the `obtain_auth_token` view.

use rango_contrib::auth::{self, Credentials};
use rango_core::exceptions::{Error, Result};
use rango_core::utils::crypto::get_random_string;
use rango_db::models::Model;
use rango_db::{Db, Executor, Value as DbValue};
use serde_json::json;

use crate::exceptions::ApiException;
use crate::views::{Response, ViewConfig, api_view};

/// `Token` (table `authtoken_token`): one key per user.
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "authtoken", db_table = "authtoken_token", verbose_name = "Token", verbose_name_plural = "Tokens", str = "key")]
pub struct Token {
    #[field(primary_key, max_length = 40, verbose_name = "Key")]
    pub key: String,
    /// The owner's primary key (a plain column: the derive cannot add reverse accessors to another crate's model).
    #[field(unique, verbose_name = "User")]
    pub user_id: i64,
    #[field(auto_now_add, verbose_name = "Created")]
    pub created: chrono::DateTime<chrono::Utc>,
}

/// `Token.objects.get_or_create(user=user)`.
pub async fn get_or_create_token(db: &dyn Executor, user_pk: DbValue) -> Result<Token> {
    match Token::objects().filter_kw("user_id", user_pk.clone()).get(db).await {
        Ok(t) => Ok(t),
        Err(Error::ObjectDoesNotExist { .. }) => {
            let mut t = Token { key: get_random_string(40, "0123456789abcdef"), user_id: user_pk.as_i64().unwrap_or_default(), ..Token::default() };
            t.save_with(db, true, None).await?;
            Ok(t)
        }
        Err(e) => Err(e),
    }
}

/// `obtain_auth_token`: `POST {"username", "password"}` -> `{"token": "..."}`.
pub fn obtain_auth_token() -> rango_urls::View {
    let config = ViewConfig { authentication: Some(vec![]), permissions: Some(vec![crate::auth::Permission::AllowAny]), throttles: None };
    api_view(&["POST"], config, |req, _args| async move {
        let get = |k: &str| req.data.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        let mut errors = serde_json::Map::new();
        for k in ["username", "password"] {
            if get(k).is_none_or(|v| v.is_empty()) {
                errors.insert(k.to_owned(), json!(["This field is required."]));
            }
        }
        if !errors.is_empty() {
            return Err(ApiException::validation(errors.into()).into());
        }
        let db = Db::default_db();
        let creds = Credentials::new(&get("username").unwrap_or_default(), &get("password").unwrap_or_default());
        match auth::authenticate(&db, &creds).await? {
            Some((user, _)) => Ok(Response::new(json!({"token": get_or_create_token(&db, user.pk()).await?.key}))),
            None => Err(ApiException::validation(json!({"non_field_errors": ["Unable to log in with provided credentials."]})).into()),
        }
    })
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.rest.authtoken"));
rango_db::register_initial_migration!("authtoken", [("auth", "0001_initial")]);
