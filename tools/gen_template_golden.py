#!/usr/bin/env python3
"""Generate crates/rango-template/tests/golden/templates.json from Django 5.2's template engine."""
import datetime, json, pathlib
import django
from django.conf import settings

settings.configure(SECRET_KEY="x", USE_TZ=False, TIME_ZONE="UTC", USE_I18N=False, STATIC_URL="/static/", INSTALLED_APPS=[])
django.setup()
from django.template import Context, Engine, TemplateSyntaxError  # noqa: E402

SHARED = {
    "base.html": "<{% block a %}A{% endblock %}|{% block b %}B{% endblock %}>",
    "child.html": "{% extends 'base.html' %}{% block a %}[{{ block.super }}x]{% endblock %}",
    "grand.html": "{% extends 'child.html' %}{% block a %}<<{{ block.super }}>>{% endblock %}{% block b %}G{% endblock %}",
    "inc.html": "inc:{{ v }}:{{ x|default:'-' }}",
    "list_item.html": "<li>{{ item }}</li>",
}
OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/rango-template/tests/golden/templates.json"
engine = lambda autoescape=True: Engine(  # noqa: E731
    debug=False, autoescape=autoescape,
    loaders=[("django.template.loaders.locmem.Loader", SHARED)], builtins=[],
    libraries={"static": "django.templatetags.static"},
)

def conv(x):
    if isinstance(x, dict):
        if "__date__" in x: return datetime.date.fromisoformat(x["__date__"])
        if "__datetime__" in x: return datetime.datetime.fromisoformat(x["__datetime__"])
        if "__time__" in x: return datetime.time.fromisoformat(x["__time__"])
        if "__safe__" in x:
            from django.utils.safestring import mark_safe
            return mark_safe(x["__safe__"])
        return {k: conv(v) for k, v in x.items()}
    if isinstance(x, list): return [conv(v) for v in x]
    return x

cases = []
def case(tpl, ctx=None, autoescape=True):
    ctx = ctx or {}
    e = engine(autoescape)
    try:
        out = e.from_string(tpl).render(Context(conv(ctx), autoescape=autoescape))
        exp = {"output": out}
    except Exception as ex:  # noqa: BLE001
        exp = {"error": type(ex).__name__}
    cases.append({"template": tpl, "context": ctx, "autoescape": autoescape, "expected": exp})

# -- variables
for t, c in [
    ("{{ a }}", {"a": "hi"}), ("{{ a.b }}", {"a": {"b": 3}}), ("{{ l.1 }}", {"l": [1, 2, 3]}), ("{{ missing }}", {}), ("{{ a.b.c }}", {"a": {"b": {"c": "deep"}}}),
    ("{{ 'lit' }}", {}), ("{{ 42 }}", {}), ("{{ 3.5 }}", {}), ("{{ h }}", {"h": "<b>&\"'"}), ("{{ h|safe }}", {"h": "<b>"}), ("{{ s }}", {"s": {"__safe__": "<i>x</i>"}}),
    ("{{ n }}", {"n": None}), ("{{ t }}", {"t": True}), ("{{ f }}", {"f": False}), ("{{ x }}", {"x": 1.0}), ("{{ x }}", {"x": 1.5}), ("{{ l }}", {"l": [1, "a", None]}),
    ("{{ d }}", {"d": {"k": "v"}}), ("{{ a.0 }}", {"a": "xyz"}), ("a{{ b }}c", {"b": "-"}), ("{{ _x }}", {}), 
    ("{{ d }}", {"d": {"__date__": "2024-01-05"}}), ("{{ d }}", {"d": {"__datetime__": "2024-01-05T14:30:00"}}), ("{{ d }}", {"d": {"__time__": "14:05:00"}}),
]:
    case(t, c)
case("{{ h }}", {"h": "<b>"}, autoescape=False)
case("{% autoescape off %}{{ h }}{% endautoescape %}|{{ h }}", {"h": "<b>"})
# -- filters
F = [
    ("add:3", [5, "2", "a", 1.5, [1]]), ("add:'x'", ["a", 1]), ("addslashes", ["it's \"q\" \\"]), ("capfirst", ["hello", "", "élan"]), ("center:'11'", ["ab", "abc"]),
    ("cut:' '", ["a b c"]), ("default:'d'", ["", None, 0, "x", []]), ("default_if_none:'d'", [None, "", 0]), ("divisibleby:3", [9, 10, "x"]), ("escape", ["<&>"]),
    ("escapejs", ["a'b\"c\n<>"]), ("filesizeformat", [0, 1, 1023, 1024, 1536, 1048576, 123456789, 10**12, -2048]), ("first", [[1, 2], "abc", []]), ("last", [[1, 2], "abc", []]),
    ("floatformat", [34.23234, 34.0, 34.26, 0.5, -0.0, 1234.5678, 0, "abc", None, 7]), ("floatformat:2", [34.23234, 34.0, 34.265, 1.005, -0.001, "3"]),
    ("floatformat:'-1'", [34.23234, 34.0]), ("floatformat:'-3'", [34.23234, 34.0]), ("floatformat:0", [34.5, 35.5, 0.4]), ("force_escape", ["<a>"]), ("get_digit:2", [123, 5, "abc"]),
    ("join:', '", [[1, 2, 3], ["<a>", "b"]]), ("join:'<'", [["a", "b"]]), ("length", ["abc", [1, 2], {}, 5, None]), ("linebreaks", ["a\nb\n\nc"]), ("linebreaksbr", ["a\nb"]),
    ("linenumbers", ["a\nb\nc"]), ("ljust:'6'", ["ab"]), ("rjust:'6'", ["ab"]), ("lower", ["ABC Ä"]), ("upper", ["abc ä"]), ("make_list", ["abc", 123]),
    ("phone2numeric", ["1-800-COLLECT"]), ("pluralize", [1, 2, 0, [1], [1, 2], "1"]), ("pluralize:'y,ies'", [1, 2]), ("pluralize:'es'", [1, 3]),
    ("safe", ["<b>"]), ("slice:':2'", ["abcde", [1, 2, 3]]), ("slice:'1:3'", ["abcde"]), ("slice:'::2'", ["abcde"]), ("slice:'-2:'", ["abcde"]),
    ("slugify", ["Hello, World! 日本"]), ("stringformat:'05d'", [42]), ("stringformat:'.2f'", [3.14159]), ("stringformat:'s'", ["x"]), ("stringformat:'x'", [255]),
    ("striptags", ["<b>bold</b> text", "a < b"]), ("title", ["hello world", "it's a o'neil"]), ("truncatechars:5", ["abcdefgh", "abc"]), ("truncatechars:1", ["abc"]),
    ("truncatewords:2", ["a b c d", "a b"]), ("urlencode", ["a b/c?d=é"]), ("urlencode:''", ["a b/c"]), ("wordcount", ["a b  c\nd"]),
    ("wordwrap:5", ["aaa bbb ccc ddd", "toolongword x"]), ("yesno", [True, False, None]), ("yesno:'ja,nein'", [True, None]), ("yesno:'a,b,c'", [None]),
    ("unordered_list", [["a", ["b", "c"], "d"], ["x", ["y", ["z"]]]]),
    ("dictsort:'n'", [[{"n": 3}, {"n": 1}, {"n": 2}]]), ("dictsortreversed:'n'", [[{"n": 3}, {"n": 1}, {"n": 2}]]),
    ("date:'Y-m-d'", [{"__date__": "2024-03-05"}]), ("date:'D, d M Y'", [{"__date__": "2024-03-05"}]), ("date:'jS F Y'", [{"__date__": "2024-03-01"}, {"__date__": "2024-03-22"}, {"__date__": "2024-03-13"}]),
    ("date:'N j, Y'", [{"__date__": "2024-05-05"}, {"__date__": "2024-03-05"}, {"__date__": "2024-09-05"}]), ("date:'l w z W t L y'", [{"__date__": "2024-03-05"}]),
    ("date:'H:i:s'", [{"__datetime__": "2024-03-05T14:30:07"}]), ("date:'g:i A a P'", [{"__datetime__": "2024-03-05T00:00:00"}, {"__datetime__": "2024-03-05T12:00:00"}, {"__datetime__": "2024-03-05T15:45:00"}]),
    ("date:'c'", [{"__datetime__": "2024-03-05T14:30:07"}]), ("date", [{"__date__": "2024-03-05"}, {"__datetime__": "2024-03-05T14:30:07"}]),
    ("date:'Y\\Y\\ m'", [{"__date__": "2024-03-05"}]), ("time:'H:i'", [{"__time__": "14:05:09"}, {"__datetime__": "2024-03-05T01:02:03"}]), ("time", [{"__time__": "14:05:09"}]),
    ("date:'Y'", ["", None, "str"]), ("json_script:'id1'", [{"a": [1, "é"]}]),
    ("urlize", ["see http://example.com/a?b=1, ok", "www.example.org.", "no link", "a<b http://x.y"]),
    ("safeseq|join:','", [["<a>", "b"]]), ("escape|upper", ["<a>"]), ("upper|escape", ["<a>"]), ("lower|capfirst", ["ABC"]), ("add:1|add:2", [1]),
]
for f, vals in F:
    for v in vals:
        case("{{ v|%s }}" % f, {"v": v})
case("{{ v|nosuchfilter }}", {"v": 1})
case("{{ v|add: }}", {"v": 1})
# -- tags
case("{% if a %}yes{% else %}no{% endif %}", {"a": 1}); case("{% if a %}yes{% else %}no{% endif %}", {"a": []})
case("{% if a and b %}1{% elif c %}2{% else %}3{% endif %}", {"a": 1, "b": 0, "c": 1})
case("{% if a or b %}1{% endif %}|{% if not a %}n{% endif %}", {"a": 0, "b": 1})
case("{% if a == 1 %}eq{% endif %}{% if a != 2 %}ne{% endif %}{% if a > 0 %}gt{% endif %}{% if a >= 1 %}ge{% endif %}{% if a < 5 %}lt{% endif %}{% if a <= 1 %}le{% endif %}", {"a": 1})
case("{% if x in l %}in{% endif %}{% if y not in l %}nin{% endif %}", {"x": 2, "y": 9, "l": [1, 2, 3]})
case("{% if a is None %}none{% endif %}{% if b is not None %}some{% endif %}", {"a": None, "b": 1})
case("{% if a and not b or c %}t{% else %}f{% endif %}", {"a": 1, "b": 1, "c": 0})
case("{% if a|length > 2 %}long{% endif %}", {"a": "abc"}); case("{% if missing %}x{% else %}y{% endif %}", {})
case("{% if a > b %}x{% endif %}", {"a": "s", "b": 3}); case("{% if %}x{% endif %}", {}); case("{% if a %}x", {"a": 1})
case("{% for x in l %}{{ x }},{% endfor %}", {"l": [1, 2, 3]}); case("{% for x in l %}{{ x }}{% empty %}none{% endfor %}", {"l": []})
case("{% for x in l reversed %}{{ x }}{% endfor %}", {"l": [1, 2, 3]})
case("{% for x in l %}{{ forloop.counter }}{{ forloop.counter0 }}{{ forloop.revcounter }}{{ forloop.revcounter0 }}{% if forloop.first %}F{% endif %}{% if forloop.last %}L{% endif %};{% endfor %}", {"l": ["a", "b", "c"]})
case("{% for a, b in l %}{{ a }}={{ b }};{% endfor %}", {"l": [[1, 2], [3, 4]]}); case("{% for k, v in d.items %}{{ k }}:{{ v }};{% endfor %}", {"d": {"a": 1, "b": 2}})
case("{% for x in l %}{% for y in l %}{{ forloop.parentloop.counter }}{{ forloop.counter }} {% endfor %}{% endfor %}", {"l": [1, 2]})
case("{% for x in s %}{{ x }}-{% endfor %}", {"s": "abc"}); case("{% for x in l %}{{ x }}{% endfor %}", {"l": None}); case("{% for x in d %}{{ x }}{% endfor %}", {"d": {"p": 1, "q": 2}})
case("{% for a, b in l %}{% endfor %}", {"l": [[1, 2, 3]]})
case("{% with a=1 b=2 %}{{ a }}{{ b }}{% endwith %}{{ a }}"); case("{% with x as y %}{{ y }}{% endwith %}", {"x": "val"}); case("{% with %}x{% endwith %}")
case("{% cycle 'a' 'b' 'c' %}{% cycle 'a' 'b' 'c' %}{% cycle 'a' 'b' 'c' %}{% cycle 'a' 'b' 'c' %}"); case("{% for i in l %}{% cycle 'x' 'y' as c %}{{ c }}{% endfor %}", {"l": [1, 2, 3]})
case("{% cycle a b %}{% cycle a b %}", {"a": 1, "b": 2}); case("{% cycle 'a' 'b' as c silent %}{{ c }}{% cycle c %}")
case("{% firstof a b 'z' %}", {"a": 0, "b": "bee"}); case("{% firstof a b %}", {}); case("{% firstof a b as r %}[{{ r }}]", {"b": "<x>"})
case("{% for x in l %}{% ifchanged x %}{{ x }}{% endifchanged %}{% endfor %}", {"l": [1, 1, 2, 2, 3]})
case("{% for x in l %}{% ifchanged %}{{ x }}{% else %}-{% endifchanged %}{% endfor %}", {"l": [1, 1, 2]})
case("{% regroup p by g as grp %}{% for g in grp %}{{ g.grouper }}:{% for i in g.list %}{{ i.n }}{% endfor %};{% endfor %}", {"p": [{"g": "a", "n": 1}, {"g": "a", "n": 2}, {"g": "b", "n": 3}]})
case("{% spaceless %}<p>  <b> x </b>  </p> <i> y </i>{% endspaceless %}"); case("{% templatetag openblock %}{% templatetag closevariable %}{% templatetag openbrace %}")
case("{% widthratio a b 100 %}|{% widthratio a b 100 as w %}[{{ w }}]", {"a": 175, "b": 200}); case("{% widthratio a b 10 %}", {"a": 0, "b": 0}); case("{% widthratio a b 10 %}", {"a": 5, "b": 2})
case("{# comment #}x{% comment %}hidden {{ x }}{% endcomment %}y"); case("{% verbatim %}{{ x }}{% if %}{% endverbatim %}")
case("{% filter upper|escape %}<b>x</b>{% endfilter %}"); case("{% filter lower %}ABC {{ v }}{% endfilter %}", {"v": "DEF"})
case("{% include 'inc.html' %}", {"v": 1}); case("{% include 'inc.html' with v=2 x='z' %}", {"v": 1}); case("{% include 'inc.html' with v=2 only %}", {"v": 1, "x": "outer"}); case("{% include 'missing.html' %}")
case("{% include name %}", {"name": "inc.html", "v": 9}); case("{% for i in l %}{% include 'list_item.html' with item=i %}{% endfor %}", {"l": ["a", "b"]})
case("{% extends 'base.html' %}"); case("{% extends 'base.html' %}{% block b %}[b]{% endblock %}"); case("{% extends 'child.html' %}{% block b %}[{{ block.super }}]{% endblock %}")
case("{% extends 'grand.html' %}"); case("{% extends 'grand.html' %}{% block a %}top{% endblock %}"); case("{% extends 'nope.html' %}")
case("{% extends 'base.html' %}{% block a %}{% for i in l %}{{ i }}{% endfor %}{% endblock %}", {"l": [1, 2]})
case("{% block x %}in{% endblock %}"); case("{% block x %}a{% block y %}b{% endblock %}{% endblock %}")
case("{% load static %}{% static 'a/b.css' %}"); case("{% nosuchtag %}"); case("{% endif %}"); case("{{ a|upper }}{% load nosuchlib %}", {"a": "x"})
case("{% if a %}{{ a }}{% elif b %}{{ b }}{% endif %}{% for i in l %}{% if forloop.first %}[{% endif %}{{ i }}{% if forloop.last %}]{% endif %}{% endfor %}", {"a": "", "b": "B", "l": [1, 2, 3]})
case("{{ v|default:'x'|upper }}", {"v": ""}); case("{{ v|default:other }}", {"v": "", "other": "o"}); case("{{ a.b|default:'n' }}", {"a": {}})
case("{% lorem 3 w %}"); case("{% lorem 1 p %}"[:0] or "{% lorem 5 w %}")
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps({"shared": SHARED, "cases": cases}, ensure_ascii=False, indent=0) + "\n")
print(len(cases), "cases")
