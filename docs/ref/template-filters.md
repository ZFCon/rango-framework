# Template filter reference

Every built-in filter of the Rango template language, the `humanize` filters, and the table of date and time format characters used by `date`, `time` and `{% now %}`. For how filters work see [the template language](../topics/templates/language.md); to add your own see [custom tags and filters](../topics/templates/custom-tags.md).

Filters are applied with a pipe: `{{ value|filter }}` or `{{ value|filter:argument }}`. A filter takes at most one argument. Filters chain left to right. Examples below show the template, the input context and the output. The outputs listed with a context are taken from the engine's golden tests, run with autoescape on, so any characters like `'` appear in their escaped form (`&#x27;`) when the result is not marked safe.

Errors: an unknown filter name is an error (`Invalid filter: 'name'`), and a filter written with a colon but no argument (`{{ v|add: }}`) is a syntax error.

## Contents

- [Text](#text): `addslashes`, `capfirst`, `center`, `cut`, `ljust`, `lower`, `phone2numeric`, `rjust`, `slugify`, `stringformat`, `title`, `truncatechars`, `truncatechars_html`, `truncatewords`, `truncatewords_html`, `upper`, `wordcount`, `wordwrap`
- [HTML and escaping](#html-and-escaping): `escape`, `force_escape`, `escapejs`, `safe`, `safeseq`, `striptags`, `linebreaks`, `linebreaksbr`, `linenumbers`, `urlize`, `urlizetrunc`, `urlencode`, `iriencode`, `json_script`, `unordered_list`
- [Numbers](#numbers): `add`, `divisibleby`, `filesizeformat`, `floatformat`, `get_digit`, `pluralize`
- [Lists and values](#lists-and-values): `dictsort`, `dictsortreversed`, `first`, `join`, `last`, `length`, `make_list`, `random`, `slice`, `default`, `default_if_none`, `yesno`, `pprint`
- [Dates and times](#dates-and-times): `date`, `time`, `timesince`, `timeuntil`
- [Humanize filters](#humanize-filters): `intcomma`, `ordinal`, `apnumber`, `naturaltime`
- [Date and time format characters](#date-and-time-format-characters)

## Text

### addslashes

Puts a backslash before backslashes, double quotes and single quotes.

```html
{{ v|addslashes }}
```

`it's "q" \` renders `it\&#x27;s \&quot;q\&quot; \\` (the result is then escaped for HTML).

### capfirst

Uppercases the first character. Works with non-ASCII letters.

`hello` renders `Hello`; `élan` renders `Élan`; an empty string stays empty.

### center

Argument: the total width. Pads the value with spaces on both sides to that width. When the padding is odd the extra space goes to the left if the width is odd, otherwise to the right. A value already as wide as the argument is returned unchanged.

`{{ v|center:'11' }}` with `ab` renders `     ab    ` (5 spaces, then 4); with `abc` it renders `    abc    `.

### cut

Argument: a string. Removes every occurrence of it from the value.

`{{ v|cut:' ' }}` with `a b c` renders `abc`.

### ljust

Argument: width. Left-aligns in a field of that width, padding on the right with spaces.

`{{ v|ljust:'6' }}` with `ab` renders `ab    `.

### lower

Converts to lowercase, including non-ASCII letters. `ABC Ä` renders `abc ä`.

### phone2numeric

Converts letters in a phone number to digits using the telephone keypad. `1-800-COLLECT` renders `1-800-2655328`.

### rjust

Argument: width. Right-aligns in a field of that width.

`{{ v|rjust:'6' }}` with `ab` renders `    ab`.

### slugify

Converts to ASCII, lowercases, removes characters that are not letters, digits, underscores, hyphens or spaces, and turns runs of spaces and hyphens into single hyphens. The output is marked safe.

`Hello, World! 日本` renders `hello-world`.

### stringformat

Argument: a printf-style specifier without the leading `%`. Supported conversions: `s`, `d`, `i`, `f`, `F`, `e`, `x`, `X`, `o`, with flags `-`, `+`, ` `, `#`, `0`, a width and a precision (`.N`). An unsupported specifier, or a value that cannot be converted, produces an empty string.

| Template | Input | Output |
|---|---|---|
| `{{ v\|stringformat:'05d' }}` | `42` | `00042` |
| `{{ v\|stringformat:'.2f' }}` | `3.14159` | `3.14` |
| `{{ v\|stringformat:'s' }}` | `x` | `x` |
| `{{ v\|stringformat:'x' }}` | `255` | `ff` |

### title

Converts to title case: the first letter of each word uppercase, the rest lowercase. An apostrophe followed by a letter does not start a new word.

`hello world` renders `Hello World`; `it's a o'neil` renders `It's A O'Neil` (apostrophes shown escaped as `&#x27;` in HTML output).

### truncatechars

Argument: maximum length in characters, including the ellipsis character `…`. Shorter values are unchanged. A non-numeric argument returns the value unchanged.

`{{ v|truncatechars:5 }}` with `abcdefgh` renders `abcd…`; with `abc` it renders `abc`. `{{ v|truncatechars:1 }}` with `abc` renders `…`.

### truncatechars_html

Same argument as `truncatechars`. In this implementation it uses the same plain-text truncation, so it does not close or preserve tags; strip or sanitize HTML first if that matters.

### truncatewords

Argument: number of words to keep. When the value is longer, the result ends with ` …` (a space and an ellipsis).

`{{ v|truncatewords:2 }}` with `a b c d` renders `a b …`; with `a b` it renders `a b`.

### truncatewords_html

Same argument as `truncatewords`, with the same plain-text behaviour as `truncatechars_html`.

### upper

Converts to uppercase. `abc ä` renders `ABC Ä`.

### wordcount

Number of whitespace-separated words. `a b  c` followed by a newline and `d` renders `4`.

### wordwrap

Argument: line width. Breaks lines at word boundaries so they are at most that wide. A word longer than the width is kept whole on its own line.

`{{ v|wordwrap:5 }}` with `aaa bbb ccc ddd` renders the four words on four lines (`aaa`, `bbb`, `ccc`, `ddd`, separated by newlines); with `toolongword x` it renders `toolongword` and `x` on two lines.

## HTML and escaping

Autoescape is on by default: values that are not marked safe are escaped when output (`<` becomes `&lt;`, `>` becomes `&gt;`, `&` becomes `&amp;`, `"` becomes `&quot;`, `'` becomes `&#x27;`). A filter chain escapes at the point where `escape` or another escaping filter appears.

### escape

Escapes the value for HTML immediately, and marks the result safe so it is not escaped again. Applying it to a value already marked safe does nothing.

`{{ v|escape }}` with `<&>` renders `&lt;&amp;&gt;`. `{{ v|upper|escape }}` with `<a>` renders `&lt;A&gt;`, while `{{ v|escape|upper }}` renders `&amp;LT;A&amp;GT;` because the already-escaped text is uppercased and then escaped again.

### force_escape

Always escapes, even for values marked safe. `{{ v|force_escape }}` with `<a>` renders `&lt;a&gt;`.

### escapejs

Escapes the value for use inside a JavaScript string literal, using `\uXXXX` sequences for quotes, angle brackets, ampersands, backslashes, control characters and similar. The output is marked safe.

`a'b"c` plus a newline plus `<>` renders `a'b"c\u000A<>`.

### safe

Marks the value as safe so autoescape leaves it alone. Use it only for HTML you trust. `{{ h|safe }}` with `<b>` renders `<b>`.

### safeseq

Applies `safe` to each item of a list. Useful before `join`.

`{{ v|safeseq|join:',' }}` with `["<a>", "b"]` renders `<a>,b`.

### striptags

Removes HTML tags. `<b>bold</b> text` renders `bold text`. A lone `<` that does not start a tag is kept, then escaped on output: `a < b` renders `a &lt; b`.

### linebreaks

Converts plain text to HTML: blank lines separate paragraphs (`<p>`), single newlines become `<br>`. The text is escaped when autoescape is on.

`a`, newline, `b`, two newlines, `c` renders `<p>a<br>b</p>`, a blank line, then `<p>c</p>`.

### linebreaksbr

Converts every newline to `<br>`. `a` newline `b` renders `a<br>b`.

### linenumbers

Prefixes each line with its number, zero-padded to the width of the largest number. `a`, `b`, `c` on three lines renders `1. a`, `2. b`, `3. c` on three lines.

### urlize

Turns URLs that start with `http://`, `https://` or `www.` into links with `rel="nofollow"`. Trailing `.`, `,`, `:`, `;` and `!` are kept outside the link. Other text is escaped when autoescape is on.

| Input | Output |
|---|---|
| `see http://example.com/a?b=1, ok` | `see <a href="http://example.com/a?b=1" rel="nofollow">http://example.com/a?b=1</a>, ok` |
| `www.example.org.` | `<a href="http://www.example.org" rel="nofollow">www.example.org</a>.` |
| `a<b http://x.y` | `a&lt;b <a href="http://x.y" rel="nofollow">http://x.y</a>` |
| `no link` | `no link` |

Email addresses are not turned into links.

### urlizetrunc

Like `urlize`, with the argument giving the maximum number of characters of link text. Longer link text is cut and ends with `…`.

### urlencode

Percent-encodes the value for use in a URL. The optional argument lists characters that are left unescaped; the default is `/`.

`{{ v|urlencode }}` with `a b/c?d=é` renders `a%20b/c%3Fd%3D%C3%A9`; `{{ v|urlencode:'' }}` with `a b/c` renders `a%20b%2Fc`.

### iriencode

Converts an IRI to a URI by percent-encoding the characters that are not allowed in URIs, leaving already-escaped sequences alone.

### json_script

Argument: optional element id. Serializes the value as JSON and wraps it in a `<script type="application/json">` element. Characters that could end the script (`<`, `>`, `&`) are escaped, so the output is safe to embed. Without an id the element has no `id` attribute.

`{{ v|json_script:'id1' }}` with `{"a": [1, "é"]}` renders `<script id="id1" type="application/json">{"a": [1, "é"]}</script>`.

### unordered_list

Renders nested lists as nested `<li>` and `<ul>` elements (no outer `<ul>`). An item followed by a list is that item's sublist. Indentation uses tabs. Items are escaped when autoescape is on.

`["a", ["b", "c"], "d"]` renders:

```html
	<li>a
	<ul>
		<li>b</li>
		<li>c</li>
	</ul>
	</li>
	<li>d</li>
```

## Numbers

### add

Argument: the value to add. If both sides can be read as integers (numbers or strings of digits), the result is their sum, with floats truncated to integers first. Otherwise lists are concatenated and two strings are joined. Any other combination renders an empty string.

| Template | Input | Output |
|---|---|---|
| `{{ v\|add:3 }}` | `5` | `8` |
| `{{ v\|add:3 }}` | `"2"` | `5` |
| `{{ v\|add:3 }}` | `"a"` | (empty) |
| `{{ v\|add:3 }}` | `1.5` | `4` |
| `{{ v\|add:3 }}` | `[1]` | (empty) |
| `{{ v\|add:'x' }}` | `"a"` | `ax` |
| `{{ v\|add:'x' }}` | `1` | (empty) |
| `{{ v\|add:1\|add:2 }}` | `1` | `4` |

### divisibleby

Argument: an integer. Outputs `True` or `False`. A non-numeric value or a zero divisor is a render error.

`{{ v|divisibleby:3 }}` renders `True` for `9` and `False` for `10`; for `"x"` it fails to render.

### filesizeformat

Formats a number of bytes in a readable unit: `bytes`, `KB`, `MB`, `GB`, `TB`, `PB` (binary multiples of 1024). The number and unit are separated by a non-breaking space. Non-numeric input renders `0 bytes`. One decimal place is shown above `KB`.

| Input | Output |
|---|---|
| `0` | `0 bytes` |
| `1` | `1 byte` |
| `1023` | `1023 bytes` |
| `1024` | `1.0 KB` |
| `1536` | `1.5 KB` |
| `1048576` | `1.0 MB` |
| `123456789` | `117.7 MB` |
| `1000000000000` | `931.3 GB` |
| `-2048` | `-2.0 KB` |

### floatformat

Argument: number of decimal places (default `-1`). Rounds halves away from zero.

- A positive argument always shows that many decimals.
- A negative argument shows up to that many decimals, and none if the number is whole.
- `0` rounds to an integer.
- Non-numeric input renders an empty string. An argument that is not a number returns the value unchanged.
- A trailing `g` (grouping) or `u` (no localization) on the argument is accepted but has no effect in this implementation.
- Negative zero is shown as `0`.

| Template | Input | Output |
|---|---|---|
| `{{ v\|floatformat }}` | `34.23234` | `34.2` |
| `{{ v\|floatformat }}` | `34.0` | `34` |
| `{{ v\|floatformat }}` | `34.26` | `34.3` |
| `{{ v\|floatformat }}` | `1234.5678` | `1234.6` |
| `{{ v\|floatformat }}` | `"abc"` | (empty) |
| `{{ v\|floatformat:2 }}` | `34.0` | `34.00` |
| `{{ v\|floatformat:2 }}` | `34.265` | `34.27` |
| `{{ v\|floatformat:2 }}` | `1.005` | `1.01` |
| `{{ v\|floatformat:2 }}` | `-0.001` | `0.00` |
| `{{ v\|floatformat:2 }}` | `"3"` | `3.00` |
| `{{ v\|floatformat:'-3' }}` | `34.23234` | `34.232` |
| `{{ v\|floatformat:'-3' }}` | `34.0` | `34` |
| `{{ v\|floatformat:0 }}` | `34.5` | `35` |
| `{{ v\|floatformat:0 }}` | `0.4` | `0` |

Write a negative argument as a quoted string (`'-3'`).

### get_digit

Argument: a position counted from the right, starting at 1. Returns that digit of an integer, or `0` if the number is shorter. An argument below 1, or a value that is not an integer, returns the value unchanged.

`{{ v|get_digit:2 }}` with `123` renders `2`; with `5` it renders `0`; with `abc` it renders `abc`.

### pluralize

Argument: optional suffixes. With no argument the plural suffix is `s`. One suffix (`'es'`) is the plural, with an empty singular. Two comma-separated suffixes (`'y,ies'`) are singular then plural. The singular is used when the value is `1` or a list with one item; a non-numeric string counts as one when it has one character. More than two suffixes renders an empty string.

| Template | Input | Output |
|---|---|---|
| `{{ v\|pluralize }}` | `1` | (empty) |
| `{{ v\|pluralize }}` | `2` or `0` | `s` |
| `{{ v\|pluralize }}` | `[1]` | (empty) |
| `{{ v\|pluralize }}` | `[1, 2]` | `s` |
| `{{ v\|pluralize }}` | `"1"` | (empty) |
| `{{ v\|pluralize:'y,ies' }}` | `1` | `y` |
| `{{ v\|pluralize:'y,ies' }}` | `2` | `ies` |
| `{{ v\|pluralize:'es' }}` | `1` | (empty) |
| `{{ v\|pluralize:'es' }}` | `3` | `es` |

## Lists and values

### dictsort

Argument: a key (dotted paths and numeric indexes allowed). Sorts a list of maps by that key, ascending. Input that is not a list renders an empty string.

`{{ v|dictsort:'n' }}` with `[{"n": 3}, {"n": 1}, {"n": 2}]` sorts to the items with `n` 1, 2, 3.

### dictsortreversed

Same as `dictsort`, in descending order.

### first

First item of a list, or first character of a string. An empty list renders an empty string.

`[1, 2]` renders `1`; `abc` renders `a`.

### join

Argument: the separator. Joins the items of a list. With autoescape on, the items and the separator are escaped unless marked safe; the result is marked safe. A value that is not a list is returned unchanged.

`{{ v|join:', ' }}` with `[1, 2, 3]` renders `1, 2, 3`; with `["<a>", "b"]` it renders `&lt;a&gt;, b`. A separator written as a quoted literal in the template is trusted, so `{{ v|join:'<' }}` with `["a", "b"]` renders `a<b`.

### last

Last item of a list or last character of a string. An empty list renders an empty string. `[1, 2]` renders `2`; `abc` renders `c`.

### length

Number of items in a list or map, or characters in a string. Anything else (numbers, `None`) is `0`.

### make_list

Splits a string, or the digits of a number, into a list of one-character strings. `abc` and `123` render as lists such as `['a', 'b', 'c']`.

### random

A random item from a list. A value that is not a non-empty list is returned unchanged.

### slice

Argument: a slice specification `start:stop:step`; every part is optional and negative numbers count from the end. Works on lists and strings. A zero step returns the value unchanged; a negative step returns an empty result.

| Template | Input | Output |
|---|---|---|
| `{{ v\|slice:':2' }}` | `"abcde"` | `ab` |
| `{{ v\|slice:':2' }}` | `[1, 2, 3]` | `[1, 2]` |
| `{{ v\|slice:'1:3' }}` | `"abcde"` | `bc` |
| `{{ v\|slice:'::2' }}` | `"abcde"` | `ace` |
| `{{ v\|slice:'-2:' }}` | `"abcde"` | `de` |

### default

Argument: the fallback. Uses it when the value is false: `None`, empty string, `0`, empty list or map, `false`, or a missing variable. The argument may be a variable.

| Template | Input | Output |
|---|---|---|
| `{{ v\|default:'d' }}` | `""` | `d` |
| `{{ v\|default:'d' }}` | `None` | `d` |
| `{{ v\|default:'d' }}` | `0` | `d` |
| `{{ v\|default:'d' }}` | `[]` | `d` |
| `{{ v\|default:'d' }}` | `"x"` | `x` |
| `{{ v\|default:other }}` | `""`, other = `"o"` | `o` |
| `{{ a.b\|default:'n' }}` | `a = {}` | `n` |

### default_if_none

Argument: the fallback. Uses it only when the value is `None`; empty strings and `0` pass through.

`{{ v|default_if_none:'d' }}` renders `d` for `None`, an empty string for `""`, and `0` for `0`.

### yesno

Argument: comma-separated words for true, false and (optionally) `None`. The default is `yes,no,maybe`. With only two words, `None` uses the second (false) word. An argument with fewer than two words returns the value unchanged.

| Template | Input | Output |
|---|---|---|
| `{{ v\|yesno }}` | `true` | `yes` |
| `{{ v\|yesno }}` | `false` | `no` |
| `{{ v\|yesno }}` | `None` | `maybe` |
| `{{ v\|yesno:'ja,nein' }}` | `true` | `ja` |
| `{{ v\|yesno:'ja,nein' }}` | `None` | `nein` |
| `{{ v\|yesno:'a,b,c' }}` | `None` | `c` |

### pprint

Outputs the debug representation of the value, such as `['a', 'b']` or `{'k': 'v'}`. Meant for debugging.

## Dates and times

Dates and times come from the context as date, time or date-time values. A value of another type (an empty string, `None`, a plain string) renders an empty string. See [localization](../topics/l10n.md) and [the time zone utilities](../topics/utils.md) for how aware date-times are produced.

### date

Argument: a [format string](#date-and-time-format-characters), or the name of a format setting. Default argument: `DATE_FORMAT`. Applies to dates and date-times; a value with no date part fails and renders an empty string.

The named formats are `DATE_FORMAT`, `DATETIME_FORMAT`, `SHORT_DATE_FORMAT`, `SHORT_DATETIME_FORMAT`, `TIME_FORMAT`, `YEAR_MONTH_FORMAT` and `MONTH_DAY_FORMAT`. They read the matching settings (defaults `N j, Y`, `N j, Y, P`, `m/d/Y`, `m/d/Y P`, `P`, `F Y`, `F j`).

| Template | Input | Output |
|---|---|---|
| `{{ v\|date:'Y-m-d' }}` | `2024-03-05` | `2024-03-05` |
| `{{ v\|date:'D, d M Y' }}` | `2024-03-05` | `Tue, 05 Mar 2024` |
| `{{ v\|date:'jS F Y' }}` | `2024-03-01` | `1st March 2024` |
| `{{ v\|date:'jS F Y' }}` | `2024-03-22` | `22nd March 2024` |
| `{{ v\|date:'jS F Y' }}` | `2024-03-13` | `13th March 2024` |
| `{{ v\|date:'N j, Y' }}` | `2024-05-05` | `May 5, 2024` |
| `{{ v\|date:'N j, Y' }}` | `2024-03-05` | `March 5, 2024` |
| `{{ v\|date:'N j, Y' }}` | `2024-09-05` | `Sept. 5, 2024` |
| `{{ v\|date:'l w z W t L y' }}` | `2024-03-05` | `Tuesday 2 65 10 31 True 24` |
| `{{ v\|date:'H:i:s' }}` | `2024-03-05T14:30:07` | `14:30:07` |
| `{{ v\|date:'g:i A a P' }}` | `2024-03-05T00:00:00` | `12:00 AM a.m. midnight` |
| `{{ v\|date:'g:i A a P' }}` | `2024-03-05T12:00:00` | `12:00 PM p.m. noon` |
| `{{ v\|date:'g:i A a P' }}` | `2024-03-05T15:45:00` | `3:45 PM p.m. 3:45 p.m.` |
| `{{ v\|date:'c' }}` | `2024-03-05T14:30:07` | `2024-03-05T14:30:07` |
| `{{ v\|date }}` | `2024-03-05` | `March 5, 2024` |
| `{{ v\|date }}` | `2024-03-05T14:30:07` | `March 5, 2024` |
| `{{ v\|date:'Y\\Y\\ m' }}` | `2024-03-05` | `2024Y 03` |
| `{{ v\|date:'Y' }}` | `""`, `None`, `"str"` | (empty) |

A backslash escapes the next character so it is output literally.

A bare date or date-time also prints without a filter using the default `N j, Y` (date), `N j, Y, P` (date-time) or `P` (time) pattern: a date renders `Jan. 5, 2024`, a date-time `Jan. 5, 2024, 2:30 p.m.` and a time `2:05 p.m.`.

### time

Argument: a format string or format name. Default: `TIME_FORMAT`. Applies to times and date-times.

| Template | Input | Output |
|---|---|---|
| `{{ v\|time:'H:i' }}` | `14:05:09` | `14:05` |
| `{{ v\|time:'H:i' }}` | `2024-03-05T01:02:03` | `01:02` |
| `{{ v\|time }}` | `14:05:09` | `2:05 p.m.` |

### timesince

Argument: optional date-time to measure to (default: now). Renders the elapsed time from the value to that moment as at most two adjacent units out of years, months, weeks, days, hours and minutes, joined with a comma and using non-breaking spaces, for example `1 day, 3 hours`. A value in the future renders `0 minutes`. A plain date counts as midnight. Other types render an empty string.

### timeuntil

Argument: optional date-time to measure from (default: now). The reverse of `timesince`: the time remaining until the value, in the same format. Only date-time values are accepted.

## Humanize filters

These filters are in the `humanize` library. They are found by name in any template once the library is linked into the program; the template does not have to `{% load humanize %}`.

### intcomma

Inserts a comma every three digits of the integer part. Keeps the sign and any decimal part. `1234567` renders `1,234,567`; `-1234.5` renders `-1,234.5`.

### ordinal

Converts an integer to its ordinal: `1` becomes `1st`, `2` becomes `2nd`, `3` becomes `3rd`, `11` to `13` become `11th` to `13th`, `21` becomes `21st`. A value that is not an integer is returned unchanged.

### apnumber

Spells out the numbers 1 to 9 (`one` to `nine`); other numbers are returned as digits. A value that is not an integer is returned unchanged.

### naturaltime

Describes a date-time relative to now with one unit: `3 hours ago` or `2 days from now`. Other types are returned unchanged.

## Date and time format characters

Used by `date`, `time` and `{% now %}`. Characters that are not listed are copied to the output unchanged, and a backslash makes the next character literal. A date value has no time of day, and a time value has no date, so using a character that needs the missing part makes the filter render an empty string (and `{% now %}` fail). Characters that need a time zone (`O`, `Z`) fail on values without a UTC offset.

| Char | Meaning | Example |
|---|---|---|
| `a` | `a.m.` or `p.m.` | `p.m.` |
| `A` | `AM` or `PM` | `PM` |
| `b` | Month, three lowercase letters | `mar` |
| `c` | ISO 8601 date and time, with microseconds and offset when present | `2024-03-05T14:30:07` |
| `d` | Day of month, two digits | `05` |
| `D` | Weekday, three letters | `Tue` |
| `e` | Time zone name: `UTC` for a zero offset, otherwise empty | `UTC` |
| `E` | Month name | `March` |
| `f` | Time in 12-hour form, minutes left out on the hour | `2`, `2:05` |
| `F` | Month, full name | `March` |
| `g` | Hour, 12-hour, no leading zero | `2` |
| `G` | Hour, 24-hour, no leading zero | `14` |
| `h` | Hour, 12-hour, two digits | `02` |
| `H` | Hour, 24-hour, two digits | `14` |
| `i` | Minutes, two digits | `05` |
| `I` | Daylight saving flag. Always `0` | `0` |
| `j` | Day of month, no leading zero | `5` |
| `l` | Weekday, full name | `Tuesday` |
| `L` | Leap year: `True` or `False` | `True` |
| `m` | Month, two digits | `03` |
| `M` | Month, three letters | `Mar` |
| `n` | Month, no leading zero | `3` |
| `N` | Month in newspaper style: `Jan.`, `Feb.`, `March`, `April`, `May`, `June`, `July`, `Aug.`, `Sept.`, `Oct.`, `Nov.`, `Dec.` | `Sept.` |
| `o` | ISO 8601 week-numbering year | `2024` |
| `O` | Offset from UTC as `+HHMM` | `+0100` |
| `P` | Time as `2 p.m.`, `2:05 p.m.`, `midnight` or `noon` | `2:05 p.m.` |
| `r` | RFC 2822 date | `Tue, 5 Mar 2024 14:30:07 +0000` |
| `s` | Seconds, two digits | `07` |
| `S` | English ordinal suffix for the day: `st`, `nd`, `rd`, `th` | `th` |
| `t` | Number of days in the month | `31` |
| `T` | Time zone abbreviation: `UTC` for a zero offset, otherwise empty | `UTC` |
| `u` | Microseconds, six digits | `000000` |
| `U` | Seconds since 1970-01-01 UTC | `1709649007` |
| `w` | Day of week, Sunday is `0` | `2` |
| `W` | ISO 8601 week number | `10` |
| `y` | Year, two digits | `24` |
| `Y` | Year, four digits | `2024` |
| `z` | Day of the year | `65` |
| `Z` | UTC offset in seconds | `3600` |

Characters `B`, `k`, `K`, `q`, `v`, `V`, `x`, `X` have no meaning and are output as written.

## Not supported

- `truncatechars_html` and `truncatewords_html` do not preserve or close HTML tags.
- `floatformat` ignores the `g` and `u` suffixes.
- `urlize` does not link email addresses.
- Filters take at most one argument.

## See also

- [Template tag reference](template-tags.md)
- [The template language](../topics/templates/language.md)
- [The template engine](../topics/templates/engine.md)
- [Custom tags and filters](../topics/templates/custom-tags.md)
