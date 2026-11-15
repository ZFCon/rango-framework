#!/usr/bin/env python3
"""Extract Django's security check messages (id -> level/msg/hint) into crates/rango-core/data/security_checks.json."""
import json, os, re
import django
from django.conf import settings
settings.configure(SECRET_KEY="x")
django.setup()
from django.core.checks.security import base, csrf, sessions

def rebrand(s):
    s = re.sub(r"django\.(middleware|contrib)", r"rango.\1", s)
    return s.replace("django-insecure-", "rango-insecure-").replace("generated automatically by Django", "generated automatically by Rango").replace("Django's security-critical", "Rango's security-critical")

out = {}
for mod in (base, csrf, sessions):
    for name, v in vars(mod).items():
        if hasattr(v, "id") and hasattr(v, "msg") and v.id and v.id.startswith("security."):
            out[v.id] = {"level": v.level, "msg": rebrand(v.msg), "hint": rebrand(v.hint) if v.hint else None}
path = os.path.join(os.path.dirname(__file__), "..", "crates", "rango-core", "data", "security_checks.json")
json.dump(dict(sorted(out.items())), open(path, "w"), indent=1, ensure_ascii=False)
print(len(out), "messages")
