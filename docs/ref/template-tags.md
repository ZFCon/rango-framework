# Template tag reference

Every built-in tag of the Rango template language: the core tags that are always available, and the tags that live in the loadable libraries `static`, `i18n` and `cache`. For how tags fit into templates see [the template language](../topics/templates/language.md); to write your own see [custom tags and filters](../topics/templates/custom-tags.md).

Examples marked with a result show output that is verified by the engine's golden tests. Other outputs are derived from the implementation.

## Conventions

- Arguments are separated by whitespace. A quoted string (`'a b'` or `"a b"`) is one argument.
- Arguments that are expressions accept variables, literals and filters: `{% include name|default:'x.html' %}`.
- `as name` stores a tag's result in a context variable instead of printing it. Tags that support it say so.
- Unknown tags raise a syntax error at compile time: `Invalid block tag: 'name'`.

| Tag | Closing tag | Library |
|---|---|---|
| [`autoescape`](#autoescape) | `endautoescape` | built in |
| [`block`](#block) | `endblock` | built in |
| [`comment`](#comment) | `endcomment` | built in |
| [`csrf_token`](#csrf_token) | | built in |
| [`cycle`](#cycle) | | built in |
| [`debug`](#debug) | | built in |
| [`extends`](#extends) | | built in |
| [`filter`](#filter) | `endfilter` | built in |
| [`firstof`](#firstof) | | built in |
| [`for`](#for) | `endfor` | built in |
| [`if`](#if) | `endif` | built in |
| [`ifchanged`](#ifchanged) | `endifchanged` | built in |
| [`include`](#include) | | built in |
| [`load`](#load) | | built in |
| [`lorem`](#lorem) | | built in |
| [`now`](#now) | | built in |
| [`regroup`](#regroup) | | built in |
| [`spaceless`](#spaceless) | `endspaceless` | built in |
| [`templatetag`](#templatetag) | | built in |
| [`url`](#url) | | built in |
| [`verbatim`](#verbatim) | `endverbatim` | built in |
| [`with`](#with) | `endwith` | built in |
| [`widthratio`](#widthratio) | | built in |
| [`static`](#static), [`get_static_prefix`](#get_static_prefix) | | `static` |
| [`trans` / `translate`](#trans--translate) | | `i18n` |
| [`blocktrans` / `blocktranslate`](#blocktrans--blocktranslate) | `endblocktrans` / `endblocktranslate` | `i18n` |
| [`language`](#language) | `endlanguage` | `i18n` |
| [`get_current_language`](#i18n-getters), [`get_current_language_bidi`](#i18n-getters), [`get_available_languages`](#i18n-getters) | | `i18n` |
| [`cache`](#cache) | `endcache` | `cache` |

## Core tags

### autoescape

```html
{% autoescape on|off %} ... {% endautoescape %}
```

Turns automatic HTML escaping on or off for the enclosed block. The argument must be exactly `on` or `off`; anything else is a syntax error. The previous setting is restored after the block.

```html
{% autoescape off %}{{ h }}{% endautoescape %}|{{ h }}
```

With `h = "<b>"` this renders `<b>|&lt;b&gt;`.

### block

```html
{% block name %} ... {% endblock %}
```

Defines a named region that child templates can override. Takes exactly one argument, the block name. Inside an overriding block, `{{ block.super }}` outputs the parent's content. Blocks can be nested. Outside inheritance a block renders its own content.

```html
{% extends 'child.html' %}{% block b %}[{{ block.super }}]{% endblock %}
```

With the shared templates used by the tests (`base.html` is `<{% block a %}A{% endblock %}|{% block b %}B{% endblock %}>`, and `child.html` overrides `a` with `[{{ block.super }}x]`), this renders `<[Ax]|[B]>`.

### comment

```html
{% comment %} ... {% endcomment %}
```

Everything up to `{% endcomment %}` is dropped, including tags and variables. For a single line use `{# ... #}`.

```html
{# comment #}x{% comment %}hidden {{ x }}{% endcomment %}y
```

Renders `xy`.

### csrf_token

```html
{% csrf_token %}
```

Writes a hidden input carrying the request's CSRF token:

```html
<input type="hidden" name="csrfmiddlewaretoken" value="...">
```

It looks up `csrf_token` in the context (the CSRF context processor and the request-aware render helpers provide it). If the variable is missing, empty, or `NOTPROVIDED`, the tag outputs nothing.

### cycle

```html
{% cycle value1 value2 ... %}
{% cycle value1 value2 ... as name %}
{% cycle value1 value2 ... as name silent %}
{% cycle name %}
```

Each evaluation of the same tag moves to the next value, wrapping around. Values are expressions (variables or quoted strings). With `as name` the current value is also stored in the variable `name`. `silent` stores the value without printing it. `{% cycle name %}` advances a previously named cycle. Referring to a name that does not exist is a syntax error (`Named cycle 'name' does not exist`).

The state of a cycle belongs to one render of the template. Two separate `{% cycle %}` tags with identical arguments are independent cycles, as the first example shows.

```html
{% for i in l %}{% cycle 'x' 'y' as c %}{{ c }}{% endfor %}
```

With `l = [1, 2, 3]` this renders `xxyyxx` (the tag prints the value, then `{{ c }}` prints it again).

```html
{% cycle 'a' 'b' 'c' %}{% cycle 'a' 'b' 'c' %}
```

Renders `aa`: each tag instance has its own counter.

```html
{% cycle 'a' 'b' as c silent %}{{ c }}{% cycle c %}
```

Renders `a`: the `silent` tag stores `a` without printing it, `{{ c }}` prints `a`, and `{% cycle c %}` advances the cycle but stays silent because the named cycle was declared `silent`.

### debug

```html
{% debug %}
```

Outputs the placeholder `<pre>{}</pre>`. It does not dump the context.

### extends

```html
{% extends "base.html" %}
{% extends parent_variable %}
```

Makes this template a child of another. It takes one argument: a quoted template name or a variable that holds the name. Content outside `{% block %}` tags in a child is ignored. Chains of any depth work, and `block.super` works at each level. A missing parent raises `TemplateDoesNotExist`.

```html
{% extends 'grand.html' %}
```

With the test templates this renders `<<<[Ax]>>|G>`.

### filter

```html
{% filter filter1|filter2:arg %} ... {% endfilter %}
```

Applies one or more filters to the rendered content of the block. `escape` and `safe` are not allowed here; using them is a syntax error (`"filter" tag does not accept 'escape' or 'safe' filters`).

```html
{% filter lower %}ABC {{ v }}{% endfilter %}
```

With `v = "DEF"` this renders `abc def`.

### firstof

```html
{% firstof var1 var2 'fallback' %}
{% firstof var1 var2 as result %}
```

Outputs the first argument that is truthy, or nothing if none is. With `as name` the value is stored instead. The output is escaped like any variable. At least one argument is required.

```html
{% firstof a b 'z' %}
```

With `a = 0, b = "bee"` renders `bee`. `{% firstof a b as r %}[{{ r }}]` with `b = "<x>"` renders `[&lt;x&gt;]`.

### for

```html
{% for item in items %} ... {% endfor %}
{% for item in items reversed %} ... {% endfor %}
{% for key, value in pairs %} ... {% empty %} ... {% endfor %}
```

Loops over a list, a string (by character), or a map (by key). Several loop variables unpack each item: `{% for a, b in l %}` needs every item to have exactly two values, otherwise rendering fails. Iterate over a map's entries with `d.items`. `reversed` must be the last word. `{% empty %}` renders when the sequence is empty or missing. A malformed header is a syntax error (`'for' statements should use the format 'for x in y'`).

Inside the loop the `forloop` variable is available:

| Variable | Meaning |
|---|---|
| `forloop.counter` | Current iteration, starting at 1. |
| `forloop.counter0` | Current iteration, starting at 0. |
| `forloop.revcounter` | Iterations left, ending at 1. |
| `forloop.revcounter0` | Iterations left, ending at 0. |
| `forloop.first` | True on the first iteration. |
| `forloop.last` | True on the last iteration. |
| `forloop.parentloop` | The enclosing loop's `forloop`, in nested loops. |

```html
{% for x in l %}{{ forloop.counter }}{{ forloop.counter0 }}{{ forloop.revcounter }}{{ forloop.revcounter0 }}{% if forloop.first %}F{% endif %}{% if forloop.last %}L{% endif %};{% endfor %}
```

With `l = ["a", "b", "c"]` renders `1032F;2121;3210L;`.

```html
{% for k, v in d.items %}{{ k }}:{{ v }};{% endfor %}
```

With `d = {"a": 1, "b": 2}` renders `a:1;b:2;`.

### if

```html
{% if condition %} ... {% elif condition %} ... {% else %} ... {% endif %}
```

Renders the first branch whose condition is truthy. Empty strings, empty lists and maps, `0`, `false` and `None` are false; a missing variable is false. Conditions combine with `and`, `or`, `not`, and the comparison operators `==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `not in`, `is` and `is not`. Filters are allowed in operands. `not` binds tighter than `and`, which binds tighter than `or`. Parentheses are not supported. An empty condition or a missing `{% endif %}` is a syntax error.

```html
{% if a and not b or c %}t{% else %}f{% endif %}
```

With `a = 1, b = 1, c = 0` renders `f`.

```html
{% if x in l %}in{% endif %}{% if y not in l %}nin{% endif %}{% if a is None %}none{% endif %}
```

Comparing values of incomparable types (for example a string with a number using `>`) is false rather than an error.

### ifchanged

```html
{% ifchanged %} ... {% else %} ... {% endifchanged %}
{% ifchanged var1 var2 %} ... {% endifchanged %}
```

Without arguments, renders its body only when the body's own output differs from the previous evaluation. With arguments, compares the listed values instead. The optional `{% else %}` branch renders when nothing changed.

```html
{% for x in l %}{% ifchanged x %}{{ x }}{% endifchanged %}{% endfor %}
```

With `l = [1, 1, 2, 2, 3]` renders `123`. With `{% ifchanged %}{{ x }}{% else %}-{% endifchanged %}` and `[1, 1, 2]` renders `1-2`.

### include

```html
{% include "partial.html" %}
{% include "partial.html" with a=1 b=var %}
{% include "partial.html" with a=1 only %}
{% include template_name_variable %}
```

Renders another template with the current context. `with` adds or overrides variables for the included template only (the pairs are `name=value`). `only` renders the included template with just those variables and nothing else from the context. A missing template raises `TemplateDoesNotExist`. Any other word after the template name is a syntax error (`Unknown argument for 'include' tag`).

```html
{% include 'inc.html' with v=2 x='z' %}
```

The test template `inc.html` is `inc:{{ v }}:{{ x|default:'-' }}`. With context `v = 1` this renders `inc:2:z`; with `only` and no `x` it renders `inc:2:-`.

### load

```html
{% load static i18n %}
{% load name1 name2 from library %}
```

Makes the tags of the named libraries available to the rest of the template. The built-in loadable libraries are `static`, `i18n` and `cache`; project libraries come from `register_library!` (see [custom tags and filters](../topics/templates/custom-tags.md)). The `from` form loads only the named tags or filters from one library. Loading a library that does not exist is a syntax error (`'name' is not a registered tag library`), as is naming something the library does not contain.

Filters of registered libraries and of `humanize` are found by name at render time and do not need `{% load %}`; see the [filter reference](template-filters.md#humanize-filters).

### lorem

```html
{% lorem %}
{% lorem count %}
{% lorem count method %}
{% lorem count method random %}
```

Generates placeholder text. `count` is a number or variable (default 1). `method` is `w` (words), `p` (paragraphs wrapped in `<p>` tags) or `b` (plain-text paragraph blocks, the default). Without `random` the output begins with the standard opening words (or the standard first paragraph); `random` omits that.

```html
{% lorem 3 w %}
```

Renders `lorem ipsum dolor`. `{% lorem 5 w %}` renders `lorem ipsum dolor sit amet`.

### now

```html
{% now "Y-m-d" %}
{% now "jS F Y H:i" as stamp %}
```

Outputs the current date and time (in the active time zone) using a [date format string](template-filters.md#date-and-time-format-characters). The format must be a quoted string literal, not a variable. Unlike the `date` filter, named formats such as `DATE_FORMAT` are not expanded. With `as name` the result is stored instead of printed. Format characters that need a missing part cannot occur here, since the full date, time and offset are available.

### regroup

```html
{% regroup list by attribute as groups %}
```

Groups consecutive items that share the same value of `attribute` (dotted paths allowed) and stores a list of groups in `groups`. Each group has `grouper` (the shared value) and `list` (its items). The list should already be sorted by the attribute, since only adjacent items are grouped; `dictsort` can sort a list of maps first. The tag needs exactly five words: `regroup`, the list, `by`, the attribute, `as`, the name. Anything else is a syntax error.

```html
{% regroup p by g as grp %}{% for g in grp %}{{ g.grouper }}:{% for i in g.list %}{{ i.n }}{% endfor %};{% endfor %}
```

With `p = [{"g": "a", "n": 1}, {"g": "a", "n": 2}, {"g": "b", "n": 3}]` renders `a:12;b:3;`.

### spaceless

```html
{% spaceless %} ... {% endspaceless %}
```

Removes whitespace between HTML tags in the block, and trims the block's leading and trailing whitespace. Whitespace inside text is kept.

```html
{% spaceless %}<p>  <b> x </b>  </p> <i> y </i>{% endspaceless %}
```

Renders `<p><b> x </b></p><i> y </i>`.

### templatetag

```html
{% templatetag openblock %}
```

Outputs one of the character sequences that is otherwise reserved by the template syntax. Takes one argument:

| Argument | Output |
|---|---|
| `openblock` | `{%` |
| `closeblock` | `%}` |
| `openvariable` | `{{` |
| `closevariable` | `}}` |
| `openbrace` | `{` |
| `closebrace` | `}` |
| `opencomment` | `{#` |
| `closecomment` | `#}` |

`{% templatetag openblock %}{% templatetag closevariable %}{% templatetag openbrace %}` renders `{%}}{`. Any other argument is a syntax error.

### url

```html
{% url "route-name" arg1 arg2 %}
{% url "route-name" key=value other=var %}
{% url "route-name" arg as link %}
```

Reverses a named route to a path (see [URL configuration](../topics/http/urls.md)). The name may be namespaced (`"app:name"`). Arguments are either all positional or all `name=value` pairs. With `as name` the URL is stored in the variable and a failed reverse stores an empty string; without `as`, a failed reverse is a render error. The printed path is HTML-escaped when autoescape is on.

### verbatim

```html
{% verbatim %} ... {% endverbatim %}
{% verbatim name %} ... {% endverbatim name %}
```

Outputs the enclosed text without interpreting template syntax. The optional name lets the block contain a plain `{% endverbatim %}`.

```html
{% verbatim %}{{ x }}{% if %}{% endverbatim %}
```

Renders `{{ x }}{% if %}`.

### with

```html
{% with a=1 b=other %} ... {% endwith %}
{% with value as name %} ... {% endwith %}
```

Binds variables for the block only; they are removed at `{% endwith %}`. Useful to cache an expensive lookup. Both the `name=value` form and the single-value `value as name` form are accepted. A `with` without any assignment is a syntax error.

```html
{% with a=1 b=2 %}{{ a }}{{ b }}{% endwith %}{{ a }}
```

Renders `12`.

### widthratio

```html
{% widthratio value max_value width %}
{% widthratio value max_value width as ratio %}
```

Computes `value / max_value * width` and rounds to the nearest integer (halves go to the even number). If `max_value` is zero or any argument is not a number the result is `0`. Exactly three arguments are required (plus the optional `as name`).

```html
{% widthratio a b 100 %}
```

With `a = 175, b = 200` renders `88`. With `a = 5, b = 2` and width `10` it renders `25`.

## Library: static

```html
{% load static %}
```

### static

```html
{% static "css/site.css" %}
{% static path_variable as url %}
```

Outputs the URL of a static file: the `static_url` setting followed by the percent-encoded path. The path is an expression. With `as name` the URL is stored instead of printed. At least one argument is required. This tag does not consult a manifest or hash the name; it only joins the prefix and path.

```html
{% load static %}{% static 'a/b.css' %}
```

With the default `static_url` renders `/static/a/b.css`.

### get_static_prefix

```html
{% get_static_prefix %}
{% get_static_prefix as prefix %}
```

Outputs the `static_url` setting (empty when unset).

## Library: i18n

```html
{% load i18n %}
```

The tags use the active language and message catalogs; see [internationalization](../topics/i18n.md).

### trans / translate

```html
{% trans "Hello" %}
{% translate "Hello" context "greeting" as msg %}
{% trans "Hello" noop %}
{% trans variable %}
```

Translates a string literal or variable. `translate` is an alias of `trans`. Options after the string, in any order:

| Option | Meaning |
|---|---|
| `context "name"` | Message context used to pick between translations of the same text. |
| `noop` | Mark the string for extraction but return it untranslated. |
| `as name` | Store the result in `name` instead of printing it. |

Any other word is a syntax error (`Unknown argument for 'trans' tag`). A translated string literal is output without escaping; a translated variable is escaped when autoescape is on.

### blocktrans / blocktranslate

```html
{% blocktranslate %}Hello {{ name }}.{% endblocktranslate %}

{% blocktranslate count n=items|length %}
  One item.
{% plural %}
  {{ n }} items.
{% endblocktranslate %}
```

Translates a block that contains text and simple variables. `blocktrans` is an alias, and either end tag (`endblocktrans`, `endblocktranslate`) closes either opening tag. Options on the opening tag:

| Option | Meaning |
|---|---|
| `with a=expr b=expr` | Bind variables for use inside the block. A bare `name=value` also works. |
| `count name=expr` | The number selecting the singular or plural form. Needs a `{% plural %}` section. The value is also available as `name`. |
| `context "name"` | Message context. |
| `trimmed` | Strip each line and join lines with one space, so source indentation does not reach the message. |
| `asvar name` | Store the result in `name` instead of printing it. |

Inside the block only plain text and bare variable names are allowed. Filters (`{{ name|upper }}`), dotted names (`{{ user.name }}`) and tags raise a syntax error; bind such values with `with` first. Variables are escaped when autoescape is on and the value is not marked safe. A literal `%` in the text is handled for you.

### language

```html
{% language "de" %} ... {% endlanguage %}
```

Renders the block with another language active, then restores the previous one. Takes exactly one argument, a language code (quoted or a variable).

### i18n getters

| Tag | Output |
|---|---|
| `{% get_current_language %}` | The active language code, such as `en-us`. |
| `{% get_current_language_bidi %}` | `True` if the active language is written right to left. |
| `{% get_available_languages %}` | List of `[code, name]` pairs from the `languages` setting. |

Like any simple tag these accept `as name`:

```html
{% get_available_languages as langs %}
{% for code, name in langs %}<option value="{{ code }}">{{ name }}</option>{% endfor %}
```

## Library: cache

```html
{% load cache %}
```

### cache

```html
{% cache timeout fragment_name vary1 vary2 %} ... {% endcache %}
{% cache 600 sidebar request_user_id using="default" %} ... {% endcache %}
```

Caches the rendered output of a block. `timeout` is a number of seconds (an expression; a value of `None` caches without expiry). `fragment_name` is a literal word that names the fragment. Further arguments vary the cache key: the fragment is stored separately for each combination of their values. `using="alias"` as the last argument selects a cache alias other than `default`. The key has the form `template.cache.<name>.<md5 of the vary values>`.

If the timeout is not a number the tag raises a render error (`'cache' tag got a non-integer timeout value`). Fewer than two arguments, or a missing `{% endcache %}`, is a syntax error. Cache errors, such as an unknown alias, are render errors. See [the cache framework](../topics/cache.md).

## Not supported

- `{% debug %}` does not print the context.
- `{% static %}` does not use a manifest; use the storage in [static files](../topics/static-files.md) for hashed names.
- Tags from other libraries are only available when registered with `register_library!`.

## See also

- [Template filter reference](template-filters.md)
- [The template language](../topics/templates/language.md)
- [The template engine](../topics/templates/engine.md)
- [Custom tags and filters](../topics/templates/custom-tags.md)
