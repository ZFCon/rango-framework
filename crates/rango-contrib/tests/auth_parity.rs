//! Password hashes: byte-identical to the reference implementation's, and externally made hashes verify.

use rango_contrib::auth::hashers::{check_password, get_hasher, make_password};
use rango_core::conf::{self, Settings};

#[test]
fn hashers_match_rango() {
    conf::configure(Settings {
        secret_key: "x".into(),
        password_hashers: ["PBKDF2PasswordHasher", "PBKDF2SHA1PasswordHasher", "ScryptPasswordHasher", "MD5PasswordHasher", "Argon2PasswordHasher", "BCryptSHA256PasswordHasher", "BCryptPasswordHasher"].map(|n| format!("rango.contrib.auth.hashers.{n}")).to_vec(),
        ..Settings::default()
    })
    .unwrap();
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("golden/hashers.json")).unwrap();
    for c in &cases {
        let (algo, pw, salt, enc) = (c["algo"].as_str().unwrap(), c["password"].as_str().unwrap(), c["salt"].as_str().unwrap(), c["encoded"].as_str().unwrap());
        assert_eq!(make_password(Some(pw), Some(salt), algo).unwrap(), enc, "encode {algo}");
        // externally made hashes verify (default hasher is pbkdf2_sha256, so other algorithms ask for an upgrade).
        let (ok, must_update) = check_password(Some(pw), enc);
        assert!(ok, "verify {algo}");
        assert_eq!(must_update, algo != "pbkdf2_sha256", "must_update {algo}");
        if pw == "secret" {
            assert!(!check_password(Some("nope"), enc).0);
        }
    }
    // Round trips for the algorithms with no golden hash here.
    for algo in ["argon2", "bcrypt_sha256", "bcrypt"] {
        let enc = make_password(Some("hunter2"), None, algo).unwrap();
        assert!(enc.starts_with(&format!("{algo}$")), "{enc}");
        assert!(check_password(Some("hunter2"), &enc).0 && !check_password(Some("hunter3"), &enc).0, "{algo}");
        assert_eq!(get_hasher(algo).unwrap().algorithm(), algo);
    }
    assert!(!check_password(Some("x"), &make_password(None, None, "default").unwrap()).0);
}
