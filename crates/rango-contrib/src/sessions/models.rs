use chrono::{DateTime, Utc};
use rango_db::models::Model;

/// `contrib.sessions.models.Session` (table `rango_session`).
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "sessions", db_table = "rango_session", verbose_name = "session", verbose_name_plural = "sessions")]
pub struct Session {
    #[field(primary_key, max_length = 40, verbose_name = "session key")]
    pub session_key: String,
    #[field(verbose_name = "session data")]
    pub session_data: String,
    #[field(db_index, verbose_name = "expire date")]
    pub expire_date: DateTime<Utc>,
}
