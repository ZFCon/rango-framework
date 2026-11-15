#!/usr/bin/env python3
"""Password hashes from real Django 5.2 (argon2/bcrypt need extra Python packages and are covered by round-trip tests only)."""
import json, os
import django
from django.conf import settings

settings.configure(SECRET_KEY="x", PASSWORD_HASHERS=[
    "django.contrib.auth.hashers.PBKDF2PasswordHasher", "django.contrib.auth.hashers.PBKDF2SHA1PasswordHasher",
    "django.contrib.auth.hashers.ScryptPasswordHasher", "django.contrib.auth.hashers.MD5PasswordHasher"])
django.setup()
from django.contrib.auth.hashers import make_password, check_password

cases = []
for algo in ["pbkdf2_sha256", "pbkdf2_sha1", "scrypt", "md5"]:
    for pw in ["secret", "pässwörd ✓"]:
        enc = make_password(pw, salt="fixedSalt123", hasher=algo)
        cases.append({"algo": algo, "password": pw, "salt": "fixedSalt123", "encoded": enc, "wrong_ok": check_password("nope", enc)})
json.dump(cases, open(os.path.join(os.path.dirname(__file__), "..", "crates", "rango-contrib", "tests", "golden", "hashers.json"), "w"), indent=1, ensure_ascii=False)
print(len(cases), "cases")
