//! ModelForm end to end on SQLite: choices, create/edit, uniqueness, many-to-many.

use rango_core::conf::{self, DatabaseConfig, Settings};
use rango_db::models::{ManyToMany, Model};
use rango_db::{Db, Executor};
use rango_forms::{Data, FileData, ModelForm, ModelFormOptions, ModelFormSet};

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "lib", str = "name")]
pub struct Author {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100, unique)]
    pub name: String,
    #[field(blank)]
    pub age: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "lib", unique_together = [["title", "published"]])]
pub struct Book {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub title: String,
    pub authors: ManyToMany<Author>,
    pub published: chrono::NaiveDate,
    #[field(max_digits = 6, decimal_places = 2, default = "9.99")]
    pub price: rust_decimal::Decimal,
}

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "lib")]
pub struct Doc {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100)]
    pub title: String,
    #[field(file, max_length = 100, upload_to = "docs", blank)]
    pub attachment: String,
}

const DDL: &str = r#"
CREATE TABLE "lib_doc" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "title" varchar(100) NOT NULL, "attachment" varchar(100) NOT NULL);
CREATE TABLE "lib_author" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "name" varchar(100) NOT NULL UNIQUE, "age" integer NULL);
CREATE TABLE "lib_book" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "title" varchar(200) NOT NULL, "published" date NOT NULL, "price" decimal NOT NULL);
CREATE TABLE "lib_book_authors" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "book_id" bigint NOT NULL, "author_id" bigint NOT NULL);
"#;

fn data(pairs: &[(&str, &[&str])]) -> Data {
    Data::from_lists(pairs.iter().map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect())))
}

#[tokio::test]
async fn model_forms() {
    conf::configure(Settings {
        secret_key: "k".into(),
        media_root: std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("model_form_media").to_str().unwrap().into(),
        use_tz: true,
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        ..Settings::default()
    })
    .unwrap();
    let db = Db::default_db();
    db.execute_batch(DDL).await.unwrap();

    // Add form: fields from the model, `blank` => not required.
    let form = ModelForm::<Author>::new(&db, ModelFormOptions::all()).await.unwrap();
    let html = form.form().as_p();
    assert!(html.contains("<label for=\"id_name\">Name:</label>"), "{html}");
    assert!(html.contains("maxlength=\"100\" required"), "{html}");
    assert!(!html.contains("name=\"age\" required") && html.contains("name=\"age\""), "{html}");

    // Invalid, then valid + save.
    let mut bad = ModelForm::<Author>::new(&db, ModelFormOptions::all()).await.unwrap().bound(data(&[("name", &[""]), ("age", &["x"])]), FileData::new());
    assert!(!bad.is_valid(&db).await.unwrap());
    let mut f = ModelForm::<Author>::new(&db, ModelFormOptions::all()).await.unwrap().bound(data(&[("name", &["Ann"]), ("age", &["41"])]), FileData::new());
    let a = f.save(&db).await.unwrap().clone();
    assert_eq!((a.name.as_str(), a.age, a.id), ("Ann", Some(41), Some(1)));

    // Unique field.
    let mut dup = ModelForm::<Author>::new(&db, ModelFormOptions::all()).await.unwrap().bound(data(&[("name", &["Ann"])]), FileData::new());
    assert!(!dup.is_valid(&db).await.unwrap());
    assert_eq!(dup.form().errors().get("name").unwrap().messages(), vec!["Author with this Name already exists."]);

    // Edit keeps its own row out of the uniqueness check; blank age clears to NULL.
    let mut edit = ModelForm::<Author>::edit(&db, a.clone(), ModelFormOptions::all()).await.unwrap();
    assert!(edit.form().as_p().contains("value=\"Ann\""));
    let mut edit = ModelForm::<Author>::edit(&db, a, ModelFormOptions::all()).await.unwrap().bound(data(&[("name", &["Ann"]), ("age", &[""])]), FileData::new());
    assert_eq!(edit.save(&db).await.unwrap().age, None);
    Author::objects().create(&db, Author { id: None, name: "Bob".into(), age: None }).await.unwrap();

    // Book: many-to-many choices (labelled through `str`), defaults, unique_together.
    let add = ModelForm::<Book>::new(&db, ModelFormOptions::all()).await.unwrap();
    let html = add.form().as_p();
    assert!(html.contains("<option value=\"1\">Ann</option>") && html.contains("<option value=\"2\">Bob</option>"), "{html}");
    assert!(html.contains("value=\"9.99\""), "default price is the initial value: {html}");
    let post = data(&[("title", &["Rust"]), ("authors", &["1", "2"]), ("published", &["2024-05-06"]), ("price", &["12.50"])]);
    let mut f = ModelForm::<Book>::new(&db, ModelFormOptions::all()).await.unwrap().bound(post.clone(), FileData::new());
    let book = f.save(&db).await.unwrap().clone();
    assert_eq!(book.price.to_string(), "12.50");
    let ids = rango_db::models::related::current_ids(&db, "lib.Book", "authors", &book.pk_value()).await.unwrap();
    assert_eq!(ids.len(), 2);
    let mut again = ModelForm::<Book>::new(&db, ModelFormOptions::all()).await.unwrap().bound(post, FileData::new());
    assert!(!again.is_valid(&db).await.unwrap());
    assert_eq!(again.form().non_field_errors().messages(), vec!["Book with this Title and Published already exists."]);
    let mut bad_choice = ModelForm::<Book>::new(&db, ModelFormOptions::all()).await.unwrap().bound(data(&[("title", &["T"]), ("authors", &["9"]), ("published", &["2024-01-01"]), ("price", &["1"])]), FileData::new());
    assert!(!bad_choice.is_valid(&db).await.unwrap());
    assert_eq!(bad_choice.form().errors().get("authors").unwrap().messages(), vec!["Select a valid choice. 9 is not one of the available choices."]);

    // Model form set: edit one, delete one, add one.
    let existing = Author::objects().fetch(&db).await.unwrap();
    assert_eq!(existing.len(), 2);
    let opts = || ModelFormOptions::fields(["name", "age"]);
    let fs = ModelFormSet::<Author>::new(&db, existing.clone(), opts(), 1).await.unwrap();
    assert!(fs.formset.as_p().contains("name=\"form-TOTAL_FORMS\" value=\"3\""));
    let post = data(&[
        ("form-TOTAL_FORMS", &["3"]), ("form-INITIAL_FORMS", &["2"]),
        ("form-0-id", &["1"]), ("form-0-name", &["Ann B"]), ("form-0-age", &["42"]),
        ("form-1-id", &["2"]), ("form-1-name", &["Bob"]), ("form-1-age", &[""]),
        ("form-2-id", &[""]), ("form-2-name", &["Cy"]), ("form-2-age", &["7"]),
    ]);
    let fs = ModelFormSet::<Author>::new(&db, existing.clone(), opts(), 1).await.unwrap().bound(post, FileData::new());
    assert!(fs.formset.is_valid(), "{:?}", fs.formset.errors());
    let saved = fs.save(&db).await.unwrap();
    assert_eq!(saved.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Ann B", "Cy"]);
    let names: Vec<String> = Author::objects().order_by(&["id"]).fetch(&db).await.unwrap().into_iter().map(|a| a.name).collect();
    assert_eq!(names, vec!["Ann B", "Bob", "Cy"]);

    // File upload: saved under MEDIA_ROOT/<upload_to>, name stored in the column, collisions renamed.
    let media = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("model_form_media");
    let _ = std::fs::remove_dir_all(&media);
    let upload = |content: &str| FileData::from_lists([("attachment".to_owned(), vec![rango_http::UploadedFile::from_bytes("my report.txt", None, content.as_bytes().to_vec())])]);
    let mut f1 = ModelForm::<Doc>::new(&db, ModelFormOptions::all()).await.unwrap().bound(data(&[("title", &["Q1"])]), upload("one"));
    let d1 = f1.save(&db).await.unwrap().clone();
    assert_eq!(d1.attachment, "docs/my_report.txt");
    assert_eq!(std::fs::read_to_string(media.join("docs/my_report.txt")).unwrap(), "one");
    let mut f2 = ModelForm::<Doc>::new(&db, ModelFormOptions::all()).await.unwrap().bound(data(&[("title", &["Q2"])]), upload("two"));
    let d2 = f2.save(&db).await.unwrap().clone();
    assert!(d2.attachment.starts_with("docs/my_report_") && d2.attachment != d1.attachment, "{}", d2.attachment);
}
