//! Views: what a URL pattern dispatches to.
//!
//! A view is any `async fn(HttpRequest, A1, ..., An) -> Result<HttpResponse>` where each `Ai`
//! implements [`FromArg`]. URL parameters bind **positionally**, in the order they appear in
//! the route (Rust cannot read parameter names), unlike the by-name kwargs.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_http::{HttpRequest, HttpResponse};

/// A converted URL parameter (`to_clean` result).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Str(String),
    Int(i64),
    Uuid(uuid::Uuid),
}

impl std::fmt::Display for Arg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Arg::Str(s) => f.write_str(s),
            Arg::Int(i) => write!(f, "{i}"),
            Arg::Uuid(u) => write!(f, "{u}"),
        }
    }
}

/// Conversion of a [`Arg`] into a view parameter type.
pub trait FromArg: Sized {
    fn from_arg(arg: &Arg) -> Option<Self>;
}

impl FromArg for String {
    fn from_arg(arg: &Arg) -> Option<Self> {
        Some(arg.to_string())
    }
}

impl FromArg for uuid::Uuid {
    fn from_arg(arg: &Arg) -> Option<Self> {
        match arg {
            Arg::Uuid(u) => Some(*u),
            Arg::Str(s) => s.parse().ok(),
            Arg::Int(_) => None,
        }
    }
}

macro_rules! from_arg_int {
    ($($t:ty),*) => {$(
        impl FromArg for $t {
            fn from_arg(arg: &Arg) -> Option<Self> {
                match arg {
                    Arg::Int(i) => <$t>::try_from(*i).ok(),
                    Arg::Str(s) => s.parse().ok(),
                    Arg::Uuid(_) => None,
                }
            }
        }
    )*};
}
from_arg_int!(i64, i32, i16, u64, u32, u16, usize, isize);

/// Arguments captured by the matched route.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewArgs {
    /// Unnamed regex groups.
    pub args: Vec<Arg>,
    /// Named parameters (route `<conv:name>` and `(?P<name>...)`), in route order.
    pub kwargs: IndexMap<String, Arg>,
}

impl ViewArgs {
    /// Parameters a view binds to: the named ones if any, else the unnamed ones.
    pub fn positional(&self) -> Vec<&Arg> {
        if self.kwargs.is_empty() { self.args.iter().collect() } else { self.kwargs.values().collect() }
    }
}

type Handler = dyn Fn(HttpRequest, ViewArgs) -> BoxFuture<'static, Result<HttpResponse>> + Send + Sync;

/// Per-view markers set by decorators (the reference implementation sets attributes on the function instead).
#[derive(Clone, Debug, Default)]
pub struct ViewFlags {
    pub csrf_exempt: bool,
    pub xframe_options_exempt: bool,
    pub login_required_exempt: bool,
    pub sensitive_post_parameters: Option<Vec<String>>,
    /// Free-form extra markers for third-party decorators/middleware.
    pub extra: HashMap<String, String>,
}

#[derive(Clone)]
pub struct View {
    /// Dotted-ish path used for `reverse()` by view and in error messages.
    pub name: Arc<str>,
    handler: Arc<Handler>,
    pub flags: ViewFlags,
}

impl std::fmt::Debug for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<view {}>", self.name)
    }
}

impl View {
    pub fn new(
        name: impl Into<Arc<str>>,
        handler: impl Fn(HttpRequest, ViewArgs) -> BoxFuture<'static, Result<HttpResponse>> + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), handler: Arc::new(handler), flags: ViewFlags::default() }
    }

    pub async fn call(&self, request: HttpRequest, args: ViewArgs) -> Result<HttpResponse> {
        (self.handler)(request, args).await
    }

    /// Wrap this view (what a decorator does). Flags are inherited, as the reference implementation's
    /// `wraps()` copies the function's `__dict__`.
    pub fn wrap<F>(&self, wrapper: F) -> View
    where
        F: Fn(View, HttpRequest, ViewArgs) -> BoxFuture<'static, Result<HttpResponse>> + Send + Sync + 'static,
    {
        let inner = self.clone();
        View {
            name: Arc::clone(&self.name),
            handler: Arc::new(move |req, args| wrapper(inner.clone(), req, args)),
            flags: self.flags.clone(),
        }
    }

    pub fn with_flags(mut self, f: impl FnOnce(&mut ViewFlags)) -> Self {
        f(&mut self.flags);
        self
    }
}

/// Anything usable as a view. `Marker` only disambiguates the blanket impls.
pub trait IntoView<Marker> {
    fn into_view(self) -> View;
}

pub struct ViewMarker;

impl IntoView<ViewMarker> for View {
    fn into_view(self) -> View {
        self
    }
}

fn arg_error(name: &str, i: usize) -> Error {
    Error::improperly_configured(format!("view {name}: URL parameter {} is missing or has the wrong type", i + 1))
}

macro_rules! impl_into_view {
    ($(($a:ident: $A:ident, $i:tt)),*) => {
        impl<F, Fut, $($A),*> IntoView<fn($($A),*)> for F
        where
            F: Fn(HttpRequest, $($A),*) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = Result<HttpResponse>> + Send + 'static,
            $($A: FromArg + Send + 'static),*
        {
            #[allow(unused_variables, unused_mut)]
            fn into_view(self) -> View {
                let name = std::any::type_name::<F>();
                View::new(name, move |req, args| {
                    let f = self.clone();
                    let pos = args.positional();
                    $(
                        let $a = match pos.get($i).and_then(|a| <$A as FromArg>::from_arg(a)) {
                            Some(v) => v,
                            None => return Box::pin(std::future::ready(Err(arg_error(name, $i)))),
                        };
                    )*
                    Box::pin(f(req, $($a),*))
                })
            }
        }
    };
}

impl_into_view!();
impl_into_view!((a0: A0, 0));
impl_into_view!((a0: A0, 0), (a1: A1, 1));
impl_into_view!((a0: A0, 0), (a1: A1, 1), (a2: A2, 2));
impl_into_view!((a0: A0, 0), (a1: A1, 1), (a2: A2, 2), (a3: A3, 3));
impl_into_view!((a0: A0, 0), (a1: A1, 1), (a2: A2, 2), (a3: A3, 3), (a4: A4, 4));
impl_into_view!((a0: A0, 0), (a1: A1, 1), (a2: A2, 2), (a3: A3, 3), (a4: A4, 4), (a5: A5, 5));
