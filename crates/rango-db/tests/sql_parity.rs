//! SQL text parity with the reference on SQLite (cases recorded by `tools/gen_sql_golden.py`).

mod common;

use chrono::{NaiveDate, TimeZone, Utc};
use common::*;
use rango_core::conf::{self, DatabaseConfig};
use rango_db::backends::Vendor;
use rango_db::models::expressions::{Count, Exists, F, OuterRef};
use rango_db::models::{Model, Q, QuerySet};
use rango_db::Value;
use serde_json::{Value as J, json};

fn params_json(p: &[Value]) -> J {
    J::Array(
        p.iter()
            .map(|v| match v {
                Value::Int(i) => json!(i),
                Value::Float(f) => json!(f),
                Value::Str(s) => json!(s),
                Value::Null => J::Null,
                Value::Bool(b) => json!(i64::from(*b)),
                other => json!(rango_db::models::fields_db::adapt_to_text(other)),
            })
            .collect(),
    )
}

#[test]
fn sql_matches_rango() {
    register_models();
    conf::configure(settings_for(DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })).unwrap();
    let golden: Vec<J> = serde_json::from_str(include_str!("golden/sql.json")).unwrap();

    let dt = Utc.with_ymd_and_hms(2024, 1, 1, 12, 30, 0).unwrap();
    let q = Question::objects;
    let c = Choice::objects;
    let a = Author::objects;
    let b = Book::objects;
    let mut got: Vec<(&str, Result<(String, J), String>)> = Vec::new();
    macro_rules! case {
        ($name:literal, $qs:expr) => {
            got.push(($name, $qs.sql(Vendor::Sqlite).map_err(|e| e.to_string()).and_then(|c| c.ok_or("empty".to_owned())).map(|c| (c.sql, params_json(&c.params)))));
        };
    }
    case!("all", q());
    case!("filter_exact", q().filter_kw("id", 1));
    case!("filter_two", q().filter_kw("id", 2).filter_kw("question_text", "a").tap_flatten());
    case!("filter_chain", q().filter_kw("question_text", "a").filter_kw("id", 2));
    case!("startswith", q().filter_kw("question_text__startswith", "What"));
    case!("icontains", q().filter_kw("question_text__icontains", "x%y_"));
    case!("contains", q().filter_kw("question_text__contains", "x"));
    case!("endswith", q().filter_kw("question_text__endswith", "x"));
    case!("iexact", q().filter_kw("question_text__iexact", "x"));
    case!("in_list", q().filter_q(Q::is_in("id", [1, 2, 3])));
    case!("range", q().filter_q(Q::lookup_rhs("id__range", rango_db::models::q::Rhs::Range(1.into_v(), 5.into_v()))));
    case!("isnull", a().filter_kw("age__isnull", true));
    case!("isnull_false", a().filter_kw("age__isnull", false));
    case!("exclude", q().exclude_kw("question_text", "a"));
    case!("exclude_nullable", a().exclude_kw("age", 3));
    case!("gt_lte", c().filter_kw("votes__gt", 1).filter_kw("votes__lte", 9));
    case!("q_or", q().filter_q(Q::lookup("question_text", "a") | Q::lookup("id", 1)));
    case!("q_and_or", q().filter_q(Q::lookup("question_text", "a") & (Q::lookup("id", 1) | Q::lookup("id", 2))));
    case!("q_not", q().filter_q(!Q::lookup("id", 1)));
    case!("order_desc", q().order_by(&["-pub_date"]));
    case!("order_two", q().order_by(&["question_text", "-id"]));
    case!("order_pk", q().order_by(&["pk"]));
    case!("slice", q().order_by(&["id"]).slice(2, Some(5)));
    case!("slice_limit", q().slice(0, Some(3)));
    case!("slice_offset", q().slice(3, None));
    case!("distinct", q().distinct());
    case!("values", q().values(&["id", "question_text"]));
    case!("only", q().only(&["question_text"]));
    case!("defer", q().defer(&["pub_date"]));
    case!("fk_filter", c().filter_kw("question__question_text", "x"));
    case!("fk_filter_id", c().filter_kw("question", 3));
    case!("fk_filter_id_attr", c().filter_kw("question_id", 3));
    case!("fk_filter_pk", c().filter_kw("question__id", 3));
    case!("reverse_filter", q().filter_kw("choice__votes__gt", 2));
    case!("reverse_two_filters", q().filter_kw("choice__votes__gt", 2).filter_kw("choice__choice_text", "a"));
    case!("reverse_same_filter", q().filter_q(Q::lookup("choice__choice_text", "a") & Q::lookup("choice__votes__gt", 2)));
    case!("m2m_filter", b().filter_kw("authors__name", "x"));
    case!("select_related", c().select_related(&["question"]));
    case!("annotate_count", q().annotate("n", Count("choice")));
    case!("annotate_filter_having", q().annotate("n", Count("choice")).filter_kw("n__gt", 1));
    case!("f_compare", c().filter_expr("votes__gt", F("question_id")));
    case!("f_arith", c().filter_expr("votes__gt", F("question_id") * 2));
    case!("year_exact", q().filter_kw("pub_date__year", 2024));
    case!("year_gte", q().filter_kw("pub_date__year__gte", 2024));
    case!("datetime_gt", q().filter_kw("pub_date__gt", dt));
    case!("date_exact", b().filter_kw("published", NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()));
    case!("decimal", b().filter_kw("price__gt", rust_decimal::Decimal::new(999, 2)));
    case!("exists_subquery", q().filter_q(Q::expr(Exists(c().filter_expr("question", OuterRef("pk")).filter_kw("votes__gt", 1).into_query()))));
    case!("in_subquery", q().filter_q(Q::lookup_rhs("id__in", rango_db::models::q::Rhs::Query(Box::new(c().filter_kw("votes__gt", 1).values(&["question_id"]).into_query())))));
    case!("union", a().filter_kw("name", "a").union(a().filter_kw("name", "b"), false));
    case!("regex", q().filter_kw("question_text__regex", "^W"));
    case!("none_exact", a().filter_q(Q::lookup_rhs("age", rango_db::models::q::Rhs::Val(Value::Null))));
    case!("pk_in", q().filter_q(Q::is_in("pk", [1, 2])));
    case!("order_related", c().order_by(&["question__question_text"]));

    let mut failures = Vec::new();
    for (name, result) in &got {
        let want = golden.iter().find(|g| g["name"] == *name).unwrap_or_else(|| panic!("no golden for {name}"));
        let expected = &want["expected"];
        match result {
            Ok((sql, params)) => {
                if expected.get("sql").and_then(J::as_str) != Some(sql.as_str()) || expected["params"] != *params {
                    failures.push(format!("{name}\n  expected {} {}\n  got      {sql} {params}", expected["sql"], expected["params"]));
                }
            }
            Err(e) => failures.push(format!("{name}: error {e}; expected {expected}")),
        }
    }
    assert!(failures.is_empty(), "{} of {} SQL cases differ from the reference implementation:\n{}", failures.len(), got.len(), failures.join("\n"));
}

trait IntoV {
    fn into_v(self) -> Value;
}
impl IntoV for i64 {
    fn into_v(self) -> Value {
        Value::Int(self)
    }
}
impl IntoV for i32 {
    fn into_v(self) -> Value {
        Value::Int(i64::from(self))
    }
}

trait TapFlatten {
    fn tap_flatten(self) -> Self;
}
impl<M: Model> TapFlatten for QuerySet<M> {
    fn tap_flatten(self) -> Self {
        self
    }
}
