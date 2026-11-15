//! `utils.crypto`.

use std::str::FromStr;

use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use subtle::ConstantTimeEq;

pub const RANDOM_STRING_CHARS: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Algorithm is not supported (`InvalidAlgorithm`).
#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not an algorithm accepted by the hashlib module.")]
pub struct InvalidAlgorithm(pub String);

/// The `hashlib` algorithms the signing/hashers use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl FromStr for HashAlgorithm {
    type Err = InvalidAlgorithm;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "md5" => Self::Md5,
            "sha1" => Self::Sha1,
            "sha224" => Self::Sha224,
            "sha256" => Self::Sha256,
            "sha384" => Self::Sha384,
            "sha512" => Self::Sha512,
            other => return Err(InvalidAlgorithm(other.to_owned())),
        })
    }
}

macro_rules! dispatch {
    ($alg:expr, $d:ident => $body:expr) => {
        match $alg {
            HashAlgorithm::Md5 => {
                type $d = md5::Md5;
                $body
            }
            HashAlgorithm::Sha1 => {
                type $d = sha1::Sha1;
                $body
            }
            HashAlgorithm::Sha224 => {
                type $d = sha2::Sha224;
                $body
            }
            HashAlgorithm::Sha256 => {
                type $d = sha2::Sha256;
                $body
            }
            HashAlgorithm::Sha384 => {
                type $d = sha2::Sha384;
                $body
            }
            HashAlgorithm::Sha512 => {
                type $d = sha2::Sha512;
                $body
            }
        }
    };
}

impl HashAlgorithm {
    pub fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }

    /// `hashlib.<alg>(data).digest()`.
    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        use sha2::Digest;
        dispatch!(self, D => D::digest(data).to_vec())
    }

    /// `hmac.new(key, msg, <alg>).digest()`.
    pub fn hmac(self, key: &[u8], msg: &[u8]) -> Vec<u8> {
        dispatch!(self, D => {
            let mut mac = <Hmac<D> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
            mac.update(msg);
            mac.finalize().into_bytes().to_vec()
        })
    }
}

/// `salted_hmac(key_salt, value, secret, algorithm=...).digest()`.
///
/// The HMAC key is `digest(key_salt + secret)`; use a different `key_salt` per application.
pub fn salted_hmac(
    key_salt: &[u8],
    value: &[u8],
    secret: &[u8],
    algorithm: HashAlgorithm,
) -> Vec<u8> {
    let mut material = Vec::with_capacity(key_salt.len() + secret.len());
    material.extend_from_slice(key_salt);
    material.extend_from_slice(secret);
    let key = algorithm.digest(&material);
    algorithm.hmac(&key, value)
}

/// Securely generated random string (`get_random_string`).
pub fn get_random_string(length: usize, allowed_chars: &str) -> String {
    let chars: Vec<char> = allowed_chars.chars().collect();
    let mut rng = rand::rng();
    (0..length)
        .map(|_| chars[rng.random_range(0..chars.len())])
        .collect()
}

/// `constant_time_compare`.
pub fn constant_time_compare(a: impl AsRef<[u8]>, b: impl AsRef<[u8]>) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// `pbkdf2(password, salt, iterations, dklen=0, digest=sha256)`; `dklen == 0` means digest size.
pub fn pbkdf2(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    dklen: usize,
    digest: HashAlgorithm,
) -> Vec<u8> {
    let len = if dklen == 0 {
        digest.digest(b"").len()
    } else {
        dklen
    };
    let mut out = vec![0u8; len];
    dispatch!(digest, D => {
        pbkdf2::pbkdf2_hmac::<D>(password, salt, iterations, &mut out);
    });
    out
}
