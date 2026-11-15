#!/usr/bin/env python3
"""Drive a small API through real Django REST framework -> crates/rango-rest/tests/golden/rest.json.

The Rust test (crates/rango-rest/tests/rest_parity.rs) builds the same API and replays the same requests.
"""
import json, os

import django
from django.conf import settings

settings.configure(
    SECRET_KEY="x", ALLOWED_HOSTS=["testserver"], ROOT_URLCONF=__name__, USE_TZ=True,
    INSTALLED_APPS=["django.contrib.contenttypes", "django.contrib.auth", "rest_framework"],
    DATABASES={"default": {"ENGINE": "django.db.backends.sqlite3", "NAME": ":memory:"}},
    PASSWORD_HASHERS=["django.contrib.auth.hashers.MD5PasswordHasher"],
    REST_FRAMEWORK={"DEFAULT_RENDERER_CLASSES": ["rest_framework.renderers.JSONRenderer"]},
)
django.setup()
from django.contrib.auth.models import User
from django.core.management import call_command
from django.db import connection, models
from django.test import Client
from django.urls import include, path
from rest_framework import filters, pagination, permissions, routers, serializers, viewsets
from rest_framework.authentication import BasicAuthentication


class Article(models.Model):
    title = models.CharField(max_length=20)
    slug = models.SlugField(unique=True)
    body = models.TextField(blank=True)
    published = models.BooleanField(default=False)
    views = models.IntegerField(default=0)
    rating = models.DecimalField(max_digits=4, decimal_places=2, null=True, blank=True)

    class Meta:
        app_label = "blog"
        ordering = ["id"]


class ArticleSerializer(serializers.ModelSerializer):
    class Meta:
        model = Article
        fields = "__all__"


class Small(pagination.PageNumberPagination):
    page_size = 2


class ArticleViewSet(viewsets.ModelViewSet):
    queryset = Article.objects.all()
    serializer_class = ArticleSerializer
    authentication_classes = []
    filter_backends = [filters.SearchFilter, filters.OrderingFilter]
    search_fields = ["title", "body"]
    ordering_fields = ["views", "title"]


class PagedViewSet(viewsets.ReadOnlyModelViewSet):
    queryset = Article.objects.all()
    serializer_class = ArticleSerializer
    authentication_classes = []
    pagination_class = Small


class PrivateViewSet(viewsets.ReadOnlyModelViewSet):
    queryset = Article.objects.all()
    serializer_class = ArticleSerializer
    authentication_classes = [BasicAuthentication]
    permission_classes = [permissions.IsAuthenticated]


router = routers.DefaultRouter()
router.register("articles", ArticleViewSet)
router.register("paged", PagedViewSet, basename="paged")
router.register("private", PrivateViewSet, basename="private")
urlpatterns = [path("api/", include(router.urls))]

call_command("migrate", verbosity=0)
with connection.schema_editor() as ed:
    ed.create_model(Article)
User.objects.create_user("ann", password="pw")

import base64
AUTH = "Basic " + base64.b64encode(b"ann:pw").decode()
BAD = "Basic " + base64.b64encode(b"ann:nope").decode()
J = "application/json"
STEPS = [
    ["GET", "/api/", None, {}],
    ["GET", "/api/articles/", None, {}],
    ["POST", "/api/articles/", {"title": "First", "slug": "first", "body": "hello world", "views": 5, "rating": "4.5"}, {}],
    ["POST", "/api/articles/", {"title": "Second", "slug": "second", "published": True, "views": "7"}, {}],
    ["POST", "/api/articles/", {"title": "Third post", "slug": "third", "body": "more words", "rating": None}, {}],
    ["POST", "/api/articles/", {}, {}],
    ["POST", "/api/articles/", {"title": "", "slug": "bad slug!", "views": "x", "published": "maybe", "rating": "123.456"}, {}],
    ["POST", "/api/articles/", {"title": "x" * 21, "slug": "first", "views": 1.5, "rating": "abc", "body": None}, {}],
    ["POST", "/api/articles/", [1, 2], {}],
    ["GET", "/api/articles/1/", None, {}],
    ["GET", "/api/articles/99/", None, {}],
    ["PUT", "/api/articles/1/", {"title": "First!", "slug": "first", "views": 6}, {}],
    ["PUT", "/api/articles/1/", {"views": 6}, {}],
    ["PATCH", "/api/articles/2/", {"views": 70}, {}],
    ["PATCH", "/api/articles/2/", {"slug": "first"}, {}],
    ["GET", "/api/articles/?search=words", None, {}],
    ["GET", "/api/articles/?ordering=-views", None, {}],
    ["GET", "/api/articles/?ordering=title&search=i", None, {}],
    ["GET", "/api/paged/", None, {}],
    ["GET", "/api/paged/?page=2", None, {}],
    ["GET", "/api/paged/?page=9", None, {}],
    ["POST", "/api/paged/", {"title": "no"}, {}],
    ["GET", "/api/private/", None, {}],
    ["GET", "/api/private/", None, {"Authorization": BAD}],
    ["GET", "/api/private/2/", None, {"Authorization": AUTH}],
    ["DELETE", "/api/articles/3/", None, {}],
    ["DELETE", "/api/articles/3/", None, {}],
    ["GET", "/api/articles/", None, {}],
]

out = []
c = Client()
for method, url, body, headers in STEPS:
    extra = {"HTTP_" + k.upper().replace("-", "_"): v for k, v in headers.items()}
    r = c.generic(method, url, json.dumps(body) if body is not None else "", content_type=J, **extra)
    out.append({"method": method, "url": url, "body": body, "headers": headers, "status": r.status_code, "content": r.content.decode(),
                "content_type": r.headers.get("Content-Type"), "www_authenticate": r.headers.get("WWW-Authenticate"), "allow": r.headers.get("Allow")})

p = os.path.join(os.path.dirname(__file__), "..", "crates", "rango-rest", "tests", "golden", "rest.json")
os.makedirs(os.path.dirname(p), exist_ok=True)
json.dump(out, open(p, "w"), indent=1, ensure_ascii=False)
for o in out:
    print(o["method"], o["url"], o["status"], o["content"][:230], o["www_authenticate"] or "", o["allow"] or "")
