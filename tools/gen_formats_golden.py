#!/usr/bin/env python3
"""get_format() and numberformat.format() from real Django -> crates/rango-core/tests/golden/formats.json."""
import json, os
import django
from django.conf import settings
settings.configure(SECRET_KEY="x", USE_THOUSAND_SEPARATOR=False)
django.setup()
from django.utils import formats, numberformat, translation

names = ["DATE_FORMAT", "DATETIME_FORMAT", "TIME_FORMAT", "YEAR_MONTH_FORMAT", "MONTH_DAY_FORMAT", "SHORT_DATE_FORMAT", "SHORT_DATETIME_FORMAT",
         "FIRST_DAY_OF_WEEK", "DECIMAL_SEPARATOR", "THOUSAND_SEPARATOR", "NUMBER_GROUPING", "DATE_INPUT_FORMATS", "TIME_INPUT_FORMATS", "DATETIME_INPUT_FORMATS"]
out = {"formats": [], "numbers": []}
for lang in ["en", "en-us", "en-gb", "de", "fr", "cs", "pt-br", "ja", "es-ar", "hi", "xx"]:
    for n in names:
        v = formats.get_format(n, lang=lang, use_l10n=True)
        out["formats"].append({"lang": lang, "name": n, "value": list(v) if isinstance(v, (list, tuple)) else v})
for lang in ["en", "de", "fr", "hi"]:
    for num, pos, force in [("1234567.891", None, True), ("1234567.891", 2, True), ("-0.5", 3, False), ("1234567", 0, True), ("12", None, True), ("-1234.5678", 1, True)]:
        sep = formats.get_format("DECIMAL_SEPARATOR", lang=lang, use_l10n=True)
        tsep = formats.get_format("THOUSAND_SEPARATOR", lang=lang, use_l10n=True)
        grp = formats.get_format("NUMBER_GROUPING", lang=lang, use_l10n=True)
        from decimal import Decimal
        out["numbers"].append({"lang": lang, "number": num, "pos": pos, "force": force, "expected": numberformat.format(Decimal(num), sep, pos, grp, tsep, force)})
json.dump(out, open(os.path.join(os.path.dirname(__file__), "..", "crates", "rango-core", "tests", "golden", "formats.json"), "w"), indent=1, ensure_ascii=False)
print(len(out["formats"]), len(out["numbers"]))
