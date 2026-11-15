//! Rango request handling: the middleware chain (`core.handlers`), built-in
//! middleware (`middleware.*`) and the development server.

pub mod commands;
pub mod handlers;
pub mod middleware;
pub mod servers;

pub use handlers::{GetResponse, Handler, Middleware};

#[doc(hidden)]
pub use inventory;

/// Referenced by the facade crate so the linker keeps this crate's registrations.
#[doc(hidden)]
pub fn __link() {}
