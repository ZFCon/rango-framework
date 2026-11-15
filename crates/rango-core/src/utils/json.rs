//! Python-compatible `json.dumps` output (`ensure_ascii=True`).
//!
//! Signed values, session payloads and `json_script` must be byte-identical to what the reference implementation
//! produces so each side can read the other's data.

use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

struct PyFormatter {
    /// `separators=(",", ":")` when true, Python's default `(", ", ": ")` otherwise.
    compact: bool,
    /// `ensure_ascii`: escape non-ASCII as `\uXXXX`.
    ascii: bool,
}

impl Formatter for PyFormatter {
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        match (first, self.compact) {
            (true, _) => Ok(()),
            (false, true) => w.write_all(b","),
            (false, false) => w.write_all(b", "),
        }
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.begin_array_value(w, first)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(if self.compact { b":" } else { b": " })
    }

    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        if !self.ascii {
            return w.write_all(fragment.as_bytes());
        }
        let mut start = 0;
        for (i, c) in fragment.char_indices() {
            if (c as u32) < 0x7f {
                continue;
            }
            w.write_all(fragment[start..i].as_bytes())?;
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                write!(w, "\\u{unit:04x}")?;
            }
            start = i + c.len_utf8();
        }
        w.write_all(fragment[start..].as_bytes())
    }
}

/// `json.dumps(value, separators=...)` with Python's `ensure_ascii=True` escaping.
pub fn dumps<T: Serialize + ?Sized>(value: &T, compact: bool) -> serde_json::Result<String> {
    dumps_with(value, compact, true)
}

/// `json.dumps(value, ensure_ascii=ensure_ascii)`.
pub fn dumps_with<T: Serialize + ?Sized>(value: &T, compact: bool, ensure_ascii: bool) -> serde_json::Result<String> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, PyFormatter { compact, ascii: ensure_ascii });
    value.serialize(&mut ser)?;
    Ok(String::from_utf8(out).expect("serde_json emits UTF-8"))
}
