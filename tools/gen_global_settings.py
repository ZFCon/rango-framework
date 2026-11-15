#!/usr/bin/env python3
"""Generate rango-core's global settings table from Django 5.2's global_settings.py.

Usage:  PYTHONPATH=<dir with django 5.2> python3 tools/gen_global_settings.py

Writes:
  crates/rango-core/src/conf/global_settings.rs   (struct + Default, generated)
  crates/rango-core/tests/golden/global_settings.json   (rebranded Django defaults)

Rebranding: dotted paths "django.x.y" -> "rango.x.y" (they name registry entries
in Rango, not Python imports), plus a few user-visible strings. Anything that
must stay byte-compatible with Django (signing salts, cookie names that matter
for interop) is NOT a setting default and is untouched.
"""
import json
import pathlib

from django.conf import global_settings as g

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT_RS = ROOT / "crates/rango-core/src/conf/global_settings.rs"
OUT_JSON = ROOT / "crates/rango-core/tests/golden/global_settings.json"

SPECIAL = {
    "[Django] ": "[Rango] ",
    "django_language": "rango_language",
    "logging.config.dictConfig": "rango.utils.log.configure_logging",
}

# name -> (rust type, rust default expr). Used where the Python default alone
# (None, {}, [], int-vs-tuple) does not determine a sensible Rust type.
OVERRIDES = {
    "ABSOLUTE_URL_OVERRIDES": ("Json", "json!({})"),
    "ADMINS": ("Vec<(String, String)>", "vec![]"),
    "MANAGERS": ("Vec<(String, String)>", "vec![]"),
    "AUTH_PASSWORD_VALIDATORS": ("Vec<Json>", "vec![]"),
    "TEMPLATES": ("Vec<Json>", "vec![]"),
    "DATABASES": ("BTreeMap<String, DatabaseConfig>", "BTreeMap::new()"),
    "CACHES": ("Json", None),
    "STORAGES": ("Json", None),
    "LOGGING": ("Json", "json!({})"),
    "MIGRATION_MODULES": ("Json", "json!({})"),
    "NUMBER_GROUPING": ("Json", "json!(0)"),
    "CSRF_COOKIE_DOMAIN": ("Option<String>", "None"),
    "EMAIL_SSL_CERTFILE": ("Option<String>", "None"),
    "EMAIL_SSL_KEYFILE": ("Option<String>", "None"),
    "EMAIL_TIMEOUT": ("Option<f64>", "None"),
    "EMAIL_PORT": ("u16", None),
    "FILE_UPLOAD_DIRECTORY_PERMISSIONS": ("Option<u32>", "None"),
    "FILE_UPLOAD_PERMISSIONS": ("Option<u32>", "Some(0o644)"),
    "FILE_UPLOAD_TEMP_DIR": ("Option<String>", "None"),
    "FIRST_DAY_OF_WEEK": ("u8", None),
    "FORCE_SCRIPT_NAME": ("Option<String>", "None"),
    "FORMAT_MODULE_PATH": ("Option<String>", "None"),
    "LANGUAGE_COOKIE_AGE": ("Option<i64>", "None"),
    "LANGUAGE_COOKIE_DOMAIN": ("Option<String>", "None"),
    "LANGUAGE_COOKIE_SAMESITE": ("Option<String>", "None"),
    "LOGOUT_REDIRECT_URL": ("Option<String>", "None"),
    "SECURE_PROXY_SSL_HEADER": ("Option<(String, String)>", "None"),
    "SECURE_SSL_HOST": ("Option<String>", "None"),
    "SESSION_COOKIE_DOMAIN": ("Option<String>", "None"),
    "SESSION_FILE_PATH": ("Option<String>", "None"),
    "STATIC_ROOT": ("Option<String>", "None"),
    "STATIC_URL": ("Option<String>", "None"),
    "WSGI_APPLICATION": ("Option<String>", "None"),
}


def rebrand(v):
    if isinstance(v, str):
        if v in SPECIAL:
            return SPECIAL[v]
        v = v.replace("DjangoTemplates", "RangoTemplates").replace("django_language", "rango_language")
        return "rango." + v[len("django."):] if v.startswith("django.") else v
    if isinstance(v, (list, tuple)):
        return [rebrand(x) for x in v]
    if isinstance(v, dict):
        return {k: rebrand(x) for k, x in v.items()}
    return v


def rs_str(s):
    return json.dumps(s, ensure_ascii=False)  # JSON string literal == valid Rust string literal here


def default_expr(name, v):
    ov = OVERRIDES.get(name)
    if ov and ov[1] is not None:
        return ov[1]
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, str):
        return f"{rs_str(v)}.into()"
    if isinstance(v, (list, tuple)):
        if v and isinstance(v[0], (list, tuple)):
            return "pairs(&[" + ", ".join(f"({rs_str(a)}, {rs_str(b)})" for a, b in v) + "])"
        return "vs([" + ", ".join(rs_str(x) for x in v) + "])"
    if isinstance(v, dict):
        return "json!(" + json.dumps(v) + ")"
    if v is None:
        raise SystemExit(f"{name}: None default needs an OVERRIDES entry")
    raise SystemExit(f"{name}: unsupported default {v!r}")


def rust_type(name, v):
    if name in OVERRIDES:
        return OVERRIDES[name][0]
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, int):
        return "i64"
    if isinstance(v, str):
        return "String"
    if isinstance(v, (list, tuple)):
        return "Vec<(String, String)>" if v and isinstance(v[0], (list, tuple)) else "Vec<String>"
    if isinstance(v, dict):
        return "Json"
    raise SystemExit(f"{name}: unsupported type")


# Settings Django reads with getattr() but does not define in global_settings.py.
EXTRA = {
    "ROOT_URLCONF": ("Option<String>", "None"),
    "ASGI_APPLICATION": ("Option<String>", "None"),
    "SITE_ID": ("Option<i64>", "None"),
    "MESSAGE_LEVEL": ("Option<i64>", "None"),
    "MESSAGE_TAGS": ("Option<Json>", "None"),
}
OVERRIDES.update(EXTRA)
# Settings of the bundled ports of django-cors-headers, DRF and drf-spectacular; CORS defaults come from the package.
from django.conf import settings as _dj_settings

_dj_settings.configure()
from corsheaders.conf import conf as _cors

EXTRA_VALUES = {n: (list(getattr(_cors, n)) if isinstance(getattr(_cors, n), tuple) else getattr(_cors, n)) for n in dir(type(_cors)) if n.startswith("CORS_")}
EXTRA_VALUES["REST_FRAMEWORK"] = {}
EXTRA_VALUES["SPECTACULAR_SETTINGS"] = {}
for _n in ("CORS_ALLOWED_ORIGINS", "CORS_ALLOWED_ORIGIN_REGEXES", "CORS_EXPOSE_HEADERS"):
    OVERRIDES[_n] = ("Vec<String>", "vs([])")
names = sorted([k for k in dir(g) if k.isupper()] + list(EXTRA) + list(EXTRA_VALUES))
golden, fields, defaults = {}, [], []
for n in names:
    v = EXTRA_VALUES[n] if n in EXTRA_VALUES else (rebrand(getattr(g, n)) if n not in EXTRA else None)
    golden[n] = json.loads(json.dumps(v))
    fields.append(f'    #[serde(rename = "{n}")]\n    pub {n.lower()}: {rust_type(n, v)},')
    defaults.append(f"            {n.lower()}: {default_expr(n, v)},")

OUT_RS.parent.mkdir(parents=True, exist_ok=True)
OUT_RS.write_text(
    "// @generated by tools/gen_global_settings.py. Do not edit by hand.\n"
    "// Re-run the generator to change defaults.\n\n"
    "use std::collections::BTreeMap;\n\n"
    "use serde::{Deserialize, Serialize};\n"
    "use serde_json::json;\n\n"
    "use super::{DatabaseConfig, Json, pairs, vs};\n\n"
    "/// Every setting, snake_cased, with its default. Serializes under the UPPER_CASE name.\n"
    "#[derive(Debug, Clone, Serialize, Deserialize)]\n"
    "#[serde(deny_unknown_fields)]\n"
    "pub struct Settings {\n" + "\n".join(fields) + "\n}\n\n"
    "impl Default for Settings {\n    fn default() -> Self {\n        Self {\n" + "\n".join(defaults) + "\n        }\n    }\n}\n"
)
OUT_JSON.parent.mkdir(parents=True, exist_ok=True)
OUT_JSON.write_text(json.dumps(golden, indent=1, sort_keys=True) + "\n")
print(f"{len(names)} settings -> {OUT_RS.relative_to(ROOT)}, {OUT_JSON.relative_to(ROOT)}")
