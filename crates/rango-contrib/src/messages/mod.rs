//! `contrib.messages`: one-time notifications between requests.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_core::signing;
use rango_http::cache_utils::patch_vary_headers;
use rango_http::{CookieOptions, HttpRequest, HttpResponse};
use rango_middleware::handlers::Middleware;
use rango_template::{Object, TemplateObject, Value as T};
use serde_json::Value as Json;

use crate::sessions::RequestSessionExt;

pub const DEBUG: i64 = 10;
pub const INFO: i64 = 20;
pub const SUCCESS: i64 = 25;
pub const WARNING: i64 = 30;
pub const ERROR: i64 = 40;

const MESSAGE_KEY: &str = "__json_message";
const COOKIE_NAME: &str = "messages";
const SESSION_KEY: &str = "_messages";
const MAX_COOKIE_SIZE: usize = 2048;
const NOT_FINISHED: &str = "__messagesnotfinished__";

/// `Message`.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub level: i64,
    pub message: String,
    pub extra_tags: String,
    /// `SafeData` messages are not escaped by the template.
    pub safe: bool,
}

impl Message {
    pub fn new(level: i64, message: &str) -> Self {
        Self { level, message: message.to_owned(), extra_tags: String::new(), safe: false }
    }

    /// `level_tag` from `MESSAGE_TAGS` (+ defaults).
    pub fn level_tag(&self) -> String {
        level_tag(self.level)
    }

    /// `tags`: `extra_tags` then `level_tag`.
    pub fn tags(&self) -> String {
        [self.extra_tags.as_str(), &self.level_tag()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }
}

/// `get_level_tags()`.
pub fn level_tag(level: i64) -> String {
    if let Some(t) = settings().message_tags.as_ref().and_then(|t| t.get(level.to_string())).and_then(|t| t.as_str()) {
        return t.to_owned();
    }
    match level {
        DEBUG => "debug",
        INFO => "info",
        SUCCESS => "success",
        WARNING => "warning",
        ERROR => "error",
        _ => "",
    }
    .to_owned()
}

struct MessageObject(Message);

impl TemplateObject for MessageObject {
    fn get(&self, key: &str) -> Option<T> {
        Some(match key {
            "message" => T::Str(self.0.message.clone()),
            "level" => T::Int(self.0.level),
            "tags" => T::Str(self.0.tags()),
            "level_tag" => T::Str(self.0.level_tag()),
            "extra_tags" => T::Str(self.0.extra_tags.clone()),
            _ => return None,
        })
    }
    fn display(&self) -> T {
        if self.0.safe { T::Safe(self.0.message.clone()) } else { T::Str(self.0.message.clone()) }
    }
}

// ------------------------------------------------------------------ storage

#[derive(Default)]
struct Inner {
    loaded: Vec<Message>,
    queued: Vec<Message>,
    used: bool,
    added_new: bool,
    level: Option<i64>,
    /// Messages found in the cookie but not yet pulled by the session (fallback).
    cookie_fallback: bool,
}

/// `request._messages`: the storage of one request.
#[derive(Clone, Default)]
pub struct Messages(Arc<Mutex<Inner>>);

pub fn encode_messages(messages: &[Message], extra_not_finished: bool) -> Result<String> {
    let mut out: Vec<Json> = messages
        .iter()
        .map(|m| {
            let mut v = vec![Json::from(MESSAGE_KEY), Json::from(i64::from(m.safe)), Json::from(m.level), Json::from(m.message.clone())];
            if !m.extra_tags.is_empty() {
                v.push(Json::from(m.extra_tags.clone()));
            }
            Json::Array(v)
        })
        .collect();
    if extra_not_finished {
        out.push(Json::from(NOT_FINISHED));
    }
    rango_core::utils::json::dumps_with(&out, true, false).map_err(|e| Error::Other(Box::new(e)))
}

pub fn decode_messages(data: &str) -> Option<Vec<Message>> {
    let v: Vec<Json> = serde_json::from_str(data).ok()?;
    let mut out = Vec::new();
    for item in v {
        match item {
            Json::Array(a) if a.first().and_then(|x| x.as_str()) == Some(MESSAGE_KEY) => {
                out.push(Message { safe: a.get(1).and_then(Json::as_i64) == Some(1), level: a.get(2).and_then(Json::as_i64)?, message: a.get(3).and_then(Json::as_str)?.to_owned(), extra_tags: a.get(4).and_then(Json::as_str).unwrap_or("").to_owned() });
            }
            Json::String(s) if s == NOT_FINISHED => out.push(Message { level: -1, message: NOT_FINISHED.into(), extra_tags: String::new(), safe: false }),
            _ => {}
        }
    }
    Some(out)
}

/// `get_cookie_signer(salt="rango.contrib.messages")`.
fn cookie_signer() -> Result<signing::Signer> {
    let key = format!("rango.http.cookies{}", settings().secret_key()?);
    signing::Signer::new(signing::SignerOptions { key: Some(key), salt: Some("rango.contrib.messages".into()), ..Default::default() })
}

fn signer_encode(messages: &[Message], not_finished: bool) -> Result<String> {
    Ok(cookie_signer()?.sign(&encode_messages(messages, not_finished)?))
}

fn signer_decode(data: &str) -> Option<Vec<Message>> {
    decode_messages(&cookie_signer().ok()?.unsign(data).ok()?)
}

impl Messages {
    /// Load stored messages (cookie, session or both per `MESSAGE_STORAGE`).
    pub fn load(request: &HttpRequest) -> Messages {
        let engine = settings().message_storage.clone();
        let mut loaded = Vec::new();
        let mut fallback = false;
        let use_cookie = !engine.ends_with("SessionStorage");
        let use_session = !engine.ends_with("CookieStorage");
        if use_cookie {
            if let Some(c) = request.cookies().get(COOKIE_NAME) {
                if let Some(mut msgs) = signer_decode(c) {
                    if msgs.last().is_some_and(|m| m.message == NOT_FINISHED) {
                        msgs.pop();
                        fallback = true;
                    }
                    loaded.extend(msgs);
                }
            }
        }
        if use_session && (fallback || !use_cookie) {
            if let Some(s) = request.try_session() {
                if let Some(Json::String(raw)) = s.get(SESSION_KEY) {
                    if let Some(msgs) = decode_messages(&raw) {
                        loaded.extend(msgs);
                    }
                } else if let Some(Json::Array(a)) = s.get(SESSION_KEY) {
                    if let Some(msgs) = decode_messages(&Json::Array(a).to_string()) {
                        loaded.extend(msgs);
                    }
                }
            }
        }
        Messages(Arc::new(Mutex::new(Inner { loaded, cookie_fallback: fallback, ..Inner::default() })))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().expect("messages lock")
    }

    pub fn level(&self) -> i64 {
        self.lock().level.or(settings().message_level).unwrap_or(INFO)
    }

    pub fn set_level(&self, level: i64) {
        self.lock().level = Some(level);
    }

    /// `add(level, message, extra_tags)`: ignored below the configured level.
    pub fn add(&self, level: i64, message: &str, extra_tags: &str) {
        if message.is_empty() || level < self.level() {
            return;
        }
        let mut i = self.lock();
        i.added_new = true;
        i.queued.push(Message { level, message: message.to_owned(), extra_tags: extra_tags.to_owned(), safe: false });
    }

    /// Iterating marks the messages as used (they are not stored again).
    pub fn take(&self) -> Vec<Message> {
        let mut i = self.lock();
        i.used = true;
        let mut all = i.loaded.clone();
        all.extend(i.queued.clone());
        all
    }

    pub fn len(&self) -> usize {
        let i = self.lock();
        i.loaded.len() + i.queued.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `storage.update(response)`: persist the unread messages.
    pub fn update(&self, request: &HttpRequest, response: &mut HttpResponse) -> Result<()> {
        let (to_store, engine) = {
            let i = self.lock();
            let engine = settings().message_storage.clone();
            if i.used {
                (Some(i.queued.clone()), engine)
            } else if i.added_new {
                let mut all = i.loaded.clone();
                all.extend(i.queued.clone());
                (Some(all), engine)
            } else {
                (None, engine)
            }
        };
        let Some(messages) = to_store else { return Ok(()) };
        let s = settings();
        let (cookie, session) = (!engine.ends_with("SessionStorage"), !engine.ends_with("CookieStorage"));
        let remaining: Vec<Message>;
        let mut to_session: Vec<Message> = Vec::new();
        if cookie {
            // CookieStorage keeps what fits in 2048 bytes; the rest (marked NOT_FINISHED) goes to the session.
            let mut fit = messages.clone();
            let mut leftover: Vec<Message> = Vec::new();
            while !fit.is_empty() && signer_encode(&fit, !leftover.is_empty() || (session && false))?.len() > MAX_COOKIE_SIZE {
                leftover.insert(0, fit.pop().expect("non-empty"));
            }
            let overflow = !leftover.is_empty() && session;
            if !leftover.is_empty() && !session {
                // CookieStorage alone: drops what does not fit.
            }
            remaining = fit.clone();
            if overflow {
                to_session = leftover;
            }
            if remaining.is_empty() && !overflow {
                response.delete_cookie(COOKIE_NAME, "/", s.session_cookie_domain.as_deref(), Some(&s.session_cookie_samesite))?;
            } else {
                let opts = CookieOptions {
                    domain: s.session_cookie_domain.clone(),
                    secure: s.session_cookie_secure,
                    httponly: s.session_cookie_httponly,
                    samesite: if s.session_cookie_samesite.is_empty() { None } else { Some(s.session_cookie_samesite.clone()) },
                    ..CookieOptions::default()
                };
                response.set_cookie(COOKIE_NAME, &signer_encode(&remaining, overflow)?, &opts)?;
            }
        } else {
            to_session = messages;
        }
        if session {
            if let Some(sess) = request.try_session() {
                if to_session.is_empty() {
                    sess.pop(SESSION_KEY);
                } else {
                    sess.set(SESSION_KEY, encode_messages(&to_session, false)?);
                }
            }
        }
        Ok(())
    }
}

impl TemplateObject for Messages {
    fn get(&self, key: &str) -> Option<T> {
        match key {
            "level" => Some(T::Int(self.level())),
            _ => None,
        }
    }
    fn display(&self) -> T {
        T::Str(format!("<Messages: {}>", self.len()))
    }
    fn iterate(&self) -> Option<Vec<T>> {
        Some(self.take().into_iter().map(|m| Object::new(MessageObject(m))).collect())
    }
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

// ------------------------------------------------------------------ API

/// `get_messages(request)`.
pub fn get_messages(request: &HttpRequest) -> Option<Messages> {
    request.ext::<Messages>().map(|m| (*m).clone())
}

/// `add_message(request, level, message, extra_tags, fail_silently)`.
pub fn add_message(request: &HttpRequest, level: i64, message: &str, extra_tags: &str, fail_silently: bool) -> Result<()> {
    match get_messages(request) {
        Some(m) => {
            m.add(level, message, extra_tags);
            Ok(())
        }
        None if fail_silently => Ok(()),
        None => Err(Error::Other("You cannot add messages without installing rango.contrib.messages.middleware.MessageMiddleware".into())),
    }
}

macro_rules! level_fn {
    ($($name:ident => $lvl:expr),*) => {$(
        pub fn $name(request: &HttpRequest, message: &str) -> Result<()> { add_message(request, $lvl, message, "", false) }
    )*};
}
level_fn!(debug => DEBUG, info => INFO, success => SUCCESS, warning => WARNING, error => ERROR);

pub fn get_level(request: &HttpRequest) -> i64 {
    get_messages(request).map_or_else(|| settings().message_level.unwrap_or(INFO), |m| m.level())
}

pub fn set_level(request: &HttpRequest, level: i64) -> bool {
    match get_messages(request) {
        Some(m) => {
            m.set_level(level);
            true
        }
        None => false,
    }
}

// ------------------------------------------------------------------ middleware & context processor

#[derive(Default)]
pub struct MessageMiddleware;

#[async_trait]
impl Middleware for MessageMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        request.set_ext(Messages::load(request));
        Ok(None)
    }

    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if let Some(m) = request.ext::<Messages>() {
            if m.lock().cookie_fallback || m.lock().added_new || m.lock().used {
                patch_vary_headers(&mut response, &["Cookie"]);
            }
            m.update(request, &mut response)?;
        }
        Ok(response)
    }
}

rango_middleware::register_middleware!("rango.contrib.messages.middleware.MessageMiddleware", || Ok(Arc::new(MessageMiddleware)));

fn messages_context_processor(request: &HttpRequest) -> indexmap::IndexMap<String, T> {
    let mut m = indexmap::IndexMap::new();
    if let Some(msgs) = get_messages(request) {
        m.insert("messages".to_owned(), Object::new(msgs));
    }
    let levels: indexmap::IndexMap<String, T> = [("DEBUG", DEBUG), ("INFO", INFO), ("SUCCESS", SUCCESS), ("WARNING", WARNING), ("ERROR", ERROR)].into_iter().map(|(k, v)| (k.to_owned(), T::Int(v))).collect();
    m.insert("DEFAULT_MESSAGE_LEVELS".to_owned(), T::Map(levels));
    m
}

rango_template::register_context_processor!("rango.contrib.messages.context_processors.messages", messages_context_processor);
rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.messages"));
