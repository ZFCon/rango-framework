//! `core.files`: `File`, `ContentFile` and storage backends.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::conf::settings;
use crate::exceptions::{Error, Result};
use crate::utils::crypto::get_random_string;
use crate::utils::http::quote;

/// `ContentFile` / uploaded content: bytes plus an optional name.
#[derive(Debug, Clone, Default)]
pub struct ContentFile {
    pub name: Option<String>,
    pub content: Vec<u8>,
}

impl ContentFile {
    pub fn new(content: impl Into<Vec<u8>>) -> Self {
        Self { name: None, content: content.into() }
    }
    pub fn named(name: &str, content: impl Into<Vec<u8>>) -> Self {
        Self { name: Some(name.to_owned()), content: content.into() }
    }
    pub fn size(&self) -> usize {
        self.content.len()
    }
}

/// `get_valid_filename("john's portrait in 2004.jpg")` -> `"johns_portrait_in_2004.jpg"`.
pub fn get_valid_filename(name: &str) -> Result<String> {
    let s: String = name.trim().replace(' ', "_");
    let s: String = s.chars().filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')).collect();
    if s.is_empty() || s == "." || s == ".." {
        return Err(Error::Suspicious { kind: crate::exceptions::SuspiciousKind::SuspiciousFileOperation, message: format!("Could not derive file name from '{name}'") });
    }
    Ok(s)
}

/// A storage backend (`core.files.storage.Storage`).
#[async_trait]
pub trait Storage: Send + Sync {
    /// Save `content` under (a free variant of) `name`; returns the name actually used.
    async fn save(&self, name: &str, content: &[u8]) -> Result<String>;
    async fn open(&self, name: &str) -> Result<Vec<u8>>;
    async fn delete(&self, name: &str) -> Result<()>;
    async fn exists(&self, name: &str) -> Result<bool>;
    async fn size(&self, name: &str) -> Result<u64>;
    async fn listdir(&self, path: &str) -> Result<(Vec<String>, Vec<String>)>;
    fn url(&self, name: &str) -> Result<String>;
    /// Filesystem path, for backends that have one.
    fn path(&self, _name: &str) -> Result<PathBuf> {
        Err(Error::Other("This backend doesn't support absolute paths.".into()))
    }
    /// `get_available_name(name)`: a name that does not collide with an existing file.
    async fn get_available_name(&self, name: &str) -> Result<String> {
        let p = Path::new(name);
        let (dir, stem, ext) = (p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(), p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default());
        let mut candidate = name.to_owned();
        while self.exists(&candidate).await? {
            let suffix = get_random_string(7, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789");
            candidate = if dir.is_empty() { format!("{stem}_{suffix}{ext}") } else { format!("{dir}/{stem}_{suffix}{ext}") };
        }
        Ok(candidate)
    }
}

/// `FileSystemStorage(location=MEDIA_ROOT, base_url=MEDIA_URL)`.
#[derive(Debug, Clone)]
pub struct FileSystemStorage {
    pub location: PathBuf,
    pub base_url: String,
}

impl FileSystemStorage {
    pub fn new(location: impl Into<PathBuf>, base_url: &str) -> Self {
        Self { location: location.into(), base_url: base_url.to_owned() }
    }

    /// Defaults from `MEDIA_ROOT` / `MEDIA_URL`.
    pub fn media() -> Self {
        let s = settings();
        Self::new(&s.media_root, &s.media_url)
    }

    fn safe_path(&self, name: &str) -> Result<PathBuf> {
        crate::utils::os::safe_join(&self.location, &[name]).map_err(|e| Error::Suspicious { kind: crate::exceptions::SuspiciousKind::SuspiciousFileOperation, message: e.to_string() })
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Other(Box::new(e))
}

#[async_trait]
impl Storage for FileSystemStorage {
    async fn save(&self, name: &str, content: &[u8]) -> Result<String> {
        let name = self.get_available_name(&name.replace('\\', "/")).await?;
        let path = self.safe_path(&name)?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(io)?;
        }
        std::fs::write(&path, content).map_err(io)?;
        Ok(name)
    }
    async fn open(&self, name: &str) -> Result<Vec<u8>> {
        std::fs::read(self.safe_path(name)?).map_err(io)
    }
    async fn delete(&self, name: &str) -> Result<()> {
        match std::fs::remove_file(self.safe_path(name)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(e)),
            _ => Ok(()),
        }
    }
    async fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.safe_path(name)?.exists())
    }
    async fn size(&self, name: &str) -> Result<u64> {
        Ok(std::fs::metadata(self.safe_path(name)?).map_err(io)?.len())
    }
    async fn listdir(&self, path: &str) -> Result<(Vec<String>, Vec<String>)> {
        let (mut dirs, mut files) = (vec![], vec![]);
        for e in std::fs::read_dir(self.safe_path(path)?).map_err(io)?.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() { dirs.push(n) } else { files.push(n) }
        }
        dirs.sort();
        files.sort();
        Ok((dirs, files))
    }
    fn url(&self, name: &str) -> Result<String> {
        if self.base_url.is_empty() {
            return Err(Error::Other("This file is not accessible via a URL.".into()));
        }
        Ok(format!("{}{}", self.base_url.trim_end_matches('/').to_owned() + "/", quote(name.trim_start_matches('/'), "/")))
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        self.safe_path(name)
    }
}

/// `default_storage` from `STORAGES["default"]` (the file system storage under `MEDIA_ROOT`).
pub fn default_storage() -> Arc<dyn Storage> {
    Arc::new(FileSystemStorage::media())
}
