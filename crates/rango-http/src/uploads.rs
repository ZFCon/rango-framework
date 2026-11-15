//! Uploaded files (`UploadedFile`) and the multipart/form-data parser (`MultiPartParser`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use indexmap::IndexMap;
use rango_core::conf::try_settings;
use rango_core::exceptions::{Error, Result, SuspiciousKind};
use rango_core::utils::datastructures::MultiValueDict;
use rango_core::utils::crypto::get_random_string;

/// Temp file that is deleted when the last handle is dropped.
#[derive(Debug)]
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Debug, Clone)]
enum Content {
    Memory(Bytes),
    Temp(Arc<TempFile>),
}

/// `InMemoryUploadedFile` / `TemporaryUploadedFile`.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub name: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub charset: Option<String>,
    pub content_type_extra: IndexMap<String, String>,
    content: Content,
}

/// `UploadedFile._set_name`: basename, at most 255 characters, extension preserved.
pub fn sanitize_upload_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    if base.chars().count() <= 255 {
        return base.to_owned();
    }
    let (stem, ext) = match base.rfind('.') {
        Some(i) => base.split_at(i),
        None => (base, ""),
    };
    let keep = 255usize.saturating_sub(ext.chars().count());
    format!("{}{ext}", stem.chars().take(keep).collect::<String>())
}

impl UploadedFile {
    pub fn from_bytes(name: &str, content_type: Option<&str>, data: impl Into<Bytes>) -> Self {
        let data: Bytes = data.into();
        Self {
            name: sanitize_upload_name(name),
            content_type: content_type.map(str::to_owned),
            size: data.len() as u64,
            charset: None,
            content_type_extra: IndexMap::new(),
            content: Content::Memory(data),
        }
    }

    /// Path of the temp file, if the upload was too large to keep in memory.
    pub fn temporary_file_path(&self) -> Option<&Path> {
        match &self.content {
            Content::Temp(t) => Some(&t.0),
            Content::Memory(_) => None,
        }
    }

    /// Whole content.
    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        match &self.content {
            Content::Memory(b) => Ok(b.to_vec()),
            Content::Temp(t) => std::fs::read(&t.0),
        }
    }
}

/// `request.FILES`.
pub type Files = MultiValueDict<UploadedFile>;

fn bad(msg: impl Into<String>) -> Error {
    Error::BadRequest(msg.into())
}

/// Parse a multipart/form-data body into `(POST fields, FILES)`.
pub async fn parse_multipart(
    body: Bytes,
    boundary: &str,
    charset: &str,
) -> Result<(crate::QueryDict, Files)> {
    let s = try_settings()?;
    let (max_fields, max_files, max_mem) = (s.data_upload_max_number_fields, s.data_upload_max_number_files, s.data_upload_max_memory_size);
    let file_mem = s.file_upload_max_memory_size.max(0) as usize;
    let temp_dir = s.file_upload_temp_dir.clone().map_or_else(std::env::temp_dir, PathBuf::from);
    drop(s);

    let mut mp = multer::Multipart::new(stream::once(async move { Ok::<_, std::io::Error>(body) }), boundary.to_owned());
    let mut post = Vec::new();
    let mut files = Files::new();
    let (mut n_fields, mut n_files, mut mem_used) = (0i64, 0i64, 0i64);
    while let Some(mut field) = mp.next_field().await.map_err(|e| bad(format!("Invalid multipart body: {e}")))? {
        let name = field.name().unwrap_or_default().to_owned();
        match field.file_name().map(str::to_owned) {
            None => {
                n_fields += 1;
                if max_fields > 0 && n_fields > max_fields {
                    return Err(Error::suspicious(SuspiciousKind::TooManyFieldsSent, "The number of GET/POST parameters exceeded settings.DATA_UPLOAD_MAX_NUMBER_FIELDS."));
                }
                let bytes = field.bytes().await.map_err(|e| bad(format!("Invalid multipart body: {e}")))?;
                mem_used += bytes.len() as i64;
                if max_mem > 0 && mem_used > max_mem {
                    return Err(Error::suspicious(SuspiciousKind::RequestDataTooBig, "Request body exceeded settings.DATA_UPLOAD_MAX_MEMORY_SIZE."));
                }
                let value = if charset.eq_ignore_ascii_case("utf-8") {
                    String::from_utf8_lossy(&bytes).into_owned()
                } else {
                    bytes.iter().map(|&b| b as char).collect()
                };
                post.push((name, value));
            }
            Some(file_name) if file_name.is_empty() => {}
            Some(file_name) => {
                n_files += 1;
                if max_files > 0 && n_files > max_files {
                    return Err(Error::suspicious(SuspiciousKind::TooManyFilesSent, "The number of files exceeded settings.DATA_UPLOAD_MAX_NUMBER_FILES."));
                }
                let content_type = field.content_type().map(|m| m.essence_str().to_owned());
                let charset_param = field.content_type().and_then(|m| m.get_param("charset").map(|c| c.as_str().to_owned()));
                let mut buf = Vec::new();
                let mut temp: Option<(PathBuf, std::fs::File)> = None;
                let mut size = 0u64;
                while let Some(chunk) = field.chunk().await.map_err(|e| bad(format!("Invalid multipart body: {e}")))? {
                    size += chunk.len() as u64;
                    if let Some((_, f)) = temp.as_mut() {
                        std::io::Write::write_all(f, &chunk).map_err(|e| Error::Other(Box::new(e)))?;
                    } else {
                        buf.extend_from_slice(&chunk);
                        if buf.len() > file_mem {
                            let path = temp_dir.join(format!("rango_upload_{}.upload", get_random_string(12, rango_core::utils::crypto::RANDOM_STRING_CHARS)));
                            let mut f = std::fs::File::create(&path).map_err(|e| Error::Other(Box::new(e)))?;
                            std::io::Write::write_all(&mut f, &buf).map_err(|e| Error::Other(Box::new(e)))?;
                            buf.clear();
                            temp = Some((path, f));
                        }
                    }
                }
                let content = match temp {
                    Some((path, _)) => Content::Temp(Arc::new(TempFile(path))),
                    None => Content::Memory(Bytes::from(buf)),
                };
                files.appendlist(
                    name,
                    UploadedFile {
                        name: sanitize_upload_name(&file_name),
                        content_type,
                        size,
                        charset: charset_param,
                        content_type_extra: IndexMap::new(),
                        content,
                    },
                );
            }
        }
    }
    Ok((crate::QueryDict::from_pairs(post, false), files))
}
