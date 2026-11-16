# Pagination

`rango::core::paginator` splits a long list into pages. It works on anything that can be counted and sliced: a
`Vec`, a database queryset (through a small adapter) or a source you write yourself. This page covers `Paginator`,
`Page`, the error values, orphans, elided page ranges, and how to use them in a view and a template. For lists of
model rows `ListView` does all of this for you ([Generic views](http/generic-views.md#pagination)); for API
responses see [REST pagination](rest/pagination-filtering.md).

```rust
use rango::core::paginator::{ErrorMessages, InvalidPage, Page, PageItem, PageSource, Paginator, ellipsis};
```

## A first paginator

```rust
use rango::core::paginator::Paginator;

let items: Vec<String> = (1..=23).map(|n| format!("item {n}")).collect();
let paginator = Paginator::new(items, 10);

assert_eq!(paginator.count().await?, 23);
assert_eq!(paginator.num_pages().await?, 3);

let page = paginator.page(2).await?.expect("page exists");
assert_eq!(page.object_list.len(), 10);
assert_eq!((page.start_index(), page.end_index()), (11, 20));
```

Almost every method is `async`, because counting and slicing a database source needs a query. Note that `page(2).await?` leaves a second `Result`: the outer `Result` reports a failure of the source (for example a database error); the inner one
reports an invalid page number (`InvalidPage`). `InvalidPage` is not converted into `rango::Error` for you, so decide what an invalid page means (the examples below use
`expect` for brevity). That split lets you treat a bad page number as a 404 and a database
failure as a 500.

`Paginator::new(object_list, per_page)` requires `per_page` to be greater than zero; zero panics when the pages are
counted.

## Page sources

`Paginator<S>` takes any `S: PageSource`:

```rust
pub trait PageSource: Send + Sync {
    type Item: Send;
    fn count(&self) -> impl Future<Output = Result<usize>> + Send;
    fn slice(&self, range: Range<usize>) -> impl Future<Output = Result<Vec<Self::Item>>> + Send;
}
```

`count` returns the total number of items and `slice` returns the items in `range` (zero-based, end exclusive). Two
implementations are provided:

- `Vec<T>` for `T: Clone + Send + Sync`: counts `len()` and copies the slice.
- `rango::views::generic::list::QuerySource<M>`, a queryset together with a connection. It counts with a `COUNT`
  query and fetches one page with `LIMIT`/`OFFSET`.

```rust
use rango::core::paginator::Paginator;
use rango::db::Db;
use rango::views::generic::list::QuerySource;

let source = QuerySource {
    qs: Article::objects().filter_kw("status", "published").order_by(&["-published", "id"]),
    db: Db::default_db(),
};
let paginator = Paginator::new(source, 25);
let page = paginator.page(1).await?.expect("page exists");           // Page<Article>
```

Always order the queryset, preferably by a unique column last. Without an order, the database may return rows in a
different order for each page and items repeat or go missing across pages.

A queryset is not a `PageSource` by itself, because the source must own the connection to use.

To page over something else (a search index, a remote API), implement the trait:

```rust
use std::ops::Range;
use rango::core::paginator::PageSource;
use rango::Result;

struct Entries(Vec<String>);

impl PageSource for Entries {
    type Item = String;

    async fn count(&self) -> Result<usize> {
        Ok(self.0.len())
    }
    async fn slice(&self, range: Range<usize>) -> Result<Vec<String>> {
        Ok(self.0[range].to_vec())
    }
}
```

## Configuring a paginator

`Paginator` has builder methods that return the paginator, and the three settings are also public fields.

| Item | Default | Meaning |
|---|---|---|
| `Paginator::new(source, per_page)` | | Items per page; must be above zero. |
| `.orphans(n)` | `0` | A last page that would hold `n` or fewer items is merged into the previous page. |
| `.allow_empty_first_page(bool)` | `true` | Whether an empty source still has one (empty) page. |
| `.error_messages(ErrorMessages)` | built-in texts | Replace the messages of `InvalidPage` errors. |

### Orphans

With 23 items, `per_page = 10` and `orphans = 3`, the last three items would form a page on their own; they are
folded into page 2 instead, which then has 13 items:

```rust
let paginator = Paginator::new(items, 10).orphans(3);
assert_eq!(paginator.num_pages().await?, 2);
assert_eq!(paginator.page(2).await?.expect("page exists").object_list.len(), 13);
```

### Empty sources

By default an empty source has one empty page, so `/list/?page=1` renders an empty list instead of failing. With
`allow_empty_first_page(false)` it has zero pages, and every page number is invalid. `get_page` must not be used in
that configuration with an empty source; use `page` and handle the error.

### Messages

```rust
use rango::core::paginator::ErrorMessages;

let paginator = Paginator::new(items, 10).error_messages(ErrorMessages {
    invalid_page: Some("Page numbers are whole numbers.".into()),
    min_page: Some("Pages start at 1.".into()),
    no_results: Some("There is nothing on that page.".into()),
});
```

Messages left as `None` use the translated defaults: "That page number is not an integer", "That page number is less
than 1" and "That page contains no results".

## Asking for a page

| Method | Returns | Use |
|---|---|---|
| `count().await` | `Result<usize>` | Total items. Counted once and cached. |
| `num_pages().await` | `Result<usize>` | Total pages: `ceil(max(count - orphans, 1) / per_page)`, or `0` for an empty source with `allow_empty_first_page(false)`. |
| `page_range().await` | `Result<Range<usize>>` | `1..num_pages + 1`, for looping over page numbers. |
| `validate_number(i64).await` | `Result<Result<usize, InvalidPage>>` | Check a number. Below 1 or above the last page is `EmptyPage`. |
| `validate_number_str(&str).await` | `Result<Result<usize, InvalidPage>>` | Same for text such as a query parameter; a non-integer is `PageNotAnInteger`. Surrounding spaces are ignored. |
| `page(i64).await` | `Result<Result<Page<T>, InvalidPage>>` | The strict lookup. |
| `get_page(Option<&str>).await` | `Result<Page<T>>` | The forgiving lookup. |
| `get_elided_page_range(number, on_each_side, on_ends).await` | `Result<Result<Vec<PageItem>, InvalidPage>>` | A short list of page links. |

### Strict or forgiving

`page` fails on a bad number, which is right when a bad URL should be a 404:

```rust
use rango::core::paginator::InvalidPage;

match paginator.page(wanted).await? {
    Ok(page) => render_page(page),
    Err(InvalidPage::PageNotAnInteger(msg)) | Err(InvalidPage::EmptyPage(msg)) => {
        return Err(rango::Error::Http404(msg));
    }
}
```

`get_page` never fails on user input: `None`, text that is not an integer, or any other parse problem gives page 1, and
a number outside the range gives the last page. It suits "show something sensible whatever the URL says":

```rust
let page = paginator.get_page(request.query().get("page")).await?;
```

(`request.query().get("page")` is `Option<&str>`.) Bad input silently becoming page 1 means search engines can index
duplicates of page 1; prefer the strict form for public lists.

## The `Page`

`Page<T>` is a plain value. It copies the totals out of the paginator, so it can outlive it.

| Item | Type | Meaning |
|---|---|---|
| `object_list` | `Vec<T>` | The items on this page. |
| `number` | `usize` | Page number, from 1. |
| `count` | `usize` | Total items on all pages. |
| `num_pages` | `usize` | Total pages. |
| `per_page` | `usize` | Page size. |
| `len()`, `is_empty()` | | Items on this page. |
| `has_next()`, `has_previous()`, `has_other_pages()` | `bool` | Neighbouring pages. |
| `next_page_number()`, `previous_page_number()` | `Result<usize, InvalidPage>` | The neighbour's number; `EmptyPage` when there is none. |
| `start_index()`, `end_index()` | `usize` | 1-based positions of the first and last item on the page among all items (`0` for the start of an empty source). |

A `Page` is an `IntoIterator<Item = T>`, so `for item in page { ... }` consumes it.

## Elided page ranges

`get_elided_page_range(number, on_each_side, on_ends)` returns a list of `PageItem` for building a pager that does not
show hundreds of links. Around the current page it keeps `on_each_side` pages on each side, at both ends it keeps
`on_ends` pages, and everything else becomes `PageItem::Ellipsis`:

```rust
use rango::core::paginator::{PageItem, ellipsis};

// 50 pages, current page 25, 3 on each side, 2 at the ends
let items = paginator.get_elided_page_range(25, 3, 2).await?.expect("page exists");
// 1 2 ... 22 23 24 25 26 27 28 ... 49 50
let labels: Vec<String> = items
    .iter()
    .map(|item| match item {
        PageItem::Page(n) => n.to_string(),
        PageItem::Ellipsis => ellipsis(),
    })
    .collect();
```

If there are few pages (at most `(on_each_side + on_ends) * 2`), all pages are returned without ellipses.
`ellipsis()` returns the translated "…" text. A page number outside the range is an `InvalidPage` error.

## In a view and template

The template engine works with template values, so convert the page into variables. This view pages over a `Vec` of
strings; for model rows map each row with `rango::views::generic::model_value`.

```rust
use rango::core::paginator::Paginator;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::render;
use rango::{Result, context};

pub async fn index(request: HttpRequest) -> Result<HttpResponse> {
    let all: Vec<String> = (1..=95).map(|n| format!("entry {n}")).collect();
    let paginator = Paginator::new(all, 20);

    let number = request.query().get("page").unwrap_or("1");
    let page = match paginator.validate_number_str(number).await? {
        Ok(n) => paginator.page(n as i64).await?.expect("validated"),
        Err(e) => return Err(rango::Error::Http404(e.to_string())),
    };

    render(
        &request,
        "entries/index.html",
        context! {
            "entries" => page.object_list.clone(),
            "number" => page.number,
            "num_pages" => page.num_pages,
            "has_previous" => page.has_previous(),
            "has_next" => page.has_next(),
            "previous" => page.previous_page_number().unwrap_or(1),
            "next" => page.next_page_number().unwrap_or(page.num_pages),
            "first_on_page" => page.start_index(),
            "last_on_page" => page.end_index(),
            "total" => page.count,
        },
    )
}
```

```html
<ul>{% for e in entries %}<li>{{ e }}</li>{% endfor %}</ul>
<p>Showing {{ first_on_page }} to {{ last_on_page }} of {{ total }}</p>
<nav>
  {% if has_previous %}<a href="?page=1">first</a> <a href="?page={{ previous }}">previous</a>{% endif %}
  Page {{ number }} of {{ num_pages }}
  {% if has_next %}<a href="?page={{ next }}">next</a> <a href="?page={{ num_pages }}">last</a>{% endif %}
</nav>
```

`InvalidPage` implements `Display` with the message, so `e.to_string()` gives the text above.

When a query string carries other filters, keep them in the links; build the link with the `urlencode` filter or in
the view, because `?page=2` alone drops them.

## Errors

| Value | Meaning |
|---|---|
| `InvalidPage::PageNotAnInteger(msg)` | The page was not an integer. |
| `InvalidPage::EmptyPage(msg)` | The page is below 1, past the last page, or (for neighbour numbers) does not exist. |

Both carry the message text and display as that text. The outer `rango::Result` of the async methods carries the
source's own errors.

## Not supported yet

- Paginating a `QuerySet` directly without `QuerySource`.
- Default values for `on_each_side` and `on_ends`; you pass both.
- A cursor- or offset-limit pager in the core paginator. The REST layer has its own pagination classes
  ([REST pagination](rest/pagination-filtering.md)).

## See also

- [Generic views](http/generic-views.md)
- [Making queries](db/queries.md)
- [REST pagination and filtering](rest/pagination-filtering.md)
- [Template language](templates/language.md)
- [Requests and responses](http/requests-responses.md)
