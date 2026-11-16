# Sending email

Rango can build and send email through five backends: SMTP, console, file, in-memory and dummy. This page covers the
sending functions, the `EmailMessage` type, the backends and their settings, testing, and the current limits (the
SMTP backend has no TLS support).

## Importing the mail API

The mail module lives in the core crate, which the `rango` facade does not re-export at the moment. Add the core
crate next to `rangoframework`, at the same version:

```toml
[dependencies]
rangoframework = "0.1"
rangoframework-core = "0.1"   # library name: rango_core
```

```rust
use rango_core::mail::{send_mail, send_mass_mail, mail_admins, mail_managers, EmailMessage};
```

The functions read the `email_*` settings from the same settings object you pass to `rango::conf::configure`.

## Quick example

```rust
use rango_core::mail::send_mail;

async fn welcome(address: &str) -> rango::Result<()> {
    let sent = send_mail(
        "Welcome",                          // subject
        "Thanks for signing up.",           // plain-text body
        Some("hello@example.com"),          // from; None uses default_from_email
        &[address],                         // recipients
        None,                               // optional HTML body
    )
    .await?;
    assert_eq!(sent, 1);
    Ok(())
}
```

Sending is `async` and returns `rango::Result<usize>`: the number of messages handed to the backend. There is no
"fail silently" switch; a failure is an `Err` you handle or propagate.

## `send_mail`

```rust
pub async fn send_mail(
    subject: &str,
    message: &str,
    from_email: Option<&str>,
    recipients: &[&str],
    html_message: Option<&str>,
) -> Result<usize>
```

- `from_email: None` uses the `default_from_email` setting (`"webmaster@localhost"` by default).
- `html_message: Some(html)` sends a `multipart/alternative` message with the plain text first and the HTML second.
- All recipients are listed in one `To:` header and receive the same message. The return value is `1`, or `0` when
  the recipient list is empty (nothing is sent in that case).

```rust
send_mail(
    "Your receipt",
    "Total: 12.00",
    None,
    &["a@example.org", "b@example.org"],
    Some("<p>Total: <b>12.00</b></p>"),
)
.await?;
```

## `send_mass_mail`

`send_mass_mail(messages: Vec<EmailMessage>)` sends several messages through one backend connection and returns how
many were sent. With the SMTP backend this means one TCP connection for the whole batch, instead of one per message
as with repeated `send_mail` calls.

```rust
use rango_core::mail::{send_mass_mail, EmailMessage};

let messages = vec![
    EmailMessage::new("Hello Ann", "Body for Ann", None, &["ann@example.org"]),
    EmailMessage::new("Hello Bob", "Body for Bob", None, &["bob@example.org"]),
];
let sent = send_mass_mail(messages).await?;
```

Unlike `send_mail`, each message has its own subject, body and recipients, so recipients do not see each other.

## `EmailMessage`

`send_mail` is a wrapper around `EmailMessage`. Build one directly when you need `Cc`, `Bcc`, `Reply-To`, extra
headers or attachments.

```rust
use rango_core::mail::EmailMessage;

let mut msg = EmailMessage::new(
    "Monthly report",
    "The report is attached.",
    Some("reports@example.com"),
    &["team@example.com"],
);
msg.cc.push("manager@example.com".into());
msg.bcc.push("archive@example.com".into());
msg.reply_to.push("support@example.com".into());
msg.headers.insert("X-Report-Id".into(), "2024-06".into());
msg.attach("report.csv", "id,total\n1,12\n", "text/csv");
let sent = msg.send().await?;
```

### Fields

All fields are public.

| Field | Type | Meaning |
|---|---|---|
| `subject` | `String` | Subject line. Non-ASCII subjects are encoded as RFC 2047 base64 words. |
| `body` | `String` | Plain-text body. |
| `from_email` | `String` | Sender. `EmailMessage::new(.., None, ..)` fills it from `default_from_email`. |
| `to` | `Vec<String>` | `To:` recipients. |
| `cc` | `Vec<String>` | `Cc:` recipients. |
| `bcc` | `Vec<String>` | Receive the message but are not written into any header. |
| `reply_to` | `Vec<String>` | `Reply-To:` addresses. |
| `headers` | `IndexMap<String, String>` | Extra headers, appended after the standard ones in insertion order. |
| `attachments` | `Vec<Attachment>` | Files added with `attach`. |
| `alternatives` | `Vec<(String, String)>` | `(content, mimetype)` pairs added with `attach_alternative`. |

`EmailMessage` implements `Default`, so struct-update syntax works, but `EmailMessage::default()` leaves
`from_email` empty. Prefer `EmailMessage::new`.

### Methods

| Method | Meaning |
|---|---|
| `EmailMessage::new(subject, body, from_email: Option<&str>, to: &[&str])` | Constructor. |
| `attach(filename, content: impl Into<Vec<u8>>, mimetype)` | Adds an attachment from bytes or text. |
| `attach_alternative(content, mimetype)` | Adds another representation of the body, usually `"text/html"`. |
| `recipients()` | `to` + `cc` + `bcc`, the envelope recipients. |
| `as_string()` | The full RFC 5322 message text. Fails when a header value contains a newline. |
| `send().await` | Sends through the configured backend. Returns `Ok(0)` without contacting the backend when there are no recipients. |

### HTML messages

There is no separate type for multi-part messages. Call `attach_alternative` on an ordinary `EmailMessage`:

```rust
let mut msg = EmailMessage::new("Hi", "Plain text for old clients.", None, &["a@example.org"]);
msg.attach_alternative("<h1>Hi</h1><p>HTML for everyone else.</p>", "text/html");
msg.send().await?;
```

The message becomes `multipart/alternative`. When it also has attachments, the alternative part is nested inside a
`multipart/mixed` message.

### Attachments

`attach` takes the content in memory. To attach a file from disk, read it first:

```rust
let pdf = std::fs::read("invoices/1042.pdf").map_err(|e| rango::Error::Other(Box::new(e)))?;
msg.attach("invoice-1042.pdf", pdf, "application/pdf");
```

Attachments are base64 encoded with `Content-Disposition: attachment; filename="..."`. The MIME type is not guessed
from the file name; pass it explicitly.

### Generated headers

`as_string()` writes, in this order: `MIME-Version`, `Content-Type`, `Subject`, `From`, `To`, `Cc` (if any),
`Reply-To` (if any), `Date` (UTC), `Message-ID` (using the domain of `from_email`), then your `headers`.

### Header injection

`as_string()`, and therefore every backend except the dummy one, rejects a `subject`, `from_email` or `to` address
containing `\n` or `\r` with `Error::BadRequest("Header values can't contain newlines")`:

```rust
assert!(EmailMessage::new("bad\nsubject", "b", None, &["a@x.org"]).as_string().is_err());
```

`cc`, `reply_to` and the keys and values of `headers` are not checked. Do not put unvalidated user input into those
fields.

## Backends

The `email_backend` setting selects where messages go.

| `email_backend` | Behaviour |
|---|---|
| `rango.core.mail.backends.smtp.EmailBackend` (default) | Sends through an SMTP server. |
| `rango.core.mail.backends.console.EmailBackend` | Prints each message to standard output, followed by a line of dashes. |
| `rango.core.mail.backends.filebased.EmailBackend` | Writes each batch to a `.log` file. |
| `rango.core.mail.backends.locmem.EmailBackend` | Appends messages to an in-process outbox. For tests. |
| `rango.core.mail.backends.dummy.EmailBackend` | Discards messages and reports them as sent. |

Any other value fails at send time with `ImproperlyConfigured: Unknown EMAIL_BACKEND '...'`.

```rust
use rango::conf::Settings;

Settings {
    // Development: show mail in the terminal that runs the server.
    email_backend: "rango.core.mail.backends.console.EmailBackend".into(),
    default_from_email: "site@example.com".into(),
    ..Settings::default()
}
```

### SMTP

| Setting | Default | Meaning |
|---|---|---|
| `email_host` | `"localhost"` | SMTP server host. |
| `email_port` | `25` | SMTP server port. |
| `email_host_user` | `""` | When non-empty, the backend authenticates with `AUTH LOGIN`. |
| `email_host_password` | `""` | Password for `AUTH LOGIN`. |
| `email_use_tls` | `false` | Must stay `false`; see below. |
| `email_use_ssl` | `false` | Must stay `false`; see below. |

The backend opens a TCP connection per `send`/`send_mass_mail` call, greets with `EHLO localhost`, optionally
authenticates, sends each message with `MAIL FROM`, one `RCPT TO` per recipient and `DATA`, then `QUIT`. A server
reply other than the expected code ends the call with an error naming the command and the reply.

**TLS is not supported.** With `email_use_tls` or `email_use_ssl` set to `true`, sending fails with
`ImproperlyConfigured: The SMTP backend does not support TLS yet; use a local relay or another EMAIL_BACKEND.`
Because the connection is plain text, including the `AUTH LOGIN` credentials, point the backend at a relay on the
same host or on a trusted private network (for example a local Postfix or a provider's relay container) and let that
relay speak TLS to the outside.

The `email_timeout`, `email_ssl_certfile`, `email_ssl_keyfile` and `email_use_localtime` settings exist in
`Settings` but are not read by the backend. The `Date` header is always UTC.

### File

The file backend writes one file per call, named `<YYYYmmdd-HHMMSS>-<6 digits>.log`, containing each message followed
by a line of 79 dashes. The directory comes from the `RANGO_EMAIL_FILE_PATH` environment variable; when it is unset
the directory is `app-messages` under the system temporary directory. The directory is created if needed.

```console
$ RANGO_EMAIL_FILE_PATH=./sent-mail cargo run -- runserver
```

### Using a backend directly

`get_connection()` returns the backend named by `email_backend` as a `Box<dyn EmailBackend>`. The trait has one
method:

```rust
#[async_trait]
pub trait EmailBackend: Send + Sync {
    async fn send_messages(&self, messages: &[EmailMessage]) -> Result<usize>;
}
```

The backend structs are public (`SmtpBackend`, `ConsoleBackend`, `LocMemBackend`, `DummyBackend`,
`FileBackend(PathBuf)`), so you can bypass the setting for one call:

```rust
use rango_core::mail::{EmailBackend, EmailMessage, FileBackend};

let audit = FileBackend("/var/log/mysite/mail".into());
audit.send_messages(&[EmailMessage::new("Audit", "copy", None, &["ops@example.com"])]).await?;
```

Your own type can implement `EmailBackend` and be called the same way. It cannot be selected through
`email_backend`: that setting only recognises the five names above.

## Mailing admins and managers

`mail_admins(subject, message)` and `mail_managers(subject, message)` send to the addresses in the `admins` and
`managers` settings, which are lists of `(name, email)` pairs.

```rust
Settings {
    admins: vec![("Ops".into(), "ops@example.com".into())],
    managers: vec![("Editor".into(), "editor@example.com".into())],
    server_email: "server@example.com".into(),
    email_subject_prefix: "[mysite] ".into(),
    ..Settings::default()
}
```

```rust
use rango_core::mail::mail_admins;

mail_admins("Disk almost full", "Only 2% left on /var.").await?;
// Subject: [mysite] Disk almost full   From: server@example.com   To: ops@example.com
```

- The subject is prefixed with `email_subject_prefix` (default `"[Rango] "`).
- The sender is `server_email` (default `"root@localhost"`).
- Both return `Ok(0)` and send nothing when the corresponding list is empty.

Rango does not email `admins` on its own when a view fails with a server error. Call `mail_admins` from your own
error handling if you want that.

## Testing

Select the in-memory backend and inspect the outbox. `outbox()` returns a copy of every message sent in this
process; `clear_outbox()` empties it.

```rust
use rango::conf::{self, Settings};
use rango_core::mail::{clear_outbox, outbox, send_mail};

#[tokio::test]
async fn sends_welcome_mail() {
    conf::configure(Settings {
        secret_key: "k".into(),
        email_backend: "rango.core.mail.backends.locmem.EmailBackend".into(),
        default_from_email: "site@example.com".into(),
        ..Settings::default()
    })
    .unwrap();

    clear_outbox();
    let sent = send_mail("Héllo", "body", None, &["a@x.org", "b@x.org"], Some("<b>hi</b>")).await.unwrap();
    assert_eq!(sent, 1);

    let messages = outbox();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].to, vec!["a@x.org", "b@x.org"]);

    let text = messages[0].as_string().unwrap();
    assert!(text.contains("From: site@example.com"));
    assert!(text.contains("To: a@x.org, b@x.org"));
    assert!(text.contains("multipart/alternative"));
}
```

Notes:

- The outbox is one process-wide list. Tests in the same binary run in parallel and share it, so assert on the
  messages you sent (for example by subject) rather than on the total count, or keep mail tests in one test function.
- The test client does not switch the backend for you. Set `email_backend` in the test settings, or wrap the code in
  `rango::conf::override_settings(|s| s.email_backend = "...locmem.EmailBackend".into(), async { ... })`.
- The in-memory backend builds each message with `as_string()` before storing it, so header-injection errors show up
  in tests too.

## Not supported yet

- TLS in the SMTP backend (`STARTTLS` and implicit TLS).
- A connection timeout for SMTP.
- Registering a custom backend under an `email_backend` name.
- A helper that attaches a file by path and guesses its MIME type.
- A dedicated multi-alternative message type (use `attach_alternative`).
- Automatic error emails to `admins`, and the `sendtestemail` command.
- An `email_file_path` setting; the file backend is configured with `RANGO_EMAIL_FILE_PATH`.

## See also

- [Settings](settings.md) and the [settings reference](../ref/settings.md)
- [Testing](testing.md)
- [Authentication views and forms](auth/views-forms.md)
- [Deployment](deployment.md)
- [Security](security.md)
