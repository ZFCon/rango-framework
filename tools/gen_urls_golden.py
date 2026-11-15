#!/usr/bin/env python3
"""Generate crates/rango-urls/tests/golden/urls.json from real Django 5.2: resolve() and reverse()
results for a fixed URLconf (mirrored by crates/rango-urls/tests/urls_parity.rs)."""
import json
import pathlib
import types

import django
from django.conf import settings

settings.configure(ROOT_URLCONF="urlconf_oracle", SECRET_KEY="x", USE_I18N=False, DEBUG=True)

from django.urls import include, path, re_path, resolve, reverse, set_urlconf  # noqa: E402
from django.urls.exceptions import NoReverseMatch, Resolver404  # noqa: E402


def v(request, *a, **k):
    pass


polls = [path("", v, name="index"), path("<int:pk>/", v, name="detail"), path("<int:pk>/vote/", v, name="vote")]
mod = types.ModuleType("urlconf_oracle")
mod.urlpatterns = [
    path("", v, name="index"),
    path("articles/<int:year>/", v, name="year"),
    path("articles/<int:year>/<slug:slug>/", v, name="article"),
    path("files/<path:p>", v, name="file"),
    path("u/<uuid:id>/", v, name="user"),
    re_path(r"^re/(?P<a>\d+)/(?P<b>[a-z]+)/$", v, name="re-named"),
    re_path(r"^pos/(\d+)/(\w+)/$", v, name="re-pos"),
    re_path(r"^opt/(?:(?P<x>\d+)/)?end/$", v, name="re-opt"),
    path("polls/", include((polls, "polls"), namespace="polls")),
    path("polls2/", include((polls, "polls"), namespace="polls2")),
    path("api/<str:version>/", include([path("items/", v, name="items"), path("items/<int:id>/", v, name="item")])),
    path("kw/", v, {"extra": 1}, name="kw"),
    path("outer/", include(([path("inner/", include(([path("leaf/<int:n>/", v, name="leaf")], "inner"), namespace="inner"))], "outer"), namespace="outer")),
]
import sys  # noqa: E402

sys.modules["urlconf_oracle"] = mod
django.setup()

OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/rango-urls/tests/golden/urls.json"
cases = []


def conv(x):
    return x if isinstance(x, (int, str)) else str(x)


for p in ["/", "/articles/2024/", "/articles/2024/hello-world/", "/articles/abc/", "/articles/2024/bad slug/", "/files/a/b/c.txt", "/files/",
          "/u/123e4567-e89b-12d3-a456-426614174000/", "/u/NOT-A-UUID/", "/re/12/abc/", "/re/12/ABC/", "/pos/1/x/", "/opt/end/", "/opt/5/end/",
          "/polls/", "/polls/3/", "/polls/3/vote/", "/polls2/9/", "/api/v1/items/", "/api/v1/items/3/", "/kw/", "/outer/inner/leaf/4/", "/nope/",
          "/polls/x/", "//x"]:
    try:
        m = resolve(p)
        exp = {"url_name": m.url_name, "app_names": m.app_names, "namespaces": m.namespaces, "view_name": m.view_name, "route": m.route,
               "args": [conv(a) for a in m.args], "kwargs": {k: conv(x) for k, x in m.kwargs.items()},
               "captured": {k: conv(x) for k, x in m.captured_kwargs.items()}, "extra": {k: conv(x) for k, x in m.extra_kwargs.items()}}
    except Resolver404:
        exp = {"error": "Resolver404"}
    cases.append({"kind": "resolve", "path": p, "expected": exp})

REV = [
    ("index", [], {}, None), ("year", [], {"year": 2024}, None), ("year", [2024], {}, None), ("article", [], {"year": 1, "slug": "a-b"}, None),
    ("article", [], {"year": "x", "slug": "a-b"}, None), ("article", [1], {}, None), ("file", [], {"p": "a/b c"}, None),
    ("user", [], {"id": "123e4567-e89b-12d3-a456-426614174000"}, None), ("user", [], {"id": "nope"}, None),
    ("re-named", [], {"a": 1, "b": "x"}, None), ("re-named", [], {"a": "x", "b": "x"}, None), ("re-pos", [1, "ab"], {}, None),
    ("re-opt", [], {}, None), ("re-opt", [], {"x": 3}, None),
    ("polls:index", [], {}, None), ("polls:detail", [], {"pk": 1}, None), ("polls:detail", [], {"pk": 1}, "polls2"), ("polls2:vote", [2], {}, None),
    ("polls:detail", [], {"pk": 1}, "polls"), ("items", [], {"version": "v2"}, None), ("item", [], {"version": "v1", "id": 3}, None),
    ("kw", [], {}, None), ("kw", [], {"extra": 2}, None), ("outer:inner:leaf", [], {"n": 5}, None), ("nope", [], {}, None),
    ("polls:nope", [], {}, None), ("zzz:index", [], {}, None), ("index", [1], {"a": 2}, None), ("year", [], {}, None), ("year", [], {"year": 1, "x": 2}, None),
    ("file", [], {"p": "é/ü"}, None), ("article", [], {"year": 5, "slug": "x y"}, None),
]
for name, args, kwargs, app in REV:
    try:
        exp = reverse(name, args=args or None, kwargs=kwargs or None, current_app=app)
    except (NoReverseMatch, ValueError) as e:
        exp = {"error": type(e).__name__}
    cases.append({"kind": "reverse", "name": name, "args": [str(a) for a in args], "kwargs": {k: str(x) for k, x in kwargs.items()}, "current_app": app, "expected": exp})

OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(cases, ensure_ascii=False, indent=1) + "\n")
print(len(cases), "cases ->", OUT)
