//! Rango ORM: models, QuerySets, SQLite and PostgreSQL backends (`db`).

pub mod backends;
pub mod connection;
pub mod management;
pub mod migrations;
pub mod models;
pub mod serializers;
pub mod value;

pub use connection::{Db, Executor, Transaction};
pub use value::{FromValue, Row, ToValue, Value};
pub use rango_core::Result;
pub use rango_macros::{filter, q};

#[doc(hidden)]
pub use chrono;
#[doc(hidden)]
pub use rust_decimal;
#[doc(hidden)]
pub use indexmap;

/// Current time as the ORM stores it (`auto_now`).
#[doc(hidden)]
pub fn chrono_now() -> chrono::DateTime<chrono::Utc> {
    rango_core::utils::timezone::now()
}

#[doc(hidden)]
pub use inventory;
