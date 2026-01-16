//! Validation utilities for ipv6ddns
//!
//! This module provides validation functions for various inputs including
//! DNS record names and IPv6 addresses.

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

/// Validates that a string is a properly formatted IPv6 address
///
/// This function checks that the address is syntactically valid AND
/// filters out reserved/special IPv6 address ranges that are not suitable
/// for DDNS:
/// - Unspecified address (::)
/// - Loopback address (::1)
/// - Link-local addresses (fe80::/10)
/// - Multicast addresses (ff00::/8)
/// - Documentation addresses (2001:db8::/32)
pub fn is_valid_ipv6(ip: &str) -> bool {
    let addr = match ip.parse::<std::net::Ipv6Addr>() {
        Ok(a) => a,
        Err(_) => return false,
    };

    // Filter out unspecified address (::)
    if addr.is_unspecified() {
        return false;
    }

    // Filter out loopback address (::1)
    if addr.is_loopback() {
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

    // Filter out IPv4-mapped IPv6 addresses (::ffff:0:0/96)
    // These have the pattern 0, 0, 0, 0, 0, 0xffff, *, *
    if segments[0] == 0 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0
        && segments[4] == 0 && segments[5] == 0xffff
    {
        return false;
    }

    // Filter out IPv4-compatible IPv6 addresses (::/96, deprecated)
    // These have the pattern 0, 0, 0, 0, 0, 0, *, *
    if segments[0] == 0 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0
        && segments[4] == 0 && segments[5] == 0
    {
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
    fn test_is_valid_ipv6() {
        // Valid global unicast addresses
        assert!(is_valid_ipv6("2606:4700:4700::1111"));
        assert!(is_valid_ipv6("2001:4860:4860::8888"));
        assert!(is_valid_ipv6("2a00:1450:4001:81b::200e"));

        // Reserved addresses that should be rejected
        assert!(!is_valid_ipv6("::")); // Unspecified
        assert!(!is_valid_ipv6("::1")); // Loopback
        assert!(!is_valid_ipv6("fe80::1")); // Link-local
        assert!(!is_valid_ipv6("fe80::dead:beef")); // Link-local
        assert!(!is_valid_ipv6("ff00::1")); // Multicast
        assert!(!is_valid_ipv6("ff02::1")); // Multicast
        assert!(!is_valid_ipv6("2001:db8::1")); // Documentation
        assert!(!is_valid_ipv6("2001:0db8::1")); // Documentation

        // Invalid formats
        assert!(!is_valid_ipv6("192.168.1.1")); // IPv4
        assert!(!is_valid_ipv6("invalid"));
        assert!(!is_valid_ipv6(""));
        assert!(!is_valid_ipv6("2001:db8::g"));
    }

    #[test]
    fn test_is_valid_ipv6_unique_local_addresses() {
        // Unique local addresses (fc00::/7) should be valid
        assert!(is_valid_ipv6("fc00::1"));
        assert!(is_valid_ipv6("fc00:abcd:ef01:2345:6789:abcd:ef01:2345"));
        assert!(is_valid_ipv6("fd00::1"));
        assert!(is_valid_ipv6("fd12:3456:789a::1"));
        assert!(is_valid_ipv6("fd12:3456:789a:1:2:3:4:5"));

        // Edge cases of fc00::/7 range
        assert!(is_valid_ipv6("fc00::"));
        assert!(is_valid_ipv6("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));

        // Just outside the range (fbff:: should be valid as global unicast)
        assert!(is_valid_ipv6("fbff::1"));

        // Just outside the range (fe00:: should be valid as global unicast)
        assert!(is_valid_ipv6("fe00::1"));
    }

    #[test]
    fn test_is_valid_ipv6_boundary_cases() {
        // Minimum valid address (first global unicast)
        assert!(is_valid_ipv6("2000::1"));

        // Maximum valid address (last global unicast before multicast)
        assert!(is_valid_ipv6("fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));

        // First multicast address (should be rejected)
        assert!(!is_valid_ipv6("ff00::"));

        // Last multicast address (should be rejected)
        assert!(!is_valid_ipv6("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));

        // First link-local address (should be rejected)
        assert!(!is_valid_ipv6("fe80::"));

        // Last link-local address (should be rejected)
        assert!(!is_valid_ipv6("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));

        // First address after link-local (should be valid)
        assert!(is_valid_ipv6("fec0::1"));
    }

    #[test]
    fn test_is_valid_ipv6_ipv4_mapped() {
        // IPv4-mapped IPv6 addresses (should be rejected as they're not native IPv6)
        assert!(!is_valid_ipv6("::ffff:192.168.1.1"));
        assert!(!is_valid_ipv6("::ffff:c0a8:101"));

        // IPv4-compatible IPv6 addresses (deprecated, should be rejected)
        assert!(!is_valid_ipv6("::192.168.1.1"));
        assert!(!is_valid_ipv6("::c0a8:101"));
    }

    #[test]
    fn test_is_valid_ipv6_malformed() {
        // Double colon in middle (invalid)
        assert!(!is_valid_ipv6("2001:db8:::1"));

        // Multiple double colons (invalid)
        assert!(!is_valid_ipv6("2001:db8::1::"));

        // Too many segments (invalid)
        assert!(!is_valid_ipv6("2001:db8:1:2:3:4:5:6:7"));

        // Missing segments (invalid)
        assert!(!is_valid_ipv6("2001:db8:1:2:3:4:5"));

        // Invalid characters (invalid)
        assert!(!is_valid_ipv6("2001:db8::g"));
        assert!(!is_valid_ipv6("2001:db8::xyz"));
    }

    #[test]
    fn test_validate_record_name_boundary_cases() {
        // Exactly 253 characters (valid)
        let long_name = format!("{}.{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61));
        assert_eq!(long_name.len(), 253);
        assert!(validate_record_name(&long_name).is_ok());

        // 254 characters (invalid)
        let too_long_name = format!("{}.{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(62));
        assert_eq!(too_long_name.len(), 254);
        assert!(validate_record_name(&too_long_name).is_err());

        // Exactly 63 characters per label (valid)
        let long_label = format!("{}.com", "a".repeat(63));
        assert!(validate_record_name(&long_label).is_ok());

        // 64 characters per label (invalid)
        let too_long_label = format!("{}.com", "a".repeat(64));
        assert!(validate_record_name(&too_long_label).is_err());
    }

    #[test]
    fn test_validate_record_name_special_cases() {
        // Multiple underscores in label (valid)
        assert!(validate_record_name("_test__label.example.com").is_ok());

        // Multiple hyphens in label (valid)
        assert!(validate_record_name("test--label.example.com").is_ok());

        // Underscore and hyphen in same label (valid)
        assert!(validate_record_name("test_-label.example.com").is_ok());

        // Leading underscore (valid)
        assert!(validate_record_name("_test.example.com").is_ok());

        // Trailing underscore (valid)
        assert!(validate_record_name("test_.example.com").is_ok());

        // Numbers in label (valid)
        assert!(validate_record_name("test123.example.com").is_ok());

        // All numbers in label (valid)
        assert!(validate_record_name("123.example.com").is_ok());
    }
}
