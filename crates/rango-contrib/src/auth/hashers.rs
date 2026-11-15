//! `contrib.auth.hashers`: password hashing with stable encodings.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_core::utils::crypto::{constant_time_compare, get_random_string};
use sha2::Digest;

pub const UNUSABLE_PASSWORD_PREFIX: &str = "!";
pub const UNUSABLE_PASSWORD_SUFFIX_LENGTH: usize = 40;
const RANDOM_STRING_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// A password hasher (`BasePasswordHasher`).
pub trait PasswordHasher: Send + Sync {
    fn algorithm(&self) -> &'static str;
    fn salt(&self) -> String {
        get_random_string(22, RANDOM_STRING_CHARS)
    }
    fn encode(&self, password: &str, salt: &str) -> Result<String>;
    fn verify(&self, password: &str, encoded: &str) -> Result<bool>;
    /// `must_update(encoded)`: weaker than the current parameters.
    fn must_update(&self, _encoded: &str) -> bool {
        false
    }
}

fn b64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn bad(msg: &str) -> Error {
    Error::Other(msg.to_owned().into())
}

// ------------------------------------------------------------------ PBKDF2

pub struct Pbkdf2 {
    pub sha1: bool,
    pub iterations: u32,
}

impl Pbkdf2 {
    pub const SHA256: Pbkdf2 = Pbkdf2 { sha1: false, iterations: 1_000_000 };
    pub const SHA1: Pbkdf2 = Pbkdf2 { sha1: true, iterations: 1_000_000 };

    fn derive(&self, password: &str, salt: &str, iterations: u32) -> Vec<u8> {
        if self.sha1 {
            let mut out = [0u8; 20];
            pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), salt.as_bytes(), iterations, &mut out);
            out.to_vec()
        } else {
            let mut out = [0u8; 32];
            pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt.as_bytes(), iterations, &mut out);
            out.to_vec()
        }
    }
}

impl PasswordHasher for Pbkdf2 {
    fn algorithm(&self) -> &'static str {
        if self.sha1 { "pbkdf2_sha1" } else { "pbkdf2_sha256" }
    }
    fn encode(&self, password: &str, salt: &str) -> Result<String> {
        if salt.contains('$') {
            return Err(bad("salt contains dollar signs"));
        }
        Ok(format!("{}${}${salt}${}", self.algorithm(), self.iterations, b64(&self.derive(password, salt, self.iterations))))
    }
    fn verify(&self, password: &str, encoded: &str) -> Result<bool> {
        let p: Vec<&str> = encoded.splitn(4, '$').collect();
        if p.len() != 4 || p[0] != self.algorithm() {
            return Err(bad("invalid encoded password"));
        }
        let iterations: u32 = p[1].parse().map_err(|_| bad("invalid iterations"))?;
        Ok(constant_time_compare(encoded, format!("{}${iterations}${}${}", p[0], p[2], b64(&self.derive(password, p[2], iterations)))))
    }
    fn must_update(&self, encoded: &str) -> bool {
        encoded.split('$').nth(1).and_then(|i| i.parse::<u32>().ok()).is_some_and(|i| i != self.iterations)
    }
}

// ------------------------------------------------------------------ Argon2

pub struct Argon2Hasher;

impl Argon2Hasher {
    fn run(password: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Vec<u8>> {
        let params = argon2::Params::new(m, t, p, Some(32)).map_err(|e| bad(&e.to_string()))?;
        let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut out = vec![0u8; 32];
        a.hash_password_into(password.as_bytes(), salt, &mut out).map_err(|e| bad(&e.to_string()))?;
        Ok(out)
    }
}

impl PasswordHasher for Argon2Hasher {
    fn algorithm(&self) -> &'static str {
        "argon2"
    }
    fn encode(&self, password: &str, salt: &str) -> Result<String> {
        let (m, t, p) = (102_400, 2, 8);
        let hash = Self::run(password, salt.as_bytes(), m, t, p)?;
        Ok(format!("argon2$argon2id$v=19$m={m},t={t},p={p}${}${}", STANDARD_NO_PAD.encode(salt), STANDARD_NO_PAD.encode(hash)))
    }
    fn verify(&self, password: &str, encoded: &str) -> Result<bool> {
        // argon2$argon2id$v=19$m=..,t=..,p=..$salt$hash
        let p: Vec<&str> = encoded.split('$').collect();
        if p.len() != 6 || p[0] != "argon2" {
            return Err(bad("invalid encoded password"));
        }
        let prm: std::collections::HashMap<&str, u32> = p[3].split(',').filter_map(|kv| kv.split_once('=').and_then(|(k, v)| v.parse().ok().map(|v| (k, v)))).collect();
        let salt = STANDARD_NO_PAD.decode(p[4]).map_err(|_| bad("bad salt"))?;
        let want = STANDARD_NO_PAD.decode(p[5]).map_err(|_| bad("bad hash"))?;
        let got = Self::run(password, &salt, *prm.get("m").ok_or_else(|| bad("m"))?, *prm.get("t").ok_or_else(|| bad("t"))?, *prm.get("p").ok_or_else(|| bad("p"))?)?;
        Ok(constant_time_compare(hex(&got), hex(&want)))
    }
}

// ------------------------------------------------------------------ bcrypt

pub struct Bcrypt {
    pub sha256: bool,
    pub rounds: u32,
}

impl Bcrypt {
    pub const SHA256: Bcrypt = Bcrypt { sha256: true, rounds: 12 };
    pub const PLAIN: Bcrypt = Bcrypt { sha256: false, rounds: 12 };

    fn prepare(&self, password: &str) -> Vec<u8> {
        if self.sha256 { hex(&sha2::Sha256::digest(password.as_bytes())).into_bytes() } else { password.as_bytes().to_vec() }
    }
}

impl PasswordHasher for Bcrypt {
    fn algorithm(&self) -> &'static str {
        if self.sha256 { "bcrypt_sha256" } else { "bcrypt" }
    }
    fn salt(&self) -> String {
        // bcrypt takes 16 random bytes; the "salt" here is their hex form.
        get_random_string(32, "0123456789abcdef")
    }
    fn encode(&self, password: &str, salt: &str) -> Result<String> {
        let mut raw = [0u8; 16];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = u8::from_str_radix(salt.get(i * 2..i * 2 + 2).unwrap_or("00"), 16).unwrap_or(0);
        }
        let parts = bcrypt::hash_with_salt(self.prepare(password), self.rounds, raw).map_err(|e| bad(&e.to_string()))?;
        Ok(format!("{}${}", self.algorithm(), parts.format_for_version(bcrypt::Version::TwoB)))
    }
    fn verify(&self, password: &str, encoded: &str) -> Result<bool> {
        let (algo, data) = encoded.split_once('$').ok_or_else(|| bad("invalid encoded password"))?;
        if algo != self.algorithm() {
            return Err(bad("invalid encoded password"));
        }
        bcrypt::verify(self.prepare(password), data).map_err(|e| bad(&e.to_string()))
    }
    fn must_update(&self, encoded: &str) -> bool {
        encoded.split('$').nth(3).and_then(|r| r.parse::<u32>().ok()).is_some_and(|r| r != self.rounds)
    }
}

// ------------------------------------------------------------------ scrypt

pub struct ScryptHasher;

impl ScryptHasher {
    fn derive(password: &str, salt: &str, n: u64, r: u32, p: u32) -> Result<Vec<u8>> {
        let log_n = n.trailing_zeros() as u8;
        let params = scrypt::Params::new(log_n, r, p).map_err(|e| bad(&e.to_string()))?;
        let mut out = vec![0u8; 64];
        scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, &mut out).map_err(|e| bad(&e.to_string()))?;
        Ok(out)
    }
}

impl PasswordHasher for ScryptHasher {
    fn algorithm(&self) -> &'static str {
        "scrypt"
    }
    fn encode(&self, password: &str, salt: &str) -> Result<String> {
        let (n, r, p) = (16384u64, 8u32, 5u32);
        Ok(format!("scrypt${n}${salt}${r}${p}${}", b64(&Self::derive(password, salt, n, r, p)?)))
    }
    fn verify(&self, password: &str, encoded: &str) -> Result<bool> {
        let p: Vec<&str> = encoded.split('$').collect();
        if p.len() != 6 || p[0] != "scrypt" {
            return Err(bad("invalid encoded password"));
        }
        let (n, r, pp) = (p[1].parse::<u64>().map_err(|_| bad("n"))?, p[3].parse::<u32>().map_err(|_| bad("r"))?, p[4].parse::<u32>().map_err(|_| bad("p"))?);
        Ok(constant_time_compare(encoded, format!("scrypt${n}${}${r}${pp}${}", p[2], b64(&Self::derive(password, p[2], n, r, pp)?))))
    }
}

// ------------------------------------------------------------------ md5

/// `MD5PasswordHasher` (insecure; kept for old databases).
pub struct Md5Hasher;

impl PasswordHasher for Md5Hasher {
    fn algorithm(&self) -> &'static str {
        "md5"
    }
    fn encode(&self, password: &str, salt: &str) -> Result<String> {
        Ok(format!("md5${salt}${}", hex(&md5::Md5::digest(format!("{salt}{password}").as_bytes()))))
    }
    fn verify(&self, password: &str, encoded: &str) -> Result<bool> {
        let salt = encoded.split('$').nth(1).unwrap_or("");
        Ok(constant_time_compare(encoded, self.encode(password, salt)?))
    }
}

// ------------------------------------------------------------------ registry & API

/// `get_hashers()`: `PASSWORD_HASHERS` in order.
pub fn get_hashers() -> Result<Vec<Box<dyn PasswordHasher>>> {
    let mut out: Vec<Box<dyn PasswordHasher>> = Vec::new();
    for name in &settings().password_hashers {
        let short = name.rsplit('.').next().unwrap_or(name);
        out.push(match short {
            "PBKDF2PasswordHasher" => Box::new(Pbkdf2::SHA256),
            "PBKDF2SHA1PasswordHasher" => Box::new(Pbkdf2::SHA1),
            "Argon2PasswordHasher" => Box::new(Argon2Hasher),
            "BCryptSHA256PasswordHasher" => Box::new(Bcrypt::SHA256),
            "BCryptPasswordHasher" => Box::new(Bcrypt::PLAIN),
            "ScryptPasswordHasher" => Box::new(ScryptHasher),
            "MD5PasswordHasher" => Box::new(Md5Hasher),
            other => return Err(Error::improperly_configured(format!("Unknown password hasher '{other}'"))),
        });
    }
    Ok(out)
}

/// `get_hasher(algorithm)`; `"default"` is the first configured one.
pub fn get_hasher(algorithm: &str) -> Result<Box<dyn PasswordHasher>> {
    let hashers = get_hashers()?;
    if algorithm == "default" {
        return hashers.into_iter().next().ok_or_else(|| Error::improperly_configured("PASSWORD_HASHERS is empty"));
    }
    hashers.into_iter().find(|h| h.algorithm() == algorithm).ok_or_else(|| Error::Other(format!("Unknown password hashing algorithm '{algorithm}'. Did you specify it in the PASSWORD_HASHERS setting?").into()))
}

/// `identify_hasher(encoded)`.
pub fn identify_hasher(encoded: &str) -> Result<Box<dyn PasswordHasher>> {
    let algorithm = encoded.split('$').next().unwrap_or("");
    get_hasher(algorithm)
}

/// `is_password_usable(encoded)`.
pub fn is_password_usable(encoded: Option<&str>) -> bool {
    encoded.is_none_or(|e| !e.starts_with(UNUSABLE_PASSWORD_PREFIX))
}

/// `make_password(password, salt, hasher)`; `None` makes an unusable password.
pub fn make_password(password: Option<&str>, salt: Option<&str>, hasher: &str) -> Result<String> {
    let Some(password) = password else {
        return Ok(format!("{UNUSABLE_PASSWORD_PREFIX}{}", get_random_string(UNUSABLE_PASSWORD_SUFFIX_LENGTH, RANDOM_STRING_CHARS)));
    };
    let h = get_hasher(hasher)?;
    let salt = salt.map_or_else(|| h.salt(), str::to_owned);
    h.encode(password, &salt)
}

/// `check_password(password, encoded)`; returns `(ok, must_update)` so the caller can re-hash.
pub fn check_password(password: Option<&str>, encoded: &str) -> (bool, bool) {
    let Some(password) = password else { return (false, false) };
    if !is_password_usable(Some(encoded)) {
        return (false, false);
    }
    let Ok(hasher) = identify_hasher(encoded) else { return (false, false) };
    let ok = hasher.verify(password, encoded).unwrap_or(false);
    let preferred = get_hasher("default").ok();
    let must_update = ok && (preferred.as_ref().is_some_and(|p| p.algorithm() != hasher.algorithm()) || hasher.must_update(encoded));
    (ok, must_update)
}
