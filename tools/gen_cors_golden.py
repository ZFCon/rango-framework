#!/usr/bin/env python3
"""Run requests through the real django-cors-headers middleware -> crates/rango/tests/golden/cors.json.

Case: {settings: {CORS_*: ...}, method, path, headers: {name: value}} -> {status, headers: {access-control-*, vary}}.
"""
import json, os

import django
from django.conf import settings

settings.configure(SECRET_KEY="x", ALLOWED_HOSTS=["testserver"], ROOT_URLCONF=__name__, MIDDLEWARE=["corsheaders.middleware.CorsMiddleware"])
django.setup()
from django.http import HttpResponse
from django.test import Client, override_settings
from django.urls import path, re_path

urlpatterns = [re_path(r"^.*$", lambda request: HttpResponse("ok"))]

O = "https://app.example.com"
CASES = []
def case(cfg, method="GET", path="/api/x/", **headers):
    CASES.append({"settings": cfg, "method": method, "path": path, "headers": headers})

case({})
case({}, origin=O)
case({"CORS_ALLOWED_ORIGINS": [O]}, origin=O)
case({"CORS_ALLOWED_ORIGINS": [O]}, origin="https://evil.example.com")
case({"CORS_ALLOWED_ORIGINS": [O + ":8443"]}, origin=O)
case({"CORS_ALLOWED_ORIGINS": ["null"]}, origin="null")
case({"CORS_ALLOWED_ORIGIN_REGEXES": [r"^https://\w+\.example\.com$"]}, origin=O)
case({"CORS_ALLOWED_ORIGIN_REGEXES": [r"^https://\w+\.example\.com$"]}, origin="https://a.b.example.com")
case({"CORS_ALLOW_ALL_ORIGINS": True}, origin=O)
case({"CORS_ALLOW_ALL_ORIGINS": True, "CORS_ALLOW_CREDENTIALS": True}, origin=O)
case({"CORS_ALLOWED_ORIGINS": [O], "CORS_ALLOW_CREDENTIALS": True, "CORS_EXPOSE_HEADERS": ["x-total", "x-page"]}, origin=O)
case({"CORS_ALLOWED_ORIGINS": [O]}, method="OPTIONS", origin=O, access_control_request_method="POST")
case({"CORS_ALLOWED_ORIGINS": [O]}, method="OPTIONS", origin="https://evil.example.com", access_control_request_method="POST")
case({"CORS_ALLOWED_ORIGINS": [O]}, method="OPTIONS", origin=O)
case({"CORS_ALLOWED_ORIGINS": [O], "CORS_PREFLIGHT_MAX_AGE": 0, "CORS_ALLOW_METHODS": ["GET"], "CORS_ALLOW_HEADERS": ["x-a", "x-b"]}, method="OPTIONS", origin=O, access_control_request_method="GET")
case({"CORS_ALLOWED_ORIGINS": [O], "CORS_URLS_REGEX": r"^/api/.*$"}, path="/other/", origin=O)
case({"CORS_ALLOWED_ORIGINS": [O], "CORS_URLS_REGEX": r"^/api/.*$"}, method="OPTIONS", path="/other/", origin=O, access_control_request_method="POST")
case({"CORS_ALLOWED_ORIGINS": [O], "CORS_ALLOW_PRIVATE_NETWORK": True}, origin=O, access_control_request_private_network="true")
case({"CORS_ALLOWED_ORIGINS": [O], "CORS_ALLOW_PRIVATE_NETWORK": True}, origin=O)

for c in CASES:
    with override_settings(**c["settings"]):
        extra = {"HTTP_" + k.upper(): v for k, v in c["headers"].items()}
        r = Client().generic(c["method"], c["path"], **extra)
        c["expected"] = {"status": r.status_code, "body": r.content.decode(), "headers": {k.lower(): v for k, v in r.headers.items() if k.lower().startswith("access-control") or k.lower() == "vary"}}

path_ = os.path.join(os.path.dirname(__file__), "..", "crates", "rango", "tests", "golden", "cors.json")
os.makedirs(os.path.dirname(path_), exist_ok=True)
json.dump(CASES, open(path_, "w"), indent=1)
print(len(CASES), "cases")
