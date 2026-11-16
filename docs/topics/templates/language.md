# The Rango template language

This page is for people who write templates. It covers the syntax of the Rango template language: variables,
filters, tags, comments, escaping, inheritance and the limits of what a template can do. For the Rust side
(settings, loading, contexts) see [the template engine](engine.md).

A template is a text file. Three kinds of markers are replaced when it is rendered; everything else is copied to
the output unchanged.

| Marker | Purpose |
|---|---|
| `{{ expression }}` | Output a value. |
| `{% tag ... %}` | Run a tag: logic, loops, inheritance, loading libraries. |
| `{# note #}` | A comment; produces no output. |

```html
{% extends "base.html" %}

{% block content %}
  <h1>{{ question.question_text }}</h1>
  <ul>
    {% for choice in choices %}
      <li>{{ choice.choice_text }} ({{ choice.votes }} vote{{ choice.votes|pluralize }})</li>
    {% empty %}
      <li>No choices yet.</li>
    {% endfor %}
  </ul>
{% endblock %}
```

A marker must open and close **on the same line**. A `{{`, `{%` or `{#` with no matching close before the end of
the line is treated as ordinary text.

## Variables

`{{ name }}` looks `name` up in the context and writes its value. A dot walks into the value:

```html
{{ question.question_text }}   {# attribute or map key #}
{{ choices.0 }}                {# list index #}
{{ title.0 }}                  {# first character of a string #}
{{ request.GET.page }}         {# nested lookups #}
```

Each part after a dot is resolved against the current value according to that value's type:

| Current value | `value.part` means |
|---|---|
| Map | The entry with key `part`. If there is no such key, `items` gives a list of `[key, value]` pairs, `keys` the list of keys and `values` the list of values. |
| List | The element at index `part` (a non-negative integer). |
| String | The character at index `part`. |
| Object (a form, a model instance, the current user, ...) | Whatever attribute the object exposes under `part`; if it has none and the object is iterable, the element at index `part`. |
| Anything else | Nothing: the lookup fails. |

If a lookup produces a callable value, it is called with no arguments and the result is used.

Rules worth knowing:

- Negative indexes (`list.-1`) are not supported. Use the `last` filter.
- A real map key wins over the `items`/`keys`/`values` helpers.
- Variable names and attributes may not start with an underscore: `{{ _secret }}` and `{{ obj._x }}` are syntax
  errors.
- The part after a dot is always a literal name or number. `{{ row.key }}` looks up the key `"key"`, not the
  value of a variable called `key`.

### Literals

Quoted strings (single or double quotes) and numbers can be used anywhere a variable is accepted:

```html
{{ "plain text" }}  {{ 'also text' }}  {{ 42 }}  {{ 3.5 }}
{{ value|default:"nothing" }}
```

Three constants are always defined: `True`, `False` and `None`.

```html
{% if user.last_login is None %}never logged in{% endif %}
```

A string literal written in a template is trusted: it is output without HTML escaping.

### How values are displayed

| Value | Output |
|---|---|
| String | The text (escaped, see [Automatic HTML escaping](#automatic-html-escaping)). |
| Integer, decimal | `42`, `3.50` |
| Float | `1.5`; whole floats keep one decimal: `1.0` |
| Boolean | `True` / `False` |
| None | `None` |
| Date | `Jan. 5, 2024` |
| Time | `2:05 p.m.` |
| Date and time | `Jan. 5, 2024, 2:30 p.m.` |
| List | `[1, 'a', None]` |
| Map | `{'k': 'v'}` |

Use the [`date`](../../ref/template-filters.md#date) and [`time`](../../ref/template-filters.md#time) filters for
other date formats, and `default` / `default_if_none` to replace empty values.

### Undefined variables

A variable that does not exist, or a lookup that fails part-way, does not raise an error:

| Where | Result |
|---|---|
| `{{ missing }}` | The engine's `string_if_invalid` option, which is the empty string by default. Filters are still applied to that empty string, so `{{ missing\|default:"n/a" }}` prints `n/a`. |
| `{% if missing %}`, `{% firstof missing ... %}` | Treated as `None` (false). |
| `{% for x in missing %}` | No iterations; the `{% empty %}` branch runs. |
| A filter argument, `{{ v\|default:missing }}` | `string_if_invalid`. |

If the project sets `string_if_invalid` to a non-empty string, that string is output instead and the filters after
the variable are skipped. A `%s` in it is replaced by the variable name, which is useful for finding typos during
development. Leave it empty in production: a non-empty value is also what `{% for x in missing %}` receives, so
the loop iterates over its characters instead of taking the `{% empty %}` branch. See
[engine options](engine.md#options).

## Filters

A filter transforms a value. Filters are applied with a pipe and can be chained; each filter receives the result of
the previous one.

```html
{{ name|lower }}
{{ name|lower|capfirst }}
{{ bio|truncatewords:30 }}
{{ tags|join:", " }}
{{ value|default:other_variable }}
```

- A filter takes at most one argument, written after a colon. The argument is a quoted string, a number or a
  variable.
- A colon with nothing after it (`{{ v|add: }}`) is a syntax error.
- Filters work everywhere an expression is accepted, including tag arguments: `{% if items|length > 2 %}`,
  `{% for c in title|lower %}`, `{% include "row.html" with label=name|upper %}`.
- An unknown filter name is reported when the expression is evaluated, with the message
  `Invalid filter: 'name'`.

Every built-in filter is listed in the [filter reference](../../ref/template-filters.md).

## Tags

Tags are written `{% name arguments %}`. Some stand alone (`{% csrf_token %}`), others enclose a body and need a
closing tag (`{% if %}...{% endif %}`). Arguments are separated by whitespace; a quoted string counts as one
argument even if it contains spaces.

The sections below cover the tags used most. Every tag is listed in the
[tag reference](../../ref/template-tags.md).

## Comments

```html
{# A one-line comment. #}

{% comment %}
  A multi-line comment. Template code in here is not run:
  {{ this_is_never_evaluated }}
{% endcomment %}
```

`{# ... #}` must fit on one line. Use `{% comment %}` for anything longer. To output template syntax literally, use
`{% verbatim %}...{% endverbatim %}` or `{% templatetag openvariable %}`.

## Automatic HTML escaping

By default every variable is HTML-escaped on output, so data cannot inject markup into the page:

| Character | Output |
|---|---|
| `<` | `&lt;` |
| `>` | `&gt;` |
| `&` | `&amp;` |
| `"` | `&quot;` |
| `'` | `&#x27;` |

```html
{{ comment }}        {# comment = "<b>hi</b>"  ->  &lt;b&gt;hi&lt;/b&gt; #}
```

Values that are already HTML are marked **safe** and are not escaped again. A value becomes safe in one of these
ways:

- the view passed it as a safe string (see [safe strings](engine.md#safe-strings));
- the template applies the `safe` filter: `{{ comment|safe }}`;
- it is the output of a filter that produces HTML (`linebreaks`, `urlize`, `json_script`, `unordered_list`, ...);
- it is a string literal in the template.

Only use `safe` on text you control. Applying it to user input reintroduces the injection risk that escaping
exists to prevent.

Escaping can be switched off for a region:

```html
{% autoescape off %}
  {{ body }}                 {# not escaped #}
  {{ title|escape }}         {# escaped on request #}
{% endautoescape %}
```

`{% autoescape on %}` turns it back on inside an `off` region. The setting carries into included templates and
into blocks.

A filter chain escapes at the point where `escape` appears, not at the end. `{{ v|escape|upper }}` therefore
upper-cases the entities and escapes again (`&amp;LT;A&amp;GT;` for `<a>`); write `{{ v|upper|escape }}`, or leave
`escape` out and let auto-escaping do it.

## Template inheritance

Inheritance lets a base template define the page skeleton and child templates fill in named blocks.

`base.html`:

```html
<!doctype html>
<html>
<head><title>{% block title %}Polls{% endblock %}</title></head>
<body>
  <nav>{% block nav %}<a href="/">Home</a>{% endblock %}</nav>
  {% block content %}{% endblock %}
</body>
</html>
```

`polls/index.html`:

```html
{% extends "base.html" %}

{% block title %}{{ block.super }} - index{% endblock %}

{% block content %}
  {% for q in questions %}<p>{{ q }}</p>{% endfor %}
{% endblock %}
```

Rendering `polls/index.html` produces the skeleton of `base.html` with `title` and `content` replaced, and `nav`
left as the base wrote it.

- `{% extends "name" %}` names the parent. The argument can also be a variable holding a template name.
- `{% block name %}...{% endblock %}` defines a block in the parent and overrides it in a child. A block that the
  child does not mention keeps the parent's content.
- `{{ block.super }}` inside an overriding block inserts the parent's version of that block. It is safe HTML and is
  not escaped again.
- Inheritance can be any number of levels deep. The most derived template's block wins, and `block.super` walks up
  one level at a time.
- Blocks can be nested, and can sit inside `{% if %}`, `{% for %}` or `{% with %}` in the child.
- A template without `{% extends %}` can still use blocks; they render in place.

Put `{% extends %}` on the first line. In a child template only the content of blocks is used; anything else after
`{% extends %}` is discarded. The engine does not reject text placed *before* `{% extends %}` - it is output ahead
of the parent - so do not put anything there.

A missing parent template is an error (`TemplateDoesNotExist`).

## Including other templates

`{% include %}` renders another template at that point, with the current context:

```html
{% include "polls/_choice.html" %}
{% include "polls/_choice.html" with choice=c highlight=True %}
{% include "polls/_choice.html" with choice=c only %}
{% include template_name_variable %}
```

- `with name=value ...` adds variables for the included template only.
- `only` hides the surrounding context: the included template sees nothing but the `with` variables (and `True`,
  `False`, `None`).
- Variables set inside the included template do not leak back out.
- The included template is rendered independently: its `{% block %}` tags are not overridden by the including
  template's inheritance chain.
- A missing template is an error; it is not silently skipped.

Use `include` for fragments and `extends` for page structure.

## Loops

```html
<ul>
{% for choice in choices %}
  <li class="{% cycle 'odd' 'even' %}">{{ forloop.counter }}. {{ choice.choice_text }}</li>
{% empty %}
  <li>Nothing to show.</li>
{% endfor %}
</ul>
```

`{% for %}` iterates over lists, over the keys of a map, over the characters of a string, and over iterable
objects. Anything else (including `None` and missing variables) counts as empty.

- `{% empty %}` is optional and runs when there is nothing to iterate.
- `{% for x in list reversed %}` iterates backwards.
- Several loop variables unpack each item: `{% for key, value in data.items %}`, `{% for x, y in points %}`. An
  item with the wrong number of elements is a render error.
- Loop variables exist only inside the loop.

Inside the loop, `forloop` describes the iteration:

| Variable | Value |
|---|---|
| `forloop.counter` | Iteration number, starting at 1. |
| `forloop.counter0` | Iteration number, starting at 0. |
| `forloop.revcounter` | Iterations left, ending at 1. |
| `forloop.revcounter0` | Iterations left, ending at 0. |
| `forloop.first` | `True` on the first iteration. |
| `forloop.last` | `True` on the last iteration. |
| `forloop.parentloop` | The `forloop` of the enclosing loop, in nested loops. |

```html
{% for row in rows %}{% for cell in row %}
  {{ forloop.parentloop.counter }}.{{ forloop.counter }}
{% endfor %}{% endfor %}
```

There is no `break` or `continue`. Filter the list in the view, or use `slice`: `{% for q in questions|slice:":5" %}`.

## Conditions

```html
{% if user.is_authenticated and not user.is_staff %}
  Welcome back, {{ user.username }}.
{% elif invited %}
  Please log in.
{% else %}
  Hello, guest.
{% endif %}
```

A value is false when it is `None`, `False`, zero, an empty string, an empty list or an empty map. Missing
variables are false. Everything else is true.

Operators, from lowest to highest precedence:

| Precedence | Operators | Notes |
|---|---|---|
| lowest | `or` | Short-circuits. |
| | `and` | Short-circuits. |
| | `not` | Unary. |
| | `in`, `not in` | Membership: element of a list, key of a map, substring of a string, element of an iterable object. |
| highest | `==`, `!=`, `<`, `>`, `<=`, `>=`, `is`, `is not` | `is` compares like `==`; use it for `is None`, `is True`. |

So `{% if a and not b or c %}` is read as `(a and (not b)) or c`.

- Operators must be surrounded by spaces: write `{% if a == 1 %}`, not `{% if a==1 %}`. Without spaces the text is
  taken as one (undefined) variable name and the condition is silently false.
- Parentheses are not supported. Nest `{% if %}` tags, or compute the value in the view.
- Do not chain comparisons (`a < b < c`); combine them with `and`.
- Numbers compare numerically across integer, float, decimal and boolean. Strings compare with strings, dates with
  dates, lists element by element. Comparing values of unrelated types with `<`, `>`, `<=`, `>=` is false rather
  than an error; `==` between unrelated types is false. The string `"1"` is not equal to the number `1`.
- Filters can be used on either side: `{% if messages|length >= 3 %}`.
- A malformed expression (`{% if %}`, `{% if a and %}`) is a syntax error, as is a missing `{% endif %}`.

## Local variables with `with`

`{% with %}` gives a name to a value for the duration of its body. Use it to avoid repeating a long lookup:

```html
{% with total=cart.items|length label="items" %}
  {{ total }} {{ label }}
{% endwith %}

{% with cart.owner.profile as profile %}{{ profile.name }}{% endwith %}
```

Both forms are accepted: `name=value` pairs (several allowed) or `value as name`. The names vanish at
`{% endwith %}`. Many tags can also store their result with `as name` (`url`, `firstof`, `now`, `widthratio`,
`static`, `cycle`); such a variable lives until the end of the enclosing block, loop or `with`.

## Links: `url`

`{% url %}` builds a URL from a route name, so templates never hard-code paths:

```html
<a href="{% url 'polls:detail' question.pk %}">{{ question }}</a>
<a href="{% url 'archive' year=2024 month=3 %}">March</a>
{% url 'polls:results' question.pk as results_url %}
{% if results_url %}<a href="{{ results_url }}">Results</a>{% endif %}
```

- The first argument is the route name: a quoted string or a variable.
- The rest are either positional values or `name=value` pairs. Do not mix the two styles in one tag.
- If no route matches, rendering fails with the reversing error. With `as name`, the variable is set to the empty
  string instead, so the link can be made conditional.

Routes and names are described in [URL dispatching](../http/urls.md).

## Static files: `static`

```html
{% load static %}
<link rel="stylesheet" href="{% static 'polls/style.css' %}">
{% static "img/logo.png" as logo %}<img src="{{ logo }}">
```

`{% static %}` joins the `static_url` setting and the given path, percent-encoding the path. It lives in the
`static` library, so the template must `{% load static %}` first. See [static files](../static-files.md).

## Forms: `csrf_token`

Every form that posts to your own site needs the CSRF token:

```html
<form method="post">
  {% csrf_token %}
  {{ form }}
  <button>Save</button>
</form>
```

The tag writes `<input type="hidden" name="csrfmiddlewaretoken" value="...">`. It produces output only when the
template was rendered with a request (the `render` shortcut does this); otherwise it writes nothing. See
[security](../security.md).

## Loading tag libraries: `load`

Tags that are not built in live in libraries, which a template loads by name:

```html
{% load static i18n %}
{% load cache %}
{% load translate from i18n %}
```

`{% load a b %}` loads whole libraries. `{% load name1 name2 from library %}` loads only the listed tags. An
unknown library, or an unknown name in the `from` form, is a syntax error.

A `{% load %}` applies to the template file it appears in. A child template does not inherit the libraries its
parent loaded, and an included template needs its own `{% load %}`.

Libraries that ship with Rango:

| Library | Provides |
|---|---|
| `static` | `{% static %}`, `{% get_static_prefix %}` |
| `i18n` | `{% translate %}`, `{% blocktranslate %}`, `{% language %}`, `{% get_current_language %}`, `{% get_current_language_bidi %}`, `{% get_available_languages %}` |
| `cache` | `{% cache %}` |

Filters are different: once a filter library is linked into the program, its filters can be used in any template
without `{% load %}`. That includes the `humanize` filters (`intcomma`, `ordinal`, `apnumber`, `naturaltime`).
Projects add their own libraries as described in [custom tags and filters](custom-tags.md).

## Whitespace

Text outside markers is copied exactly, including the newlines and indentation around tags. There is no
whitespace-trimming syntax such as `{%- ... -%}`. Two tools help:

```html
{% spaceless %}
  <ul>
    <li> one </li>
  </ul>
{% endspaceless %}
```

`{% spaceless %}` removes whitespace *between* HTML tags (the output above is `<ul><li> one </li></ul>`); it does
not touch whitespace inside text. For everything else, put tags on the same line as the text they control.

## What templates cannot do

The language is deliberately small. Logic belongs in views; templates decide presentation.

- **No method calls with arguments.** `{{ obj.method }}` works when the object exposes `method` as an attribute or
  a no-argument callable. There is no syntax for `obj.method(arg)`. Compute the value in the view, or write a
  [custom filter or tag](custom-tags.md).
- **No database queries.** Rendering is synchronous and has no database connection. A model instance in a
  template exposes the column values it was loaded with and nothing else: `{{ choice.question }}` gives the
  stored foreign-key value, and there is no `question.choice_set`. Fetch related rows in the view and pass them in
  the context.
- **No assignment or arithmetic.** `{% with %}` and `as name` bind names; `add`, `widthratio` and `divisibleby`
  cover simple arithmetic. Anything more belongs in the view.
- **No arbitrary Rust.** Templates only see what is in the context.
- **No parentheses in `{% if %}`**, no `break`/`continue` in loops, no negative indexes.

## Errors

| Error | Typical cause |
|---|---|
| Syntax error (with a line number) | Unknown tag, unclosed block tag, malformed `{% if %}`, empty `{{ }}`, a filter missing its argument, a name starting with `_`, `{% load %}` of an unknown library. Reported when the template is compiled. |
| Template does not exist | `{% extends %}` or `{% include %}` names a template that cannot be found. |
| Render error | Unknown filter, `{% url %}` with no matching route, unpacking the wrong number of loop values, `divisibleby` with a non-number. Reported while rendering. |

## See also

- [Built-in tag reference](../../ref/template-tags.md)
- [Built-in filter reference](../../ref/template-filters.md)
- [The template engine](engine.md) - settings, loading and rendering from Rust
- [Custom tags and filters](custom-tags.md)
- [Form rendering](../forms/rendering.md)
- [URL dispatching](../http/urls.md)
- [Static files](../static-files.md)
- [Internationalization](../i18n.md)
- [Security](../security.md)
