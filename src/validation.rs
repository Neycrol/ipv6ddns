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
        return Err(anyhow!(
            "Record name cannot contain consecutive dots"
        ));
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
            return Err(anyhow!(
                "Record name label cannot start or end with hyphen"
            ));
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
pub fn is_valid_ipv6(ip: &str) -> bool {
    ip.parse::<std::net::Ipv6Addr>().is_ok()
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
        assert!(is_valid_ipv6("2001:db8::1"));
        assert!(is_valid_ipv6("::1"));
        assert!(is_valid_ipv6("fe80::1"));
        assert!(is_valid_ipv6("2001:0db8:0000:0000:0000:0000:0000:0001"));
        assert!(!is_valid_ipv6("192.168.1.1"));
        assert!(!is_valid_ipv6("invalid"));
        assert!(!is_valid_ipv6(""));
        assert!(!is_valid_ipv6("2001:db8::g"));
    }
}