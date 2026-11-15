//! `utils._os` (`safe_join`).

use std::path::{Component, Path, PathBuf};

use crate::exceptions::{Error, Result, SuspiciousKind};

/// Lexically normalize a path (`os.path.normpath`): resolves `.` and `..` without touching the disk.
pub fn normpath(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// `safe_join(base, *paths)`: join and make sure the result is still inside `base`.
pub fn safe_join(base: &Path, paths: &[&str]) -> Result<PathBuf> {
    let mut joined = base.to_path_buf();
    for p in paths {
        let pp = Path::new(p);
        if pp.is_absolute() {
            joined = pp.to_path_buf();
        } else {
            joined.push(pp);
        }
    }
    let final_path = normpath(&std::path::absolute(&joined).map_err(|e| Error::Other(Box::new(e)))?);
    let base_path = normpath(&std::path::absolute(base).map_err(|e| Error::Other(Box::new(e)))?);
    if !final_path.starts_with(&base_path) || base_path.parent().is_none() && final_path != base_path && !final_path.starts_with(&base_path) {
        return Err(Error::suspicious(
            SuspiciousKind::SuspiciousFileOperation,
            format!("The joined path ({}) is located outside of the base path component ({})", final_path.display(), base_path.display()),
        ));
    }
    Ok(final_path)
}
