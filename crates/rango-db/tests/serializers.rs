//! dumpdata/loaddata round trip in the fixture format.

mod common;

use chrono::{TimeZone, Utc};
use common::*;
use rango_core::conf::{self, DatabaseConfig};
use rango_db::models::{ForeignKey, Model};
use rango_db::serializers::{dump, format_objects, load_objects, parse_objects, select_models};
use rango_db::{Db, Executor};

#[tokio::test]
async fn dump_and_load() {
    register_models();
    conf::configure(settings_for(DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })).unwrap();
    let db = Db::default_db();
    db.execute_batch(SQLITE_DDL).await.unwrap();
    let q = Question::objects().create(&db, Question { id: None, question_text: "What's up?".into(), pub_date: Utc.with_ymd_and_hms(2024, 1, 5, 12, 0, 0).unwrap() }).await.unwrap();
    Choice::objects().create(&db, Choice { id: None, question: ForeignKey::to(&q), choice_text: "Not much".into(), votes: 3 }).await.unwrap();
    let a = Author::objects().create(&db, Author { id: None, name: "Ann".into(), age: None }).await.unwrap();
    let b = Book::objects().create(&db, Book { id: None, title: "Rust".into(), authors: Default::default(), published: "2024-05-06".parse().unwrap(), price: "12.50".parse().unwrap() }).await.unwrap();
    rango_db::models::related::add(&db, "polls.Book", "authors", &b.pk_value(), &[a.pk_value()]).await.unwrap();

    let models = select_models(&["polls".to_owned()]).unwrap();
    let objects = dump(&db, &models).await.unwrap();
    let text = format_objects(&objects, "json", None).unwrap();
    assert!(text.contains(r#"{"model": "polls.question", "pk": 1, "fields": {"question_text": "What's up?", "pub_date": "2024-01-05T12:00:00Z"}}"#), "{text}");
    assert!(text.contains(r#""model": "polls.choice", "pk": 1, "fields": {"question": 1, "choice_text": "Not much", "votes": 3}"#), "{text}");
    assert!(text.contains(r#""fields": {"title": "Rust", "published": "2024-05-06", "price": "12.50", "authors": [1]}"#), "{text}");
    assert_eq!(parse_objects(&format_objects(&objects, "jsonl", None).unwrap(), "jsonl").unwrap(), objects);

    // wipe, reload, and the dump is identical
    for t in ["polls_book_authors", "polls_book", "polls_author", "polls_choice", "polls_question"] {
        db.execute(&format!("DELETE FROM {t}"), &[]).await.unwrap();
    }
    assert_eq!(load_objects(&db, &parse_objects(&text, "json").unwrap()).await.unwrap(), 4);
    assert_eq!(dump(&db, &models).await.unwrap(), objects);
    // loading again updates in place
    load_objects(&db, &objects).await.unwrap();
    assert_eq!(dump(&db, &models).await.unwrap(), objects);
    flush_and_inspect(&db).await;
}

async fn flush_and_inspect(db: &Db) {
    let sql = rango_db::management::sql_flush(db.vendor(), &["polls_author".to_owned(), "polls_book".to_owned()]);
    assert_eq!(sql, vec!["DELETE FROM \"polls_author\";", "DELETE FROM \"polls_book\";", "DELETE FROM \"sqlite_sequence\" WHERE \"name\" IN ('polls_author', 'polls_book');"]);
    let src = rango_db::management::inspect(db, "polls", &["polls_choice".to_owned(), "polls_author".to_owned()]).await.unwrap();
    assert!(src.contains("pub struct PollsAuthor {") && src.contains("#[field(primary_key)] pub id: i32,") && src.contains("#[field(max_length = 100)] pub name: String,") && src.contains("pub age: Option<i32>,"), "{src}");
    assert!(src.contains("pub question: rango::db::models::ForeignKey<PollsQuestion>,"), "{src}");
}
