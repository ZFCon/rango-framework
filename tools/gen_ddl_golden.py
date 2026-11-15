#!/usr/bin/env python3
"""Generate crates/rango-db/tests/golden/sql.json: Django 5.2's SQLite SQL for fixed querysets
(mirrored case-by-case in crates/rango-db/tests/sql_parity.rs)."""
import json, pathlib, sys, types
import django
from django.conf import settings

import tempfile, os, textwrap  # noqa: E402

_tmp = tempfile.mkdtemp()
os.makedirs(os.path.join(_tmp, "polls"))
open(os.path.join(_tmp, "polls", "__init__.py"), "w").close()
open(os.path.join(_tmp, "polls", "models.py"), "w").write(textwrap.dedent("""
    from django.db import models

    class Question(models.Model):
        question_text = models.CharField(max_length=200)
        pub_date = models.DateTimeField("date published")

    class Choice(models.Model):
        question = models.ForeignKey(Question, on_delete=models.CASCADE)
        choice_text = models.CharField(max_length=200)
        votes = models.IntegerField(default=0)

    class Author(models.Model):
        name = models.CharField(max_length=100)
        age = models.IntegerField(null=True)

    class Book(models.Model):
        title = models.CharField(max_length=200)
        authors = models.ManyToManyField(Author)
        published = models.DateField()
        price = models.DecimalField(max_digits=6, decimal_places=2)
"""))
sys.path.insert(0, _tmp)
settings.configure(
    SECRET_KEY="x", USE_TZ=True, TIME_ZONE="UTC", USE_I18N=False,
    DATABASES={"default": {"ENGINE": "django.db.backends.sqlite3", "NAME": ":memory:"}},
    INSTALLED_APPS=["polls"],
)
django.setup()
from django.db import models  # noqa: E402
from django.db.models import Count, F, Q, Sum, Max, Avg, Exists, OuterRef  # noqa: E402
from polls.models import Question, Choice, Author, Book  # noqa: E402
import datetime  # noqa: E402


OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/rango-db/tests/golden/ddl_sqlite.json"
from django.db import connection  # noqa: E402
with connection.schema_editor(collect_sql=True) as editor:
    for m in [Question, Choice, Author, Book]:
        editor.create_model(m)
sql = [s for s in editor.collected_sql]
OUT.write_text(json.dumps(sql, indent=1) + "\n")
print(len(sql), "statements")
