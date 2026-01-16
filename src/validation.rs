//! Validation utilities for ipv6ddns
//!
//! This module provides validation functions for various inputs including
//! DNS record names and IPv6 addresses.

use anyhow::{anyhow, Result};

/// Validates that a string is a valid DNS record name
///
/// This function validates DNS record names according to RFC 1035 and common DNS conventions.
/// It enforces the following rules:
///
/// # Validation Rules
///
/// 1. **Length constraints**:
///    - Maximum total length: 253 characters (excluding trailing dot)
///    - Maximum label length: 63 characters
///
/// 2. **Syntax rules**:
///    - Labels must be separated by dots (`.`)
///    - Labels cannot start or end with hyphens (`-`)
///    - Labels cannot contain spaces
///    - Empty labels are not allowed (e.g., `example..com`)
///    - Cannot start with a dot
///    - Cannot contain consecutive dots
///
/// 3. **Allowed characters**:
///    - Letters (a-z, A-Z)
///    - Digits (0-9)
///    - Hyphens (`-`) - not at start or end of label
///    - Underscores (`_`) - commonly used for TXT/ACME records
///    - Wildcard (`*`) - allowed as a complete label only
///
/// 4. **Special cases**:
///    - `@` represents the apex/root of the zone
///    - Trailing dot (FQDN notation) is allowed and ignored for validation
///    - `_acme-challenge` style labels are supported for ACME DNS-01 challenges
///    - Wildcard records (`*.example.com`) are supported
///
/// # Arguments
///
/// * `record_name` - A string slice containing the DNS record name to validate
///
/// # Returns
///
/// Returns `Ok(())` if the record name is valid, or an error with a descriptive message
/// explaining why validation failed.
///
/// # Examples
///
/// ```
/// use ipv6ddns::validation::validate_record_name;
///
/// // Valid record names
/// assert!(validate_record_name("@").is_ok());                           // Apex
/// assert!(validate_record_name("example.com").is_ok());                // Standard
/// assert!(validate_record_name("sub.example.com").is_ok());            // Subdomain
/// assert!(validate_record_name("_acme-challenge.example.com").is_ok()); // ACME
/// assert!(validate_record_name("*.example.com").is_ok());              // Wildcard
/// assert!(validate_record_name("example.com.").is_ok());               // FQDN
///
/// // Invalid record names
/// assert!(validate_record_name("").is_err());                          // Empty
/// assert!(validate_record_name(" ").is_err());                         // Space
/// assert!(validate_record_name(".example.com").is_err());              // Starts with dot
/// assert!(validate_record_name("example..com").is_err());              // Consecutive dots
/// assert!(validate_record_name("-example.com").is_err());              // Starts with hyphen
/// assert!(validate_record_name("example-.com").is_err());              // Ends with hyphen
/// assert!(validate_record_name("ex@mple.com").is_err());               // Invalid character
/// ```
///
/// # Error Messages
///
/// The function returns descriptive error messages for common validation failures:
/// - `"Record name cannot be empty"` - Empty or whitespace-only input
/// - `"Record name cannot contain spaces"` - Spaces are not allowed
/// - `"Record name cannot start with a dot"` - Leading dot is invalid
/// - `"Record name cannot contain consecutive dots"` - Empty label detected
/// - `"Record name contains empty label"` - Empty label detected
/// - `"Record name label too long (max 63 characters, got X)"` - Label exceeds 63 chars
/// - `"Record name too long (max 253 characters, got X)"` - Total exceeds 253 chars
/// - `"Record name label cannot start or end with hyphen"` - Hyphen at label boundary
/// - `"Record name contains invalid character: 'X' (allowed: ...)"` - Invalid character
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

/// Validates that a string is a properly formatted IPv6 address suitable for DDNS
///
/// This function performs two levels of validation:
///
/// 1. **Syntax validation**: Checks if the string can be parsed as a valid IPv6 address
///    according to RFC 4291. This includes proper use of colons, double-colon compression,
///    and hexadecimal notation.
///
/// 2. **Semantic validation**: Filters out reserved/special IPv6 address ranges that are
///    not suitable for public DNS records:
///    - **Unspecified address** (`::`): Used as a placeholder, not routable
///    - **Loopback address** (`::1`): Local host only, not routable
///    - **Link-local addresses** (`fe80::/10`): Only valid on local network segment
///    - **Multicast addresses** (`ff00::/8`): Used for one-to-many communication
///    - **Documentation addresses** (`2001:db8::/32`): Reserved for documentation
///    - **IPv4-mapped addresses** (`::ffff:0:0/96`): Transition mechanism, not native IPv6
///    - **IPv4-compatible addresses** (`::/96`): Deprecated transition mechanism
///
/// # Arguments
///
/// * `ip` - A string slice containing the IPv6 address to validate
///
/// # Returns
///
/// Returns `true` if the address is syntactically valid AND is a global unicast address
/// suitable for DDNS. Returns `false` for reserved addresses, invalid formats, or
/// addresses that are not routable on the public internet.
///
/// # Examples
///
/// ```
/// use ipv6ddns::validation::is_valid_ipv6;
///
/// // Valid global unicast addresses
/// assert!(is_valid_ipv6("2606:4700:4700::1111"));
/// assert!(is_valid_ipv6("2001:4860:4860::8888"));
///
/// // Reserved addresses are rejected
/// assert!(!is_valid_ipv6("::"));          // Unspecified
/// assert!(!is_valid_ipv6("::1"));         // Loopback
/// assert!(!is_valid_ipv6("fe80::1"));     // Link-local
/// assert!(!is_valid_ipv6("ff00::1"));     // Multicast
/// assert!(!is_valid_ipv6("2001:db8::1")); // Documentation
///
/// // Invalid formats are rejected
/// assert!(!is_valid_ipv6("invalid"));
/// assert!(!is_valid_ipv6("192.168.1.1")); // IPv4
/// ```
///
/// # Security Considerations
///
/// This function is used to validate IPv6 addresses before they are sent to Cloudflare
/// for DNS record updates. Filtering out reserved addresses prevents:
///
/// - **Information leakage**: Not exposing internal network topology
/// - **Service disruption**: Not updating DNS with non-routable addresses
/// - **Security issues**: Not using addresses meant for local or documentation purposes
///
/// # Note
///
/// Unique local addresses (`fc00::/7`) are **accepted** by this function. While these
/// are not globally routable, they may be valid for private networks that use DDNS
/// for internal services. If you need to reject these addresses, add additional filtering
/// after calling this function.
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
}
