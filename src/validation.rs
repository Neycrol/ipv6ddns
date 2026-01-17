//! Validation utilities for ipv6ddns
//!
//! This module provides validation functions for various inputs including
//! DNS record names and IPv6 addresses.
//!
//! # Functions
//!
//! - `validate_record_name`: Validates DNS record names according to RFC standards
//! - `is_valid_ipv6`: Validates IPv6 addresses and filters out reserved ranges
//!
//! # DNS Record Name Validation
//!
//! The `validate_record_name` function validates DNS record names according to
//! RFC 1035 and RFC 1123, with support for:
//! - Apex records (@)
//! - Wildcard records (*.example.com)
//! - ACME challenge records (_acme-challenge.example.com)
//! - FQDNs with trailing dots (example.com.)
//!
//! # IPv6 Address Validation
//!
//! The `is_valid_ipv6` function validates IPv6 addresses and filters out:
//! - Unspecified address (::)
//! - Loopback address (::1, unless allow_loopback is true)
//! - Link-local addresses (fe80::/10)
//! - Multicast addresses (ff00::/8)
//! - Documentation addresses (2001:db8::/32)
//!
//! Unique-local addresses (fc00::/7) are allowed by design, since DDNS is often
//! used on private networks.

use anyhow::{anyhow, Result};

/// Validates that a string is a reasonable DNS record name.
///
/// Allows common DNS conventions used for TXT/ACME and wildcard records:
/// - `@` for apex
/// - `_` in labels (e.g. `_acme-challenge`)
/// - `*` as a whole label (e.g. `*.example.com`)
/// - trailing dot (FQDN), which is ignored for validation
pub fn validate_record_name(record_name: &str) -> Result<()> {
    let trimmed = record_name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Record name cannot be empty"));
    }
    if trimmed == "@" {
        return Ok(());
    }
    if trimmed.contains(' ') {
        return Err(anyhow!("Record name cannot contain spaces"));
    }

    let name = trimmed.strip_suffix('.').unwrap_or(trimmed);
    if name.is_empty() {
        return Err(anyhow!("Record name cannot be empty"));
    }
    if name.len() > 253 {
        return Err(anyhow!(
            "Record name too long (max 253 characters, got {})",
            name.len()
        ));
    }
    if name.starts_with('.') {
        return Err(anyhow!("Record name cannot start with a dot"));
    }
    if name.contains("..") {
        return Err(anyhow!("Record name cannot contain consecutive dots"));
    }

    for label in name.split('.') {
        if label.is_empty() {
            return Err(anyhow!("Record name contains empty label"));
        }
        if label == "*" {
            continue;
        }
        if label.len() > 63 {
            return Err(anyhow!(
                "Record name label too long (max 63 characters, got {})",
                label.len()
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(anyhow!("Record name label cannot start or end with hyphen"));
        }
        for ch in label.chars() {
            if !ch.is_alphanumeric() && ch != '-' && ch != '_' {
                return Err(anyhow!(
                    "Record name contains invalid character: '{}' (allowed: letters, digits, '-', '_', or wildcard labels)",
                    ch
                ));
            }
        }
    }

    Ok(())
}

/// Validates that a string is a properly formatted IPv6 address.
///
/// This function checks that the address is syntactically valid AND filters out
/// reserved/special IPv6 address ranges that are not suitable for DDNS:
/// - Unspecified address (::)
/// - Loopback address (::1)
/// - Link-local addresses (fe80::/10)
/// - Multicast addresses (ff00::/8)
/// - Documentation addresses (2001:db8::/32)
///
/// Note: unique-local addresses (fc00::/7) are allowed by design, since DDNS
/// is often used on private networks.
pub fn is_valid_ipv6(ip: &str, allow_loopback: bool) -> bool {
    let addr = match ip.parse::<std::net::Ipv6Addr>() {
        Ok(a) => a,
        Err(_) => return false,
    };

    // Filter out unspecified address (::)
    if addr.is_unspecified() {
        return false;
    }

    // Filter out loopback address (::1)
    if addr.is_loopback() && !allow_loopback {
        return false;
    }

    let segments = addr.segments();

    // Filter out link-local addresses (fe80::/10)
    // Link-local addresses have first 10 bits as 1111111010
    if segments[0] & 0xffc0 == 0xfe80 {
        return false;
    }

    // Filter out multicast addresses (ff00::/8)
    // Multicast addresses have first 8 bits as 11111111
    if segments[0] & 0xff00 == 0xff00 {
        return false;
    }

    // Filter out documentation addresses (2001:db8::/32)
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_record_name_valid_cases() {
        assert!(validate_record_name("@").is_ok());
        assert!(validate_record_name("example.com").is_ok());
        assert!(validate_record_name("sub.example.com").is_ok());
        assert!(validate_record_name("_acme-challenge.example.com").is_ok());
        assert!(validate_record_name("*.example.com").is_ok());
        assert!(validate_record_name("a-b.example.com").is_ok());
        assert!(validate_record_name("example.com.").is_ok());
        assert!(validate_record_name(&("a".repeat(63) + ".com")).is_ok());
    }

    #[test]
    fn test_validate_record_name_invalid_cases() {
        assert!(validate_record_name("").is_err());
        assert!(validate_record_name(" ").is_err());
        assert!(validate_record_name("example com").is_err());
        assert!(validate_record_name(".example.com").is_err());
        assert!(validate_record_name("example..com").is_err());
        assert!(validate_record_name("-example.com").is_err());
        assert!(validate_record_name("example-.com").is_err());
        assert!(validate_record_name("ex@mple.com").is_err());
        assert!(validate_record_name(&"a.".repeat(254)).is_err());
    }

    #[test]
    fn test_validate_record_name_boundaries() {
        let max_name = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        assert_eq!(max_name.len(), 253);
        assert!(validate_record_name(&max_name).is_ok());

        let too_long = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(62)
        );
        assert_eq!(too_long.len(), 254);
        assert!(validate_record_name(&too_long).is_err());

        let max_label = format!("{}.com", "a".repeat(63));
        assert!(validate_record_name(&max_label).is_ok());

        let too_long_label = format!("{}.com", "a".repeat(64));
        assert!(validate_record_name(&too_long_label).is_err());
    }

    #[test]
    fn test_is_valid_ipv6() {
        // Valid global unicast addresses
        assert!(is_valid_ipv6("2606:4700:4700::1111", false));
        assert!(is_valid_ipv6("2001:4860:4860::8888", false));
        assert!(is_valid_ipv6("2a00:1450:4001:81b::200e", false));

        // Unique-local addresses are allowed
        assert!(is_valid_ipv6("fc00::1", false));
        assert!(is_valid_ipv6("fd12:3456:789a::1", false));
        // Reserved addresses that should be rejected
        assert!(!is_valid_ipv6("::", false)); // Unspecified
        assert!(!is_valid_ipv6("::1", false)); // Loopback (default reject)
        assert!(!is_valid_ipv6("fe80::1", false)); // Link-local
        assert!(!is_valid_ipv6("fe80::dead:beef", false)); // Link-local
        assert!(!is_valid_ipv6("ff00::1", false)); // Multicast
        assert!(!is_valid_ipv6("ff02::1", false)); // Multicast
        assert!(!is_valid_ipv6("2001:db8::1", false)); // Documentation
        assert!(!is_valid_ipv6("2001:0db8::1", false)); // Documentation

        // Invalid formats
        assert!(!is_valid_ipv6("192.168.1.1", false)); // IPv4
        assert!(!is_valid_ipv6("invalid", false));
        assert!(!is_valid_ipv6("", false));
        assert!(!is_valid_ipv6("2001:db8::g", false));
    }

    #[test]
    fn test_is_valid_ipv6_allow_loopback() {
        assert!(is_valid_ipv6("::1", true));
        assert!(!is_valid_ipv6("::", true));
    }

    // Additional edge case tests for IPv6 validation

    #[test]
    fn test_ipv6_compression_variants() {
        // Valid compressed addresses (not in documentation range)
        assert!(is_valid_ipv6("2001:4860::8888", false));
        assert!(is_valid_ipv6("2001:4860:0:0:0:0:0:8888", false));
        assert!(is_valid_ipv6("2001::", false));
    }

    #[test]
    fn test_ipv6_with_port() {
        // IPv6 addresses with port notation should be rejected
        assert!(!is_valid_ipv6("[2001:db8::1]:8080", false));
    }

    #[test]
    fn test_ipv6_zone_id() {
        // IPv6 addresses with zone ID should be rejected
        assert!(!is_valid_ipv6("fe80::1%eth0", false));
    }

    #[test]
    fn test_ipv6_max_compression() {
        assert!(!is_valid_ipv6("::", false)); // Fully compressed - unspecified, rejected
        assert!(is_valid_ipv6("2001::", false)); // Trailing zeroes
        assert!(!is_valid_ipv6("::1", false)); // Leading zeroes - loopback, rejected
    }

    #[test]
    fn test_ipv6_boundary_values() {
        // Minimum valid IPv6 (all zeros)
        assert!(!is_valid_ipv6("::", false)); // Unspecified, rejected
        // Maximum valid IPv6 (all F's)
        assert!(!is_valid_ipv6("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", false)); // Multicast-ish
    }

    #[test]
    fn test_ipv6_partial_compression() {
        assert!(is_valid_ipv6("2001:4860:0:0:1:0:0:1", false));
        assert!(is_valid_ipv6("2001:4860::1:0:0:1", false));
        assert!(is_valid_ipv6("2001:4860:0:0:1::1", false));
    }

    #[test]
    fn test_ipv6_multiple_double_colon() {
        // Only one :: is allowed
        assert!(!is_valid_ipv6("2001::db8::1", false));
    }

    #[test]
    fn test_ipv6_leading_trailing_colons() {
        assert!(!is_valid_ipv6(":2001:db8::1", false));
        assert!(!is_valid_ipv6("2001:db8::1:", false));
    }

    #[test]
    fn test_ipv6_invalid_characters() {
        assert!(!is_valid_ipv6("2001:db8::g", false));
        assert!(!is_valid_ipv6("2001:db8::1.2.3.4", false));
        assert!(!is_valid_ipv6("2001:db8::12345", false)); // Too many digits
    }

    // Additional edge case tests for DNS record name validation

    #[test]
    fn test_validate_record_name_single_label() {
        assert!(validate_record_name("example").is_ok());
        assert!(validate_record_name("a").is_ok());
        assert!(validate_record_name(&"a".repeat(63)).is_ok());
    }

    #[test]
    fn test_validate_record_name_multiple_underscores() {
        assert!(validate_record_name("_test.example.com").is_ok());
        assert!(validate_record_name("__test__.example.com").is_ok());
        assert!(validate_record_name("a_b.c_d.example.com").is_ok());
    }

    #[test]
    fn test_validate_record_name_wildcard_variations() {
        assert!(validate_record_name("*.example.com").is_ok());
        assert!(validate_record_name("*.sub.example.com").is_ok());
        assert!(validate_record_name("*").is_ok());
        assert!(!validate_record_name("**.example.com").is_ok()); // Double wildcard
        assert!(!validate_record_name("a*.example.com").is_ok()); // Partial wildcard
    }

    #[test]
    fn test_validate_record_name_trailing_dots() {
        assert!(validate_record_name("example.com.").is_ok());
        assert!(validate_record_name("example.com..").is_err()); // Double trailing dot
        assert!(validate_record_name(".example.com").is_err()); // Leading dot
    }

    #[test]
    fn test_validate_record_name_special_characters() {
        assert!(validate_record_name("a-b.example.com").is_ok());
        assert!(validate_record_name("a_b.example.com").is_ok());
        assert!(validate_record_name("a.b@example.com").is_err()); // @ not allowed
        assert!(validate_record_name("a$b.example.com").is_err()); // $ not allowed
        assert!(validate_record_name("a%b.example.com").is_err()); // % not allowed
    }

    #[test]
    fn test_validate_record_name_numeric_only() {
        assert!(validate_record_name("123.example.com").is_ok());
        assert!(validate_record_name("123.456.789.012").is_ok());
    }

    #[test]
    fn test_validate_record_name_mixed_case() {
        assert!(validate_record_name("Example.Com").is_ok());
        assert!(validate_record_name("EXAMPLE.COM").is_ok());
        assert!(validate_record_name("eXaMpLe.CoM").is_ok());
    }

    #[test]
    fn test_validate_record_name_whitespace() {
        // Leading/trailing whitespace is trimmed, so these should pass
        assert!(validate_record_name(" example.com").is_ok()); // Leading space (trimmed)
        assert!(validate_record_name("example.com ").is_ok()); // Trailing space (trimmed)
        // Internal whitespace should fail
        assert!(validate_record_name("ex ample.com").is_err()); // Internal space
        assert!(validate_record_name("example\t.com").is_err()); // Tab
        assert!(validate_record_name("example\n.com").is_err()); // Newline
    }
}
