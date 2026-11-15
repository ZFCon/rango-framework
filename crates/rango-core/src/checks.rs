//! `core.checks`: the system-check framework.
//!
//! A check is an async function registered at link time with [`register_check!`]; subsystems
//! register their own. The security/urls/models/... checks arrive with those subsystems.

use std::fmt;

use futures_util::future::BoxFuture;

use crate::conf::try_settings;
use crate::exceptions::Result;

pub const DEBUG: u32 = 10;
pub const INFO: u32 = 20;
pub const WARNING: u32 = 30;
pub const ERROR: u32 = 40;
pub const CRITICAL: u32 = 50;

/// `core.checks.Tags`.
pub mod tags {
    pub const ADMIN: &str = "admin";
    pub const ASYNC_SUPPORT: &str = "async_support";
    pub const CACHES: &str = "caches";
    pub const COMPATIBILITY: &str = "compatibility";
    pub const DATABASE: &str = "database";
    pub const FILES: &str = "files";
    pub const MODELS: &str = "models";
    pub const SECURITY: &str = "security";
    pub const SIGNALS: &str = "signals";
    pub const SITES: &str = "sites";
    pub const STATICFILES: &str = "staticfiles";
    pub const TEMPLATES: &str = "templates";
    pub const TRANSLATION: &str = "translation";
    pub const URLS: &str = "urls";
}

/// `CheckMessage` (and its `Debug`/`Info`/`Warning`/`Error`/`Critical` flavours).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckMessage {
    pub level: u32,
    pub msg: String,
    pub hint: Option<String>,
    /// What the message is about (a model label, a setting name, ...).
    pub obj: Option<String>,
    /// Identifier such as `"security.W004"`, used for silencing.
    pub id: Option<String>,
}

impl CheckMessage {
    pub fn new(level: u32, msg: impl Into<String>) -> Self {
        Self {
            level,
            msg: msg.into(),
            hint: None,
            obj: None,
            id: None,
        }
    }
    pub fn debug(msg: impl Into<String>) -> Self {
        Self::new(DEBUG, msg)
    }
    pub fn info(msg: impl Into<String>) -> Self {
        Self::new(INFO, msg)
    }
    pub fn warning(msg: impl Into<String>) -> Self {
        Self::new(WARNING, msg)
    }
    pub fn error(msg: impl Into<String>) -> Self {
        Self::new(ERROR, msg)
    }
    pub fn critical(msg: impl Into<String>) -> Self {
        Self::new(CRITICAL, msg)
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn with_obj(mut self, obj: impl Into<String>) -> Self {
        self.obj = Some(obj.into());
        self
    }
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// `is_serious(level=ERROR)`.
    pub fn is_serious(&self, level: u32) -> bool {
        self.level >= level
    }

    /// `is_silenced()`: id listed in `SILENCED_SYSTEM_CHECKS`.
    pub fn is_silenced(&self) -> bool {
        match (&self.id, try_settings()) {
            (Some(id), Ok(s)) => s.silenced_system_checks.contains(id),
            _ => false,
        }
    }

    /// Level name as `manage.py check` prints it (`ERRORS`, `WARNINGS`, ...).
    pub fn level_name(level: u32) -> &'static str {
        match level {
            0..DEBUG => "DEBUGS",
            DEBUG..INFO => "DEBUGS",
            INFO..WARNING => "INFOS",
            WARNING..ERROR => "WARNINGS",
            ERROR..CRITICAL => "ERRORS",
            _ => "CRITICALS",
        }
    }
}

impl fmt::Display for CheckMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let obj = self.obj.as_deref().unwrap_or("?");
        let id = self
            .id
            .as_ref()
            .map(|i| format!("({i}) "))
            .unwrap_or_default();
        let hint = self
            .hint
            .as_ref()
            .map(|h| format!("\n\tHINT: {h}"))
            .unwrap_or_default();
        write!(f, "{obj}: {id}{}{hint}", self.msg)
    }
}

/// Arguments every check receives (`app_configs`, `databases`).
#[derive(Debug, Clone, Default)]
pub struct CheckContext {
    /// App labels to restrict model-related checks to (`None` = all).
    pub app_labels: Option<Vec<String>>,
    /// Database aliases database checks may touch (`None` = none).
    pub databases: Option<Vec<String>>,
}

/// A registered check. Create with [`register_check!`].
pub struct CheckRegistration {
    pub tags: &'static [&'static str],
    /// Deployment checks only run with `check --deploy`.
    pub deploy: bool,
    pub run: for<'a> fn(&'a CheckContext) -> BoxFuture<'a, Vec<CheckMessage>>,
}

inventory::collect!(CheckRegistration);

/// Register a check: `register_check!(tags = [tags::SECURITY], deploy = true, my_check);`
/// where `my_check` is `async fn(&CheckContext) -> Vec<CheckMessage>`.
#[macro_export]
macro_rules! register_check {
    (tags = [$($tag:expr),* $(,)?], deploy = $deploy:expr, $f:path) => {
        $crate::inventory::submit! {
            $crate::checks::CheckRegistration {
                tags: &[$($tag),*],
                deploy: $deploy,
                run: |ctx| ::std::boxed::Box::pin($f(ctx)),
            }
        }
    };
    (tags = [$($tag:expr),* $(,)?], $f:path) => {
        $crate::register_check!(tags = [$($tag),*], deploy = false, $f);
    };
}

fn selected(
    include_deployment_checks: bool,
    tags: Option<&[String]>,
) -> Vec<&'static CheckRegistration> {
    inventory::iter::<CheckRegistration>
        .into_iter()
        .filter(|c| include_deployment_checks || !c.deploy)
        .filter(|c| tags.is_none_or(|t| c.tags.iter().any(|ct| t.iter().any(|x| x == ct))))
        .collect()
}

/// `registry.run_checks()`.
pub async fn run_checks(
    context: &CheckContext,
    tags: Option<&[String]>,
    include_deployment_checks: bool,
) -> Result<Vec<CheckMessage>> {
    let mut errors = Vec::new();
    for check in selected(include_deployment_checks, tags) {
        errors.extend((check.run)(context).await);
    }
    Ok(errors)
}

/// `registry.tags_available()`.
pub fn tags_available(deployment_checks: bool) -> Vec<&'static str> {
    let mut all: Vec<&'static str> = inventory::iter::<CheckRegistration>
        .into_iter()
        .filter(|c| deployment_checks || !c.deploy)
        .flat_map(|c| c.tags.iter().copied())
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}
