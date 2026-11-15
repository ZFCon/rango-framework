//! Migrations: DDL parity with the reference implementation (SQLite) and a full autodetect → migrate → use → unapply
//! cycle on SQLite and PostgreSQL.

mod common;

use common::*;
use rango_core::conf::{self, DatabaseConfig};
use rango_db::backends::Vendor;
use rango_db::migrations::autodetector::{changes, migration_source};
use rango_db::migrations::executor::{Migration, MigrationLoader, Target, applied, migrate, migration_sql};
use rango_db::migrations::schema::SchemaEditor;
use rango_db::migrations::state::ProjectState;
use rango_db::models::{Field, FieldType, Model, QuerySet, registry};
use rango_db::{Db, Executor};

fn initial_migration() -> Migration {
    let to = ProjectState::from_registry(&registry(), Some(&["polls".to_owned()]));
    let found = changes(&ProjectState::new(), &to).unwrap();
    let mut m = Migration::new("polls", "0001_initial").initial();
    for op in &found["polls"] {
        m = m.operation(op.clone());
    }
    m
}

#[tokio::test]
async fn migrations_work() {
    conf::configure(settings_for(DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })).unwrap();

    // 1. DDL text equals the schema editor output on SQLite.
    let initial = initial_migration();
    let loader = MigrationLoader::from_migrations(vec![initial.clone()]).unwrap();
    let got = migration_sql(&loader, &("polls".into(), "0001_initial".into()), Vendor::Sqlite, false).unwrap();
    let want: Vec<String> = serde_json::from_str::<Vec<String>>(include_str!("golden/ddl_sqlite.json")).unwrap().into_iter().map(|s| s.trim_end_matches(';').to_owned()).collect();
    assert_eq!(got, want);

    // 2. The writer produces a migration source that mentions every model.
    let ops: Vec<_> = initial.operations.clone();
    let src = migration_source("polls", "0001_initial", true, &[], &ops);
    assert!(src.contains("Migration::new(\"polls\", \"0001_initial\")") && src.contains("ModelMeta::new(\"polls\", \"Choice\")") && src.contains("register_migration!(migration)"), "{src}");

    // 3. Apply on SQLite: a second migration adds a column; use the schema; then unapply everything.
    let mut state = loader.full_state().unwrap();
    let mut changed = state.clone();
    changed.get_mut("polls.Author").unwrap().fields.push(Field::new("email", FieldType::Email).null());
    let second_ops = changes(&state, &changed).unwrap();
    assert_eq!(second_ops["polls"].len(), 1);
    let m2 = Migration::new("polls", "0002_author_email").dependency("polls", "0001_initial").operation(second_ops["polls"][0].clone());
    let loader = MigrationLoader::from_migrations(vec![initial.clone(), m2.clone()]).unwrap();
    state = loader.full_state().unwrap();
    assert!(state.get("polls.Author").unwrap().fields.iter().any(|f| f.name == "email"));

    let db = Db::default_db();
    let lines = migrate(&db, &loader, &Target::All).await.unwrap();
    assert_eq!(lines, vec!["  Applying polls.0001_initial... OK", "  Applying polls.0002_author_email... OK"]);
    assert_eq!(applied(&db).await.unwrap().len(), 2);
    assert_eq!(migrate(&db, &loader, &Target::All).await.unwrap(), vec!["  No migrations to apply."]);
    // The new column exists in the database.
    let cols: Vec<String> = db.query("PRAGMA table_info(polls_author)", &[]).await.unwrap().into_iter().map(|r| r.values[1].as_str().unwrap().to_owned()).collect();
    assert!(cols.contains(&"email".to_owned()), "{cols:?}");
    let a = Author::objects().create(&db, Author { id: None, name: "Ann".into(), age: None }).await.unwrap();
    assert_eq!(a.id, Some(1));
    // Backwards to 0001 removes the column (SQLite 3.35+ DROP COLUMN), `zero` drops the tables.
    migrate(&db, &loader, &Target::Migration("polls".into(), "0001".into())).await.unwrap();
    assert_eq!(applied(&db).await.unwrap().len(), 1);
    migrate(&db, &loader, &Target::Migration("polls".into(), "zero".into())).await.unwrap();
    assert!(applied(&db).await.unwrap().is_empty());
    assert!(db.query("SELECT 1 FROM polls_author", &[]).await.is_err());

    // 4. PostgreSQL: same cycle.
    if tokio::net::TcpStream::connect("127.0.0.1:54329").await.is_ok() {
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
                pg.execute_batch("DROP TABLE IF EXISTS rango_migrations, polls_book_authors, polls_book, polls_author, polls_choice, polls_question CASCADE").await.unwrap();
                let lines = migrate(&pg, &loader, &Target::All).await.unwrap();
                assert_eq!(lines.len(), 2, "{lines:?}");
                let q = Question::objects().create(&pg, Question { id: None, question_text: "pg".into(), pub_date: chrono::Utc::now() }).await.unwrap();
                Choice::objects().create(&pg, Choice { id: None, question: rango_db::models::ForeignKey::new(q.id.unwrap()), choice_text: "c".into(), votes: 1 }).await.unwrap();
                assert_eq!(QuerySet::<Choice>::new().count(&pg).await.unwrap(), 1);
                let cols: Vec<String> = pg.query("SELECT column_name FROM information_schema.columns WHERE table_name = 'polls_author'", &[]).await.unwrap().into_iter().map(|r| r.values[0].as_str().unwrap().to_owned()).collect();
                assert!(cols.contains(&"email".to_owned()), "{cols:?}");
                migrate(&pg, &loader, &Target::Migration("polls".into(), "zero".into())).await.unwrap();
                assert!(pg.query("SELECT 1 FROM polls_author", &[]).await.is_err());
                let _ = SchemaEditor::new(Vendor::Postgresql, &registry());
                println!("postgres migrations OK");
            },
        )
        .await;
    }
}
