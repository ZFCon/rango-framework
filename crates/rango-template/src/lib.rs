//! Rango template engine: Rango Template Language (`template`).
//!
//! ```
//! use rango_template::{Engine, context};
//! let mut engine = Engine::new();
//! engine.locmem(&[("hello.html", "Hello, {{ name|upper }}!{% if n %} ({{ n }}){% endif %}")]);
//! let mut ctx = context! { "name" => "world", "n" => 3 };
//! assert_eq!(engine.render_to_string("hello.html", &mut ctx).unwrap(), "Hello, WORLD! (3)");
//! ```

pub mod cachetag;
pub mod context;
pub mod dateformat;
pub mod engine;
pub mod loader;
pub mod filters;
pub mod i18n;
pub mod nodes;
pub mod parser;
pub mod tags;
pub mod value;

pub use context::Context;
pub use loader::{TemplateResponse, get_template, render_to_string, select_template};
pub use engine::{Engine, Library, Template, TemplateError};
pub use value::{Object, TemplateObject, Value, safe, to_value};

#[doc(hidden)]
pub use inventory;
