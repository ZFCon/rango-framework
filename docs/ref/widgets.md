# Widgets reference

A widget turns a field's value into HTML and reads the submitted value back out of the request data. This page lists
every widget constructor, the HTML it renders, and the options that change it. The examples are the exact output of the
form test-suite (`crates/rango-forms/tests/golden/forms.json`), with form-level `auto_id` `id_%s`.

```rust
use rango::forms::{attrs, Field, Form, Widget};
```

A widget is a plain value (`Widget`), built with a constructor and refined with builder methods. It is attached to a
field with `Field::widget(...)`; every field already has a sensible default (see the
[form fields reference](form-fields.md)).

```rust
let f = Form::new("F").field(
    "bio",
    Field::char().widget(Widget::textarea().with_attrs(attrs! { "rows" => "3", "class" => "big" })),
);
```

## Constructors

All constructors are associated functions of `Widget` and take no arguments unless shown.

| Constructor | Class name | `type` / tag | Notes |
|---|---|---|---|
| `Widget::text_input()` | `TextInput` | `input type="text"` | |
| `Widget::number_input()` | `NumberInput` | `input type="number"` | |
| `Widget::email_input()` | `EmailInput` | `input type="email"` | |
| `Widget::url_input()` | `URLInput` | `input type="url"` | |
| `Widget::color_input()` | `ColorInput` | `input type="color"` | |
| `Widget::search_input()` | `SearchInput` | `input type="search"` | |
| `Widget::tel_input()` | `TelInput` | `input type="tel"` | |
| `Widget::password_input()` | `PasswordInput` | `input type="password"` | Never renders its value unless `.render_value(true)`. |
| `Widget::hidden_input()` | `HiddenInput` | `input type="hidden"` | |
| `Widget::multiple_hidden_input()` | `MultipleHiddenInput` | one hidden input per value | Value is a list. |
| `Widget::file_input()` | `FileInput` | `input type="file"` | Never renders its value. |
| `Widget::clearable_file_input()` | `ClearableFileInput` | file input plus "clear" checkbox | Default for file and image fields. |
| `Widget::textarea()` | `Textarea` | `textarea` | Default attrs `cols="40"`, `rows="10"`. |
| `Widget::date_input()` | `DateInput` | `input type="text"` | Value formatted with the first `DATE_INPUT_FORMATS` entry. |
| `Widget::datetime_input()` | `DateTimeInput` | `input type="text"` | First `DATETIME_INPUT_FORMATS` entry. |
| `Widget::time_input()` | `TimeInput` | `input type="text"` | First `TIME_INPUT_FORMATS` entry. |
| `Widget::checkbox_input()` | `CheckboxInput` | `input type="checkbox"` | Checked when the value is truthy. |
| `Widget::select()` | `Select` | `select` | Uses the widget's choices. |
| `Widget::null_boolean_select()` | `NullBooleanSelect` | `select` | Fixed choices `Unknown`, `Yes`, `No`. |
| `Widget::select_multiple()` | `SelectMultiple` | `select multiple` | |
| `Widget::radio_select()` | `RadioSelect` | `input type="radio"` group | |
| `Widget::checkbox_select_multiple()` | `CheckboxSelectMultiple` | `input type="checkbox"` group | |
| `Widget::multi(widgets, decompress)` | `MultiWidget` | the subwidgets side by side | `decompress: Fn(&Value) -> Vec<Value>` splits one value into one per subwidget; subwidget names are `<name>_0`, `<name>_1`, ... |
| `Widget::split_datetime()` | `SplitDateTimeWidget` | a date input and a time input | Names `<name>_0`, `<name>_1`. |
| `Widget::split_hidden_datetime()` | `SplitHiddenDateTimeWidget` | the same, both hidden | |
| `Widget::select_date(years: Option<Vec<i32>>)` | `SelectDateWidget` | three `select` elements | `None` lists the current year and the next nine. |

## Builder methods

| Method | Effect |
|---|---|
| `with_attrs(Attrs)` | Merge HTML attributes. A `"type"` key on an input widget changes its `type` instead of being rendered as a plain attribute. |
| `with_choices(Vec<Choice>)` | Set choices of a select or radio style widget. |
| `with_format(&str)` | `strftime` pattern for `DateInput`, `DateTimeInput` and `TimeInput` (otherwise the first configured input format). |
| `render_value(bool)` | `PasswordInput` only: echo the submitted value back. |
| `with_template(&str)` | Render with another template name instead of the built-in one. |
| `with_media(Media)` | Attach CSS and JavaScript assets (`Media::new(css, js)`). A form's `media()` merges them. |

`attrs!` builds an `Attrs` map: `attrs! { "class" => "big", "required" => true }`. Values may be `&str`, `String`,
`i64` or `bool`; a `true` attribute renders as a bare flag, `false` is omitted.

Choices are `Choice::option(value, label)` or `Choice::Group(label, vec![(value, label), ...])` (an `<optgroup>`);
`rango::forms::choices([("a", "A"), ("b", "B")])` builds a list of options.

Other public methods worth knowing: `is_hidden()`, `needs_multipart_form()` (true for file widgets, which makes
`form.is_multipart()` true), `input_type()`, `template_name()`, `render(name, &value, attrs, renderer)` to render a
widget on its own, and `media()`.

## Rendered output

The widget element for each case, as rendered by `{{ form.field }}` (field name `f`).

### Text-like inputs

```html
<input type="text" name="f" value="2020-03-04" required id="id_f">
```

`text_input`, `number_input`, `email_input`, `url_input`, `color_input`, `search_input`, `tel_input` and the
temporal inputs share one template: `type`, `name`, `value` (omitted when empty), then the attributes, then
`required` and `id`. Attributes derived by the field come before `required`:

```html
<input type="number" name="f" min="1" max="9" required id="id_f">
<input type="number" name="f" step="0.01" required id="id_f">
<input type="number" name="f" step="any" required id="id_f">
<input type="text" name="f" maxlength="5" id="id_f">
```

The first line is an integer field with `min_value(1).max_value(9)`, the second a decimal field with two places,
the third a float field (floats get `step="any"`), the last an optional char field with `max_length(5)`.

A date field with `Widget::date_input().with_attrs(attrs! { "type" => "date" })`:

```html
<input type="date" name="f" required id="id_f">
```

A date, date-time and time field with initial values `2020-03-04`, `2020-03-04 05:06:07` and `05:06:07`:

```html
<input type="text" name="f" value="2020-03-04" required id="id_f">
<input type="text" name="f" value="2020-03-04 05:06:07" required id="id_f">
<input type="text" name="f" value="05:06:07" required id="id_f">
```

### Password and hidden

```html
<input type="password" name="f" required id="id_f">
<input type="hidden" name="f" value="h" id="id_f">
```

The password widget printed no `value` although `secret` was submitted. A hidden widget never gets `required`.

### Textarea

```html
<textarea name="f" cols="40" rows="10" required id="id_f">
line1
line2</textarea>
```

With `attrs! { "rows" => "3", "class" => "big" }` and the submitted text `<hi>`:

```html
<textarea name="f" cols="40" rows="3" class="big" required id="id_f">
&lt;hi&gt;</textarea>
```

Values are escaped.

### Checkbox

```html
<input type="checkbox" name="f" id="id_f" checked>
```

An optional boolean field, submitted as `on` (or with the initial value `true`). The box is checked when the value
is not `false`, `None` or empty. Submitted values `true`/`false` (any case) are read literally; any other non-empty
value is true; a missing key is false.

### File inputs

```html
<input type="file" name="f" required id="id_f">
```

A file input never shows a value. `ClearableFileInput` (the default for `Field::file()` and `Field::image()`) is
the same element when there is no stored file. When the initial value is a stored name it renders
`Currently: <a href="...">name</a>`, a `<name>-clear` checkbox with a "Clear" label (only when the field is not
required) and `Change:` before the input. Submitting both a new file and the clear box is rejected with the error
code `contradiction`. `required` is left off a file input that already has an initial file.

### Select

```html
<select name="f" id="id_f">
  <option value="a">A</option>

  <option value="b" selected>B</option>

</select>
```

A choice field with choices `a`/`A` and `b`/`B` and submitted value `b`. `required` is only written when the
first choice has an empty value (the "blank" entry); a select without one never carries it.

With a group:

```html
<select name="f" id="id_f">
  <optgroup label="Group">
  <option value="x">X</option>

  <option value="y" selected>Y</option>

  </optgroup>
  <option value="z">Z</option>

</select>
```

An optional field whose first choice is `("", "---")` selects it when nothing is submitted:

```html
<select name="f" id="id_f">
  <option value="" selected>---</option>

  <option value="a">A</option>

</select>
```

### Select multiple

```html
<select name="f" required id="id_f" multiple>
  <option value="a">A</option>

  <option value="b" selected>B</option>

</select>
```

### Null boolean select

```html
<select name="f" id="id_f">
  <option value="unknown">Unknown</option>

  <option value="true" selected>Yes</option>

  <option value="false">No</option>

</select>
```

Submitted `true`, `True` or `2` is true; `false`, `False` or `3` is false; anything else is `None`.

### Radio select

```html
<div id="id_f"><div>
    <label for="id_f_0"><input type="radio" name="f" value="a" required id="id_f_0">
 A</label>

</div><div>
    <label for="id_f_1"><input type="radio" name="f" value="b" required id="id_f_1" checked>
 B</label>

</div>
</div>
```

The wrapper `div` carries the field id, and each option gets `id_f_<index>`. The field's `<label>` has no `for`
attribute (the group is not one control). `form.get("f").unwrap().subwidgets()` yields one `BoundWidget` per option;
`.tag(true)` renders it:

```html
<label for="id_f_0"><input type="radio" name="f" value="a" id="id_f_0" required>
 A</label>
```

### Checkbox select multiple

```html
<div id="id_f"><div>
    <label for="id_f_0"><input type="checkbox" name="f" value="a" id="id_f_0" checked>
 A</label>

</div><div>
    <label for="id_f_1"><input type="checkbox" name="f" value="b" id="id_f_1">
 B</label>

</div><div>
    <label for="id_f_2"><input type="checkbox" name="f" value="c" id="id_f_2" checked>
 C</label>

</div>
</div>
```

`required` is never written on the individual boxes.

### Multiple hidden input

```html
<input type="hidden" name="f" value="a" id="id_f_0"><input type="hidden" name="f" value="b" id="id_f_1">
```

### Split date and time

```html
<input type="text" name="f_0" value="2020-03-04" required id="id_f_0"><input type="text" name="f_1" value="05:06:07" required id="id_f_1">
```

For a `Field::split_datetime()` with the initial value `2020-03-04 05:06:07`. The value is converted to the
current time zone before it is split.

### Select date

`Widget::select_date(None)` on a date field renders three selects in the order of the `DATE_FORMAT` setting
(month, day, year for the default format). Each is a plain `<select>` named `<name>_month`, `<name>_day` and
`<name>_year` with ids `id_<name>_month` and so on:

```html
<select name="f_month" required id="id_f_month"><option value="1">January</option><option value="2">February</option><option value="3" selected>March</option>...</select>
<select name="f_day" required id="id_f_day"><option value="1">1</option>...<option value="4" selected>4</option>...</select>
<select name="f_year" required id="id_f_year"><option value="2026">2026</option>...</select>
```

(Whitespace and the elided options are abbreviated here; in the output the three elements are adjacent.) When the
field is optional each select starts with an empty `---` entry. The submitted triple is combined into a date string
in the first `DATE_INPUT_FORMATS` format; three empty parts mean "no value", and an impossible date (such as 31 February)
is passed on as `year-month-day` text so that the date field reports `invalid`.
The field's label `for` points at the first select.

### Multi widget

`Widget::multi(widgets, decompress)` renders every subwidget (named `<name>_0`, `<name>_1`, ...) one after another.
Pair it with `Field::multi_value(fields, widget, compress)`; see the form fields reference.

## Submitted data

How each widget reads the request (`Widget::value_from_datadict`):

| Widget | Reads |
|---|---|
| Most inputs, `Select`, `RadioSelect`, `Textarea` | the first value for `name`, as text; `None` when absent |
| `SelectMultiple`, `CheckboxSelectMultiple`, `MultipleHiddenInput` | every value for `name`, as a list |
| `FileInput` | the uploaded file named `name` |
| `FileInput` with `multiple` set in `WidgetKind::File { multiple: true }` | every uploaded file named `name` (no constructor builds this; set `widget.kind` yourself) |
| `ClearableFileInput` | the upload, or "cleared" when `<name>-clear` is set |
| `CheckboxInput` | boolean, see above |
| `NullBooleanSelect` | `Some(true)`, `Some(false)` or `None` |
| `MultiWidget`, `SplitDateTimeWidget` | the list of the subwidgets' values, names `<name>_0`, ... |
| `SelectDateWidget` | `<name>_year`, `<name>_month`, `<name>_day` combined |

A widget also reports whether its value was omitted from the data (`value_omitted_from_data`); a checkbox or a
multi-select counts as never omitted, which is why an unchecked box means `false` rather than "keep the default".

## Media

`Media::new(css: IndexMap<String, Vec<&str>>, js: Vec<&str>)` collects stylesheet paths per medium and script paths.
Adding two `Media` values with `+` merges them, keeping every list's own order. `render()` returns the
`<link>` and `<script>` tags; `render_css()` and `render_js()` return them separately. Relative paths are prefixed with
the `static_url` setting; paths starting with `/`, `http://` or `https://` are used as they are.

```rust
use indexmap::IndexMap;
use rango::forms::{Media, Widget};

let mut css = IndexMap::new();
css.insert("all".to_owned(), vec!["pretty.css"]);
let w = Widget::text_input().with_media(Media::new(css, vec!["pretty.js"]));
```

In a template, `{{ form.media }}` renders the merged tags.

## Not supported yet

- `CheckboxInput` cannot be given a custom "is checked" test through a constructor or builder.
- There is no constructor for a multiple-file input.
- `SelectDateWidget` only offers `years` as an option; month names and the empty labels cannot be changed through a
  constructor.

## See also

- [Form fields reference](form-fields.md)
- [Rendering forms](../topics/forms/rendering.md)
- [Working with forms](../topics/forms/forms.md)
