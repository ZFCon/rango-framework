# Utilities

A tour of `rango::utils`, the small functions and types the framework itself is built on and that you can call
from your own code: HTML escaping, text helpers, URL and HTTP helpers, dates and durations, hashing, time zones,
translation and a multi-value map. Examples marked with `=>` show verified input and output.

All modules are under `rango::utils`:

```rust
use rango::utils::{crypto, datastructures, dateparse, duration, encoding, formats, html, http, ipv6, json, os,
                   regex_helper, safestring, text, timezone, translation};
```

## `utils::html` and `utils::safestring`

HTML-safe text is a distinct type, `SafeString`, so that "already escaped" is visible in signatures. A plain
`str` or `String` is untrusted text.

```rust
use rango::format_html;
use rango::utils::html::{escape, conditional_escape, escapejs, json_script, linebreaks, strip_tags};
use rango::utils::safestring::{mark_safe, SafeString, ToHtml};
```

| Item | Signature | Notes |
|---|---|---|
| `mark_safe` | `fn mark_safe(s: impl Into<String>) -> SafeString` | Promise that `s` needs no escaping. |
| `SafeString` | `new()`, `as_str()`, `into_string()`, `push_safe(&SafeString)`; `Deref<Target = str>`, `Display`, `+ &SafeString` | Convert back with `String::from`. |
| `ToHtml` | `trait { fn conditional_escape(&self) -> SafeString }` | Implemented for `str`, `String`, `Cow<str>`, `SafeString` (which passes through) and references to these. |
| `escape` | `fn escape(text: &str) -> SafeString` | Always escapes, so it can double-escape. |
| `conditional_escape` | `fn conditional_escape<T: ToHtml + ?Sized>(text: &T) -> SafeString` | Escapes unless already safe. |
| `format_html!` | `format_html!("<b>{}</b>", value)` | Escapes each argument, marks the result safe. Inline `{name}` captures are not escaped; pass arguments. |
| `format_html_join` | `fn format_html_join<T: ToHtml + ?Sized>(sep: &T, fragments: impl IntoIterator<Item = SafeString>) -> SafeString` | Joins fragments with an escaped separator. |
| `escapejs` | `fn escapejs(value: &str) -> SafeString` | Hex-escapes for use inside JavaScript string literals. |
| `json_script` | `fn json_script<T: Serialize + ?Sized>(value: &T, element_id: Option<&str>) -> serde_json::Result<SafeString>` | JSON in a `<script type="application/json">` element. |
| `linebreaks` | `fn linebreaks(value: &str, autoescape: bool) -> String` | Newlines to `<p>` and `<br>`. |
| `strip_spaces_between_tags` | `fn(&str) -> String` | Removes whitespace between tags. |
| `avoid_wrapping` | `fn(&str) -> String` | Replaces spaces with non-breaking spaces. |
| `strip_tags` | `fn strip_tags(value: &str) -> rango::Result<String>` | Errors with a "suspicious operation" error on pathologically nested input. |

Verified results:

```text
escape("<b>&\"'</b>")                  => "&lt;b&gt;&amp;&quot;&#x27;&lt;/b&gt;"
escapejs("it's \"quoted\" <tag> & x")  => "it's "quoted" <tag> & x"
json_script({"a": "<script>&"}, "data") => <script id="data" type="application/json">{"a": "<script>&"}</script>
linebreaks("a\nb\n\nc", true)           => "<p>a<br>b</p>\n\n<p>c</p>"
strip_tags("<b>bold</b> text")         => "bold text"
```

`strip_tags` is not a sanitiser. Escape output; do not rely on stripping to make untrusted HTML safe.

## `utils::text`

```rust
use rango::utils::text::{slugify, capfirst, camel_case_to_spaces, get_valid_filename, get_text_list, wrap,
                         Truncator, normalize_newlines, phone2numeric, smart_split, unescape_string_literal,
                         trim_value, compress_string};
```

| Function | Signature | Example |
|---|---|---|
| `slugify` | `(value: &str, allow_unicode: bool) -> String` | `"Hello, World!"` => `"hello-world"`; `"  --Foo__Bar--  "` => `"foo__bar"` |
| `capfirst` | `(&str) -> String` | `"hello"` => `"Hello"` |
| `camel_case_to_spaces` | `(&str) -> String` | `"HTTPResponse"` => `"http response"` |
| `get_valid_filename` | `(name: &str) -> rango::Result<String>` | `"john's portrait in 2004.jpg"` => `"johns_portrait_in_2004.jpg"`; an unusable name (empty, `.`, `..`) is an error |
| `get_text_list` | `(items: &[impl AsRef<str>], last_word: Option<&str>) -> String` | `["a"]` => `"a"`; `[]` => `""` |
| `wrap` | `(text: &str, width: usize) -> String` | `("Join them together to create a single phrase", 10)` => `"Join them\ntogether\nto create\na single\nphrase"` |
| `Truncator::new(text)` | `.chars(num: i64, truncate: Option<&str>) -> String`, `.words(num, truncate)` | `chars(10)` on `"The quick brown fox jumped over the lazy dog."` => `"The quick…"`; `words(3)` => `"The quick brown…"` |
| `normalize_newlines` | `(&str) -> String` | Converts `\r\n` and `\r` to `\n`. |
| `phone2numeric` | `(&str) -> String` | `"1-800-FLOWERS"` => `"1-800-3569377"` |
| `smart_split` | `(&str) -> Vec<&str>` | Splits on whitespace but keeps quoted parts together: `"This is \"a person's\" test."` => `["This", "is", "\"a person's\"", "test."]` |
| `unescape_string_literal` | `(&str) -> Option<String>` | `"\"abc\""` => `Some("abc")` |
| `trim_value` | `(value: &str, limit: usize) -> (String, String)` | `("abcdef", 3)` => `("abc", "… <trimmed 3 chars out of 6>")` |
| `compress_string` | `(data: &[u8], max_random_bytes: Option<usize>) -> Vec<u8>` | Gzip; with `max_random_bytes` a random-length filler is added to the header to blunt length-based attacks. |
| `is_word_char`, `is_space` | `(char) -> bool` | Character classes used by the other helpers. |

## `utils::http`

```rust
use rango::utils::http::{urlencode, quote, quote_plus, unquote, http_date, parse_http_date, parse_http_date_safe,
                         int_to_base36, base36_to_int, urlsafe_base64_encode, urlsafe_base64_decode,
                         parse_etags, quote_etag, is_same_domain, url_has_allowed_host_and_scheme, urlsplit,
                         urljoin, escape_leading_slashes, content_disposition_header, parse_header_parameters};
```

| Function | Signature | Example |
|---|---|---|
| `urlencode` | `(query: impl IntoIterator<Item = (K, V)>) -> String` | `[("a","1"),("b","x y")]` => `"a=1&b=x+y"` |
| `urlencode_multi` | `(&MultiValueDict<String>) -> String` | Repeats keys with several values. |
| `quote`, `quote_plus`, `unquote` | `(s: &str, safe: &str)`, `(&str)`, `(&str)` -> `String` | `quote_plus` turns spaces into `+`. |
| `http_date` | `(epoch_seconds: Option<f64>) -> String` | `0` => `"Thu, 01 Jan 1970 00:00:00 GMT"` |
| `parse_http_date` | `(&str) -> Result<i64, HttpDateError>` | `"Sun, 06 Nov 1994 08:49:37 GMT"` => `784111777`; the older RFC 850 form is accepted too. |
| `parse_http_date_safe` | `(&str) -> Option<i64>` | Same, `None` on failure. |
| `int_to_base36`, `base36_to_int` | `(u64) -> String`, `(&str) -> Result<u64, String>` | `35` <=> `"z"`; inputs longer than 13 characters are rejected. |
| `urlsafe_base64_encode`, `urlsafe_base64_decode` | `(&[u8]) -> String`, `(&str) -> Result<Vec<u8>, base64::DecodeError>` | No padding on encode; padding optional on decode. `"hello"` => `"aGVsbG8"` |
| `parse_etags` | `(&str) -> Vec<String>` | `"\"abc\", W/\"def\""` => `["\"abc\"", "W/\"def\""]`; `"*"` => `["*"]` |
| `quote_etag` | `(&str) -> String` | `abc` => `"abc"` with quotes; an already quoted tag is unchanged. |
| `is_same_domain` | `(host: &str, pattern: &str) -> bool` | `("foo.example.com", ".example.com")` => `true`; a leading dot matches subdomains and the bare domain. |
| `url_has_allowed_host_and_scheme` | `(url: &str, allowed_hosts: &HashSet<String>, require_https: bool) -> bool` | `("/path", {}, false)` => `true`; `("//evil.com", {"good.com"}, false)` => `false` |
| `urlsplit` | `(&str) -> Result<SplitResult, String>` | Fields `scheme`, `netloc`, `path`, `query`, `fragment`; methods `hostname()`, `unsplit()`. |
| `urljoin` | `(base: &str, url: &str) -> String` | `("http://a/b/c/d;p?q", "../g")` => `"http://a/b/g"` |
| `escape_leading_slashes` | `(&str) -> String` | `"//x/y"` => `"/%2Fx/y"` (stops a path being read as a network-path reference). |
| `content_disposition_header` | `(as_attachment: bool, filename: Option<&str>) -> Option<String>` | `(true, "a.txt")` => `attachment; filename="a.txt"` |
| `parse_header_parameters` | `(&str) -> Result<(String, Vec<(String, String)>), String>` | `"text/html; charset=utf-8"` => `("text/html", [("charset","utf-8")])` |

Constants: `MAX_URL_LENGTH` (2048), `MAX_URL_REDIRECT_LENGTH` (16384), `RFC3986_GENDELIMS`,
`RFC3986_SUBDELIMS`.

Always run a user-supplied redirect target such as a `next` parameter through `url_has_allowed_host_and_scheme`
before redirecting to it. It rejects other hosts, `//host` forms, backslash tricks and over-long URLs, and with
`require_https: true` it rejects `http` targets.

## `utils::encoding`

| Function | Signature | Example |
|---|---|---|
| `iri_to_uri` | `(&str) -> String` | `"/I ♥ Rango/"` => `"/I%20%E2%99%A5%20Rango/"` |
| `uri_to_iri` | `(&str) -> String` | `"/I%20%E2%99%A5%20Rango/"` => `"/I%20♥%20Rango/"` |
| `escape_uri_path` | `(&str) -> String` | `"/a b/ü"` => `"/a%20b/%C3%BC"` |
| `filepath_to_uri` | `(&str) -> String` | `"a\\b c/ü"` => `"a/b%20c/%C3%BC"` |
| `punycode` | `(domain: &str) -> Result<String, String>` | Encodes an internationalised domain label. |
| `repercent_broken_unicode` | `(&[u8]) -> Vec<u8>` | Percent-encodes invalid UTF-8 bytes. |

## `utils::dateparse` and `utils::duration`

Parsing returns `Ok(None)` when the text does not look like the format and `Err(ParseError)` when it looks right
but is not a real value (for example `2023-02-29`).

```rust
use rango::utils::dateparse::{parse_date, parse_time, parse_datetime, parse_duration};
use rango::utils::duration::{duration_string, duration_iso_string, duration_microseconds};

assert_eq!(parse_date("2024-02-29")?.unwrap().to_string(), "2024-02-29");
assert!(parse_date("2023-02-29").is_err());
let dt = parse_datetime("2012-04-23T10:20:30.400+02:30")?.unwrap(); // ParsedDateTime { naive, offset }
```

| Function | Signature |
|---|---|
| `parse_date` | `(&str) -> Result<Option<NaiveDate>, ParseError>` |
| `parse_time` | `(&str) -> Result<Option<NaiveTime>, ParseError>` (offsets not supported) |
| `parse_datetime` | `(&str) -> Result<Option<ParsedDateTime>, ParseError>`; `ParsedDateTime { naive: NaiveDateTime, offset: Option<FixedOffset> }` |
| `parse_duration` | `(&str) -> Option<chrono::TimeDelta>`; accepts `"1 day, 2:03:04"`, `"-1 days 2:03:04"`, and ISO 8601 durations |
| `duration_string` | `(TimeDelta) -> String`: `1 day 1:02:03` is `"1 01:02:03"`, zero is `"00:00:00"` |
| `duration_iso_string` | `(TimeDelta) -> String`: `"P1DT01H02M03S"` |
| `duration_microseconds` | `(TimeDelta) -> i64` |

## `utils::crypto`

```rust
use rango::utils::crypto::{HashAlgorithm, salted_hmac, constant_time_compare, get_random_string, pbkdf2,
                           RANDOM_STRING_CHARS};
```

| Item | Signature | Notes |
|---|---|---|
| `HashAlgorithm` | `enum { Md5, Sha1, Sha224, Sha256, Sha384, Sha512 }` | `FromStr` from `"sha256"` etc.; `.name()`, `.digest(&[u8])`, `.hmac(key, msg)`. |
| `salted_hmac` | `(key_salt: &[u8], value: &[u8], secret: &[u8], algorithm: HashAlgorithm) -> Vec<u8>` | The HMAC key is `digest(key_salt + secret)`. Use a different `key_salt` per purpose. |
| `get_random_string` | `(length: usize, allowed_chars: &str) -> String` | Cryptographically secure random choice. `RANDOM_STRING_CHARS` is letters and digits. |
| `constant_time_compare` | `(impl AsRef<[u8]>, impl AsRef<[u8]>) -> bool` | Use for comparing tokens and signatures. Different lengths return `false`. |
| `pbkdf2` | `(password: &[u8], salt: &[u8], iterations: u32, dklen: usize, digest: HashAlgorithm) -> Vec<u8>` | `dklen == 0` means the digest size. |

```text
salted_hmac("salt", "value", "secret", sha256) => debaddcd9ecbaab181289870baf8af49b0af0a7050d620d1a63f1e69bd87ceb8 (hex)
pbkdf2("password", "salt", 1000, 0, sha256)    => 632c2812e46d4604102ba7618e9d6d7d2f8128f6266b4a03264d2a0460b7dcb3 (hex)
```

For signing values see [Cryptographic signing](signing.md); for passwords use the auth hashers, not these
primitives.

## `utils::timezone`

Time zones are `chrono_tz::Tz` values. The current zone is task-local, so it follows an `await` chain but not a
task spawned with `tokio::spawn`.

| Function | Signature |
|---|---|
| `get_default_timezone` | `() -> Result<Tz>`: `settings.time_zone`; an invalid name is an "improperly configured" error |
| `get_current_timezone` | `() -> Result<Tz>`: innermost override, else the default |
| `override_tz` | `async (tz: Tz, fut: impl Future) -> F::Output` |
| `override_tz_sync` | `(tz: Tz, body: impl FnOnce() -> R) -> R` |
| `now` | `() -> DateTime<Utc>` |
| `now_naive` | `() -> Result<NaiveDateTime>` in the default zone |
| `localtime` | `(value: DateTime<Utc>, tz: Option<Tz>) -> Result<DateTime<Tz>>` |
| `make_aware`, `make_naive` | `(NaiveDateTime, Tz) -> DateTime<Tz>`, `(&DateTime<Z>, Tz) -> NaiveDateTime` |
| `get_fixed_timezone` | `(offset_minutes: i32) -> Option<FixedOffset>` |

```rust
use rango::utils::timezone::{self, override_tz};
let cairo: chrono_tz::Tz = "Africa/Cairo".parse().unwrap();
let local = override_tz(cairo, async { timezone::localtime(timezone::now(), None) }).await?;
```

## `utils::translation`

Message lookup with `.po`/`.mo` catalogs from `locale_paths` and each installed app's `locale/` directory
(`<dir>/<lang>/LC_MESSAGES/rango.{mo,po}`). Call these where the message is produced so the active language of
the request is used. See [Internationalization](i18n.md) for the full workflow.

| Function | Signature |
|---|---|
| `gettext` | `(&str) -> String` |
| `ngettext` | `(singular: &str, plural: &str, n: u64) -> String` |
| `pgettext` | `(context: &str, message: &str) -> String` |
| `npgettext` | `(context, singular, plural, n) -> String` |
| `get_language`, `get_language_bidi`, `to_locale` | `() -> String`, `() -> bool`, `(&str) -> String` |
| `activate`, `deactivate` | Set or clear the language for the current thread (synchronous code and tests). |
| `add_locale_dir`, `reset_catalogs`, `available_languages` | Register an extra catalog directory, clear the cache, list languages with catalogs. |
| `Catalog` | `from_po(&str)`, `from_mo(&[u8]) -> Option<Catalog>`, `entries()`, `Catalog::to_mo(entries, header) -> Vec<u8>` |

A message with no translation is returned unchanged; `ngettext` falls back to the singular when `n == 1` and to
the plural otherwise.

## `utils::formats`

Locale-aware format lookup and number formatting.

| Function | Signature |
|---|---|
| `get_format` | `(name: &str, lang: Option<&str>) -> serde_json::Value`: the locale's value, else the setting's. Input-format lists also gain the ISO formats. |
| `get_format_str` / `get_format_list` | `(name) -> String` / `(name) -> Vec<String>` |
| `number_grouping` | `() -> Vec<usize>` |
| `number_format` | `(number: &str, decimal_sep: &str, decimal_pos: Option<usize>, grouping: &[usize], thousand_sep: &str, force_grouping: bool) -> String` |
| `localize_number` | `(number: &str, decimal_pos: Option<usize>) -> String` |

`name` is the setting in upper case, for example `"DATE_INPUT_FORMATS"` or `"DECIMAL_SEPARATOR"`.

## `utils::ipv6`

```rust
use rango::utils::ipv6::{clean_ipv6_address, is_valid_ipv6_address, MAX_IPV6_ADDRESS_LENGTH};
```

| Function | Signature | Example |
|---|---|---|
| `is_valid_ipv6_address` | `(&str) -> bool` | `"::1"` => `true`; `"1.2.3.4"` => `false` |
| `clean_ipv6_address` | `(ip_str: &str, unpack_ipv4: bool, error_message: Option<&str>, max_length: usize) -> Result<String, ValidationError>` | `("2001:0DB8:0000:0000:0000:0000:0000:0001", false, None, MAX_IPV6_ADDRESS_LENGTH)` => `"2001:db8::1"` |

## `utils::json`

`dumps(value, compact) -> serde_json::Result<String>` and `dumps_with(value, compact, ensure_ascii)` serialise
with `, ` and `: ` separators (or none when `compact` is `true`) and, by default, escape every non-ASCII character
as `\uXXXX`. Signed values and `json_script` rely on this so their output is pure ASCII.

## `utils::os`

`safe_join(base: &Path, paths: &[&str]) -> rango::Result<PathBuf>` joins path parts and fails with a
"suspicious file operation" error when the result is outside `base`. Use it for any path built from user input,
such as an uploaded file name. `normpath(&Path) -> PathBuf` collapses `.` and `..` components without touching the
file system.

## `utils::regex_helper`

`normalize(pattern: &str) -> Result<Vec<(String, Vec<String>)>, String>` turns a route regular expression into the
format string and parameter names used for reversing, for example
`^articles/(?P<year>[0-9]{4})/$` => `[("articles/%(year)s/", ["year"])]` and `^a/(\d+)/$` =>
`[("a/%(_0)s/", ["_0"])]` (unnamed groups are named `_0`, `_1`, ...). It is mainly of interest to
[URL reversing](http/urls.md).

## `utils::datastructures`

`MultiValueDict<V>` maps a key to a list of values, as query strings and forms need.

```rust
use rango::utils::datastructures::MultiValueDict;

let mut d = MultiValueDict::from_pairs([("tag".to_owned(), "a".to_owned()), ("tag".to_owned(), "b".to_owned())]);
assert_eq!(d.get("tag"), Some(&"b".to_owned()));          // last value wins for get
assert_eq!(d.getlist("tag"), &["a".to_owned(), "b".to_owned()]);
d.appendlist("tag", "c".to_owned());
```

| Method | Meaning |
|---|---|
| `new()`, `from_lists(iter of (String, Vec<V>))`, `from_pairs(iter of (String, V))` | Constructors. |
| `get(&str) -> Option<&V>` | Last value for the key. |
| `try_get(&str) -> Result<&V, MultiValueDictKeyError>` | Like `get` with an error for a missing key. |
| `getlist(&str) -> &[V]` | All values; empty slice when absent. |
| `set(key, value)`, `setlist(key, Vec<V>)` | Replace the values of a key. |
| `setdefault(key, V)`, `setlistdefault(key, Vec<V>)` | Insert only when absent. |
| `appendlist(key, V)`, `update(iter of (String, V))` | Add values. |
| `items()`, `lists()`, `values()`, `keys()` | Iteration (one value per key for `items` and `values`). |
| `contains_key`, `remove(&str) -> Option<Vec<V>>`, `len`, `is_empty`, `to_dict()` | As named. |

## See also

- [Cryptographic signing](signing.md)
- [Security](security.md)
- [Internationalization](i18n.md)
- [Localization](l10n.md)
- [Templates: language](templates/language.md)
- [URLs](http/urls.md)
- [Validators](validators.md)
