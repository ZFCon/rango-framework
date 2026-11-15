//! `core.validators`.
//!
//! A validator is anything implementing [`Validator<T>`]; plain functions and closures
//! `Fn(&T) -> Result<(), ValidationError>` qualify too, so the function validators
//! (`validate_slug`, `validate_email`, ...) are ordinary functions here.
//!
//! Default messages are produced through `gettext` *when an error is raised*, so the active
//! language at that moment is honoured.

use std::fmt::{self, Debug, Display};
use std::sync::LazyLock;

use fancy_regex::Regex;
use rust_decimal::Decimal;

use crate::exceptions::{Error, Result, ValidationError};
use crate::utils::http::{MAX_URL_LENGTH, urlsplit};
use crate::utils::ipv6::is_valid_ipv6_address;
use crate::utils::translation::{gettext, ngettext};

/// A callable that checks a value and returns a [`ValidationError`] when it is not acceptable.
pub trait Validator<T: ?Sized>: Send + Sync + Debug {
    fn validate(&self, value: &T) -> std::result::Result<(), ValidationError>;
}

/// Wrapper to use a closure as a [`Validator`]: `FnValidator(|v: &str| ...)`.
pub struct FnValidator<F>(pub F);

impl<F> Debug for FnValidator<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FnValidator(..)")
    }
}

impl<T: ?Sized, F> Validator<T> for FnValidator<F>
where
    F: Fn(&T) -> std::result::Result<(), ValidationError> + Send + Sync,
{
    fn validate(&self, value: &T) -> std::result::Result<(), ValidationError> {
        (self.0)(value)
    }
}

type VResult = std::result::Result<(), ValidationError>;

fn invalid(message: String, code: &str, value: &str) -> ValidationError {
    ValidationError::new(message)
        .with_code(code)
        .with_params([("value", value)])
}

/// Compile a Python-flavoured pattern: `\Z` (end of string) becomes `\z`.
fn compile(pattern: &str) -> Result<Regex> {
    Regex::new(&pattern.replace("\\Z", "\\z")).map_err(|e| {
        Error::improperly_configured(format!("invalid validator regex {pattern:?}: {e}"))
    })
}

/// `RegexValidator`.
#[derive(Debug, Clone)]
pub struct RegexValidator {
    regex: Regex,
    message: Option<String>,
    default_message: fn() -> String,
    code: String,
    inverse_match: bool,
}

impl RegexValidator {
    /// Use `(?i)` in `pattern` for `re.IGNORECASE`; `\Z` is accepted as in Python.
    pub fn new(pattern: &str) -> Result<Self> {
        Ok(Self {
            regex: compile(pattern)?,
            message: None,
            default_message: || gettext("Enter a valid value."),
            code: "invalid".into(),
            inverse_match: false,
        })
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_default_message(mut self, default: fn() -> String) -> Self {
        self.default_message = default;
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }

    pub fn inverse_match(mut self, inverse: bool) -> Self {
        self.inverse_match = inverse;
        self
    }

    fn message(&self) -> String {
        self.message
            .clone()
            .unwrap_or_else(|| (self.default_message)())
    }
}

impl PartialEq for RegexValidator {
    fn eq(&self, other: &Self) -> bool {
        self.regex.as_str() == other.regex.as_str()
            && self.message() == other.message()
            && self.code == other.code
            && self.inverse_match == other.inverse_match
    }
}

impl Validator<str> for RegexValidator {
    fn validate(&self, value: &str) -> VResult {
        let matched = self.regex.is_match(value).unwrap_or(false);
        if matched == self.inverse_match {
            return Err(invalid(self.message(), &self.code, value));
        }
        Ok(())
    }
}

const UL: &str = "\\u00a1-\\uffff";

fn hostname_re() -> String {
    format!("[a-z{UL}0-9](?:[a-z{UL}0-9-]{{0,61}}[a-z{UL}0-9])?")
}

fn domain_re() -> String {
    format!("(?:\\.(?!-)[a-z{UL}0-9-]{{1,63}}(?<!-))*")
}

fn tld_no_fqdn_re() -> String {
    format!("\\.(?!-)(?:[a-z{UL}-]{{2,63}}|xn--[a-z0-9]{{1,59}})(?<!-)")
}

/// `DomainNameValidator`.
#[derive(Debug, Clone)]
pub struct DomainNameValidator {
    inner: RegexValidator,
    accept_idna: bool,
}

impl DomainNameValidator {
    const MAX_LENGTH: usize = 255;

    pub fn new(accept_idna: bool) -> Result<Self> {
        let pattern = if accept_idna {
            format!(
                "(?i)^{}{}{}\\.?\\Z",
                hostname_re(),
                domain_re(),
                tld_no_fqdn_re()
            )
        } else {
            "(?i)^[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\\.(?!-)[a-zA-Z0-9-]{1,63}(?<!-))*\\.(?!-)(?:[a-zA-Z0-9-]{2,63})(?<!-)\\.?\\Z".to_owned()
        };
        Ok(Self {
            inner: RegexValidator::new(&pattern)?
                .with_default_message(|| gettext("Enter a valid domain name.")),
            accept_idna,
        })
    }
}

impl Validator<str> for DomainNameValidator {
    fn validate(&self, value: &str) -> VResult {
        if value.len() > Self::MAX_LENGTH || (!self.accept_idna && !value.is_ascii()) {
            return Err(invalid(self.inner.message(), &self.inner.code, value));
        }
        self.inner.validate(value)
    }
}

/// `validate_domain_name`.
pub fn validate_domain_name(value: &str) -> VResult {
    static V: LazyLock<DomainNameValidator> =
        LazyLock::new(|| DomainNameValidator::new(true).expect("valid regex"));
    V.validate(value)
}

/// `URLValidator`.
#[derive(Debug, Clone)]
pub struct UrlValidator {
    inner: RegexValidator,
    pub schemes: Vec<String>,
}

impl UrlValidator {
    pub fn new() -> Result<Self> {
        let ipv4 = r"(?:0|25[0-5]|2[0-4][0-9]|1[0-9]?[0-9]?|[1-9][0-9]?)(?:\.(?:0|25[0-5]|2[0-4][0-9]|1[0-9]?[0-9]?|[1-9][0-9]?)){3}";
        let ipv6 = r"\[[0-9a-f:.]+\]";
        let host = format!(
            "({}{}{}\\.?|localhost)",
            hostname_re(),
            domain_re(),
            tld_no_fqdn_re()
        );
        let pattern = format!(
            r"(?i)^(?:[a-z0-9.+-]*)://(?:[^\s:@/]+(?::[^\s:@/]*)?@)?(?:{ipv4}|{ipv6}|{host})(?::[0-9]{{1,5}})?(?:[/?#][^\s]*)?\Z"
        );
        Ok(Self {
            inner: RegexValidator::new(&pattern)?
                .with_default_message(|| gettext("Enter a valid URL.")),
            schemes: ["http", "https", "ftp", "ftps"].map(String::from).to_vec(),
        })
    }

    pub fn with_schemes(mut self, schemes: &[&str]) -> Self {
        self.schemes = schemes.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.inner = self.inner.with_message(message);
        self
    }
}

impl Validator<str> for UrlValidator {
    fn validate(&self, value: &str) -> VResult {
        let fail = || invalid(self.inner.message(), &self.inner.code, value);
        if value.len() > MAX_URL_LENGTH || value.contains(['\t', '\r', '\n']) {
            return Err(fail());
        }
        let scheme = value.split("://").next().unwrap_or("").to_lowercase();
        if !self.schemes.contains(&scheme) {
            return Err(fail());
        }
        let parts = urlsplit(value).map_err(|_| fail())?;
        self.inner.validate(value)?;
        let netloc = &parts.netloc;
        if let Some(rest) = netloc.strip_prefix('[')
            && let Some((ip, tail)) = rest.split_once(']')
        {
            let port_ok = tail.is_empty()
                || tail.strip_prefix(':').is_some_and(|p| {
                    (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit())
                });
            if port_ok && !is_valid_ipv6_address(ip) {
                return Err(fail());
            }
        }
        match parts.hostname() {
            Some(h) if h.len() <= 253 => Ok(()),
            _ => Err(fail()),
        }
    }
}

/// `validate_integer`.
pub fn validate_integer(value: &str) -> VResult {
    static V: LazyLock<RegexValidator> = LazyLock::new(|| {
        RegexValidator::new(r"^-?\d+\Z")
            .expect("valid regex")
            .with_default_message(|| gettext("Enter a valid integer."))
    });
    V.validate(value)
}

/// `EmailValidator`.
#[derive(Debug, Clone)]
pub struct EmailValidator {
    message: Option<String>,
    code: String,
    pub domain_allowlist: Vec<String>,
}

static EMAIL_USER: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r#"(?i)(^[-!#$%&'*+/=?^_`{}|~0-9A-Z]+(\.[-!#$%&'*+/=?^_`{}|~0-9A-Z]+)*\Z|^"([\x01-\x08\x0b\x0c\x0e-\x1f!#-\[\]-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*"\Z)"#,
    )
    .expect("valid regex")
});
static EMAIL_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    compile(&format!(
        "(?i)^{}{}{}\\Z",
        hostname_re(),
        domain_re(),
        tld_no_fqdn_re()
    ))
    .expect("valid regex")
});
static EMAIL_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| compile(r"(?i)\[([A-F0-9:.]+)\]\Z").expect("valid regex"));

impl Default for EmailValidator {
    fn default() -> Self {
        Self {
            message: None,
            code: "invalid".into(),
            domain_allowlist: vec!["localhost".into()],
        }
    }
}

impl EmailValidator {
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }

    pub fn with_allowlist(mut self, allowlist: Vec<String>) -> Self {
        self.domain_allowlist = allowlist;
        self
    }

    fn validate_domain_part(&self, domain_part: &str) -> bool {
        if EMAIL_DOMAIN.is_match(domain_part).unwrap_or(false) {
            return true;
        }
        if let Ok(Some(c)) = EMAIL_LITERAL.captures(domain_part) {
            return validate_ipv46_address(&c[1]).is_ok();
        }
        false
    }
}

impl Validator<str> for EmailValidator {
    fn validate(&self, value: &str) -> VResult {
        let fail = || {
            invalid(
                self.message
                    .clone()
                    .unwrap_or_else(|| gettext("Enter a valid email address.")),
                &self.code,
                value,
            )
        };
        if value.is_empty() || !value.contains('@') || value.chars().count() > 320 {
            return Err(fail());
        }
        let (user, domain) = value.rsplit_once('@').expect("contains @");
        if !EMAIL_USER.is_match(user).unwrap_or(false) {
            return Err(fail());
        }
        if !self.domain_allowlist.iter().any(|d| d == domain) && !self.validate_domain_part(domain)
        {
            return Err(fail());
        }
        Ok(())
    }
}

/// `validate_email`.
pub fn validate_email(value: &str) -> VResult {
    EmailValidator::default().validate(value)
}

/// `validate_slug`.
pub fn validate_slug(value: &str) -> VResult {
    static V: LazyLock<RegexValidator> = LazyLock::new(|| {
        RegexValidator::new(r"^[-a-zA-Z0-9_]+\Z")
            .expect("valid regex")
            .with_default_message(|| {
                gettext(
                    "Enter a valid “slug” consisting of letters, numbers, underscores or hyphens.",
                )
            })
    });
    V.validate(value)
}

/// `validate_unicode_slug`.
pub fn validate_unicode_slug(value: &str) -> VResult {
    static V: LazyLock<RegexValidator> = LazyLock::new(|| {
        RegexValidator::new(r"^[-\w]+\Z").expect("valid regex").with_default_message(|| {
            gettext("Enter a valid “slug” consisting of Unicode letters, numbers, underscores, or hyphens.")
        })
    });
    V.validate(value)
}

fn protocol_error(protocol: &str, value: &str) -> ValidationError {
    ValidationError::new(gettext("Enter a valid %(protocol)s address."))
        .with_code("invalid")
        .with_params([("protocol", gettext(protocol)), ("value", value.to_owned())])
}

/// `validate_ipv4_address`.
pub fn validate_ipv4_address(value: &str) -> VResult {
    value
        .parse::<std::net::Ipv4Addr>()
        .map(|_| ())
        .map_err(|_| protocol_error("IPv4", value))
}

/// `validate_ipv6_address`.
pub fn validate_ipv6_address(value: &str) -> VResult {
    if is_valid_ipv6_address(value) {
        Ok(())
    } else {
        Err(protocol_error("IPv6", value))
    }
}

/// `validate_ipv46_address`.
pub fn validate_ipv46_address(value: &str) -> VResult {
    validate_ipv4_address(value)
        .or_else(|_| validate_ipv6_address(value))
        .map_err(|_| protocol_error("IPv4 or IPv6", value))
}

/// `ip_address_validators()`: validator for `GenericIPAddressField(protocol=..., unpack_ipv4=...)`.
pub fn ip_address_validators(protocol: &str, unpack_ipv4: bool) -> Result<fn(&str) -> VResult> {
    if protocol != "both" && unpack_ipv4 {
        return Err(Error::Other(
            "You can only use `unpack_ipv4` if `protocol` is set to 'both'".into(),
        ));
    }
    match protocol.to_lowercase().as_str() {
        "both" => Ok(validate_ipv46_address),
        "ipv4" => Ok(validate_ipv4_address),
        "ipv6" => Ok(validate_ipv6_address),
        _ => Err(Error::Other(
            format!(
                "The protocol '{protocol}' is unknown. Supported: [\"both\", \"ipv4\", \"ipv6\"]"
            )
            .into(),
        )),
    }
}

/// `int_list_validator()`.
pub fn int_list_validator(
    sep: &str,
    message: Option<&str>,
    code: &str,
    allow_negative: bool,
) -> Result<RegexValidator> {
    let neg = if allow_negative { "(-)?" } else { "" };
    let mut v = RegexValidator::new(&format!(
        r"^{neg}\d+(?:{sep}{neg}\d+)*\Z",
        sep = fancy_regex::escape(sep)
    ))?
    .with_code(code);
    if let Some(m) = message {
        v = v.with_message(m);
    }
    Ok(v)
}

/// `validate_comma_separated_integer_list`.
pub fn validate_comma_separated_integer_list(value: &str) -> VResult {
    static V: LazyLock<RegexValidator> = LazyLock::new(|| {
        int_list_validator(",", None, "invalid", false)
            .expect("valid regex")
            .with_default_message(|| gettext("Enter only digits separated by commas."))
    });
    V.validate(value)
}

/// Comparison policy of the limit validators (`BaseValidator.compare`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Max,
    Min,
}

/// Limit of a [`MaxValueValidator`]/[`MinValueValidator`]: a constant, or a callable evaluated
/// at validation time (e.g. "today").
pub enum Limit<T> {
    Value(T),
    Func(Box<dyn Fn() -> T + Send + Sync>),
}

impl<T: Clone> Limit<T> {
    fn get(&self) -> T {
        match self {
            Self::Value(v) => v.clone(),
            Self::Func(f) => f(),
        }
    }
}

impl<T: Debug> Debug for Limit<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(v) => v.fmt(f),
            Self::Func(_) => f.write_str("<fn>"),
        }
    }
}

macro_rules! limit_validator {
    ($(#[$doc:meta])* $name:ident, $cmp:expr, $code:literal, $msg:literal) => {
        $(#[$doc])*
        #[derive(Debug)]
        pub struct $name<T> {
            pub limit: Limit<T>,
            message: Option<String>,
        }

        impl<T: Clone> $name<T> {
            pub fn new(limit: T) -> Self {
                Self { limit: Limit::Value(limit), message: None }
            }
            pub fn from_fn(f: impl Fn() -> T + Send + Sync + 'static) -> Self {
                Self { limit: Limit::Func(Box::new(f)), message: None }
            }
            pub fn with_message(mut self, message: impl Into<String>) -> Self {
                self.message = Some(message.into());
                self
            }
        }

        impl<T> Validator<T> for $name<T>
        where
            T: PartialOrd + Display + Clone + Send + Sync + Debug,
        {
            fn validate(&self, value: &T) -> VResult {
                let limit = self.limit.get();
                let violated = match $cmp {
                    Cmp::Max => *value > limit,
                    Cmp::Min => *value < limit,
                };
                if violated {
                    let message = self.message.clone().unwrap_or_else(|| gettext($msg));
                    return Err(ValidationError::new(message).with_code($code).with_params([
                        ("limit_value", limit.to_string()),
                        ("show_value", value.to_string()),
                        ("value", value.to_string()),
                    ]));
                }
                Ok(())
            }
        }
    };
}

limit_validator!(
    /// `MaxValueValidator`.
    MaxValueValidator,
    Cmp::Max,
    "max_value",
    "Ensure this value is less than or equal to %(limit_value)s."
);
limit_validator!(
    /// `MinValueValidator`.
    MinValueValidator,
    Cmp::Min,
    "min_value",
    "Ensure this value is greater than or equal to %(limit_value)s."
);

/// `StepValueValidator` (floating-point arithmetic, like Python's `math.remainder`).
#[derive(Debug, Clone)]
pub struct StepValueValidator {
    pub limit: f64,
    pub offset: Option<f64>,
    message: Option<String>,
}

impl StepValueValidator {
    pub fn new(step: f64) -> Self {
        Self {
            limit: step,
            offset: None,
            message: None,
        }
    }

    pub fn with_offset(mut self, offset: f64) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

impl Validator<f64> for StepValueValidator {
    fn validate(&self, value: &f64) -> VResult {
        let offset = self.offset.unwrap_or(0.0);
        let x = value - offset;
        let remainder = x - (x / self.limit).round_ties_even() * self.limit;
        if remainder.abs() <= 1e-9 {
            return Ok(());
        }
        let (default, mut params) = match self.offset {
            None => (
                gettext("Ensure this value is a multiple of step size %(limit_value)s."),
                vec![
                    ("limit_value", self.limit.to_string()),
                    ("show_value", value.to_string()),
                    ("value", value.to_string()),
                ],
            ),
            Some(off) => (
                gettext(
                    "Ensure this value is a multiple of step size %(limit_value)s, starting from %(offset)s, e.g. %(offset)s, %(valid_value1)s, %(valid_value2)s, and so on.",
                ),
                vec![
                    ("limit_value", self.limit.to_string()),
                    ("offset", off.to_string()),
                    ("valid_value1", (off + self.limit).to_string()),
                    ("valid_value2", (off + 2.0 * self.limit).to_string()),
                ],
            ),
        };
        params.shrink_to_fit();
        Err(
            ValidationError::new(self.message.clone().unwrap_or(default))
                .with_code("step_size")
                .with_params(params),
        )
    }
}

macro_rules! length_validator {
    ($(#[$doc:meta])* $name:ident, $cmp:expr, $code:literal, $singular:literal, $plural:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone)]
        pub struct $name {
            pub limit: usize,
            message: Option<String>,
        }

        impl $name {
            pub fn new(limit: usize) -> Self {
                Self { limit, message: None }
            }
            pub fn with_message(mut self, message: impl Into<String>) -> Self {
                self.message = Some(message.into());
                self
            }
        }

        impl Validator<str> for $name {
            fn validate(&self, value: &str) -> VResult {
                let len = value.chars().count();
                let violated = match $cmp {
                    Cmp::Max => len > self.limit,
                    Cmp::Min => len < self.limit,
                };
                if violated {
                    let message = self
                        .message
                        .clone()
                        .unwrap_or_else(|| ngettext($singular, $plural, self.limit as u64));
                    return Err(ValidationError::new(message).with_code($code).with_params([
                        ("limit_value", self.limit.to_string()),
                        ("show_value", len.to_string()),
                        ("value", value.to_owned()),
                    ]));
                }
                Ok(())
            }
        }
    };
}

length_validator!(
    /// `MinLengthValidator`.
    MinLengthValidator,
    Cmp::Min,
    "min_length",
    "Ensure this value has at least %(limit_value)d character (it has %(show_value)d).",
    "Ensure this value has at least %(limit_value)d characters (it has %(show_value)d)."
);
length_validator!(
    /// `MaxLengthValidator`.
    MaxLengthValidator,
    Cmp::Max,
    "max_length",
    "Ensure this value has at most %(limit_value)d character (it has %(show_value)d).",
    "Ensure this value has at most %(limit_value)d characters (it has %(show_value)d)."
);

/// `DecimalValidator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalValidator {
    pub max_digits: Option<u32>,
    pub decimal_places: Option<u32>,
}

impl DecimalValidator {
    pub fn new(max_digits: Option<u32>, decimal_places: Option<u32>) -> Self {
        Self {
            max_digits,
            decimal_places,
        }
    }
}

impl Validator<Decimal> for DecimalValidator {
    fn validate(&self, value: &Decimal) -> VResult {
        let digit_count = if value.mantissa() == 0 {
            1
        } else {
            value.mantissa().unsigned_abs().to_string().len() as u32
        };
        let scale = value.scale();
        let (digits, decimals) = if scale > digit_count {
            (scale, scale)
        } else {
            (digit_count, scale)
        };
        let whole_digits = digits - decimals;
        let err = |msg: String, code: &str, max: u32| {
            ValidationError::new(msg)
                .with_code(code)
                .with_params([("max", max.to_string()), ("value", value.to_string())])
        };
        if let Some(max) = self.max_digits.filter(|&m| digits > m) {
            return Err(err(
                ngettext(
                    "Ensure that there are no more than %(max)s digit in total.",
                    "Ensure that there are no more than %(max)s digits in total.",
                    max.into(),
                ),
                "max_digits",
                max,
            ));
        }
        if let Some(max) = self.decimal_places.filter(|&m| decimals > m) {
            return Err(err(
                ngettext(
                    "Ensure that there are no more than %(max)s decimal place.",
                    "Ensure that there are no more than %(max)s decimal places.",
                    max.into(),
                ),
                "max_decimal_places",
                max,
            ));
        }
        if let (Some(md), Some(dp)) = (self.max_digits, self.decimal_places)
            && whole_digits > md.saturating_sub(dp)
        {
            let max = md.saturating_sub(dp);
            return Err(err(
                ngettext(
                    "Ensure that there are no more than %(max)s digit before the decimal point.",
                    "Ensure that there are no more than %(max)s digits before the decimal point.",
                    max.into(),
                ),
                "max_whole_digits",
                max,
            ));
        }
        Ok(())
    }
}

/// `FileExtensionValidator` (validates a file *name*).
#[derive(Debug, Clone)]
pub struct FileExtensionValidator {
    pub allowed_extensions: Option<Vec<String>>,
    message: Option<String>,
    code: String,
}

impl FileExtensionValidator {
    pub fn new(allowed_extensions: Option<&[&str]>) -> Self {
        Self {
            allowed_extensions: allowed_extensions
                .map(|e| e.iter().map(|s| s.to_lowercase()).collect()),
            message: None,
            code: "invalid_extension".into(),
        }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

impl Validator<str> for FileExtensionValidator {
    fn validate(&self, name: &str) -> VResult {
        let ext = std::path::Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if let Some(allowed) = &self.allowed_extensions
            && !allowed.contains(&ext)
        {
            let message = self.message.clone().unwrap_or_else(|| {
                    gettext("File extension “%(extension)s” is not allowed. Allowed extensions are: %(allowed_extensions)s.")
                });
            return Err(ValidationError::new(message)
                .with_code(self.code.clone())
                .with_params([
                    ("extension", ext),
                    ("allowed_extensions", allowed.join(", ")),
                    ("value", name.to_owned()),
                ]));
        }
        Ok(())
    }
}

/// `ProhibitNullCharactersValidator`.
#[derive(Debug, Clone, Default)]
pub struct ProhibitNullCharactersValidator {
    message: Option<String>,
    code: Option<String>,
}

impl Validator<str> for ProhibitNullCharactersValidator {
    fn validate(&self, value: &str) -> VResult {
        if value.contains('\0') {
            return Err(invalid(
                self.message
                    .clone()
                    .unwrap_or_else(|| gettext("Null characters are not allowed.")),
                self.code
                    .as_deref()
                    .unwrap_or("null_characters_not_allowed"),
                value,
            ));
        }
        Ok(())
    }
}
