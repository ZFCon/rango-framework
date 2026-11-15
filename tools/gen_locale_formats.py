#!/usr/bin/env python3
"""Extract every django/conf/locale/<lang>/formats.py into crates/rango-core/data/locale_formats.json."""
import importlib, json, os
import django
from django.conf import settings
settings.configure(SECRET_KEY="x")
django.setup()
import django.conf.locale as loc

NAMES = ["DATE_FORMAT", "DATETIME_FORMAT", "TIME_FORMAT", "YEAR_MONTH_FORMAT", "MONTH_DAY_FORMAT", "SHORT_DATE_FORMAT",
         "SHORT_DATETIME_FORMAT", "FIRST_DAY_OF_WEEK", "DECIMAL_SEPARATOR", "THOUSAND_SEPARATOR", "NUMBER_GROUPING",
         "DATE_INPUT_FORMATS", "TIME_INPUT_FORMATS", "DATETIME_INPUT_FORMATS"]
out = {}
base = os.path.dirname(loc.__file__)
for d in sorted(os.listdir(base)):
    if not os.path.isfile(os.path.join(base, d, "formats.py")):
        continue
    m = importlib.import_module(f"django.conf.locale.{d}.formats")
    out[d] = {n: getattr(m, n) for n in NAMES if hasattr(m, n)}
json.dump(out, open(os.path.join(os.path.dirname(__file__), "..", "crates", "rango-core", "data", "locale_formats.json"), "w"), ensure_ascii=False, separators=(",", ":"))
print(len(out), "locales")
