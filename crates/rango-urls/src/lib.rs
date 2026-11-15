//! Rango URL routing (`urls`).
//!
//! ```ignore
//! register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
//!     path("", index).name("index"),
//!     path("<int:question_id>/", detail).name("detail"),
//! ]);
//! // ROOT_URLCONF = "mysite.urls";   reverse!("polls:detail", question_id = 1)
//! ```

pub mod converters;
pub mod resolvers;
pub mod view;

pub use converters::{PathConverter, get_converter, register_converter};
pub use resolvers::{
    Include, IncludeArg, Pattern, Resolver404, ResolverMatch, Reverse, UrlConf, UrlConfRegistration, UrlEntry,
    UrlPattern, UrlResolver, clear_url_caches, get_resolver, get_script_prefix, get_urlconf, include, is_valid_path,
    path, re_path, resolve, reverse, set_script_prefix, valid_match, with_script_prefix, with_urlconf, RequestUrlconf,
};
pub use view::{Arg, FromArg, IntoView, View, ViewArgs, ViewFlags};

#[doc(hidden)]
pub use inventory;
