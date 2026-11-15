//! `core.signing`: URL-safe, HMAC-signed values and objects.
//!
//! Output is byte-compatible with the (same salts, key derivation, base64/zlib framing),
//! so a value signed by either side verifies on the other. The salts therefore keep their
//! `` prefix on purpose.
//!
//! Divergence: `SIGNING_BACKEND` is ignored (cookie signing always uses [`TimestampSigner`]).

use std::io::{Read, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::conf::try_settings;
use crate::exceptions::Result;
use crate::utils::crypto::{HashAlgorithm, constant_time_compare, salted_hmac};

const BASE62_ALPHABET: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// `BadSignature` / `SignatureExpired`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BadSignature {
    /// The signature does not match (or the value is malformed).
    #[error("{0}")]
    Mismatch(String),
    /// `SignatureExpired`: timestamp older than `max_age`.
    #[error("{0}")]
    Expired(String),
}

impl BadSignature {
    pub fn is_expired(&self) -> bool {
        matches!(self, Self::Expired(_))
    }
}

pub fn b62_encode(s: i64) -> String {
    if s == 0 {
        return "0".into();
    }
    let sign = if s < 0 { "-" } else { "" };
    let mut n = s.unsigned_abs();
    let mut digits = Vec::new();
    while n > 0 {
        digits.push(BASE62_ALPHABET.as_bytes()[(n % 62) as usize]);
        n /= 62;
    }
    digits.reverse();
    format!("{sign}{}", String::from_utf8(digits).expect("ascii"))
}

pub fn b62_decode(s: &str) -> Option<i64> {
    if s == "0" {
        return Some(0);
    }
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1, s),
    };
    let mut decoded: i64 = 0;
    for c in digits.chars() {
        let d = BASE62_ALPHABET.find(c)? as i64;
        decoded = decoded.checked_mul(62)?.checked_add(d)?;
    }
    Some(sign * decoded)
}

fn b64_encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

fn b64_decode(s: &str) -> std::result::Result<Vec<u8>, BadSignature> {
    URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .map_err(|e| BadSignature::Mismatch(e.to_string()))
}

fn base64_hmac(salt: &str, value: &str, key: &[u8], algorithm: HashAlgorithm) -> String {
    b64_encode(&salted_hmac(
        salt.as_bytes(),
        value.as_bytes(),
        key,
        algorithm,
    ))
}

/// Constructor arguments of `Signer` / `TimestampSigner`.
#[derive(Debug, Clone)]
pub struct SignerOptions {
    /// Defaults to `settings.SECRET_KEY`.
    pub key: Option<String>,
    pub sep: String,
    /// Defaults to `core.signing.Signer` / `core.signing.TimestampSigner`.
    pub salt: Option<String>,
    /// Defaults to sha256.
    pub algorithm: Option<HashAlgorithm>,
    /// Defaults to `settings.SECRET_KEY_FALLBACKS`.
    pub fallback_keys: Option<Vec<String>>,
}

impl Default for SignerOptions {
    fn default() -> Self {
        Self {
            key: None,
            sep: ":".into(),
            salt: None,
            algorithm: None,
            fallback_keys: None,
        }
    }
}

/// `Signer`: signs strings as `value<sep>signature`.
#[derive(Debug, Clone)]
pub struct Signer {
    key: Vec<u8>,
    fallback_keys: Vec<Vec<u8>>,
    sep: String,
    salt: String,
    algorithm: HashAlgorithm,
}

impl Signer {
    pub fn new(options: SignerOptions) -> Result<Self> {
        Self::build(options, "rango.core.signing.Signer")
    }

    fn build(options: SignerOptions, default_salt: &str) -> Result<Self> {
        let key = match options.key.filter(|k| !k.is_empty()) {
            Some(k) => k.into_bytes(),
            None => try_settings()?.secret_key()?.as_bytes().to_vec(),
        };
        let fallback_keys = match options.fallback_keys {
            Some(f) => f.into_iter().map(String::into_bytes).collect(),
            None => try_settings()?
                .secret_key_fallbacks
                .iter()
                .map(|k| k.clone().into_bytes())
                .collect(),
        };
        // `_SEP_UNSAFE = ^[A-z0-9-_=]*$`: separators that could appear in base64 are unsafe.
        if options
            .sep
            .chars()
            .all(|c| matches!(c, 'A'..='z' | '0'..='9' | '-' | '='))
        {
            return Err(crate::exceptions::Error::Other(
                format!(
                    "Unsafe Signer separator: {:?} (cannot be empty or consist of only A-z0-9-_=)",
                    options.sep
                )
                .into(),
            ));
        }
        Ok(Self {
            key,
            fallback_keys,
            sep: options.sep,
            salt: options
                .salt
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_salt.to_owned()),
            algorithm: options.algorithm.unwrap_or(HashAlgorithm::Sha256),
        })
    }

    fn signature(&self, value: &str, key: &[u8]) -> String {
        base64_hmac(&format!("{}signer", self.salt), value, key, self.algorithm)
    }

    pub fn sign(&self, value: &str) -> String {
        format!("{value}{}{}", self.sep, self.signature(value, &self.key))
    }

    pub fn unsign(&self, signed_value: &str) -> std::result::Result<String, BadSignature> {
        let Some((value, sig)) = signed_value.rsplit_once(self.sep.as_str()) else {
            return Err(BadSignature::Mismatch(format!(
                "No \"{}\" found in value",
                self.sep
            )));
        };
        for key in std::iter::once(&self.key).chain(&self.fallback_keys) {
            if constant_time_compare(sig, self.signature(value, key)) {
                return Ok(value.to_owned());
            }
        }
        Err(BadSignature::Mismatch(format!(
            "Signature \"{sig}\" does not match"
        )))
    }

    /// `sign_object()`: JSON, optionally zlib-compressed (only if smaller), base64, then signed.
    pub fn sign_object<T: Serialize + ?Sized>(&self, obj: &T, compress: bool) -> Result<String> {
        let base64d = encode_object(obj, compress)?;
        Ok(self.sign(&base64d))
    }

    /// `unsign_object()`.
    pub fn unsign_object<T: DeserializeOwned>(
        &self,
        signed_obj: &str,
    ) -> std::result::Result<T, BadSignature> {
        decode_object(&self.unsign(signed_obj)?)
    }
}

fn encode_object<T: Serialize + ?Sized>(obj: &T, compress: bool) -> Result<String> {
    let mut data = crate::utils::json::dumps(obj, true)
        .map_err(|e| crate::exceptions::Error::Other(Box::new(e)))?
        .into_bytes();
    let mut is_compressed = false;
    if compress {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&data)
            .map_err(|e| crate::exceptions::Error::Other(Box::new(e)))?;
        let compressed = enc
            .finish()
            .map_err(|e| crate::exceptions::Error::Other(Box::new(e)))?;
        if compressed.len() + 1 < data.len() {
            data = compressed;
            is_compressed = true;
        }
    }
    let b64 = b64_encode(&data);
    Ok(if is_compressed {
        format!(".{b64}")
    } else {
        b64
    })
}

fn decode_object<T: DeserializeOwned>(base64d: &str) -> std::result::Result<T, BadSignature> {
    let (decompress, b64) = match base64d.strip_prefix('.') {
        Some(rest) => (true, rest),
        None => (false, base64d),
    };
    let mut data = b64_decode(b64)?;
    if decompress {
        let mut out = Vec::new();
        ZlibDecoder::new(&data[..])
            .read_to_end(&mut out)
            .map_err(|e| BadSignature::Mismatch(e.to_string()))?;
        data = out;
    }
    serde_json::from_slice(&data).map_err(|e| BadSignature::Mismatch(e.to_string()))
}

/// `TimestampSigner`: like [`Signer`] but embeds a base62 timestamp so values can expire.
#[derive(Debug, Clone)]
pub struct TimestampSigner(Signer);

impl TimestampSigner {
    pub fn new(options: SignerOptions) -> Result<Self> {
        Signer::build(options, "rango.core.signing.TimestampSigner").map(Self)
    }

    pub fn sign(&self, value: &str) -> String {
        self.sign_at(value, now_secs())
    }

    /// `sign()` with an explicit timestamp (testing, replays).
    pub fn sign_at(&self, value: &str, timestamp: i64) -> String {
        self.0
            .sign(&format!("{value}{}{}", self.0.sep, b62_encode(timestamp)))
    }

    pub fn unsign(
        &self,
        value: &str,
        max_age: Option<Duration>,
    ) -> std::result::Result<String, BadSignature> {
        self.unsign_at(value, max_age, now_secs() as f64)
    }

    /// `unsign()` against an explicit "now".
    pub fn unsign_at(
        &self,
        value: &str,
        max_age: Option<Duration>,
        now: f64,
    ) -> std::result::Result<String, BadSignature> {
        let result = self.0.unsign(value)?;
        let (value, timestamp) = result.rsplit_once(self.0.sep.as_str()).ok_or_else(|| {
            BadSignature::Mismatch(format!("No \"{}\" found in value", self.0.sep))
        })?;
        let timestamp = b62_decode(timestamp)
            .ok_or_else(|| BadSignature::Mismatch("Invalid timestamp".to_owned()))?;
        if let Some(max_age) = max_age {
            let age = now - timestamp as f64;
            if age > max_age.as_secs_f64() {
                return Err(BadSignature::Expired(format!(
                    "Signature age {age} > {} seconds",
                    max_age.as_secs_f64()
                )));
            }
        }
        Ok(value.to_owned())
    }

    pub fn sign_object<T: Serialize + ?Sized>(&self, obj: &T, compress: bool) -> Result<String> {
        self.sign_object_at(obj, compress, now_secs())
    }

    /// `sign_object()` with an explicit timestamp.
    pub fn sign_object_at<T: Serialize + ?Sized>(
        &self,
        obj: &T,
        compress: bool,
        timestamp: i64,
    ) -> Result<String> {
        Ok(self.sign_at(&encode_object(obj, compress)?, timestamp))
    }

    pub fn unsign_object<T: DeserializeOwned>(
        &self,
        signed_obj: &str,
        max_age: Option<Duration>,
    ) -> std::result::Result<T, BadSignature> {
        self.unsign_object_at(signed_obj, max_age, now_secs() as f64)
    }

    /// `unsign_object()` against an explicit "now".
    pub fn unsign_object_at<T: DeserializeOwned>(
        &self,
        signed_obj: &str,
        max_age: Option<Duration>,
        now: f64,
    ) -> std::result::Result<T, BadSignature> {
        decode_object(&self.unsign_at(signed_obj, max_age, now)?)
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Default salt of `dumps()` / `loads()`.
pub const DEFAULT_DUMPS_SALT: &str = "rango.core.signing";

/// `dumps()`: URL-safe, HMAC-signed, optionally compressed JSON. `key` defaults to `SECRET_KEY`.
pub fn dumps<T: Serialize + ?Sized>(
    obj: &T,
    key: Option<&str>,
    salt: &str,
    compress: bool,
) -> Result<String> {
    TimestampSigner::new(SignerOptions {
        key: key.map(str::to_owned),
        salt: Some(salt.to_owned()),
        ..SignerOptions::default()
    })?
    .sign_object(obj, compress)
}

/// `loads()`: reverse of [`dumps`].
pub fn loads<T: DeserializeOwned>(
    s: &str,
    key: Option<&str>,
    salt: &str,
    max_age: Option<Duration>,
    fallback_keys: Option<Vec<String>>,
) -> Result<std::result::Result<T, BadSignature>> {
    let signer = TimestampSigner::new(SignerOptions {
        key: key.map(str::to_owned),
        salt: Some(salt.to_owned()),
        fallback_keys,
        ..SignerOptions::default()
    })?;
    Ok(signer.unsign_object(s, max_age))
}

fn cookie_signer_key(key: &str) -> String {
    format!("rango.http.cookies{key}")
}

/// `_cookie_signer_salt()`: the v2 salt used for signed cookies.
pub fn cookie_signer_salt(cookie_name: &str, salt: &str) -> String {
    format!(
        "rango.http.cookies.v2:{}:{salt}{cookie_name}",
        salt.chars().count()
    )
}

/// `get_cookie_signer()`.
pub fn get_cookie_signer(salt: &str) -> Result<TimestampSigner> {
    let s = try_settings()?;
    TimestampSigner::new(SignerOptions {
        key: Some(cookie_signer_key(s.secret_key()?)),
        fallback_keys: Some(
            s.secret_key_fallbacks
                .iter()
                .map(|k| cookie_signer_key(k))
                .collect(),
        ),
        salt: Some(salt.to_owned()),
        ..SignerOptions::default()
    })
}

/// `_unsign_cookie()` (used by `HttpRequest.get_signed_cookie`): v2 salt first, then the
/// legacy `cookie_name + salt` salt when `SIGNED_COOKIE_LEGACY_SALT_FALLBACK` is on.
pub fn unsign_cookie(
    signed_value: &str,
    cookie_name: &str,
    salt: &str,
    max_age: Option<Duration>,
) -> Result<std::result::Result<String, BadSignature>> {
    match get_cookie_signer(&cookie_signer_salt(cookie_name, salt))?.unsign(signed_value, max_age) {
        Err(e) if !e.is_expired() && try_settings()?.signed_cookie_legacy_salt_fallback => {
            Ok(get_cookie_signer(&format!("{cookie_name}{salt}"))?.unsign(signed_value, max_age))
        }
        other => Ok(other),
    }
}

/// `get_cookie_signer()` salt used by `HttpResponse.set_signed_cookie` (v2 format).
pub fn sign_cookie(value: &str, cookie_name: &str, salt: &str) -> Result<String> {
    Ok(get_cookie_signer(&cookie_signer_salt(cookie_name, salt))?.sign(value))
}
