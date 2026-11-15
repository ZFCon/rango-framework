//! `views.generic`: class-based views.
//!
//! the inheritance and mixins become concrete builder structs holding hooks (closures)
//! that implement the [`View`] trait; `view.as_view()` gives the value `path()` accepts.
//! Anything the builders do not cover can be done by implementing [`View`] directly.

pub mod base;
pub mod dates;
pub mod detail;
pub mod edit;
pub mod list;
mod util;

pub use base::{RedirectView, TemplateView, View};
pub use dates::{DateView, DateViewKind};
pub use detail::DetailView;
pub use edit::{CreateView, DeleteView, FormView, UpdateView};
pub use list::ListView;
pub use util::model_value;
