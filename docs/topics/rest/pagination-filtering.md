# Pagination and filtering

This page covers how a `ModelViewSet` list endpoint splits results into pages, and how clients narrow and order them
with query parameters: exact-value filters, text search and ordering. Pagination, search, ordering and filters apply to
the `list` action only.

```rust
use rango::rest::{ModelSerializer, ModelViewSet, Pagination};
```

## Order of operations

For every `list` request the viewset:

1. starts from the viewset's queryset (see [Restricting the queryset](viewsets-routers.md#restricting-the-queryset));
2. applies the exact-value filters (`filterset_fields`);
3. applies the search terms (`search_fields`);
4. applies the ordering (`?ordering=`, or the default `ordering`);
5. counts the rows and slices out the requested page, if pagination is on;
6. serializes each row.

Filters and search narrow the set before it is counted, so `count` in a paginated response is the size of the filtered
set, not of the table.

## Turning pagination on

A viewset is paginated when it has a `Pagination` or when the project configures a default. Without either, `list`
returns every row as a plain JSON array.

```rust
let articles = ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .pagination(Some(Pagination::page_number(20)));
```

| Call | Effect |
| --- | --- |
| `.pagination(Some(p))` | Paginate this viewset with `p`. |
| `.pagination(None)` | Never paginate this viewset, even if the project has a default. |
| not called | Use the project default (see [Project default](#project-default)); no default means no pagination. |

### Page number pagination

`Pagination::page_number(page_size)` pages by number: `?page=2`.

```rust
Pagination::page_number(2)
```

`page_size` must be greater than zero. Request behaviour:

| Query | Result |
| --- | --- |
| no `page` | page 1 |
| `?page=3` | page 3 |
| `?page=last` | the last page |
| `?page=0`, `?page=abc`, or a number past the last page | `404` with `{"detail":"Invalid page."}` |

An empty result set still has one page, so `?page=1` on an empty table answers `200` with `"results": []`.

The page size is fixed unless you enable a client-controlled size. The enum variant has public fields, so build it
directly:

```rust
Pagination::PageNumber {
    page_size: 20,
    page_query_param: "page".into(),
    page_size_query_param: Some("page_size".into()),
    max_page_size: Some(100),
}
```

With `page_size_query_param` set, `?page_size=50` overrides `page_size`. A value that is not a positive integer is
ignored; a value above `max_page_size` is lowered to it. Without `max_page_size` there is no upper bound. `page_query_param`
renames the page parameter.

### Limit and offset pagination

`Pagination::limit_offset(default_limit)` pages by position: `?limit=10&offset=20`.

```rust
Pagination::limit_offset(25)
```

| Query | Result |
| --- | --- |
| no `limit` | `default_limit` rows |
| `?limit=10` | 10 rows; zero or a non-integer falls back to `default_limit` |
| `?offset=20` | skip 20 rows; a missing or non-integer value means 0 |

An offset beyond the end is not an error: the response has an empty `results` array. To cap `limit`, build the variant
directly:

```rust
Pagination::LimitOffset { default_limit: 25, max_limit: Some(100) }
```

The parameter names `limit` and `offset` are fixed.

## The response envelope

A paginated list returns an object instead of an array:

```console
$ curl http://testserver/api/paged/
{"count":3,"next":"http://testserver/api/paged/?page=2","previous":null,"results":[{"id":1, ...},{"id":2, ...}]}

$ curl "http://testserver/api/paged/?page=2"
{"count":3,"next":null,"previous":"http://testserver/api/paged/","results":[{"id":3, ...}]}
```

| Key | Meaning |
| --- | --- |
| `count` | Number of rows in the filtered set, over all pages. |
| `next` | Absolute URL of the next page, or `null` on the last page. |
| `previous` | Absolute URL of the previous page, or `null` on the first. |
| `results` | The serialized rows of this page. |

The `next` and `previous` URLs are built from the request URL, so the scheme and host follow the request. Every other
query parameter (`search`, `ordering`, filters) is kept, with parameters sorted by name. Details:

- Page number: the link to page 1 omits the `page` parameter.
- Limit and offset: `next` and `previous` always carry `limit`; `previous` omits `offset` when it would be 0.
- Keys and values are percent-encoded.

## Project default

Set a default in the `rest_framework` setting. It takes effect only when both keys are present and valid:

```rust
use rango::conf::Settings;

Settings {
    rest_framework: rango::json!({
        "DEFAULT_PAGINATION_CLASS": "PageNumberPagination",
        "PAGE_SIZE": 20,
    }),
    ..Settings::default()
}
```

| Key | Value |
| --- | --- |
| `DEFAULT_PAGINATION_CLASS` | `"PageNumberPagination"` or `"LimitOffsetPagination"`. Only the part after the last dot is read. Any other name means no default. |
| `PAGE_SIZE` | A positive integer: the page size, or the default limit for limit and offset. Without it, no default pagination. |
| `SEARCH_PARAM` | Name of the search parameter. Default `"search"`. |
| `ORDERING_PARAM` | Name of the ordering parameter. Default `"ordering"`. |

The default pagination uses the standard parameter names and has no client-controlled page size or maximum. For those,
set `.pagination(Some(...))` on the viewset. The setting is read on every request. The other keys of `rest_framework`
are described in [Authentication, permissions and throttling](auth-permissions.md#global-defaults).

## Exact-value filters

`.filterset_fields(&[...])` lets clients filter on model fields by equality: `?published=true&views=70`.

```rust
ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .filterset_fields(&["published", "views", "author"])
```

Query strings are text, so each value is converted to the type of the column before it is compared:

| Column type | Conversion |
| --- | --- |
| Boolean | `true`, `1`, `yes`, `on` (any case) mean true; any other value means false. |
| Integer kinds (including small, big and positive variants), auto fields and foreign keys | Parsed as an integer. A value that does not parse is compared as text, which matches no rows. |
| Anything else | Compared as text. |

Rules:

- A parameter that is absent or empty is ignored, so `?published=` does not filter.
- Several filters combine with AND.
- Only the listed fields are read; other query parameters are ignored by the filter step.
- Names must be model fields. A foreign key is filtered by the related row's primary key (`?author=3`).
- Only equality is supported; there are no range or `__in` lookups.

## Search

`.search_fields(&[...])` enables `?search=`. A term matches a row when any listed field contains it, ignoring case.

```rust
ModelViewSet::new(ModelSerializer::<Article>::all()?).search_fields(&["title", "body"])
```

```console
$ curl "http://testserver/api/articles/?search=words"
[{"id":3,"title":"Third post","slug":"third","body":"more words","published":false,"views":0,"rating":null}]
```

- The value is split on spaces and commas into terms. Every term must match at least one field (AND across terms, OR
  across fields): `?search=hello world` finds rows that contain both words, possibly in different fields.
- Matching is a case-insensitive "contains". There are no prefix, exact or full-text operators.
- An empty or missing parameter does not filter.
- Without `search_fields` the parameter is ignored.
- Search fields must be fields of the model.

## Ordering

`.ordering_fields(&[...])` enables `?ordering=` and lists the fields clients may sort by. A leading `-` sorts
descending, and several fields are separated by commas.

```rust
ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .ordering_fields(&["views", "title"])
    .ordering(&["-views"])
```

```console
$ curl "http://testserver/api/articles/?ordering=-views"
$ curl "http://testserver/api/articles/?ordering=title,-views"
```

- Terms naming a field outside `ordering_fields` are dropped silently; the other terms still apply.
- If no term survives, or the parameter is absent, the default from `.ordering(&[...])` is used.
- Without either, the model's own default ordering applies. Set one: a paginated list without a stable order can repeat or skip rows between pages.
- `.ordering(&[...])` is applied even when `ordering_fields` was never called, but then clients cannot change it.

## Combining everything

```rust
router.register(
    "articles",
    ModelViewSet::new(ModelSerializer::<Article>::all()?)
        .pagination(Some(Pagination::page_number(20)))
        .filterset_fields(&["published"])
        .search_fields(&["title", "body"])
        .ordering_fields(&["views", "title"])
        .ordering(&["-views"]),
);
```

```console
$ curl "http://testserver/api/articles/?published=true&search=rust&ordering=title&page=2"
```

All of these parameters are listed in the generated [OpenAPI document](openapi.md#what-the-document-contains).

## Not supported yet

- Range, `contains` and other lookups in `filterset_fields`; filtering on related fields through `__`.
- Cursor pagination.
- Search prefixes (`^`, `=`, `@`, `$`) and search across relations.
- Custom filter backends; write an [API view](api-views.md) or a [custom viewset](viewsets-routers.md#writing-your-own-viewset).
- Pagination of extra actions: they receive the request and return what they like.

## See also

- [Viewsets and routers](viewsets-routers.md)
- [Serializers](serializers.md)
- [Authentication, permissions and throttling](auth-permissions.md)
- [OpenAPI schema](openapi.md)
- [REST quickstart](quickstart.md)
- [Pagination of plain querysets](../pagination.md)
- [Making queries](../db/queries.md)
