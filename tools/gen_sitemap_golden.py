#!/usr/bin/env python3
"""Render Django's sitemap templates for a fixed context -> crates/rango-contrib/tests/golden/sitemaps.json."""
import datetime, json, os
import django
from django.conf import settings
tpl = os.path.join(os.path.dirname(django.__file__), "contrib", "sitemaps", "templates")
settings.configure(USE_TZ=True, TIME_ZONE="UTC", SECRET_KEY="x", TEMPLATES=[{"BACKEND": "django.template.backends.django.DjangoTemplates", "DIRS": [tpl], "OPTIONS": {}}])
django.setup()
from django.template.loader import render_to_string
utc = datetime.timezone.utc
urls = [
    {"location": "http://example.com/a/", "lastmod": datetime.datetime(2024, 1, 5, 12, 0, tzinfo=utc), "changefreq": "daily", "priority": "0.5", "alternates": []},
    {"location": "http://example.com/b/?x=1&y=2", "lastmod": None, "changefreq": None, "priority": "", "alternates": []},
]
index = [{"location": "http://example.com/sitemap-news.xml", "last_mod": datetime.datetime(2024, 3, 9, 8, 30, tzinfo=utc)}, {"location": "http://example.com/sitemap-x.xml?p=2", "last_mod": None}]
out = {"sitemap": render_to_string("sitemap.xml", {"urlset": urls}), "index": render_to_string("sitemap_index.xml", {"sitemaps": index})}
json.dump(out, open(os.path.join(os.path.dirname(__file__), "..", "crates", "rango-contrib", "tests", "golden", "sitemaps.json"), "w"), indent=1)
print("ok")
