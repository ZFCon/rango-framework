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

Q_, C_, A_, B_ = Question.objects, Choice.objects, Author.objects, Book.objects
OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/rango-db/tests/golden/sql.json"
cases = []

def case(name, qs=None, sql=None):
    try:
        s, p = (qs.query.sql_with_params() if sql is None else sql)
        p = [x.isoformat(" ") if hasattr(x, "isoformat") and not isinstance(x, datetime.date) or isinstance(x, datetime.datetime) else (str(x) if isinstance(x, (datetime.date,)) else x) for x in p]
        exp = {"sql": s, "params": [str(x) if not isinstance(x, (int, float, str, type(None))) else x for x in p]}
    except Exception as e:  # noqa
        exp = {"error": type(e).__name__}
    cases.append({"name": name, "expected": exp})

case("all", Q_.all())
case("filter_exact", Q_.filter(id=1))
case("filter_two", Q_.filter(question_text="a", id=2))
case("filter_chain", Q_.filter(question_text="a").filter(id=2))
case("startswith", Q_.filter(question_text__startswith="What"))
case("icontains", Q_.filter(question_text__icontains="x%y_"))
case("contains", Q_.filter(question_text__contains="x"))
case("endswith", Q_.filter(question_text__endswith="x"))
case("iexact", Q_.filter(question_text__iexact="x"))
case("in_list", Q_.filter(id__in=[1, 2, 3]))
case("range", Q_.filter(id__range=(1, 5)))
case("isnull", A_.filter(age__isnull=True))
case("isnull_false", A_.filter(age__isnull=False))
case("exclude", Q_.exclude(question_text="a"))
case("exclude_nullable", A_.exclude(age=3))
case("gt_lte", C_.filter(votes__gt=1, votes__lte=9))
case("q_or", Q_.filter(Q(question_text="a") | Q(id=1)))
case("q_and_or", Q_.filter(Q(question_text="a") & (Q(id=1) | Q(id=2))))
case("q_not", Q_.filter(~Q(id=1)))
case("order_desc", Q_.order_by("-pub_date"))
case("order_two", Q_.order_by("question_text", "-id"))
case("order_pk", Q_.order_by("pk"))
case("slice", Q_.order_by("id")[2:5])
case("slice_limit", Q_.all()[:3])
case("slice_offset", Q_.all()[3:])
case("distinct", Q_.distinct())
case("values", Q_.values("id", "question_text"))
case("values_list", Q_.values_list("id", flat=True))
case("only", Q_.only("question_text"))
case("defer", Q_.defer("pub_date"))
case("fk_filter", C_.filter(question__question_text="x"))
case("fk_filter_id", C_.filter(question=3))
case("fk_filter_id_attr", C_.filter(question_id=3))
case("fk_filter_pk", C_.filter(question__id=3))
case("fk_two_fields", C_.filter(question__question_text="x", question__id=3))
case("reverse_filter", Q_.filter(choice__votes__gt=2))
case("reverse_two_filters", Q_.filter(choice__votes__gt=2).filter(choice__choice_text="a"))
case("reverse_same_filter", Q_.filter(choice__votes__gt=2, choice__choice_text="a"))
case("m2m_filter", B_.filter(authors__name="x"))
case("m2m_reverse", A_.filter(book__title="x"))
case("select_related", C_.select_related("question"))
case("select_related_order", C_.select_related("question").order_by("question__pub_date"))
case("annotate_count", Q_.annotate(n=Count("choice")))
case("annotate_filter_having", Q_.annotate(n=Count("choice")).filter(n__gt=1))
case("annotate_values", Q_.values("id").annotate(n=Count("choice")).order_by("-n"))
case("f_compare", C_.filter(votes__gt=F("question_id")))
case("f_arith", C_.filter(votes__gt=F("question_id") * 2))
case("year_exact", Q_.filter(pub_date__year=2024))
case("year_gte", Q_.filter(pub_date__year__gte=2024))
case("datetime_gt", Q_.filter(pub_date__gt=datetime.datetime(2024, 1, 1, 12, 30, tzinfo=datetime.timezone.utc)))
case("date_exact", B_.filter(published=datetime.date(2024, 3, 1)))
case("decimal", B_.filter(price__gt=__import__("decimal").Decimal("9.99")))
case("exists_subquery", Q_.filter(Exists(C_.filter(question=OuterRef("pk"), votes__gt=1))))
case("in_subquery", Q_.filter(id__in=C_.filter(votes__gt=1).values("question_id")))
case("union", A_.filter(name="a").union(A_.filter(name="b")))
case("regex", Q_.filter(question_text__regex=r"^W"))
case("none_exact", A_.filter(age=None))
case("pk_in", Q_.filter(pk__in=[1, 2]))
case("order_related", C_.order_by("question__question_text"))

# count / exists SQL
q = Q_.filter(id__gt=1).query.clone()
case("exists_sql", None, sql=q.exists().get_compiler("default").as_sql())
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(cases, indent=1) + "\n")
print(len(cases), "cases")
