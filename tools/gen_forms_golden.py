#!/usr/bin/env python3
"""Run form cases through real Django and write crates/rango-forms/tests/golden/forms.json.

Case: {fields: [[name, ClassName, kwargs]], data: {name: [values]} | null, files: {name: [[filename, content]]},
       initial: {...}, form: {prefix, auto_id, label_suffix, ...}, out: [...]}
"""
import datetime, json, os, sys

import django
from django.conf import settings

settings.configure(USE_TZ=True, TIME_ZONE="UTC", SECRET_KEY="x", USE_I18N=False, FORMS_URLFIELD_ASSUME_HTTPS=True,
                   STATIC_URL="/static/", INSTALLED_APPS=[], DEFAULT_CHARSET="utf-8")
django.setup()
from django import forms
from django.core.files.uploadedfile import SimpleUploadedFile
from django.utils import formats
from django.utils.datastructures import MultiValueDict


def wid(spec):
    if spec is None:
        return None
    name, attrs = (spec, {}) if isinstance(spec, str) else spec
    cls = getattr(forms, name)
    return cls(attrs=attrs) if attrs else cls()

def build(cls, kw):
    kw = dict(kw)
    if "widget" in kw:
        kw["widget"] = wid(kw["widget"])
    if "choices" in kw:
        kw["choices"] = [(c[0], [tuple(x) for x in c[1]]) if isinstance(c[1], list) else tuple(c) for c in kw["choices"]]
    if "coerce" in kw:
        kw["coerce"] = {"int": int, "float": float}[kw["coerce"]]
    if "fields" in kw:
        kw["fields"] = [build(*f) for f in kw["fields"]]
    if "max_value" in kw and cls == "DecimalField":
        import decimal; kw["max_value"] = decimal.Decimal(str(kw["max_value"]))
    return getattr(forms, cls)(**kw)

def jsonify(v):
    import decimal, uuid
    if isinstance(v, decimal.Decimal): return {"decimal": str(v)}
    if isinstance(v, datetime.datetime):  # compared as an instant: Rust normalises aware values to UTC
        return {"datetime": v.astimezone(datetime.timezone.utc).isoformat() if v.tzinfo else v.isoformat()}
    if isinstance(v, datetime.date): return {"date": v.isoformat()}
    if isinstance(v, datetime.time): return {"time": v.isoformat()}
    if isinstance(v, datetime.timedelta): return {"duration": str(v.total_seconds())}
    if isinstance(v, uuid.UUID): return {"uuid": str(v)}
    if isinstance(v, (list, tuple)): return [jsonify(x) for x in v]
    if isinstance(v, dict): return {k: jsonify(x) for k, x in v.items()}
    if hasattr(v, "name") and hasattr(v, "size"): return {"file": v.name}
    if v is False and False: return v
    return v

CASES = []
def case(fields, data=None, out=None, **kw):
    CASES.append(dict(fields=fields, data=data, out=out or ["is_valid", "errors", "cleaned"], **kw))

def single(cls, kwargs, *values, **kw):
    for v in values:
        case([["f", cls, kwargs]], {"f": [v]} if v is not None else {}, **kw)

# --- field cleaning
single("CharField", {"max_length": 5}, "abc", "  abcdef ", "")
single("CharField", {"min_length": 3, "required": False}, "ab", "")
single("CharField", {"strip": False}, "  x ")
single("CharField", {"empty_value": None, "required": False}, "")
single("IntegerField", {}, "12", " 7 ", "1.0", "1.5", "abc", "", "1_000", "+5")
single("IntegerField", {"min_value": 3, "max_value": 10}, "2", "11", "5")
single("IntegerField", {"min_value": 0, "step_size": 5}, "10", "7")
single("FloatField", {}, "1.5", "inf", "nan", "x", "1e3", "")
single("DecimalField", {"max_digits": 5, "decimal_places": 2}, "123.45", "1234.5", "1.234", "x")
single("DecimalField", {"min_value": 1, "max_value": 9, "required": False}, "0", "10", "5.5", "")
single("DateField", {}, "2020-02-29", "02/30/2020", "10/25/2006", "Oct 25 2006", "nope", "")
single("DateField", {"input_formats": ["%d.%m.%Y"]}, "25.10.2006", "2006-10-25")
single("TimeField", {}, "14:30:59", "14:30", "2:30 PM", "99:00")
single("DateTimeField", {}, "2020-01-02 03:04:05", "2020-01-02T03:04:05+02:00", "2020-01-02", "01/02/2020 10:11", "garbage")
single("DurationField", {}, "1 02:03:04", "10", "bad", "P1DT2H")
single("RegexField", {"regex": "^[a-z]+$"}, "abc", "ABC")
single("EmailField", {}, "a@b.co", "bad", "  x@y.org ", "a@b..c")
single("URLField", {}, "http://example.com", "example.com", "//example.com/x", "ftp://x.org", "bad url")
single("BooleanField", {}, "on", "false", "0", "True")
single("BooleanField", {"required": False}, "false", "off")
case([["f", "BooleanField", {"required": False}]], {}, )
single("NullBooleanField", {}, "true", "false", "1", "x")
single("ChoiceField", {"choices": [["a", "A"], ["b", "B"]]}, "a", "c", "")
single("ChoiceField", {"choices": [["Group", [["x", "X"], ["y", "Y"]]], ["z", "Z"]], "required": False}, "y", "q")
single("TypedChoiceField", {"choices": [["1", "one"], ["2", "two"]], "coerce": "int"}, "2", "3")
case([["f", "MultipleChoiceField", {"choices": [["a", "A"], ["b", "B"]]}]], {"f": ["a", "b"]})
case([["f", "MultipleChoiceField", {"choices": [["a", "A"], ["b", "B"]]}]], {"f": ["a", "z"]})
case([["f", "MultipleChoiceField", {"choices": [["a", "A"], ["b", "B"]]}]], {})
case([["f", "TypedMultipleChoiceField", {"choices": [["1", "one"], ["2", "two"]], "coerce": "int"}]], {"f": ["1", "2"]})
single("SlugField", {}, "my-slug_1", "bad slug")
single("UUIDField", {}, "12345678-1234-5678-1234-567812345678", "nope")
single("JSONField", {}, '{"a": [1, 2]}', "[1,", '"str"', "")
single("GenericIPAddressField", {}, "192.168.0.1", "::1", "1:2:3", "300.1.1.1")
single("GenericIPAddressField", {"protocol": "IPv4"}, "::1", "1.2.3.4")
case([["f", "SplitDateTimeField", {}]], {"f_0": ["2020-01-02"], "f_1": ["03:04:05"]})
case([["f", "SplitDateTimeField", {}]], {"f_0": ["2020-01-02"], "f_1": [""]})
case([["f", "SplitDateTimeField", {}]], {"f_0": ["bad"], "f_1": ["03:04"]})
case([["f", "SplitDateTimeField", {"required": False}]], {"f_0": [""], "f_1": [""]})
case([["f", "FileField", {}]], {}, files={"f": [["a.txt", "hello"]]})
case([["f", "FileField", {}]], {}, files={"f": [["a.txt", ""]]})
case([["f", "FileField", {}]], {})
case([["f", "FileField", {"max_length": 3}]], {}, files={"f": [["abcdef.txt", "x"]]})

# --- form behaviour & rendering
RENDER = ["is_valid", "errors", "cleaned", "as_div", "as_p", "as_table", "as_ul"]
person = [["name", "CharField", {"max_length": 20, "help_text": "Your <b>name</b>"}], ["age", "IntegerField", {"min_value": 0, "required": False}],
          ["email", "EmailField", {"label": "E-mail"}]]
case(person, None, out=RENDER)
case(person, {"name": ["Ann"], "age": ["x"], "email": ["bad"]}, out=RENDER)
case(person, {"name": ["Ann"], "age": ["3"], "email": ["a@b.co"]}, out=RENDER)
case(person, None, out=RENDER, form={"prefix": "p"}, initial={"name": "Bob"})
case(person, None, out=RENDER, form={"auto_id": False})
case(person, None, out=RENDER, form={"auto_id": "f_%s", "label_suffix": " ->"})
case(person, {"name": [""], "email": [""]}, out=RENDER, form={"required_css_class": "req"})  # css_classes() order is a Python set: only one class is deterministic
case(person, {"name": ["x"]}, out=["as_p", "changed_data"], initial={"name": "x", "email": "e@e.org"})
case([["a", "CharField", {}], ["b", "CharField", {"widget": "HiddenInput"}], ["c", "CharField", {"disabled": True, "initial": "dis"}]], {"a": ["1"], "b": [""]}, out=RENDER)
case([["a", "CharField", {"show_hidden_initial": True}]], None, out=["as_p"], initial={"a": "v"})
case([["a", "CharField", {}], ["b", "CharField", {}]], None, out=["as_ul"], form={"field_order": ["b", "a"]})

# --- widgets
single("CharField", {"widget": "Textarea"}, "line1\nline2", out=["is_valid", "cleaned", "bf:f"])
case([["f", "CharField", {"widget": ["Textarea", {"rows": "3", "class": "big"}]}]], {"f": ["<hi>"]}, out=["bf:f"])
case([["f", "CharField", {"widget": "PasswordInput"}]], {"f": ["secret"]}, out=["bf:f"])
case([["f", "ChoiceField", {"choices": [["a", "A"], ["b", "B"]]}]], {"f": ["b"]}, out=["bf:f", "label:f"])
case([["f", "ChoiceField", {"choices": [["a", "A"], ["b", "B"]], "widget": "RadioSelect"}]], {"f": ["b"]}, out=["bf:f", "as_div", "label:f", "subwidgets:f"])
case([["f", "MultipleChoiceField", {"choices": [["a", "A"], ["b", "B"], ["c", "C"]], "widget": "CheckboxSelectMultiple"}]], {"f": ["a", "c"]}, out=["bf:f", "as_p"])
case([["f", "MultipleChoiceField", {"choices": [["a", "A"], ["b", "B"]]}]], {"f": ["b"]}, out=["bf:f"])
case([["f", "ChoiceField", {"choices": [["Group", [["x", "X"], ["y", "Y"]]], ["z", "Z"]]}]], {"f": ["y"]}, out=["bf:f"])
case([["f", "ChoiceField", {"choices": [["", "---"], ["a", "A"]], "required": False}]], None, out=["bf:f"])
case([["f", "BooleanField", {"required": False}]], {"f": ["on"]}, out=["bf:f", "as_p"])
case([["f", "BooleanField", {"required": False}]], None, out=["bf:f"], initial={"f": True})
case([["f", "NullBooleanField", {}]], {"f": ["true"]}, out=["bf:f"])
case([["f", "DateField", {}]], None, out=["bf:f"], initial={"f": {"date": "2020-03-04"}})
case([["f", "DateTimeField", {}]], None, out=["bf:f"], initial={"f": {"datetime": "2020-03-04T05:06:07+00:00"}})
case([["f", "TimeField", {}]], None, out=["bf:f"], initial={"f": {"time": "05:06:07"}})
case([["f", "DateField", {"widget": ["DateInput", {"type": "date"}]}]], None, out=["bf:f"])
case([["f", "CharField", {"widget": "HiddenInput"}]], {"f": ["h"]}, out=["bf:f", "as_p"])
case([["f", "MultipleChoiceField", {"choices": [["a", "A"], ["b", "B"]], "widget": "MultipleHiddenInput"}]], {"f": ["a", "b"]}, out=["bf:f"])
case([["f", "SplitDateTimeField", {}]], None, out=["bf:f", "as_p"], initial={"f": {"datetime": "2020-03-04T05:06:07+00:00"}})
case([["f", "DateField", {"widget": "SelectDateWidget"}]], {"f_year": ["2020"], "f_month": ["3"], "f_day": ["4"]}, out=["is_valid", "cleaned", "bf:f"])
case([["f", "NullBooleanField", {"widget": "NullBooleanSelect"}]], None, out=["bf:f"])
case([["f", "IntegerField", {"min_value": 1, "max_value": 9}]], None, out=["bf:f"])
case([["f", "DecimalField", {"decimal_places": 2, "max_digits": 6}]], None, out=["bf:f"])
case([["f", "FloatField", {}]], None, out=["bf:f"])
case([["f", "FileField", {}]], None, out=["bf:f", "as_p"], form={})
case([["f", "EmailField", {}], ["g", "URLField", {}], ["h", "SlugField", {"max_length": 9}]], None, out=["as_table"])

def run(c):
    fields = {n: build(cls, kw) for n, cls, kw in c["fields"]}
    kw = dict(c.get("form", {}))
    for attr in ("required_css_class", "error_css_class"):
        if attr in kw:
            fields[attr] = kw.pop(attr)
    F = type("F", (forms.Form,), fields)
    data = c["data"]
    files = c.get("files")
    if "initial" in c:
        def thaw(v):
            if isinstance(v, dict) and len(v) == 1:
                (k, x), = v.items()
                if k == "date": return datetime.date.fromisoformat(x)
                if k == "datetime": return datetime.datetime.fromisoformat(x)
                if k == "time": return datetime.time.fromisoformat(x)
            return v
        kw["initial"] = {k: thaw(v) for k, v in c["initial"].items()}
    mv = MultiValueDict({k: v for k, v in (data or {}).items()}) if data is not None else None
    fl = MultiValueDict({k: [SimpleUploadedFile(n, b.encode()) for n, b in v] for k, v in files.items()}) if files else None
    if files and mv is None:
        mv = MultiValueDict()
    form = F(mv, fl, **kw)
    out = {}
    for o in c["out"]:
        if o == "is_valid": out[o] = form.is_valid()
        elif o == "errors": out[o] = form.errors.get_json_data() if form.is_bound else {}
        elif o == "cleaned": out[o] = jsonify(form.cleaned_data) if form.is_bound else {}
        elif o == "changed_data": out[o] = form.changed_data
        elif o in ("as_div", "as_p", "as_table", "as_ul"): out[o] = getattr(form, o)()
        elif o.startswith("bf:"): out[o] = str(form[o[3:]])
        elif o.startswith("label:"): out[o] = form[o[6:]].label_tag()
        elif o.startswith("subwidgets:"): out[o] = [str(w) for w in form[o[11:]]]
    return out

FORMSETS = []
def fs_case(fields, opts=None, data=None, initial=None, out=None, prefix=None):
    FORMSETS.append(dict(fields=fields, opts=opts or {}, data=data, initial=initial, prefix=prefix,
                         out=out or ["is_valid", "total", "non_form_errors", "errors", "cleaned"]))

pf = [["name", "CharField", {"max_length": 5}], ["n", "IntegerField", {"required": False}]]
def mgmt(total, initial, **forms_data):
    d = {"form-TOTAL_FORMS": [str(total)], "form-INITIAL_FORMS": [str(initial)], "form-MIN_NUM_FORMS": ["0"], "form-MAX_NUM_FORMS": ["1000"]}
    d.update({k: [v] for k, v in forms_data.items()})
    return d
fs_case(pf, {"extra": 2}, out=["total", "as_div", "as_table", "empty_form"])
fs_case(pf, {"extra": 0, "min_num": 2}, out=["total", "as_p"])
fs_case(pf, {"extra": 3, "max_num": 2}, out=["total"])
fs_case(pf, {"extra": 1}, initial=[{"name": "a", "n": 1}, {"name": "b", "n": 2}], out=["total", "as_ul"])
fs_case(pf, {"can_delete": True, "can_order": True, "extra": 1}, initial=[{"name": "a"}], out=["total", "as_p"])
fs_case(pf, {}, mgmt(2, 0, **{"form-0-name": "ab", "form-0-n": "1", "form-1-name": "cd"}))
fs_case(pf, {}, mgmt(2, 0, **{"form-0-name": "toolong!", "form-0-n": "x", "form-1-name": ""}))
fs_case(pf, {}, {"form-0-name": ["a"]})
fs_case(pf, {"validate_max": True, "max_num": 1}, mgmt(2, 0, **{"form-0-name": "a", "form-1-name": "b"}))
fs_case(pf, {"validate_min": True, "min_num": 2, "extra": 0}, mgmt(2, 0, **{"form-0-name": "a", "form-1-name": ""}))
fs_case(pf, {"can_delete": True}, mgmt(2, 2, **{"form-0-name": "a", "form-1-name": "toolong!!", "form-1-DELETE": "on"}), out=["is_valid", "cleaned", "deleted"])
fs_case(pf, {"can_order": True}, mgmt(3, 0, **{"form-0-name": "a", "form-0-ORDER": "3", "form-1-name": "b", "form-1-ORDER": "1", "form-2-name": "c"}), out=["is_valid", "ordered"])
fs_case(pf, {"extra": 1}, mgmt(1, 0, **{"form-0-name": "a"}), prefix="pre", out=["is_valid"])
fs_case(pf, {"extra": 1}, {"pre-TOTAL_FORMS": ["1"], "pre-INITIAL_FORMS": ["0"], "pre-0-name": ["a"]}, prefix="pre", out=["is_valid", "cleaned"])

def run_fs(c):
    fields = {n: build(cls, kw) for n, cls, kw in c["fields"]}
    F = type("TestForm", (forms.Form,), fields)
    FS = forms.formset_factory(F, **c["opts"])
    kw = {}
    if c["prefix"]: kw["prefix"] = c["prefix"]
    if c["initial"] is not None: kw["initial"] = c["initial"]
    fs = FS(MultiValueDict(c["data"]) if c["data"] is not None else None, **kw)
    out = {}
    for o in c["out"]:
        if o == "is_valid": out[o] = fs.is_valid()
        elif o == "total": out[o] = fs.total_form_count()
        elif o == "non_form_errors": out[o] = list(fs.non_form_errors())
        elif o == "errors": out[o] = [e.get_json_data() for e in fs.errors] if fs.is_bound else []
        elif o == "cleaned": out[o] = jsonify(fs.cleaned_data) if fs.is_valid() else None
        elif o == "deleted": out[o] = [fs.forms.index(f) for f in fs.deleted_forms]
        elif o == "ordered": out[o] = [fs.forms.index(f) for f in fs.ordered_forms]
        elif o == "empty_form": out[o] = fs.empty_form.as_p()
        elif o in ("as_div", "as_p", "as_table", "as_ul"): out[o] = getattr(fs, o)()
    return out
for c in FORMSETS:
    c["expected"] = run_fs(c)

for c in CASES:
    c["expected"] = run(c)
path = os.path.join(os.path.dirname(__file__), "..", "crates", "rango-forms", "tests", "golden", "forms.json")
json.dump({"forms": CASES, "formsets": FORMSETS}, open(path, "w"), indent=1, ensure_ascii=False)
print(len(CASES), "cases", len(FORMSETS), "formsets")
