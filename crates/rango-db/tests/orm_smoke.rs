//! ORM behaviour on SQLite and (when RANGO_PG_URL-like env is reachable) PostgreSQL, same assertions.

mod common;

use chrono::{TimeZone, Utc};
use common::*;
use rango_core::conf::{self, DatabaseConfig};
use rango_db::models::expressions::{Avg, Count, F, Max, Sum};
use rango_db::models::{ForeignKey, Model, Q, QuerySet, related};
use rango_db::filter;
use rango_db::{Db, Executor, Value};
use rust_decimal::Decimal;

fn dt(y: i32, m: u32, d: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
}

async fn exercise(db: &dyn Executor) {
    // --- create / save / get
    let mut q1 = Question { id: None, question_text: "What's new?".into(), pub_date: dt(2024, 1, 5) };
    q1.save(db).await.unwrap();
    assert_eq!(q1.id, Some(1));
    let q2 = Question::objects().create(db, Question { id: None, question_text: "Why?".into(), pub_date: dt(2025, 6, 1) }).await.unwrap();
    let got = Question::objects().filter_kw("id", 1).get(db).await.unwrap();
    assert_eq!(got, q1);
    assert!(matches!(Question::objects().filter_kw("id", 99).get(db).await, Err(rango_core::Error::ObjectDoesNotExist { .. })));

    // --- update through save
    let mut q = got;
    q.question_text = "What's up?".into();
    q.save(db).await.unwrap();
    assert_eq!(Question::objects().get(db).await.err().map(|e| matches!(e, rango_core::Error::MultipleObjectsReturned { .. })), Some(true));
    assert_eq!(Question::objects().filter_kw("question_text__startswith", "What").count(db).await.unwrap(), 1);

    // --- lookups
    let qs = Question::objects();
    assert_eq!(qs.clone().filter_kw("question_text__icontains", "WHY").count(db).await.unwrap(), 1);
    assert_eq!(qs.clone().filter_kw("question_text__contains", "100%").count(db).await.unwrap(), 0);
    assert_eq!(qs.clone().filter_q(Q::is_in("id", [1, 2, 3])).count(db).await.unwrap(), 2);
    assert_eq!(qs.clone().filter_q(Q::is_in("id", Vec::<i64>::new())).count(db).await.unwrap(), 0);
    assert_eq!(qs.clone().exclude_kw("question_text", "Why?").count(db).await.unwrap(), 1);
    assert_eq!(qs.clone().filter_kw("pub_date__year", 2025).count(db).await.unwrap(), 1);
    assert_eq!(qs.clone().filter_kw("pub_date__year__gte", 2024).count(db).await.unwrap(), 2);
    assert_eq!(qs.clone().filter_kw("pub_date__gt", dt(2024, 12, 31)).count(db).await.unwrap(), 1);
    assert_eq!(qs.clone().filter_q(Q::lookup("question_text", "Why?") | Q::lookup("id", 1)).count(db).await.unwrap(), 2);
    assert_eq!(qs.clone().filter_q(!Q::lookup("id", 1)).count(db).await.unwrap(), 1);
    assert_eq!(qs.clone().filter_kw("id__isnull", true).count(db).await.unwrap(), 0);
    assert!(qs.clone().filter_kw("nope", 1).count(db).await.is_err());

    // --- typed filters: compile-time checked field names, keyword-style kwargs
    assert_eq!(Question::objects().filter(filter!(question_text__startswith = "What")).count(db).await.unwrap(), 1);
    assert_eq!(Question::objects().filter(filter!((id = 1) | (question_text = "Why?"))).count(db).await.unwrap(), 2);
    assert_eq!(Question::objects().exclude(filter!(id__in = [1])).count(db).await.unwrap(), 1);
    assert_eq!(Question::objects().filter(|f| f.pub_date().year().gte(2025) & !f.id().eq(1)).count(db).await.unwrap(), 1);

    // --- ordering, slicing, first/last
    let ordered = qs.clone().order_by(&["-pub_date"]).fetch(db).await.unwrap();
    assert_eq!(ordered.iter().map(|q| q.id).collect::<Vec<_>>(), vec![Some(2), Some(1)]);
    assert_eq!(qs.clone().order_by(&["pub_date"]).slice(1, Some(2)).fetch(db).await.unwrap()[0].id, Some(2));
    assert_eq!(qs.clone().first(db).await.unwrap().unwrap().id, Some(1));
    assert_eq!(qs.clone().last(db).await.unwrap().unwrap().id, Some(2));
    assert_eq!(qs.clone().values(&["question_text"]).order_by(&["id"]).values_flat::<String>(db).await.unwrap(), vec!["What's up?", "Why?"]);
    assert!(qs.clone().exists(db).await.unwrap() && !qs.clone().filter_kw("id", 5).exists(db).await.unwrap());

    // --- relations: FK joins, reverse joins, select_related, annotate
    for (i, (text, votes)) in [("Not much", 3), ("The sky", 5)].into_iter().enumerate() {
        let _ = i;
        Choice::objects().create(db, Choice { id: None, question: ForeignKey::new(1), choice_text: text.into(), votes }).await.unwrap();
    }
    Choice::objects().create(db, Choice { id: None, question: ForeignKey::new(2), choice_text: "Because".into(), votes: 1 }).await.unwrap();
    assert_eq!(Choice::objects().filter_kw("question__question_text__startswith", "What").count(db).await.unwrap(), 2);
    assert_eq!(Choice::objects().filter(filter!(question = 2)).count(db).await.unwrap(), 1);
    assert_eq!(Choice::objects().filter(filter!(question__question_text__startswith = "What")).count(db).await.unwrap(), 2);
    assert_eq!(Question::objects().filter(|f| f.reverse::<ChoiceFields>("choice").votes().gt(2)).distinct().count(db).await.unwrap(), 1);
    let q1_choices = q1.choice_set().count(db).await.unwrap();
    assert_eq!(q1_choices, 2);
    assert_eq!(Question::objects().filter_kw("choice__votes__gt", 2).distinct().count(db).await.unwrap(), 1);
    let with_q = Choice::objects().select_related(&["question"]).order_by(&["id"]).fetch(db).await.unwrap();
    assert_eq!(with_q[0].question.cached().unwrap().question_text, "What's up?");
    let annotated = Question::objects().annotate("n", Count("choice")).order_by(&["-n", "id"]).values(&["id", "n"]).values_rows(db).await.unwrap();
    assert_eq!(annotated[0]["id"], Value::Int(1));
    assert_eq!(annotated[0]["n"], Value::Int(2));
    let agg = Choice::objects().aggregate(db, vec![("total", Sum("votes")), ("avg", Avg("votes")), ("most", Max("votes")), ("n", Count("*"))]).await.unwrap();
    assert_eq!(agg["total"], Value::Int(9));
    assert_eq!(agg["n"], Value::Int(3));
    // HAVING
    let busy = Question::objects().annotate("n", Count("choice")).filter_kw("n__gt", 1).fetch(db).await.unwrap();
    assert_eq!(busy.len(), 1);
    // F expressions
    Choice::objects().filter_kw("question", 1).update(db, vec![("votes", (F("votes") + 10).into())]).await.unwrap();
    assert_eq!(Choice::objects().filter_kw("votes__gte", 13).count(db).await.unwrap(), 2);
    assert_eq!(Choice::objects().filter_expr("votes__gt", F("question_id") * 5).count(db).await.unwrap(), 2);

    // --- many-to-many + prefetch
    let a1 = Author::objects().create(db, Author { id: None, name: "Ann".into(), age: Some(40) }).await.unwrap();
    let a2 = Author::objects().create(db, Author { id: None, name: "Bob".into(), age: None }).await.unwrap();
    let b = Book::objects()
        .create(db, Book { id: None, title: "Rust".into(), published: chrono::NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(), price: Decimal::new(1999, 2), authors: Default::default() })
        .await
        .unwrap();
    related::add(db, "polls.Book", "authors", &b.pk_value(), &[a1.pk_value(), a2.pk_value()]).await.unwrap();
    related::add(db, "polls.Book", "authors", &b.pk_value(), &[a1.pk_value()]).await.unwrap(); // idempotent
    assert_eq!(Book::objects().filter_kw("authors__name", "Ann").count(db).await.unwrap(), 1);
    assert_eq!(Author::objects().filter_kw("book__title", "Rust").count(db).await.unwrap(), 2);
    let fetched = Book::objects().prefetch_related(&["authors"]).fetch(db).await.unwrap();
    assert_eq!(fetched[0].authors.prefetched().unwrap().len(), 2);
    assert_eq!(fetched[0].price, Decimal::new(1999, 2));
    related::remove(db, "polls.Book", "authors", &b.pk_value(), &[a2.pk_value()]).await.unwrap();
    assert_eq!(Author::objects().filter_kw("book", b.id).count(db).await.unwrap(), 1);
    related::set(db, "polls.Book", "authors", &b.pk_value(), &[a2.pk_value()]).await.unwrap();
    assert_eq!(Author::objects().filter_kw("book__id", b.id).values_flat::<String>(db).await.is_err(), false);
    // isnull on nullable column; exclude keeps NULLs
    assert_eq!(Author::objects().filter_kw("age__isnull", true).count(db).await.unwrap(), 1);
    assert_eq!(Author::objects().exclude_kw("age", 40).count(db).await.unwrap(), 1);

    // --- get_or_create / update_or_create / bulk
    let (_, created) = Author::objects().get_or_create(db, Q::lookup("name", "Cy"), || Author { id: None, name: "Cy".into(), age: Some(1) }).await.unwrap();
    assert!(created);
    let (_, created) = Author::objects().get_or_create(db, Q::lookup("name", "Cy"), || unreachable!()).await.unwrap();
    assert!(!created);
    let (cy, _) = Author::objects().update_or_create(db, Q::lookup("name", "Cy"), |a| a.age = Some(2), || unreachable!()).await.unwrap();
    assert_eq!(cy.age, Some(2));
    let n = Author::objects().bulk_create(db, (0..5).map(|i| Author { id: None, name: format!("bulk{i}"), age: Some(i) }).collect(), None).await.unwrap();
    assert_eq!(n, 5);
    assert_eq!(Author::objects().filter_kw("name__startswith", "bulk").count(db).await.unwrap(), 5);
    assert_eq!(Author::objects().filter_kw("name__startswith", "bulk").update(db, vec![("age", 0.into())]).await.unwrap(), 5);

    // --- union, in_bulk, none
    let u = Author::objects().filter_kw("name", "Ann").union(Author::objects().filter_kw("name", "Bob"), false).fetch(db).await.unwrap();
    assert_eq!(u.len(), 2);
    assert_eq!(Author::objects().none().count(db).await.unwrap(), 0);
    assert_eq!(Author::objects().in_bulk(db, vec![Value::Int(1), Value::Int(2)]).await.unwrap().len(), 2);

    // --- lookups on the relation itself, and sticky build errors
    assert_eq!(Choice::objects().filter_q(Q::is_in("question", [1, 2])).count(db).await.unwrap(), 3);
    assert_eq!(Choice::objects().filter_q(Q::is_in("question", [2])).count(db).await.unwrap(), 1);
    assert_eq!(Choice::objects().filter_kw("question__isnull", false).count(db).await.unwrap(), 3);
    let bad = Choice::objects().filter_kw("typo", 1);
    assert!(bad.get(db).await.is_err() && bad.first(db).await.is_err() && bad.count(db).await.is_err(), "a bad filter name must not be dropped by clone");

    // --- cascading delete, protect
    let (total, per_model) = Question::objects().filter_kw("id", 1).delete(db).await.unwrap();
    assert_eq!(total, 3, "{per_model:?}");
    assert_eq!(Choice::objects().count(db).await.unwrap(), 1);
    let (n, _) = q2.delete(db).await.unwrap();
    assert_eq!(n, 2);
    // M2M rows vanish with the book
    b.delete(db).await.unwrap();
    assert_eq!(Author::objects().filter_kw("book__title", "Rust").count(db).await.unwrap(), 0);
}

async fn transactions(db: &Db) {
    use futures_util::FutureExt;
    let before = Author::objects().count(db).await.unwrap();
    let r: rango_core::Result<()> = db
        .atomic(|tx| {
            async move {
                Author::objects().create(tx, Author { id: None, name: "tx".into(), age: None }).await?;
                tx.atomic(|inner| {
                    async move {
                        Author::objects().create(inner, Author { id: None, name: "inner".into(), age: None }).await?;
                        Err::<(), _>(rango_core::Error::Other("boom".into()))
                    }
                    .boxed()
                })
                .await
                .ok();
                Ok(())
            }
            .boxed()
        })
        .await;
    r.unwrap();
    assert_eq!(Author::objects().count(db).await.unwrap(), before + 1, "savepoint rolled back the inner create only");
    let r: rango_core::Result<()> = db
        .atomic(|tx| {
            async move {
                Author::objects().create(tx, Author { id: None, name: "gone".into(), age: None }).await?;
                Err(rango_core::Error::Other("rollback".into()))
            }
            .boxed()
        })
        .await;
    assert!(r.is_err());
    assert_eq!(Author::objects().count(db).await.unwrap(), before + 1);
}

#[tokio::test]
async fn sqlite_and_postgres() {
    register_models();
    // SQLite (in memory).
    conf::configure(settings_for(DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })).unwrap();
    let db = Db::default_db();
    db.execute_batch(SQLITE_DDL).await.unwrap();
    exercise(&db).await;
    transactions(&db).await;
    println!("sqlite OK");

    // PostgreSQL (throwaway container on 127.0.0.1:54329), skipped if unreachable.
    if tokio::net::TcpStream::connect("127.0.0.1:54329").await.is_ok() {
        rango_db::connection::close_all().await;
        conf::override_settings(
            |s| {
                s.databases.insert(
                    "pg".into(),
                    DatabaseConfig {
                        engine: "rango.db.backends.postgresql".into(),
                        name: "rango".into(),
                        user: "postgres".into(),
                        password: "rango".into(),
                        host: "127.0.0.1".into(),
                        port: "54329".into(),
                        ..DatabaseConfig::default()
                    },
                );
            },
            async {
                let pg = Db::using("pg");
                pg.execute_batch(POSTGRES_DDL).await.unwrap();
                exercise(&pg).await;
                transactions(&pg).await;
                println!("postgres OK");
            },
        )
        .await;
    } else {
        println!("postgres skipped (no server on 127.0.0.1:54329)");
    }
}

