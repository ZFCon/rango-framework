//! `utils.ipv6`.

use std::net::Ipv6Addr;

use crate::exceptions::ValidationError;
use crate::utils::translation::gettext;

pub const MAX_IPV6_ADDRESS_LENGTH: usize = 39;

/// Parse like `ipaddress.IPv6Address(ip_str)` (scope ids are accepted and dropped).
fn ipv6_address_from_str(ip_str: &str, max_length: usize) -> Option<Ipv6Addr> {
    if ip_str.len() > max_length {
        return None;
    }
    let addr = match ip_str.split_once('%') {
        Some((addr, scope)) if !scope.is_empty() && !scope.contains('%') => addr,
        Some(_) => return None,
        None => ip_str,
    };
    addr.parse().ok()
}

/// `clean_ipv6_address()`: compressed, lowercase form; IPv4-mapped addresses are unpacked on request.
pub fn clean_ipv6_address(
    ip_str: &str,
    unpack_ipv4: bool,
    error_message: Option<&str>,
    max_length: usize,
) -> Result<String, ValidationError> {
    let Some(addr) = ipv6_address_from_str(ip_str, max_length) else {
        let msg = error_message.map_or_else(
            || gettext("This is not a valid IPv6 address."),
            str::to_owned,
        );
        return Err(ValidationError::new(msg)
            .with_code("invalid")
            .with_params([("protocol", gettext("IPv6"))]));
    };
    if let Some(v4) = addr.to_ipv4_mapped() {
        return Ok(if unpack_ipv4 {
            v4.to_string()
        } else {
            format!("::ffff:{v4}")
        });
    }
    Ok(addr.to_string())
}

/// `is_valid_ipv6_address()`.
pub fn is_valid_ipv6_address(ip_addr: &str) -> bool {
    ipv6_address_from_str(ip_addr, MAX_IPV6_ADDRESS_LENGTH).is_some()
}
