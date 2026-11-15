#!/usr/bin/env python3
"""Generate crates/rango-core/tests/golden/core.json from real Django 5.2 (the oracle).

Usage:  PYTHONPATH=<dir with django 5.2> python3 tools/gen_core_golden.py

Each case is {"fn": name, "args": [...], "expected": value} where value is the JSON result,
or {"error": ExceptionClassName} if Django raised. The Rust test (tests/core_parity.rs)
replays every case against rango-core and reports all mismatches at once.
"""
import datetime
import json
import pathlib
import types
import unittest.mock

import django
from django.conf import settings

settings.configure(
    SECRET_KEY="test-secret-key",
    SECRET_KEY_FALLBACKS=["old-secret-key"],
    USE_I18N=False,
    USE_TZ=True,
    TIME_ZONE="UTC",
)
django.setup()

from django.core import signing, validators  # noqa: E402
from django.core.exceptions import ValidationError  # noqa: E402
from django.core.paginator import EmptyPage, PageNotAnInteger, Paginator  # noqa: E402
from django.utils import crypto, dateparse, duration, encoding, html, http, ipv6, regex_helper, text  # noqa: E402

OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/rango-core/tests/golden/core.json"
cases = []
NOW = 1_700_000_000


def case(fn, *args, call=None):
    try:
        expected = (call or FUNCS[fn])(*args)
    except Exception as e:  # noqa: BLE001
        expected = {"error": type(e).__name__}
    cases.append({"fn": fn, "args": list(args), "expected": expected})


def vmsg(validator, value):
    try:
        validator(value)
    except ValidationError as e:
        return {"error": "ValidationError", "messages": e.messages, "codes": [x.code for x in e.error_list]}
    return "ok"


def iso(v):
    if v is None:
        return None
    return v.isoformat()


def td(v):
    return None if v is None else {"days": v.days, "seconds": v.seconds, "microseconds": v.microseconds}


def paginate(objs, per_page, orphans, allow_empty, number, on_each_side, on_ends):
    p = Paginator(objs, per_page, orphans, allow_empty)
    out = {"count": p.count, "num_pages": p.num_pages}
    try:
        pg = p.page(number)
        out["page"] = {"items": list(pg.object_list), "start": pg.start_index(), "end": pg.end_index(),
                       "has_next": pg.has_next(), "has_prev": pg.has_previous()}
    except (EmptyPage, PageNotAnInteger) as e:
        out["page"] = {"error": type(e).__name__}
    try:
        out["elided"] = [("…" if x is p.ELLIPSIS else x) for x in
                         p.get_elided_page_range(number, on_each_side=on_each_side, on_ends=on_ends)]
    except (EmptyPage, PageNotAnInteger) as e:
        out["elided"] = {"error": type(e).__name__}
    return out


def signer_sign(key, salt, algorithm, value):
    return signing.Signer(key=key, salt=salt, algorithm=algorithm).sign(value)


def ts_sign(key, salt, ts, value):
    with unittest.mock.patch.object(signing.time, "time", lambda: ts):
        return signing.TimestampSigner(key=key, salt=salt).sign(value)


def ts_unsign(key, salt, signed, max_age, now, fallback):
    with unittest.mock.patch.object(signing.time, "time", lambda: now):
        s = signing.TimestampSigner(key=key, salt=salt, fallback_keys=fallback)
        return s.unsign(signed, max_age=None if max_age is None else datetime.timedelta(seconds=max_age))


def dumps_at(obj, key, salt, compress, ts):
    with unittest.mock.patch.object(signing.time, "time", lambda: ts):
        return signing.dumps(obj, key=key, salt=salt, compress=compress)


def loads_at(signed, key, salt, max_age, now):
    with unittest.mock.patch.object(signing.time, "time", lambda: now):
        return signing.loads(signed, key=key, salt=salt, max_age=max_age)


def cookie_sign(name, salt, value, ts):
    with unittest.mock.patch.object(signing.time, "time", lambda: ts):
        return signing.get_cookie_signer(salt=signing._cookie_signer_salt(name, salt)).sign(value)


def hdr(line):
    key, params = http.parse_header_parameters(line)
    return [key, sorted(params.items())]


FUNCS = {
    "Signer.sign": signer_sign,
    "TimestampSigner.sign": ts_sign,
    "TimestampSigner.unsign": ts_unsign,
    "signing.dumps": dumps_at,
    "signing.loads": loads_at,
    "signing.cookie_sign": cookie_sign,
    "b62_encode": signing.b62_encode,
    "b62_decode": signing.b62_decode,
    "salted_hmac": lambda salt, value, secret, alg: crypto.salted_hmac(salt, value, secret, algorithm=alg).hexdigest(),
    "pbkdf2": lambda pw, salt, it, dklen, alg: crypto.pbkdf2(pw, salt, it, dklen, getattr(__import__("hashlib"), alg)).hex(),
    "constant_time_compare": crypto.constant_time_compare,
    "capfirst": text.capfirst,
    "slugify": text.slugify,
    "get_valid_filename": text.get_valid_filename,
    "camel_case_to_spaces": text.camel_case_to_spaces,
    "get_text_list": lambda l, w: text.get_text_list(l, w) if w else text.get_text_list(l),
    "phone2numeric": text.phone2numeric,
    "smart_split": lambda s: list(text.smart_split(s)),
    "unescape_string_literal": text.unescape_string_literal,
    "wrap": text.wrap,
    "normalize_newlines": text.normalize_newlines,
    "Truncator.chars": lambda s, n, t: text.Truncator(s).chars(n, t),
    "Truncator.words": lambda s, n, t: text.Truncator(s).words(n, t),
    "trim_value": lambda v, l: list(text.trim_value(v, l)),
    "escape": lambda s: str(html.escape(s)),
    "escapejs": lambda s: str(html.escapejs(s)),
    "json_script": lambda v, i: str(html.json_script(v, i)),
    "linebreaks": html.linebreaks,
    "strip_tags": html.strip_tags,
    "strip_spaces_between_tags": html.strip_spaces_between_tags,
    "avoid_wrapping": html.avoid_wrapping,
    "urlencode": lambda pairs: http.urlencode(pairs),
    "http_date": http.http_date,
    "parse_http_date": http.parse_http_date_safe,
    "int_to_base36": http.int_to_base36,
    "base36_to_int": http.base36_to_int,
    "urlsafe_base64_encode": lambda s: http.urlsafe_base64_encode(s.encode()),
    "urlsafe_base64_decode": lambda s: http.urlsafe_base64_decode(s).decode(),
    "parse_etags": http.parse_etags,
    "quote_etag": http.quote_etag,
    "is_same_domain": http.is_same_domain,
    "url_has_allowed_host_and_scheme": lambda u, hosts, https: http.url_has_allowed_host_and_scheme(u, set(hosts), require_https=https),
    "escape_leading_slashes": http.escape_leading_slashes,
    "parse_header_parameters": hdr,
    "content_disposition_header": http.content_disposition_header,
    "iri_to_uri": encoding.iri_to_uri,
    "uri_to_iri": encoding.uri_to_iri,
    "escape_uri_path": encoding.escape_uri_path,
    "filepath_to_uri": encoding.filepath_to_uri,
    "clean_ipv6_address": lambda s, unpack: ipv6.clean_ipv6_address(s, unpack),
    "is_valid_ipv6_address": ipv6.is_valid_ipv6_address,
    "parse_date": lambda s: iso(dateparse.parse_date(s)),
    "parse_time": lambda s: iso(dateparse.parse_time(s)),
    "parse_datetime": lambda s: iso(dateparse.parse_datetime(s)),
    "parse_duration": lambda s: td(dateparse.parse_duration(s)),
    "duration_string": lambda d, s, us: duration.duration_string(datetime.timedelta(days=d, seconds=s, microseconds=us)),
    "duration_iso_string": lambda d, s, us: duration.duration_iso_string(datetime.timedelta(days=d, seconds=s, microseconds=us)),
    "paginate": paginate,
    "normalize": lambda p: [[f, list(a)] for f, a in regex_helper.normalize(p)],
    "urljoin": lambda b, u: __import__("urllib.parse").parse.urljoin(b, u),
    "validate_email": lambda v: vmsg(validators.validate_email, v),
    "validate_slug": lambda v: vmsg(validators.validate_slug, v),
    "validate_unicode_slug": lambda v: vmsg(validators.validate_unicode_slug, v),
    "validate_ipv4_address": lambda v: vmsg(validators.validate_ipv4_address, v),
    "validate_ipv6_address": lambda v: vmsg(validators.validate_ipv6_address, v),
    "validate_ipv46_address": lambda v: vmsg(validators.validate_ipv46_address, v),
    "validate_integer": lambda v: vmsg(validators.validate_integer, v),
    "validate_comma_separated_integer_list": lambda v: vmsg(validators.validate_comma_separated_integer_list, v),
    "validate_domain_name": lambda v: vmsg(validators.validate_domain_name, v),
    "URLValidator": lambda v: vmsg(validators.URLValidator(), v),
    "MaxValueValidator": lambda limit, v: vmsg(validators.MaxValueValidator(limit), v),
    "MinValueValidator": lambda limit, v: vmsg(validators.MinValueValidator(limit), v),
    "MinLengthValidator": lambda limit, v: vmsg(validators.MinLengthValidator(limit), v),
    "MaxLengthValidator": lambda limit, v: vmsg(validators.MaxLengthValidator(limit), v),
    "StepValueValidator": lambda step, v: vmsg(validators.StepValueValidator(step), v),
    "ProhibitNullCharactersValidator": lambda v: vmsg(validators.ProhibitNullCharactersValidator(), v),
    "FileExtensionValidator": lambda exts, v: vmsg(validators.FileExtensionValidator(exts), types.SimpleNamespace(name=v)),
}

# ---- signing -------------------------------------------------------------------------------
for key, salt, alg, value in [("k", None, None, "hello"), ("k", "s", "sha1", "héllo wörld"), ("secret", "ns", "sha512", ""),
                              ("k", None, "sha256", "a:b:c")]:
    case("Signer.sign", key, salt, alg, value)
for value in ["hello", "ünï", ""]:
    case("TimestampSigner.sign", "k", "s", NOW, value)
case("TimestampSigner.sign", "k", None, 0, "x")
for v, ts in [("hello", NOW), ("a:b", NOW - 1000)]:
    signed = ts_sign("k", "s", ts, v)
    case("TimestampSigner.unsign", "k", "s", signed, None, NOW + 5, None)
    case("TimestampSigner.unsign", "k", "s", signed, 100, NOW + 5, None)
    case("TimestampSigner.unsign", "k", "s", signed, 3, NOW + 50_000, None)
    case("TimestampSigner.unsign", "wrong", "s", signed, None, NOW, None)
    case("TimestampSigner.unsign", "new", "s", ts_sign("old", "s", ts, v), None, NOW, ["old"])
case("TimestampSigner.unsign", "k", "s", "no-separator", None, NOW, None)
case("TimestampSigner.unsign", "k", "s", "a:b:tampered", None, NOW, None)
objs = [{"a": 1, "b": [1, 2, "é"]}, "plain string", [1, 2.5, None, True], {"nested": {"k": ["ü", "𝄞"]}}, 42, {"big": "x" * 400}]
for o in objs:
    for compress in (False, True):
        d = dumps_at(o, "k", "salt", compress, NOW)
        case("signing.dumps", o, "k", "salt", compress, NOW)
        case("signing.loads", d, "k", "salt", None, NOW + 1)
case("signing.loads", dumps_at({"a": 1}, "k", "salt", False, NOW), "k", "other-salt", None, NOW)
case("signing.loads", dumps_at({"a": 1}, "k", "salt", False, NOW), "k", "salt", 10, NOW + 1000)
for n in [0, 1, 61, 62, 63, 3844, NOW, -5]:
    case("b62_encode", n)
for s in ["0", "1", "z", "10", "1QaUZC", "-3"]:
    case("b62_decode", s)
for name, salt, value in [("sid", "", "val"), ("sid", "extra", "v:w"), ("cookie", "ü", "x")]:
    case("signing.cookie_sign", name, salt, value, NOW)

# ---- crypto --------------------------------------------------------------------------------
for alg in ["sha1", "sha256", "sha512", "md5"]:
    case("salted_hmac", "salt", "value", "secret", alg)
case("salted_hmac", "ünï", "välue", "sécret", "sha256")
case("pbkdf2", "password", "salt", 1000, 0, "sha256")
case("pbkdf2", "password", "salt", 10, 20, "sha1")
case("pbkdf2", "pässword", "sält", 5, 64, "sha512")
for a, b in [("abc", "abc"), ("abc", "abd"), ("abc", "ab"), ("", ""), ("é", "é")]:
    case("constant_time_compare", a, b)

# ---- text ----------------------------------------------------------------------------------
for s in ["", "hello", "élan", "ǆ", "1abc", "ß"]:
    case("capfirst", s)
for s, uni in [("Hello, World!", False), ("  --Foo__Bar--  ", False), ("Ünïcode Straße", False), ("Ünïcode Straße", True),
               ("a  b---c", False), ("日本語 text", True), ("日本語 text", False), ("İstanbul", True), ("x_y-z", False),
               ("ﬁne ℌ ²", True)]:
    case("slugify", s, uni)
for s in ["john's portrait in 2004.jpg", "  a b  ", "", ".", "..", "../etc", "ñandú.txt", "a/b\\c"]:
    case("get_valid_filename", s)
for s in ["CamelCase", "HTTPResponse", "already lower", "ABC", "aB", "A", "XMLHttpRequest", " Lead", "snake_Case"]:
    case("camel_case_to_spaces", s)
for l, w in [([], None), (["a"], None), (["a", "b"], None), (["a", "b", "c", "d"], None), (["a", "b", "c"], "and")]:
    case("get_text_list", l, w)
for s in ["1-800-FLOWERS", "abc DEF", ""]:
    case("phone2numeric", s)
for s in ['This is "a person\'s" test.', "Another 'person\\'s' test.", 'A "\\"funky\\" style" test.', "  lead  trail ", "a=\"b c\" d"]:
    case("smart_split", s)
for s in ['"abc"', "'abc'", '"a \\"bc\\""', "'\\'ab\\' c'", "abc", "", '"', "'x\"", '"a\\\\b"']:
    case("unescape_string_literal", s)
for s, w in [("Join them together to create a single phrase", 10), ("short", 80), ("a\nb c d e f g", 3), ("verylongwordthatexceeds", 5),
             ("trailing\n", 4), ("  indented text here", 8), ("tab\tseparated words here", 10), ("", 5), ("one two\n\nthree four", 7)]:
    case("wrap", s, w)
for s in ["a\r\nb\rc\nd", "", "\r\r\n"]:
    case("normalize_newlines", s)
for s, n, t in [("The quick brown fox jumped over the lazy dog.", 10, None), ("short", 10, None), ("The quick brown fox", 9, " [more]"),
                ("Abc", 1, None), ("Hello", 0, None), ("Hello world", 5, "%(truncated_text)s..."), ("Ünï cödé text here", 8, None),
                ("éééé", 3, None), ("exactly", 7, None)]:
    case("Truncator.chars", s, n, t)
for s, n, t in [("The quick brown fox jumped over the lazy dog.", 3, None), ("one two", 5, None), ("a  b\nc d", 2, "..."),
                ("a b c", 0, None), ("  spaced   out  ", 1, None)]:
    case("Truncator.words", s, n, t)
for v, l in [("abcdef", 3), ("abc", 3), ("", 5)]:
    case("trim_value", v, l)

# ---- html ----------------------------------------------------------------------------------
for s in ["<b>&\"'</b>", "plain", "", "a&amp;b", "ünï"]:
    case("escape", s)
for s in ["it's \"quoted\" <tag> & stuff = - ; `", "line\nbreak\ttab\x00\x1f", "  ", "plain"]:
    case("escapejs", s)
for v, i in [({"a": "<script>&"}, "data"), ([1, "é", None], None), ("x", ""), ({"k": 1.5}, "id")]:
    case("json_script", v, i)
for s in ["a\nb\n\nc", "a\r\nb", "single", "", "<x>\n\n&"]:
    case("linebreaks", s)
for s in ["<b>bold</b> text", "no tags", "<a href='x>y'>link</a>", "1 < 2 and 3 > 2", "<scr<script>ipt>", "a<!-- c -->b", "&amp; &#39; &foo",
          "<p>unclosed", "<br/>x<br />y", "text <img src=a.png> more", "<![CDATA[x]]>y", "<?php echo 1 ?>z", "<>", "a < b"]:
    case("strip_tags", s)
for s in ["<a> \n <b>", "<a>x</a>  <b>", "no  tags  here"]:
    case("strip_spaces_between_tags", s)
case("avoid_wrapping", "10 MB")

# ---- http ----------------------------------------------------------------------------------
for pairs in [[["a", "1"], ["b", "x y"]], [["k", "é&="], ["k", "~_.-"]], [], [["a b", "c+d"]]]:
    case("urlencode", pairs)
for e in [0, NOW, 1_000_000_000.5, 4_102_444_800]:
    case("http_date", e)
for s in ["Sun, 06 Nov 1994 08:49:37 GMT", "Sunday, 06-Nov-94 08:49:37 GMT", "Sun Nov  6 08:49:37 1994", "garbage", "Sun, 31 Feb 2000 00:00:00 GMT",
          "Mon, 01 Jan 2024 00:00:00 GMT"]:
    case("parse_http_date", s)
for n in [0, 35, 36, 1295, 1296, 10**12]:
    case("int_to_base36", n)
for s in ["0", "z", "10", "zz", "a" * 14, "ZZ", ""]:
    case("base36_to_int", s)
for s in ["hello", "ünï", ""]:
    case("urlsafe_base64_encode", s)
for s in ["aGVsbG8", "aGVsbG8=", "!!!", "w7xuw68"]:
    case("urlsafe_base64_decode", s)
for s in ['"abc", W/"def", "ghi"', "*", ' "x" ,garbage, W/"y" ', "", '"unterminated']:
    case("parse_etags", s)
for s in ['"abc"', "abc", 'W/"x"', ""]:
    case("quote_etag", s)
for host, pat in [("example.com", "example.com"), ("foo.example.com", ".example.com"), ("example.com", ".example.com"), ("evil.com", ".example.com"),
                  ("a.b", ""), ("EXAMPLE.com", "example.COM")]:
    case("is_same_domain", host, pat)
for u, hosts, https in [("/path", [], False), ("//evil.com", ["good.com"], False), ("https://good.com/x", ["good.com"], False),
                        ("http://good.com/x", ["good.com"], True), ("javascript:alert(1)", [], False), ("///evil.com", [], False),
                        ("/\\evil.com", [], False), ("", [], False), ("  /ok  ", [], False), ("https:///x", ["good.com"], False),
                        ("\x01//x", [], False), ("http://[::1]/x", ["[::1]"], False), ("http://[bad/x", [], False),
                        ("good.com", ["good.com"], False), ("//good.com", ["good.com"], False), ("ftp://good.com", ["good.com"], False)]:
    case("url_has_allowed_host_and_scheme", u, hosts, https)
for u in ["//x/y", "/x", "///", "x//"]:
    case("escape_leading_slashes", u)
for line in ["text/html; charset=utf-8", "TEXT/HTML", "", "form-data; name=\"f\"; filename=\"a b.txt\"", "a/b; q=1; r=\"x;y\"",
             "attachment; filename*=utf-8''%e2%82%ac%20rates", "x; a=1; b", "multipart/form-data; boundary=----X123"]:
    case("parse_header_parameters", line)
for att, name in [(True, "a.txt"), (False, "a b.txt"), (True, "é.txt"), (True, 'q"uote.txt'), (True, None), (False, None), (True, "back\\slash")]:
    case("content_disposition_header", att, name)

# ---- encoding ------------------------------------------------------------------------------
for s in ["/I ♥ Rango/", "/a b?x=ü&y=%20#frag", "plain", "~tilde_ok", "[ipv6]"]:
    case("iri_to_uri", s)
for s in ["/I%20%E2%99%A5%20Rango/", "%C3%BC%2F%41", "%E2%99", "no-escapes", "100%", "%zz%41"]:
    case("uri_to_iri", s)
for s in ["/a b/ü", "x@y+z", "a?b#c"]:
    case("escape_uri_path", s)
for s in ["a\\b c/ü", "dir/~file (1)"]:
    case("filepath_to_uri", s)

# ---- ipv6 / validators ---------------------------------------------------------------------
for s, u in [("::1", False), ("2001:0DB8:0000:0000:0000:0000:0000:0001", False), ("::ffff:1.2.3.4", False), ("::ffff:1.2.3.4", True),
             ("1:2:3:4:5:6:7:8", False), ("not-an-ip", False), ("fe80::1%eth0", False), ("1::2::3", False), ("::", False)]:
    case("clean_ipv6_address", s, u)
for s in ["::1", "1.2.3.4", "fe80::1%eth0", "", "12345::", "::ffff:1.2.3.4", "g::1"]:
    case("is_valid_ipv6_address", s)

for v in ["a@b.com", "user.name+tag@example.co.uk", "plain", "@x.com", "a@", "a@localhost", "a@[127.0.0.1]", "a@[::1]", "a@-bad.com", "a@b",
          "a b@c.com", '"quoted"@x.com', "x@" + "a" * 64 + ".com", "ünï@dömain.com", "a@xn--bcher-kva.com", "a" * 320 + "@x.com", ""]:
    case("validate_email", v)
for v in ["good-slug_1", "bad slug", "", "ünï", "a/b"]:
    case("validate_slug", v)
    case("validate_unicode_slug", v)
for v in ["1.2.3.4", "256.1.1.1", "1.2.3", "::1", "01.2.3.4", "1.2.3.4.5", ""]:
    case("validate_ipv4_address", v)
    case("validate_ipv6_address", v)
    case("validate_ipv46_address", v)
for v in ["123", "-5", "1.5", "", "abc", "+3", "١٢٣"]:
    case("validate_integer", v)
for v in ["1,2,3", "1,,2", "1", "a,b", "1, 2"]:
    case("validate_comma_separated_integer_list", v)
for v in ["example.com", "sub.example.co.uk", "-bad.com", "bad-.com", "localhost", "ünï.com", "a" * 64 + ".com", "x.y", "exa mple.com", "example.com."]:
    case("validate_domain_name", v)
for v in ["http://example.com", "https://example.com/path?q=1#f", "ftp://x.org", "mailto:a@b.c", "http://", "http://localhost:8000/x",
          "http://127.0.0.1", "http://[::1]/x", "http://[bad]/x", "http://user:pw@example.com", "http://exa mple.com", "http://example",
          "HTTP://EXAMPLE.COM", "http://ünï.com", "http://example.com\n", "//example.com", "http://a.b-.com", "javascript://x.com",
          "http://example.com:99999", "http://" + "a" * 60 + ".com", "https://" + "a." * 130 + "com"]:
    case("URLValidator", v)
for lim, v in [(10, 5), (10, 10), (10, 11), (0, -1), (1.5, 1.6)]:
    case("MaxValueValidator", lim, v)
    case("MinValueValidator", lim, v)
for lim, v in [(3, "abcd"), (3, "abc"), (3, "ab"), (1, ""), (1, "é"), (2, "ab"), (1, "ab")]:
    case("MinLengthValidator", lim, v)
    case("MaxLengthValidator", lim, v)
for step, v in [(2, 4), (2, 5), (0.1, 0.3), (5, 15), (3, 10)]:
    case("StepValueValidator", step, v)
for v in ["ok", "bad\x00null", ""]:
    case("ProhibitNullCharactersValidator", v)
for exts, v in [(["jpg", "png"], "a.JPG"), (["jpg"], "a.gif"), (["jpg"], "noext"), (["tar.gz"], "a.tar.gz"), (["txt"], "dir.d/file.txt")]:
    case("FileExtensionValidator", exts, v)

# ---- dateparse / duration ------------------------------------------------------------------
for s in ["2024-02-29", "2023-02-29", "2024-2-9", "24-01-01", "garbage", "2024-13-01", "2024-01-01T00:00", ""]:
    case("parse_date", s)
for s in ["10:20", "10:20:30", "10:20:30.123456", "10:20:30,5", "25:00", "1:2:3", "10:20:30+02:00", "garbage", "10:20:30.1234567"]:
    case("parse_time", s)
for s in ["2012-04-23T10:20:30.400+02:30", "2012-04-23 10:20:30", "2012-04-23T10:20:30Z", "2012-04-23T10:20", "2012-4-3 1:2", "2012-04-23T10:20:30-0530",
          "2012-04-23T10:20:30 +02", "2012-13-23T10:20", "garbage", "2012-04-23", "2012-04-23T10:20:30.123456789"]:
    case("parse_datetime", s)
for s in ["1 day, 2:03:04", "-1 days 2:03:04", "4 15:30", "15:30", "30", "1:2:3", "10:15:30.5", "-4 15:30", "P4DT1H15M20S", "-P1DT2H", "PT0.5S", "P1.5D",
          "4 days 11:00:00.001", "garbage", "", "1 2:3:4.123456789", "-00:00:01", "+1 day", "1,5"]:
    case("parse_duration", s)
for d, s, us in [(0, 0, 0), (1, 3723, 0), (-1, 86399, 999999), (0, 59, 5), (400, 0, 123456), (-2, 0, 0)]:
    case("duration_string", d, s, us)
    case("duration_iso_string", d, s, us)

# ---- paginator -----------------------------------------------------------------------------
for n, per, orph, empty, num, side, ends in [
    (0, 10, 0, True, 1, 3, 2), (0, 10, 0, False, 1, 3, 2), (1, 10, 0, True, 1, 3, 2), (10, 3, 0, True, 4, 3, 2), (10, 3, 1, True, 3, 3, 2),
    (10, 3, 1, True, 4, 3, 2), (100, 2, 0, True, 25, 3, 2), (100, 2, 0, True, 1, 3, 2), (100, 2, 0, True, 50, 3, 2), (100, 2, 5, True, 48, 1, 1),
    (23, 5, 0, True, 0, 3, 2), (23, 5, 0, True, 6, 3, 2), (23, 5, 2, True, 4, 3, 2), (7, 7, 0, True, 1, 3, 2)]:
    case("paginate", list(range(1, n + 1)), per, orph, empty, num, side, ends)

# ---- regex_helper / urljoin -----------------------------------------------------------------
for p in [r"^articles/(?P<year>[0-9]{4})/$", r"^a/(\d+)/$", r"^(?:foo/)?(?P<x>\w+)/", "x?", "a|b", "[abc]d", "(?P<a>x)(?P<b>y)", "^[a-z]+/",
          r"^\w+\.html$", r"^(?:(?P<p>\d+)/)?end", "^a{2}b", "(?P=a)", "", "^$", r"^b/(?P<n>[-\w]+)/(?:c/)*d", "(?:a|b)", "(?=x)y", "^x*(?P<k>y)?",
          r"^page-(?P<n>\d+)?/$", "(?P<n>\d+){2}", "(?i)x", "^\\(a\\)$", r"^/(?:(?P<a>\d+)/(?P<b>\d+)/)?$"]:
    case("normalize", p)
for b, u in [("http://a/b/c/d;p?q", "g"), ("http://a/b/c/d;p?q", "../g"), ("http://a/b/c/d;p?q", "/g"), ("http://a/b/c/d;p?q", "//g"),
             ("http://a/b/c/d;p?q", "?y"), ("http://a/b/c/d;p?q", "g?y#s"), ("http://a/b/c/d;p?q", "../../../g"), ("http://a/b/c/d;p?q", "./g/."),
             ("http://a/b/c/d;p?q", "https://x/y"), ("http://a/b/c/", "d"), ("http://a", "/x"), ("http://a/b", ""), ("", "x"), ("http://testserver/p/", "q"),
             ("http://a/b/c", "#f"), ("http://a/b/c", "mailto:x@y"), ("http://a/b/c", "../.."), ("http://a/b//c", "d")]:
    case("urljoin", b, u)

# ---- exceptions ----------------------------------------------------------------------------
# Rango uses its own default salts ("rango.core.signing...", "rango.http.cookies"), so the cases that
# depend on them were recorded from Rango itself (tools/core_golden_overrides.json).
_ov = json.loads((pathlib.Path(__file__).parent / "core_golden_overrides.json").read_text())
for c in cases:
    for o in _ov:
        if c["fn"] == o["fn"] and c["args"] == o["args"]:
            c["expected"] = o["expected"]
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(cases, ensure_ascii=False, indent=0) + "\n")
print(f"{len(cases)} cases -> {OUT}")
