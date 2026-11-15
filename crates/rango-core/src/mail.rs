//! `core.mail`: `EmailMessage`, `send_mail`, backends (console, locmem, file, dummy, smtp).

use std::sync::{LazyLock, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use chrono::Utc;
use indexmap::IndexMap;

use crate::conf::settings;
use crate::exceptions::{Error, Result};
use crate::utils::crypto::get_random_string;

/// An attachment (`(filename, content, mimetype)`).
#[derive(Debug, Clone)]
pub struct Attachment {
    pub filename: String,
    pub content: Vec<u8>,
    pub mimetype: String,
}

/// `EmailMessage` / `EmailMultiAlternatives`.
#[derive(Debug, Clone, Default)]
pub struct EmailMessage {
    pub subject: String,
    pub body: String,
    pub from_email: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub reply_to: Vec<String>,
    pub headers: IndexMap<String, String>,
    pub attachments: Vec<Attachment>,
    /// `attach_alternative(content, mimetype)`.
    pub alternatives: Vec<(String, String)>,
}

impl EmailMessage {
    pub fn new(subject: &str, body: &str, from_email: Option<&str>, to: &[&str]) -> Self {
        Self { subject: subject.to_owned(), body: body.to_owned(), from_email: from_email.map_or_else(|| settings().default_from_email.clone(), str::to_owned), to: to.iter().map(|s| (*s).to_owned()).collect(), ..Self::default() }
    }

    pub fn attach(&mut self, filename: &str, content: impl Into<Vec<u8>>, mimetype: &str) {
        self.attachments.push(Attachment { filename: filename.to_owned(), content: content.into(), mimetype: mimetype.to_owned() });
    }

    pub fn attach_alternative(&mut self, content: &str, mimetype: &str) {
        self.alternatives.push((content.to_owned(), mimetype.to_owned()));
    }

    /// `recipients()`: to + cc + bcc.
    pub fn recipients(&self) -> Vec<String> {
        self.to.iter().chain(&self.cc).chain(&self.bcc).cloned().collect()
    }

    /// `message().as_bytes()`: the RFC 5322 text.
    pub fn as_string(&self) -> Result<String> {
        for v in std::iter::once(&self.subject).chain(self.to.iter()).chain(std::iter::once(&self.from_email)) {
            if v.contains('\n') || v.contains('\r') {
                return Err(Error::BadRequest("Header values can't contain newlines".into()));
            }
        }
        let mut h = String::new();
        let b = |s: &str| if s.is_ascii() { s.to_owned() } else { format!("=?utf-8?b?{}?=", base64::engine::general_purpose::STANDARD.encode(s)) };
        let mut line = |k: &str, v: &str| h.push_str(&format!("{k}: {v}\n"));
        let alt = !self.alternatives.is_empty();
        let mixed = !self.attachments.is_empty();
        let boundary = |n: &str| format!("==============={}{n}==", get_random_string(19, "0123456789"));
        line("MIME-Version", "1.0");
        let body_part = |text: &str, sub: &str| {
            let (enc, cs) = if text.is_ascii() { ("7bit", "utf-8") } else { ("8bit", "utf-8") };
            format!("Content-Type: text/{sub}; charset=\"{cs}\"\nContent-Transfer-Encoding: {enc}\n\n{text}\n")
        };
        let mut body = String::new();
        let top_ct;
        if mixed || alt {
            let bound_outer = boundary("=");
            let mut parts = String::new();
            if alt {
                let bound_alt = boundary("1");
                let mut alts = format!("--{bound_alt}\n{}", body_part(&self.body, "plain"));
                for (c, mt) in &self.alternatives {
                    alts.push_str(&format!("--{bound_alt}\n{}", body_part(c, mt.strip_prefix("text/").unwrap_or("html"))));
                }
                alts.push_str(&format!("--{bound_alt}--\n"));
                if mixed {
                    parts.push_str(&format!("--{bound_outer}\nContent-Type: multipart/alternative; boundary=\"{bound_alt}\"\nMIME-Version: 1.0\n\n{alts}"));
                } else {
                    body = alts;
                    top_ct = format!("multipart/alternative; boundary=\"{bound_alt}\"");
                    return self.finish(h, &top_ct, &body, b);
                }
            } else {
                parts.push_str(&format!("--{bound_outer}\n{}", body_part(&self.body, "plain")));
            }
            for a in &self.attachments {
                parts.push_str(&format!(
                    "--{bound_outer}\nContent-Type: {}\nMIME-Version: 1.0\nContent-Transfer-Encoding: base64\nContent-Disposition: attachment; filename=\"{}\"\n\n{}\n",
                    a.mimetype,
                    a.filename,
                    base64::engine::general_purpose::STANDARD.encode(&a.content).as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\n")
                ));
            }
            parts.push_str(&format!("--{bound_outer}--\n"));
            body = parts;
            top_ct = format!("multipart/mixed; boundary=\"{bound_outer}\"");
        } else {
            let enc = if self.body.is_ascii() { "7bit" } else { "8bit" };
            body = format!("{}\n", self.body);
            top_ct = format!("text/plain; charset=\"utf-8\"\nContent-Transfer-Encoding: {enc}");
        }
        self.finish(h, &top_ct, &body, b)
    }

    fn finish(&self, mut h: String, content_type: &str, body: &str, enc: impl Fn(&str) -> String) -> Result<String> {
        h.push_str(&format!("Content-Type: {content_type}\n"));
        h.push_str(&format!("Subject: {}\n", enc(&self.subject)));
        h.push_str(&format!("From: {}\n", self.from_email));
        h.push_str(&format!("To: {}\n", self.to.join(", ")));
        if !self.cc.is_empty() {
            h.push_str(&format!("Cc: {}\n", self.cc.join(", ")));
        }
        if !self.reply_to.is_empty() {
            h.push_str(&format!("Reply-To: {}\n", self.reply_to.join(", ")));
        }
        h.push_str(&format!("Date: {}\n", Utc::now().format("%a, %d %b %Y %H:%M:%S +0000")));
        let domain = self.from_email.rsplit('@').next().unwrap_or("localhost");
        h.push_str(&format!("Message-ID: <{}.{}@{domain}>\n", Utc::now().format("%Y%m%d%H%M%S"), get_random_string(16, "0123456789abcdef")));
        for (k, v) in &self.headers {
            h.push_str(&format!("{k}: {v}\n"));
        }
        Ok(format!("{h}\n{body}"))
    }

    /// `send(fail_silently)` through the configured `EMAIL_BACKEND`.
    pub async fn send(&self) -> Result<usize> {
        if self.recipients().is_empty() {
            return Ok(0);
        }
        get_connection()?.send_messages(std::slice::from_ref(self)).await
    }
}

/// An email backend.
#[async_trait]
pub trait EmailBackend: Send + Sync {
    async fn send_messages(&self, messages: &[EmailMessage]) -> Result<usize>;
}

static OUTBOX: LazyLock<Mutex<Vec<EmailMessage>>> = LazyLock::new(|| Mutex::new(Vec::new()));

/// `mail.outbox` (locmem backend).
pub fn outbox() -> Vec<EmailMessage> {
    OUTBOX.lock().expect("outbox").clone()
}

pub fn clear_outbox() {
    OUTBOX.lock().expect("outbox").clear();
}

pub struct LocMemBackend;
pub struct ConsoleBackend;
pub struct DummyBackend;
pub struct FileBackend(pub std::path::PathBuf);
pub struct SmtpBackend;

#[async_trait]
impl EmailBackend for LocMemBackend {
    async fn send_messages(&self, m: &[EmailMessage]) -> Result<usize> {
        for msg in m {
            msg.as_string()?;
        }
        OUTBOX.lock().expect("outbox").extend_from_slice(m);
        Ok(m.len())
    }
}

#[async_trait]
impl EmailBackend for ConsoleBackend {
    async fn send_messages(&self, m: &[EmailMessage]) -> Result<usize> {
        for msg in m {
            println!("{}\n{}", msg.as_string()?, "-".repeat(79));
        }
        Ok(m.len())
    }
}

#[async_trait]
impl EmailBackend for DummyBackend {
    async fn send_messages(&self, m: &[EmailMessage]) -> Result<usize> {
        Ok(m.len())
    }
}

#[async_trait]
impl EmailBackend for FileBackend {
    async fn send_messages(&self, m: &[EmailMessage]) -> Result<usize> {
        std::fs::create_dir_all(&self.0).map_err(|e| Error::Other(Box::new(e)))?;
        let path = self.0.join(format!("{}-{}.log", Utc::now().format("%Y%m%d-%H%M%S"), get_random_string(6, "0123456789")));
        let mut text = String::new();
        for msg in m {
            text.push_str(&format!("{}\n{}\n", msg.as_string()?, "-".repeat(79)));
        }
        std::fs::write(path, text).map_err(|e| Error::Other(Box::new(e)))?;
        Ok(m.len())
    }
}

#[async_trait]
impl EmailBackend for SmtpBackend {
    /// Plain SMTP (with `AUTH LOGIN`). TLS (`EMAIL_USE_TLS`/`EMAIL_USE_SSL`) is not supported yet.
    async fn send_messages(&self, messages: &[EmailMessage]) -> Result<usize> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let s = settings();
        if s.email_use_tls || s.email_use_ssl {
            return Err(Error::improperly_configured("The SMTP backend does not support TLS yet; use a local relay or another EMAIL_BACKEND."));
        }
        let stream = tokio::net::TcpStream::connect((s.email_host.as_str(), s.email_port)).await.map_err(|e| Error::Other(Box::new(e)))?;
        let (r, mut w) = stream.into_split();
        let mut r = BufReader::new(r);
        async fn read(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Result<String> {
            let mut last = String::new();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).await.map_err(|e| Error::Other(Box::new(e)))? == 0 {
                    return Err(Error::Other("SMTP connection closed".into()));
                }
                let more = line.as_bytes().get(3) == Some(&b'-');
                last = line;
                if !more {
                    return Ok(last);
                }
            }
        }
        async fn cmd(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, w: &mut tokio::net::tcp::OwnedWriteHalf, c: &str, ok: &str) -> Result<()> {
            w.write_all(format!("{c}\r\n").as_bytes()).await.map_err(|e| Error::Other(Box::new(e)))?;
            let resp = read(r).await?;
            if resp.starts_with(ok) { Ok(()) } else { Err(Error::Other(format!("SMTP error after {c:?}: {}", resp.trim()).into())) }
        }
        read(&mut r).await?;
        cmd(&mut r, &mut w, "EHLO localhost", "250").await?;
        if !s.email_host_user.is_empty() {
            let b = |x: &str| base64::engine::general_purpose::STANDARD.encode(x);
            cmd(&mut r, &mut w, "AUTH LOGIN", "334").await?;
            cmd(&mut r, &mut w, &b(&s.email_host_user), "334").await?;
            cmd(&mut r, &mut w, &b(&s.email_host_password), "235").await?;
        }
        let mut sent = 0;
        for m in messages {
            cmd(&mut r, &mut w, &format!("MAIL FROM:<{}>", m.from_email), "250").await?;
            for rcpt in m.recipients() {
                cmd(&mut r, &mut w, &format!("RCPT TO:<{rcpt}>"), "25").await?;
            }
            cmd(&mut r, &mut w, "DATA", "354").await?;
            let data = m.as_string()?.replace('\n', "\r\n").replace("\r\n.", "\r\n..");
            cmd(&mut r, &mut w, &format!("{data}\r\n."), "250").await?;
            sent += 1;
        }
        cmd(&mut r, &mut w, "QUIT", "221").await.ok();
        Ok(sent)
    }
}

/// `get_connection()`: the backend named by `EMAIL_BACKEND`.
pub fn get_connection() -> Result<Box<dyn EmailBackend>> {
    let name = settings().email_backend.clone();
    Ok(match name.rsplit('.').next().unwrap_or("") {
        "EmailBackend" if name.contains("smtp") => Box::new(SmtpBackend),
        "EmailBackend" if name.contains("console") => Box::new(ConsoleBackend),
        "EmailBackend" if name.contains("locmem") => Box::new(LocMemBackend),
        "EmailBackend" if name.contains("dummy") => Box::new(DummyBackend),
        "EmailBackend" if name.contains("filebased") => Box::new(FileBackend(std::env::var("RANGO_EMAIL_FILE_PATH").map_or_else(|_| std::env::temp_dir().join("app-messages"), Into::into))),
        other => return Err(Error::improperly_configured(format!("Unknown EMAIL_BACKEND '{name}' ({other})"))),
    })
}

/// `send_mail(subject, message, from_email, recipient_list, html_message=...)`.
pub async fn send_mail(subject: &str, message: &str, from_email: Option<&str>, recipients: &[&str], html_message: Option<&str>) -> Result<usize> {
    let mut m = EmailMessage::new(subject, message, from_email, recipients);
    if let Some(h) = html_message {
        m.attach_alternative(h, "text/html");
    }
    m.send().await
}

/// `send_mass_mail(datatuple)`: one connection, many messages.
pub async fn send_mass_mail(messages: Vec<EmailMessage>) -> Result<usize> {
    get_connection()?.send_messages(&messages).await
}

/// `mail_admins(subject, message)`: to everyone in `ADMINS`.
pub async fn mail_admins(subject: &str, message: &str) -> Result<usize> {
    let s = settings();
    if s.admins.is_empty() {
        return Ok(0);
    }
    let to: Vec<&str> = s.admins.iter().map(|(_, e)| e.as_str()).collect();
    EmailMessage::new(&format!("{}{subject}", s.email_subject_prefix), message, Some(&s.server_email), &to).send().await
}

/// `mail_managers(subject, message)`.
pub async fn mail_managers(subject: &str, message: &str) -> Result<usize> {
    let s = settings();
    if s.managers.is_empty() {
        return Ok(0);
    }
    let to: Vec<&str> = s.managers.iter().map(|(_, e)| e.as_str()).collect();
    EmailMessage::new(&format!("{}{subject}", s.email_subject_prefix), message, Some(&s.server_email), &to).send().await
}
