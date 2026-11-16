//! `core.exceptions` (plus `Http404`, which the reference implementation keeps in `http`).
//!
//! the exception classes become variants of a single [`Error`] enum so `?` works across
//! the whole framework; use [`Error::is`]-style matching (`matches!(e, Error::PermissionDenied(_))`)
//! where the reference implementation code would write `except PermissionDenied`.

use std::fmt;

use indexmap::IndexMap;

/// Key under which non-field validation errors are stored (`NON_FIELD_ERRORS`).
pub const NON_FIELD_ERRORS: &str = "__all__";

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The `SuspiciousOperation` family. Each kind logs to its own `rango.security.*` logger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspiciousKind {
    Operation,
    DisallowedHost,
    DisallowedRedirect,
    InvalidSessionKey,
    SuspiciousFileOperation,
    SuspiciousMultipartForm,
    SuspiciousSession,
    TooManyFieldsSent,
    TooManyFilesSent,
    RequestDataTooBig,
}

impl SuspiciousKind {
    /// Logger name, e.g. `rango.security.DisallowedHost`.
    pub fn logger(self) -> String {
        format!("rango.security.{self:?}")
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `AppRegistryNotReady`: the app registry is not loaded yet.
    #[error("{0}")]
    AppRegistryNotReady(String),
    /// `ObjectDoesNotExist` (each model's `DoesNotExist` carries its `model` label).
    #[error("{message}")]
    ObjectDoesNotExist {
        model: Option<String>,
        message: String,
    },
    /// `MultipleObjectsReturned`.
    #[error("{message}")]
    MultipleObjectsReturned {
        model: Option<String>,
        message: String,
    },
    /// `SuspiciousOperation` and subclasses; the handler turns these into a 400.
    #[error("{message}")]
    Suspicious {
        kind: SuspiciousKind,
        message: String,
    },
    /// `PermissionDenied`: handler turns it into a 403.
    #[error("{0}")]
    PermissionDenied(String),
    /// `NoReverseMatch` (`urls`).
    #[error("{0}")]
    NoReverseMatch(String),
    /// A lookup error (unknown app label, unknown model, ...).
    #[error("{0}")]
    LookupError(String),
    /// `ViewDoesNotExist`.
    #[error("{0}")]
    ViewDoesNotExist(String),
    /// `MiddlewareNotUsed`: raised by a middleware constructor to opt out.
    #[error("{0}")]
    MiddlewareNotUsed(String),
    /// `ImproperlyConfigured`.
    #[error("{0}")]
    ImproperlyConfigured(String),
    /// `FieldError`: invalid model field name in a query/lookup.
    #[error("{0}")]
    FieldError(String),
    /// `FieldDoesNotExist`.
    #[error("{0}")]
    FieldDoesNotExist(String),
    /// `BadRequest`: handler turns it into a 400.
    #[error("{0}")]
    BadRequest(String),
    /// `RequestAborted`: the client closed the connection mid-request.
    #[error("{0}")]
    RequestAborted(String),
    /// `Http404`.
    #[error("{0}")]
    Http404(String),
    /// `ValidationError`.
    #[error("{0}")]
    Validation(ValidationError),
    /// `EmptyResultSet` (query compiler: the WHERE can never match).
    #[error("EmptyResultSet")]
    EmptyResultSet,
    /// `FullResultSet` (query compiler: the WHERE always matches).
    #[error("FullResultSet")]
    FullResultSet,
    /// Anything that is not a framework exception (I/O, driver errors, ...).
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    pub fn improperly_configured(msg: impl Into<String>) -> Self {
        Self::ImproperlyConfigured(msg.into())
    }
    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self::PermissionDenied(msg.into())
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }
    pub fn http404(msg: impl Into<String>) -> Self {
        Self::Http404(msg.into())
    }
    pub fn suspicious(kind: SuspiciousKind, msg: impl Into<String>) -> Self {
        Self::Suspicious {
            kind,
            message: msg.into(),
        }
    }
    pub fn field_error(msg: impl Into<String>) -> Self {
        Self::FieldError(msg.into())
    }
}

impl From<ValidationError> for Error {
    fn from(e: ValidationError) -> Self {
        Self::Validation(e)
    }
}

/// `%(name)s` parameters of a [`ValidationError`] message, already stringified.
pub type Params = IndexMap<String, String>;

/// `core.exceptions.ValidationError`.
///
/// the single class has three shapes depending on the constructor argument; they are
/// explicit variants here. [`ValidationError::error_list`] / [`ValidationError::messages`]
/// behave like properties of the same name.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// One message with optional `code` and `params`.
    Single {
        message: String,
        code: Option<String>,
        params: Option<Params>,
    },
    /// A flat list (`error_list`); every item is a `Single`.
    List(Vec<ValidationError>),
    /// Errors by field (`error_dict`); every value is a list of `Single`.
    Dict(IndexMap<String, Vec<ValidationError>>),
}

impl ValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Single {
            message: message.into(),
            code: None,
            params: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        if let Self::Single { code: c, .. } = &mut self {
            *c = Some(code.into());
        }
        self
    }

    pub fn with_params<K: Into<String>, V: ToString>(
        mut self,
        params: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        if let Self::Single { params: p, .. } = &mut self {
            *p = Some(
                params
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.to_string()))
                    .collect(),
            );
        }
        self
    }

    /// `ValidationError([e1, e2, ...])`: nested lists/dicts are flattened.
    pub fn from_list(items: impl IntoIterator<Item = ValidationError>) -> Self {
        let mut out = Vec::new();
        for item in items {
            match item {
                Self::Dict(d) => out.extend(d.into_values().flatten()),
                other => out.extend(other.error_list()),
            }
        }
        Self::List(out)
    }

    /// `ValidationError({field: error, ...})`.
    pub fn from_dict(items: impl IntoIterator<Item = (String, ValidationError)>) -> Self {
        Self::Dict(
            items
                .into_iter()
                .map(|(f, e)| (f, e.error_list()))
                .collect(),
        )
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Single { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    /// `error_dict`, if this error is keyed by field.
    pub fn error_dict(&self) -> Option<&IndexMap<String, Vec<ValidationError>>> {
        match self {
            Self::Dict(d) => Some(d),
            _ => None,
        }
    }

    /// `error_list`: every `Single` in this error (for a `Dict`, all fields concatenated).
    pub fn error_list(&self) -> Vec<ValidationError> {
        match self {
            Self::Single { .. } => vec![self.clone()],
            Self::List(l) => l.clone(),
            Self::Dict(d) => d.values().flatten().cloned().collect(),
        }
    }

    /// `message_dict`: field -> formatted messages (empty for non-dict errors).
    pub fn message_dict(&self) -> IndexMap<String, Vec<String>> {
        match self {
            Self::Dict(d) => d
                .iter()
                .map(|(f, errs)| (f.clone(), errs.iter().map(Self::formatted).collect()))
                .collect(),
            _ => IndexMap::new(),
        }
    }

    /// `messages`: all formatted messages, flattened.
    pub fn messages(&self) -> Vec<String> {
        match self {
            Self::Dict(d) => d.values().flatten().map(Self::formatted).collect(),
            Self::List(l) => l.iter().map(Self::formatted).collect(),
            Self::Single { .. } => vec![self.formatted()],
        }
    }

    /// `update_error_dict`: merge into `error_dict`, using [`NON_FIELD_ERRORS`] for non-dict errors.
    pub fn update_error_dict(&self, error_dict: &mut IndexMap<String, Vec<ValidationError>>) {
        match self {
            Self::Dict(d) => {
                for (field, errs) in d {
                    error_dict
                        .entry(field.clone())
                        .or_default()
                        .extend(errs.iter().cloned());
                }
            }
            _ => error_dict
                .entry(NON_FIELD_ERRORS.to_owned())
                .or_default()
                .extend(self.error_list()),
        }
    }

    /// `message % params` for a `Single` (params are only applied when non-empty).
    fn formatted(&self) -> String {
        match self {
            Self::Single {
                message,
                params: Some(p),
                ..
            } if !p.is_empty() => format_percent(message, p),
            Self::Single { message, .. } => message.clone(),
            other => other.messages().join(" "),
        }
    }
}

impl fmt::Display for ValidationError {
    /// String form: the dict or list of messages.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dict(_) => {
                let items: Vec<String> = self
                    .message_dict()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", py_repr_str(k), py_repr_list(v)))
                    .collect();
                write!(f, "{{{}}}", items.join(", "))
            }
            _ => write!(f, "{}", py_repr_list(&self.messages())),
        }
    }
}

impl std::error::Error for ValidationError {}

/// `%(name)s` / `%(name)d` / `%%` substitution used by message templates.
pub fn format_percent(template: &str, params: &Params) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some('(') => {
                chars.next();
                let name: String = chars.by_ref().take_while(|&ch| ch != ')').collect();
                // conversion type character (s, d, ...)
                let conv = chars.next();
                match params.get(&name) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str(&format!("%({name}){}", conv.unwrap_or(' ')));
                    }
                }
            }
            _ => out.push('%'),
        }
    }
    out
}

/// Quoted string form.
pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `repr()` of a `list[str]`.
pub fn py_repr_list(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| py_repr_str(s))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
