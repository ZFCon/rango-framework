# Custom template tags and filters

When the built-in tags and filters are not enough, a project can add its own. This page shows how to write a tag
library in Rust: filters, simple tags, inclusion tags and full block tags, how to register the library and how
templates load it.

```rust
use std::sync::Arc;

use rango::template::{Context, Engine, Library, TemplateError, Value};
use rango::template::nodes::{CustomNode, Node, render_nodes, render_value, resolve_expr};
use rango::template::parser::{FilterExpr, parse_filter_expr};
```

## A library

A `Library` is a named collection of tags and filters. Build it in a function and register that function under
the name templates will use with `{% load %}`:

```rust
// src/polls/templatetags.rs
use rango::template::Library;

fn polls_tags() -> Library {
    let mut l = Library::new();
    // l.filter(...), l.simple_tag(...), l.inclusion_tag(...), l.tag(...)
    l
}

rango::register_library!("polls_tags", polls_tags);
```

`register_library!(name, build)` takes the load name and a `fn() -> Library`. Registration happens at link time, so
the module only has to be part of the program (`mod templatetags;` in the app's `mod.rs`); there is nothing to
add to settings.

| Method | Registers |
|---|---|
| `Library::new()` | An empty library. |
| `filter(name, f, is_safe, needs_autoescape)` | A filter. |
| `simple_tag(name, f)` | A tag that takes arguments and returns one value. |
| `inclusion_tag(name, template, f)` | A tag that renders another template. |
| `tag(name, compile)` | A tag with its own parsing and rendering. |

Each method returns `&mut Self`, so calls can be chained. The `tags` and `filters` maps are public fields.

## Using a library in templates

```html
{% load polls_tags %}
{% greeting user.username punct="?" %}
{{ total|times:2 }}
```

- **Tags** must be loaded: `{% load polls_tags %}` in every template file that uses them (a child template does
  not inherit its parent's loads). `{% load repeat greeting from polls_tags %}` loads only the listed names.
- **Filters** need no `{% load %}`. Once a registered library is linked into the program, its filters resolve by
  name in every template. Choose filter names that will not collide with another library's; when two registered
  libraries define the same filter name, which one is used is unspecified.
- A built-in filter always wins over a library filter of the same name.
- The names of built-in tags (`if`, `for`, `block`, `include`, `url`, ...) are reserved. A library tag with one of
  those names is never reached.
- When two loaded libraries define the same tag, the one loaded last wins.

Two engine options change how libraries are found (see [engine options](engine.md#options)):

```rust
"OPTIONS": {
    "builtins": ["polls_tags"],                    // tags available everywhere, no {% load %}
    "libraries": { "p": "polls_tags" },            // {% load p %}
}
```

## Filters

A filter is a plain function:

```rust
fn(value: &Value, arg: Option<&Value>, autoescape: bool) -> Result<Value, TemplateError>
```

`value` is the input, `arg` the optional argument after the colon, and `autoescape` whether escaping is active
where the filter is used. The alias is `rango::template::engine::FilterFn`.

```rust
use rango::template::{Library, TemplateError, Value};

/// {{ 3|times:4 }} -> 12
fn times(value: &Value, arg: Option<&Value>, _autoescape: bool) -> Result<Value, TemplateError> {
    match (value.as_i64(), arg.and_then(Value::as_i64)) {
        (Some(a), Some(b)) => Ok(Value::Int(a * b)),
        _ => Ok(Value::Str(String::new())),
    }
}

/// {{ "Ada Lovelace"|initials }} -> AL
fn initials(value: &Value, _arg: Option<&Value>, _autoescape: bool) -> Result<Value, TemplateError> {
    let out: String = value.to_str().split_whitespace().filter_map(|w| w.chars().next()).collect();
    Ok(Value::Str(out))
}

fn polls_tags() -> Library {
    let mut l = Library::new();
    l.filter("times", times, false, false);
    l.filter("initials", initials, false, false);
    l
}
```

Non-capturing closures work too: `l.filter("shout", |v, _, _| Ok(Value::Str(v.to_str().to_uppercase())), false, false);`.

Guidelines, following what the built-in filters do:

- Be forgiving. Templates are edited by people who cannot see your Rust code; for unusable input return the input
  unchanged or an empty string rather than an error. Return `Err(TemplateError::Render(..))` only when continuing
  would hide a real mistake (the built-in `divisibleby` does this for non-numbers).
- Read input with `value.to_str()`, `as_i64()`, `as_f64()`, `iterate()`, `truthy()` or by matching on the `Value`
  variants.
- A filter takes at most one argument. To pass several, accept a string and split it, as `yesno:"yes,no,maybe"`
  does.
- When the argument is a variable that does not exist, the filter receives the engine's `string_if_invalid`
  (an empty string by default), not `None`. `None` means no argument was written.

### Filters and escaping

Whether a filter's result is escaped depends on what it returns and on the `is_safe` flag.

| Filter returns | `is_safe` | Result |
|---|---|---|
| `Value::Safe(..)` | either | Safe. Never escaped. |
| `Value::Str(..)` | `true` | Safe if the input was safe, otherwise escaped on output. |
| `Value::Str(..)` | `false` | Escaped on output. |
| any other variant | either | Displayed as usual; strings inside lists are still plain strings. |

Set `is_safe` to `true` when the filter cannot introduce HTML-significant characters: if the input was safe, the
output still is. `lower`, `capfirst` and `truncatewords` are registered this way. Leave it `false` when the filter
may produce `<`, `>`, `&` or quotes from harmless input, or when it removes characters in a way that could
unbalance markup.

A filter that **generates HTML** returns `Value::Safe` and therefore must escape its input itself. Use the
`autoescape` parameter and the input's own safety, the way the built-in `linebreaksbr` does:

```rust
use rango::template::{TemplateError, Value};
use rango::utils::html::escape;

/// {{ term|highlight }} -> <mark>term</mark>
fn highlight(value: &Value, _arg: Option<&Value>, autoescape: bool) -> Result<Value, TemplateError> {
    let text = value.to_str();
    let text = if autoescape && !value.is_safe() { escape(&text).into_string() } else { text };
    Ok(Value::Safe(format!("<mark>{text}</mark>")))
}

// l.filter("highlight", highlight, true, true);
```

Flags:

| Flag | Meaning |
|---|---|
| `is_safe` | Described above. |
| `needs_autoescape` | Declares that the filter reads the `autoescape` parameter. It is informational: every filter receives the parameter whether or not the flag is set. |

There is no `expects_localtime` flag. Date-time values carry their UTC offset (`Value::DateTime(naive, offset)`);
a filter that needs a particular time zone converts the value itself.

## Simple tags

A simple tag takes any number of arguments, does some work and returns one value. Parsing of the arguments,
`as name` and escaping of the result are handled for you.

```rust
use std::sync::Arc;
use rango::template::{Library, Value};

fn polls_tags() -> Library {
    let mut l = Library::new();
    l.simple_tag("greeting", Arc::new(|_ctx, args, kwargs| {
        let name = args.first().map(Value::to_str).unwrap_or_default();
        let punct = kwargs.get("punct").map(Value::to_str).unwrap_or_else(|| "!".to_owned());
        Ok(Value::Str(format!("Hello, {name}{punct}")))
    }));
    l
}
```

```html
{% load polls_tags %}
{% greeting "Ann" %}                       {# Hello, Ann! #}
{% greeting user.username punct="?" %}     {# Hello, ann? #}
{% greeting "Ann" as hello %}{{ hello }}   {# stored, then printed #}
```

The function type is `rango::template::engine::SimpleTag`:

```rust
Arc<dyn Fn(&mut Context, Vec<Value>, IndexMap<String, Value>) -> Result<Value, TemplateError> + Send + Sync>
```

- Arguments written as `name=value` (the name made of letters, digits and underscores) arrive in the map; all
  others arrive in the vector, in order. Each argument is a full expression, so filters work:
  `{% greeting user.username|title %}`.
- The tag does not check how many arguments it received. Validate `args.len()` in the function and return
  `TemplateError::Render` if the call is wrong.
- The returned value is written like a variable: escaped unless it is `Value::Safe`. Return
  `rango::template::safe(html)` for markup you built with `format_html!`.
- With `as name` the value is stored in the context instead of being written.
- The function receives the context. Read other variables with `ctx.get("name")`, or set some with
  `ctx.set("name", value)`.

The built-in i18n library defines its lookup tags this way:

```rust
l.simple_tag("get_current_language", Arc::new(|_c, _a, _k| Ok(Value::Str(get_language()))));
```

which is why `{% get_current_language as lang %}` works without any parsing code.

## Inclusion tags

An inclusion tag computes a small context and renders a template with it. Use it for reusable widgets.

```rust
use std::sync::Arc;
use rango::template::{Library, Value};

fn polls_tags() -> Library {
    let mut l = Library::new();
    l.inclusion_tag("choice_list", "polls/_choices.html", Arc::new(|_ctx, args, kwargs| {
        let choices = args.into_iter().next().unwrap_or(Value::None);
        let title = kwargs.get("title").cloned().unwrap_or(Value::from("Choices"));
        Ok([("choices".to_owned(), choices), ("title".to_owned(), title)].into_iter().collect())
    }));
    l
}
```

`polls/_choices.html`:

```html
<h3>{{ title }}</h3>
<ul>{% for c in choices %}<li>{{ c.choice_text }}</li>{% endfor %}</ul>
```

```html
{% load polls_tags %}
{% choice_list choices title="Results" %}
```

- Arguments are passed exactly as for simple tags. The function returns the variables for the included template
  as an `IndexMap<String, Value>`.
- The template is rendered with a **new** context holding only those variables (plus `csrf_token`, when the outer
  context has one, so forms inside the fragment work). It does not see the caller's variables; pass what it needs,
  or read it from the `ctx` parameter and add it to the returned map.
- The auto-escaping state of the caller carries over.
- The template is looked up through the engine when the tag renders. A missing file is a
  `TemplateError::DoesNotExist` at that moment.
- `as name` is not supported on inclusion tags.

## Block tags and other custom nodes

For tags that enclose a body, take unusual syntax or need control over rendering, register a **compile function**
and return a node.

1. The compile function runs once, when the template is compiled. It receives the parser and the tag's words,
   checks the syntax, parses any body, and returns a `Node`.
2. The node's `render` runs each time the template is rendered.

The following `{% repeat n %}...{% endrepeat %}` tag renders its body `n` times and exposes the iteration number:

```rust
use std::sync::Arc;

use rango::template::{Context, Engine, Library, TemplateError};
use rango::template::nodes::{CustomNode, Node, render_nodes, resolve_expr};
use rango::template::parser::{FilterExpr, parse_filter_expr};

#[derive(Debug)]
struct RepeatNode {
    count: FilterExpr,
    nodes: Vec<Node>,
}

impl CustomNode for RepeatNode {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError> {
        let n = resolve_expr(&self.count, ctx, engine)?
            .as_i64()
            .ok_or_else(|| TemplateError::Render("'repeat' needs a number".into()))?;
        let mut out = String::new();
        let mut result = Ok(());
        ctx.push();
        for i in 0..n.max(0) {
            ctx.set("iteration", i + 1);
            result = render_nodes(&self.nodes, ctx, engine, &mut out);
            if result.is_err() {
                break;
            }
        }
        ctx.pop();
        result.map(|()| out)
    }
}

fn polls_tags() -> Library {
    let mut l = Library::new();
    l.tag("repeat", Arc::new(|p, bits| {
        if bits.len() != 2 {
            return Err(TemplateError::Syntax("'repeat' takes one argument".into()));
        }
        let count = parse_filter_expr(&bits[1])?;
        let (nodes, _) = p.parse(&["endrepeat"])?;
        Ok(Node::Custom(Arc::new(RepeatNode { count, nodes })))
    }));
    l
}
```

```html
{% load polls_tags %}
{% repeat 3 %}<span>{{ iteration }}</span>{% endrepeat %}
```

### The compile function

Its type is `rango::template::engine::TagCompile`:

```rust
Arc<dyn Fn(&mut Parser<'_>, Vec<String>) -> Result<Node, TemplateError> + Send + Sync>
```

`bits` is the tag's content split on whitespace, with quoted strings kept whole (quotes included). `bits[0]` is
the tag name. Return `TemplateError::Syntax` for a malformed tag; the engine appends the line number.

Parser methods available to compile functions:

| Method | Purpose |
|---|---|
| `p.parse(&["endx", ...])` | Parse nodes up to the first tag whose name is in the list. Returns `(nodes, Some(contents of that tag))`. If the template ends first, this is an "Unclosed tag" syntax error. |
| `p.skip_past("endx")` | Discard everything up to and including `{% endx %}` without compiling it. |
| `p.raw_until("endx")` | Return the source text up to `{% endx %}` uncompiled. |
| `p.next_id()` | A number unique within this template, for nodes that keep per-node state. |

Turn an argument into an expression with `rango::template::parser::parse_filter_expr`. It accepts everything
`{{ }}` accepts: variables, literals and filter chains.

Tags with a middle section parse in two steps and look at which tag ended the first part. The built-in
`{% ifchanged %}` is compiled like this:

```rust
let (nodes, term) = p.parse(&["else", "endifchanged"])?;
let else_nodes = if term.as_deref() == Some("else") { p.parse(&["endifchanged"])?.0 } else { vec![] };
```

A tag without a body does not call `p.parse` at all; it only reads `bits` and returns its node.

### The node

A node implements `rango::template::nodes::CustomNode`:

```rust
pub trait CustomNode: Send + Sync + std::fmt::Debug {
    fn render(&self, ctx: &mut Context, engine: &Engine) -> Result<String, TemplateError>;
}
```

Helpers from `rango::template::nodes`:

| Function | Purpose |
|---|---|
| `resolve_expr(&FilterExpr, ctx, engine) -> Result<Value, TemplateError>` | Evaluate an expression. A missing variable gives `string_if_invalid`. |
| `render_nodes(&[Node], ctx, engine, &mut String) -> Result<(), TemplateError>` | Render a parsed body into a buffer. |
| `render_value(&Value, ctx) -> String` | Format a value the way `{{ }}` does, applying auto-escaping. |

Rules for `render`:

- **The returned string is written to the output as is.** It is not escaped. Pass any value that came from the
  context through `render_value`, or escape it with `rango::utils::html::escape`, before putting it in the result.
  Text produced by `render_nodes` is already escaped.
- Nodes are shared between renders and threads, so `render` takes `&self`. Keep per-render data in the context,
  not in the node.
- If the tag introduces variables, call `ctx.push()` before and `ctx.pop()` after, on the error path as well, so
  they do not leak into the rest of the template.
- To store a result instead of writing it (an `as name` option), call `ctx.set(name, value)` and return an empty
  string.

The built-in `{% cache %}` tag is a complete example of the pattern: its compile function reads the timeout,
fragment name and vary-on expressions from `bits` and parses the body up to `{% endcache %}`; its node resolves the
expressions, returns the cached fragment if there is one, and otherwise renders the body with `render_nodes` and
stores the result.

Built-in node kinds can also be returned directly. The `static` library's compile function, for instance, returns
`Node::Static { path, asvar }` rather than defining a node type of its own.

## Testing a library

A library can be exercised without settings by attaching it to an `Engine`:

```rust
use std::sync::Arc;
use rango::context;
use rango::template::Engine;

#[test]
fn repeat_and_times() {
    let mut engine = Engine::new();
    engine.extra_libraries.push(("polls_tags".to_owned(), Arc::new(polls_tags())));
    engine.locmem(&[(
        "t.html",
        "{% load polls_tags %}{% repeat 2 %}{{ iteration }}{% endrepeat %}|{{ 3|times:4 }}",
    )]);
    let out = engine.render_to_string("t.html", &mut context! {}).unwrap();
    assert_eq!(out, "12|12");
}
```

`extra_libraries` is a list of `(load name, Arc<Library>)` pairs consulted before the registered libraries, for
both `{% load %}` and filter lookup.

## Not supported

- Filters with more than one argument.
- An `expects_localtime` filter flag.
- `as name` on inclusion tags, and automatic argument-count checking for simple and inclusion tags.
- Overriding a built-in tag.
- Scoping filters to the templates that load their library: registered filters are global.

## See also

- [The Rango template language](language.md)
- [The template engine](engine.md)
- [Built-in tag reference](../../ref/template-tags.md)
- [Built-in filter reference](../../ref/template-filters.md)
- [Internationalization](../i18n.md) - the `i18n` library
- [Cache](../cache.md) - the `cache` library
- [Static files](../static-files.md) - the `static` library
