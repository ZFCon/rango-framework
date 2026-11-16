# Rendering forms

A form renders itself through templates: one for the form layout, one per field group, one per widget and one for
error lists. This page shows the ready-made layouts, how to lay a form out by hand in a template, what a bound
field exposes, and how to restyle or replace the built-in templates.

The HTML samples below are the framework's verified output with blank lines removed.

## Passing a form to a template

`Form` converts into a template value, either implicitly in `context!` or with `form.to_template()`:

```rust
use rango::shortcuts::render;
use rango::context;

render(&req, "contact.html", context! { "form" => form })
```

The form never renders the surrounding `<form>` tag, the CSRF token or the submit button. Those belong to your
template:

```html
<form method="post"{% if form.is_multipart %} enctype="multipart/form-data"{% endif %}>
  {% csrf_token %}
  {{ form }}
  <button type="submit">Save</button>
</form>
```

## The built-in layouts

| In a template | In Rust | Wraps each field in | You provide |
|---|---|---|---|
| `{{ form }}` | `form.render()` | the renderer's default, `<div>` | — |
| `{{ form.as_div }}` | `form.as_div()` | `<div>` | — |
| `{{ form.as_p }}` | `form.as_p()` | `<p>` | — |
| `{{ form.as_table }}` | `form.as_table()` | `<tr><th>…</th><td>…</td></tr>` | the `<table>` element |
| `{{ form.as_ul }}` | `form.as_ul()` | `<li>` | the `<ul>` element |

Given this form:

```rust
use rango::forms::{Field, Form};

let form = Form::new("Person")
    .field("name", Field::char().max_length(20).help_text("Your <b>name</b>"))
    .field("age", Field::integer().min_value(0).required(false))
    .field("email", Field::email().label("E-mail"));
```

`form.as_div()` on the unbound form produces:

```html
<div>
    <label for="id_name">Name:</label>
<div class="helptext" id="id_name_helptext">Your <b>name</b></div>
<input type="text" name="name" maxlength="20" required aria-describedby="id_name_helptext" id="id_name">
</div>
  <div>
    <label for="id_age">Age:</label>
<input type="number" name="age" min="0" id="id_age">
</div>
  <div>
    <label for="id_email">E-mail:</label>
<input type="email" name="email" maxlength="320" required id="id_email">
</div>
```

and `form.as_p()` after binding `name=Ann`, `age=x`, `email=bad`:

```html
<p>
    <label for="id_name">Name:</label>
    <input type="text" name="name" value="Ann" maxlength="20" required aria-describedby="id_name_helptext" id="id_name">
      <span class="helptext" id="id_name_helptext">Your <b>name</b></span>
  </p>
  <ul class="errorlist" id="id_age_error"><li>Enter a whole number.</li></ul>
  <p>
    <label for="id_age">Age:</label>
    <input type="number" name="age" value="x" min="0" aria-invalid="true" aria-describedby="id_age_error" id="id_age">
  </p>
  <ul class="errorlist" id="id_email_error"><li>Enter a valid email address.</li></ul>
  <p>
    <label for="id_email">E-mail:</label>
    <input type="email" name="email" value="bad" maxlength="320" required aria-invalid="true" aria-describedby="id_email_error" id="id_email">
  </p>
```

Things the layouts do for you:

- Labels come from `.label(...)` or from the field name (`first_name` → "First name"), followed by the form's
  label suffix (`:`).
- Ids follow the form's `auto_id` pattern (`id_<name>`); the label's `for` attribute matches.
- Required fields get the HTML `required` attribute; fields with errors get `aria-invalid="true"`; help text and
  error lists are linked with `aria-describedby`.
- Hidden fields are emitted after the last visible field. Errors on hidden fields are listed at the top of the
  form as `(Hidden field <name>) <message>`, together with the non-field errors.
- Widgets that consist of several controls (radio buttons, checkbox lists, split date/time, date selects) are
  wrapped in `<fieldset>` with a `<legend>` by `as_div`.

Help text is inserted **without escaping**, so it may contain markup. Never build help text from user input.

## Rendering fields by hand

Inside a template the form gives access to each field by name. A bound field prints as its widget:

```html
<form method="post">
  {% csrf_token %}
  {{ form.non_field_errors }}

  <div class="row">
    {{ form.name.errors }}
    {{ form.name.label_tag }}
    {{ form.name }}
    {% if form.name.help_text %}<small>{{ form.name.help_text }}</small>{% endif %}
  </div>

  <div class="row">
    {{ form.email.errors }}
    <label for="{{ form.email.id_for_label }}">E-mail address</label>
    {{ form.email }}
  </div>

  <button type="submit">Send</button>
</form>
```

- `{{ form.name }}` renders the widget: `<input type="text" name="name" maxlength="20" required id="id_name">`.
- `{{ form.name.label_tag }}` renders `<label for="id_name">Name:</label>`.
- `{{ form.name.errors }}` renders `<ul class="errorlist" id="id_name_error"><li>…</li></ul>`, or nothing when
  there are no errors.
- `{{ form.name.id_for_label }}` is the id to use in a hand-written `<label for>`.

### Looping over fields

Iterating a form yields its bound fields in order:

```html
{% for field in form %}
  <div class="row{% if field.errors %} has-error{% endif %}">
    {{ field.errors }}
    {{ field.label_tag }} {{ field }}
    {% if field.help_text %}<p class="help">{{ field.help_text }}</p>{% endif %}
  </div>
{% endfor %}
```

To treat hidden fields separately, use the two filtered lists:

```html
{% for field in form.hidden_fields %}{{ field }}{% endfor %}
{% for field in form.visible_fields %}
  <div>{{ field.errors }}{{ field.label_tag }} {{ field }}</div>
{% endfor %}
```

### Field groups

`{{ field.as_field_group }}` renders label, help text, errors and widget in one go, using the
`rango/forms/field.html` template. It is what `as_div` puts inside each `<div>`:

```html
{% for field in form %}
  <div class="row">{{ field.as_field_group }}</div>
{% endfor %}
```

A single field can use its own group template: `Field::char().template_name("forms/fancy_field.html")`.

### Individual radio buttons and checkboxes

Iterating a bound field yields its sub-widgets, one per choice for choice widgets:

```html
{% for radio in form.pick %}
  <label for="{{ radio.id_for_label }}">{{ radio.tag }} {{ radio.choice_label }}</label>
{% endfor %}
```

`{{ radio.tag }}` is the bare `<input>`, `{{ radio }}` is the input wrapped in its label, and
`{{ radio.choice_label }}` is the label text. The sub-widget's data (`name`, `value`, `label`, `selected`, `index`,
`attrs`, `type`) is available as `{{ radio.value }}`, `{{ radio.selected }}` and so on.

## What a form exposes to templates

| Expression | Value |
|---|---|
| `{{ form }}` | the default layout |
| `form.as_div`, `form.as_p`, `form.as_table`, `form.as_ul` | the layouts |
| `form.<field name>` | the bound field |
| `{% for field in form %}` | all bound fields |
| `form.visible_fields`, `form.hidden_fields` | lists of bound fields |
| `form.errors` | all errors; prints as a nested list; `form.errors.<field>` is one field's list |
| `form.non_field_errors` | errors not tied to a field (`<ul class="errorlist nonfield">`) |
| `form.is_bound`, `form.is_valid`, `form.is_multipart` | booleans |
| `form.media` | the widgets' CSS and JavaScript tags |
| `form.prefix`, `form.auto_id` | the form's prefix and id pattern |
| `form.cleaned_data` | map of cleaned values |

The names in the left column take precedence over fields. A field called `errors`, `media` or `prefix` cannot be
reached as `form.errors`; loop over the form to get at it, or pick another field name.

## Bound field attributes

`form.get("name")` returns a `BoundField` in Rust; templates see the same thing as `form.name`.

| Template attribute | Rust | Meaning |
|---|---|---|
| `{{ field }}` | `render()` | the widget (plus a hidden copy of the initial value when `show_hidden_initial` is set) |
| `field.name` | `name` | field name on the form |
| `field.html_name` | `html_name` | the `name` attribute, including the form prefix |
| `field.label` | `label` | label text |
| `field.label_tag` | `label_tag(contents, attrs, label_suffix, tag)` | `<label for="…">Label:</label>`; plain text when ids are disabled |
| `field.legend_tag` | `legend_tag(contents, attrs, label_suffix)` | the same as a `<legend>` |
| `field.id_for_label` | `id_for_label()` | id of the control a label should point at (empty for widgets with several controls) |
| `field.auto_id` | `auto_id()` | the id derived from the form's `auto_id` pattern |
| `field.help_text` | `help_text` | help text, marked safe |
| `field.errors` | `errors()` | the field's `ErrorList` |
| `field.value` | `value()` | what the widget shows: submitted data if bound, else the initial value |
| `field.initial` | `initial()` | the initial value |
| `field.data` | `data()` | the raw submitted value |
| `field.is_hidden` | `is_hidden()` | whether the widget is a hidden input |
| `field.css_classes` | `css_classes(extra)` | the row classes (see below) |
| `field.widget_type` | `widget_type()` | short widget name: `text`, `number`, `email`, `textarea`, `checkbox`, `select`, `selectmultiple`, `radioselect`, `checkboxselectmultiple`, `clearablefile`, `selectdate`, … |
| `field.use_fieldset` | `use_fieldset()` | whether the widget should sit in a `<fieldset>` |
| `field.aria_describedby` | `aria_describedby()` | ids of the help text and error list, when present |
| `field.as_widget` | `as_widget(widget, attrs, only_initial)` | render with the field's widget; in Rust, optionally another widget or extra attributes |
| `field.as_text`, `field.as_textarea`, `field.as_hidden` | `as_text(attrs)`, `as_textarea(attrs)`, `as_hidden(attrs, only_initial)` | render as a different kind of control |
| `field.as_field_group` | `as_field_group()` | label, help text, errors and widget |
| `field.subwidgets`, `{% for w in field %}` | `subwidgets()` | one entry per choice |
| `field.html_initial_name`, `field.html_initial_id` | same | name and id of the hidden initial input |
| `field.field` | `field` | the field definition: `required`, `disabled`, `label`, `help_text`, `max_length`, `show_hidden_initial`, `widget` |
| `field.field.widget` | `field.widget` | `is_hidden`, `is_required`, `needs_multipart_form`, `attrs`, `input_type`, `template_name`, `use_fieldset`, `is_multi` |
| `field.form` | `form` | the owning form |

In Rust, pass extra attributes for a one-off rendering:

```rust
use rango::forms::attrs;

let bf = form.get("name").expect("field exists");
let html = bf.as_widget(None, Some(&attrs! { "class" => "wide", "placeholder" => "Full name" }), false);
let label = bf.label_tag(Some("Full name"), Some(&attrs! { "class" => "lbl" }), None, None);
```

## Error lists

An `ErrorList` prints as an unordered list and can be looped over for full control:

```html
{{ form.name.errors }}
<!-- <ul class="errorlist" id="id_name_error"><li>This field is required.</li></ul> -->

{% if form.name.errors %}
  {% for error in form.name.errors %}<p class="error">{{ error }}</p>{% endfor %}
{% endif %}
```

| Template | Result |
|---|---|
| `{{ errors }}`, `{{ errors.as_ul }}` | `<ul class="errorlist">…</ul>` (empty string when there are no errors) |
| `{{ errors.as_text }}` | `* message` lines |
| `{{ errors.as_json }}` | JSON array of `{"message", "code"}` |
| `{% for error in errors %}` | the message strings |
| `{% if errors %}` | true when not empty |

The list class is `errorlist`; non-field errors use `errorlist nonfield` and formset-level errors
`errorlist nonform`. Field error lists carry the id `<auto_id>_error` so the widget's `aria-describedby` can point
at them.

`{{ form.errors }}` prints every error grouped by field; `{% for name in form.errors %}` yields the field names
and `{{ form.errors.as_json }}` the JSON form.

## Widgets and HTML attributes

Each field has a default widget. Replace it with `.widget(...)`, or keep it and add attributes with
`.widget_attrs(...)`:

```rust
use rango::forms::{attrs, Field, Form, Widget};

let form = Form::new("Comment")
    .field("name", Field::char().widget_attrs(attrs! { "class" => "wide", "autofocus" => true }))
    .field("body", Field::char().widget(Widget::textarea().with_attrs(attrs! { "rows" => "3", "class" => "big" })))
    .field("birthday", Field::date().widget(Widget::date_input().with_attrs(attrs! { "type" => "date" })));
```

```html
<textarea name="body" cols="40" rows="3" class="big" required id="id_body">
</textarea>
<input type="date" name="birthday" required id="id_birthday">
```

`attrs!` builds an `Attrs` map. Values may be strings, `i64` numbers or booleans: `true` renders the bare
attribute (`autofocus`), `false` omits it. Attribute values are escaped.

Three rules worth knowing:

- Call `.widget_attrs(...)` after `.widget(...)`. Replacing the widget discards attributes set on the old one;
  attributes derived from the field (`maxlength`, `min`, `max`, `step`) are re-applied automatically.
- To change an input's `type`, pass it through `Widget::…().with_attrs(...)` as above. `with_attrs` treats `type`
  specially; `widget_attrs` would add a second `type` attribute.
- An `id` given in the widget attributes replaces the automatic one, and labels follow it.

All widgets are described in the [widgets reference](../../ref/widgets.md).

## CSS classes for required and invalid fields

```rust
let form = person_form().required_css_class("req").error_css_class("err");
```

The built-in layouts put these classes on the element that wraps the field (`<div>`, `<p>`, `<tr>` or `<li>`);
the required class is also added to the `<label>`:

```html
<p class="err req">
    <label for="id_name" class="req">Name:</label>
    <input type="text" name="name" maxlength="20" required aria-invalid="true" aria-describedby="id_name_helptext id_name_error" id="id_name">
```

In your own markup use `{{ field.css_classes }}`:

```html
<div class="row {{ field.css_classes }}">…</div>
```

## Media: CSS and JavaScript for widgets

A widget can declare the assets it needs. A form collects the media of all its widgets, without duplicates:

```rust
use rango::db::indexmap::IndexMap;
use rango::forms::{Field, Media, Widget};

let css: IndexMap<String, Vec<&str>> = [("all".to_owned(), vec!["css/calendar.css"])].into_iter().collect();
let calendar = Widget::date_input().with_media(Media::new(css, vec!["js/calendar.js"]));
let field = Field::date().widget(calendar);
```

```html
<head>
  {{ form.media }}
</head>
```

```html
<link href="/static/css/calendar.css" media="all" rel="stylesheet">
<script src="/static/js/calendar.js"></script>
```

- Paths starting with `http://`, `https://` or `/` are used as they are; other paths are prefixed with the
  `static_url` setting.
- `{{ form.media.render_css }}` and `{{ form.media.render_js }}` print the two halves separately. In Rust:
  `media.render()`, `media.render_css()`, `media.render_js()` (the last two return a `Vec<String>` of tags).
- `Media` values add up with `+`; when two lists disagree, the relative order within each list is kept.
- CSS is grouped by medium (`all`, `print`, …) and printed before the scripts.

## Renderers

A renderer decides where the form templates come from. It is a small trait:

```rust
pub trait Renderer: Send + Sync {
    fn render(&self, template_name: &str, context: rango::template::Value) -> Result<String, rango::template::TemplateError>;
    fn form_template_name(&self) -> &str    { "rango/forms/div.html" }
    fn formset_template_name(&self) -> &str { "rango/forms/formsets/div.html" }
    fn field_template_name(&self) -> &str   { "rango/forms/field.html" }
}
```

Two implementations ship in `rango::forms::renderers`:

| Renderer | Looks templates up in |
|---|---|
| `RangoTemplates` (default) | the templates compiled into the framework only |
| `TemplatesSetting` | the project's template engine (the `templates` setting) first, then the built-in set |

### The `form_renderer` setting

```rust
use rango::conf::Settings;

Settings {
    form_renderer: "rango.forms.renderers.TemplatesSetting".into(),
    ..Settings::default()
}
```

The default is `"rango.forms.renderers.RangoTemplates"`. The setting selects between these two renderers; any
other value falls back to `RangoTemplates`. A form picks up the renderer when it is created (`Form::new`), and
passes it on to its bound fields, widgets and error lists.

To use another renderer for one form, set it explicitly:

```rust
use std::sync::Arc;
use rango::forms::renderers::TemplatesSetting;

let form = contact_form().renderer(Arc::new(TemplatesSetting));
```

### Overriding the built-in templates

With `TemplatesSetting` active, a template of the same name in one of your template directories wins over the
built-in one. For example, to change how every error list looks, create
`templates/rango/forms/errors/list/ul.html`:

```html
{% if errors %}<div class="alert">{% for error in errors %}<p>{{ error }}</p>{% endfor %}</div>{% endif %}
```

The built-in template names:

| Template | Renders | Context |
|---|---|---|
| `rango/forms/div.html`, `p.html`, `table.html`, `ul.html` | the form layouts | `form`, `fields` (pairs of bound field and its errors), `hidden_fields`, `errors` (non-field and hidden-field errors) |
| `rango/forms/field.html` | one field group | `field` |
| `rango/forms/label.html` | a label or legend | `field`, `label`, `attrs`, `use_tag`, `tag` |
| `rango/forms/attrs.html` | attributes of a label | `attrs` |
| `rango/forms/errors/list/default.html`, `ul.html`, `text.txt` | an error list | `errors`, `error_class` |
| `rango/forms/errors/dict/default.html`, `ul.html`, `text.txt` | all errors of a form | `errors`, `error_class` |
| `rango/forms/formsets/div.html`, `p.html`, `table.html`, `ul.html` | a formset | `formset` |
| `rango/forms/widgets/*.html` | one widget each (see the [widgets reference](../../ref/widgets.md)) | `widget` |

> **Included templates.** With `TemplatesSetting`, the whole render — including every `{% include %}` inside a
> built-in template — goes through the project's engine. Most built-in widget templates include
> `rango/forms/widgets/input.html` and `rango/forms/widgets/attrs.html`, and the layouts include
> `rango/forms/attrs.html`. Those included templates are only found if your template directories contain them, so
> when switching to `TemplatesSetting`, copy the `rango/forms/` template tree from the framework source
> (`crates/rango-forms/templates/`) into your project and edit the copies.

To change one widget only, leave the renderer alone and point the widget at another template:

```rust
let w = Widget::text_input().with_template("widgets/fancy_text.html");
```

The named template must be loadable by the form's renderer.

### A custom renderer

Implement the trait to change the default layout or to load templates from somewhere else. This renderer makes
`{{ form }}` use the paragraph layout:

```rust
use std::sync::Arc;
use rango::forms::renderers::{RangoTemplates, Renderer};
use rango::template::{TemplateError, Value};

struct ParagraphForms;

impl Renderer for ParagraphForms {
    fn render(&self, template_name: &str, context: Value) -> Result<String, TemplateError> {
        RangoTemplates.render(template_name, context)
    }
    fn form_template_name(&self) -> &str {
        "rango/forms/p.html"
    }
}

let form = contact_form().renderer(Arc::new(ParagraphForms));
```

A custom renderer is attached per form (or per formset); it cannot be named in the `form_renderer` setting.

If a form, field or widget template cannot be loaded or fails to render, the rendering call panics with the
template name. Error-list templates are the exception: a failure there yields an empty string.

## Not supported yet

- Selecting a custom renderer through the `form_renderer` setting.
- Arguments to `label_tag` from templates (`{{ field.label_tag }}` always uses the defaults; call it from Rust
  for custom contents or attributes).
- Localized number and date output per field: `Field::localize(true)` is stored but no built-in widget reads it.

## See also

- [Working with forms](forms.md)
- [Form and field validation](validation.md)
- [Formsets](formsets.md)
- [Widgets reference](../../ref/widgets.md)
- [Form fields reference](../../ref/form-fields.md)
- [The template language](../templates/language.md)
- [Settings reference](../../ref/settings.md)
