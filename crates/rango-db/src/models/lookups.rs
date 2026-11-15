//! Lookup and transform names (`db.models.lookups`); SQL generation is in `sql.rs`.

const LOOKUPS: &[&str] = &[
    "exact", "iexact", "contains", "icontains", "in", "gt", "gte", "lt", "lte", "startswith", "istartswith", "endswith", "iendswith", "range", "isnull",
    "regex", "iregex",
];

const TRANSFORMS: &[&str] = &[
    "lower", "upper", "length", "year", "iso_year", "month", "day", "week_day", "iso_week_day", "week", "quarter", "hour", "minute", "second", "date", "time",
];

pub fn is_lookup(name: &str) -> bool {
    LOOKUPS.contains(&name)
}

pub fn is_transform(name: &str) -> bool {
    TRANSFORMS.contains(&name)
}

/// Escape `\`, `%` and `_` for LIKE patterns (`prep_for_like_query`).
pub fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}
