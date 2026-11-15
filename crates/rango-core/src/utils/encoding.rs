//! Port of the URL-encoding half of `utils.encoding`
//! (`force_str`/`force_bytes` have no Rust counterpart: strings are `str`, bytes are `[u8]`).

use super::http::quote;

const IRI_SAFE: &str = "/#%[]=:;$&()+,!?*@'~";

/// `iri_to_uri()`: percent-encode an IRI's non-ASCII/unsafe characters.
pub fn iri_to_uri(iri: &str) -> String {
    quote(iri, IRI_SAFE)
}

/// `escape_uri_path()`.
pub fn escape_uri_path(path: &str) -> String {
    quote(path, "/:@&+$,-_.!~*'()")
}

/// `filepath_to_uri()`.
pub fn filepath_to_uri(path: &str) -> String {
    quote(&path.replace('\\', "/"), "/~!*()'")
}

/// `punycode()`: the ASCII (IDNA) form of a domain.
pub fn punycode(domain: &str) -> Result<String, String> {
    idna::domain_to_ascii(domain).map_err(|e| e.to_string())
}

/// `repercent_broken_unicode()`: percent-encode bytes that are not valid UTF-8.
pub fn repercent_broken_unicode(mut path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    loop {
        match std::str::from_utf8(path) {
            Ok(_) => {
                out.extend_from_slice(path);
                return out;
            }
            Err(e) => {
                let start = e.valid_up_to();
                let bad = e.error_len().unwrap_or(path.len() - start);
                out.extend_from_slice(&path[..start]);
                for &b in &path[start..start + bad] {
                    if b.is_ascii_alphanumeric()
                        || IRI_SAFE.as_bytes().contains(&b)
                        || b"_.-".contains(&b)
                    {
                        out.push(b);
                    } else {
                        out.extend_from_slice(format!("%{b:02X}").as_bytes());
                    }
                }
                path = &path[start + bad..];
            }
        }
    }
}

/// `uri_to_iri()`: decode `%XX` escapes of non-ASCII and unreserved characters.
pub fn uri_to_iri(uri: &str) -> String {
    let mut bits = uri.as_bytes().split(|&b| b == b'%');
    let mut out: Vec<u8> = bits.next().unwrap_or_default().to_vec();
    for item in bits {
        let decoded = item.get(..2).and_then(|hex| {
            let v = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            (v >= 0x80 || v.is_ascii_alphanumeric() || b"-._~".contains(&v)).then_some(v)
        });
        match decoded {
            Some(v) => {
                out.push(v);
                out.extend_from_slice(&item[2..]);
            }
            None => {
                out.push(b'%');
                out.extend_from_slice(item);
            }
        }
    }
    String::from_utf8(repercent_broken_unicode(&out)).expect("repercent yields valid UTF-8")
}
