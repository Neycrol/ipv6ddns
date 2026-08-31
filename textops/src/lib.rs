//! Validation and log-redaction primitives for ipv6ddns.
//!
//! This lives in its own crate for a performance reason, not organization:
//! the root release profile uses `opt-level = "s"` (smaller binary, lower
//! RSS), but the root Cargo.toml overrides this package back to
//! `opt-level = 3` via `[profile.release.package.textops]`. Measured with
//! alternating A/B micro-benchmarks (see `cpu_bench` below), `"s"` codegen
//! cost these functions 9-17% while the rest of the dependency tree only
//! benefited. Splitting them out recovers full speed on the hot paths while
//! keeping the size/memory win everywhere else.
//!
//! # Functions
//!
//! - [`validate_record_name`]: Validates DNS record names per RFC 1035/1123
//! - [`is_valid_ipv6`]: Validates IPv6 addresses, filtering reserved ranges
//! - [`redact_secrets`]: Scrubs API tokens/zone IDs from log messages

use anyhow::{Result, anyhow};

/// Maximum total length of a DNS record name (RFC 1035)
pub const MAX_RECORD_NAME_LENGTH: usize = 253;
/// Maximum length of a single DNS label (RFC 1035)
pub const MAX_LABEL_LENGTH: usize = 63;

/// Validates that a string is a reasonable DNS record name.
///
/// Allows common DNS conventions used for TXT/ACME and wildcard records:
/// - `@` for apex
/// - `_` in labels (e.g. `_acme-challenge`)
/// - `*` as a whole label (e.g. `*.example.com`)
/// - trailing dot (FQDN), which is ignored for validation
//
// `inline(never)` keeps this function's machine code identical regardless of
// the caller's optimization level: the body is always the opt-level 3 version
// compiled into this crate, never an "s"-codegen inlined copy produced by
// LTO. Measured via same-binary A/B in the root crate's cpu_bench.
#[inline(never)]
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
    if name.len() > MAX_RECORD_NAME_LENGTH {
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
        if label.len() > MAX_LABEL_LENGTH {
            return Err(anyhow!(
                "Record name label too long (max 63 characters, got {})",
                label.len()
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(anyhow!("Record name label cannot start or end with hyphen"));
        }
        // Byte-level scan: DNS labels are ASCII per RFC 1035, so we can skip
        // UTF-8 decoding entirely. This is also ~5x faster than a
        // `char::is_alphanumeric()` check, which walks Unicode tables for
        // every character. Non-ASCII bytes fail `is_ascii_alphanumeric` and
        // are rejected below.
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            // Slow path only for the error message: locate the first offender.
            let bad = label
                .chars()
                .find(|ch| !ch.is_ascii_alphanumeric() && *ch != '-' && *ch != '_')
                .expect("label contains an invalid character");
            return Err(anyhow!(
                "Record name contains invalid character: '{bad}' (allowed: letters, digits, '-', '_', or wildcard labels)"
            ));
        }
    }

    Ok(())
}

/// Validates an IPv6 address and filters out reserved ranges.
///
/// This function filters out reserved/special IPv6 addresses that are not
/// suitable for DDNS:
/// - Unspecified address (::)
/// - Loopback address (::1)
/// - Link-local addresses (fe80::/10)
/// - Multicast addresses (ff00::/8)
/// - Documentation addresses (2001:db8::/32)
///
/// Note: unique-local addresses (fc00::/7) are allowed by design, since DDNS
/// is often used on private networks.
#[must_use]
pub fn is_valid_ipv6(addr: std::net::Ipv6Addr, allow_loopback: bool) -> bool {
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

/// Convenience wrapper: parses a string and validates it with [`is_valid_ipv6`].
///
/// Prefer the `Ipv6Addr`-based [`is_valid_ipv6`] on hot paths to avoid the
/// parse cost; this wrapper is for textual inputs.
#[must_use]
pub fn is_valid_ipv6_str(ip: &str, allow_loopback: bool) -> bool {
    match ip.parse::<std::net::Ipv6Addr>() {
        Ok(addr) => is_valid_ipv6(addr, allow_loopback),
        Err(_) => false,
    }
}

/// Redacts known secrets from a log message before it hits output.
///
/// Returns a borrowed slice when neither secret appears (the common case):
/// zero allocation, zero rewrite. Only lines that actually contain a secret
/// pay for copying.
pub fn redact_secrets<'a>(
    message: &'a str,
    api_token: &str,
    zone_id: &str,
) -> std::borrow::Cow<'a, str> {
    let has_token = !api_token.is_empty() && message.contains(api_token);
    let has_zone = !zone_id.is_empty() && message.contains(zone_id);
    if !has_token && !has_zone {
        return std::borrow::Cow::Borrowed(message);
    }

    // Note: a hand-rolled single-pass rewrite of this (earliest-match walk,
    // one allocation) was benchmarked and REJECTED — it measured ~204 ns vs
    // ~196 ns for these two std `replace` calls on typical log lines. At
    // this string length, std's optimized replace wins. Keep the simple form.
    let mut sanitized = message.to_string();
    if has_token {
        sanitized = sanitized.replace(api_token, "***REDACTED***");
    }
    if has_zone {
        sanitized = sanitized.replace(zone_id, "***REDACTED***");
    }

    std::borrow::Cow::Owned(sanitized)
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
        assert!(is_valid_ipv6_str("2606:4700:4700::1111", false));
        assert!(is_valid_ipv6_str("2001:4860:4860::8888", false));
        assert!(is_valid_ipv6_str("2a00:1450:4001:81b::200e", false));

        // Unique-local addresses are allowed
        assert!(is_valid_ipv6_str("fc00::1", false));
        assert!(is_valid_ipv6_str("fd12:3456:789a::1", false));
        // Reserved addresses that should be rejected
        assert!(!is_valid_ipv6_str("::", false)); // Unspecified
        assert!(!is_valid_ipv6_str("::1", false)); // Loopback (default reject)
        assert!(!is_valid_ipv6_str("fe80::1", false)); // Link-local
        assert!(!is_valid_ipv6_str("fe80::dead:beef", false)); // Link-local
        assert!(!is_valid_ipv6_str("ff00::1", false)); // Multicast
        assert!(!is_valid_ipv6_str("ff02::1", false)); // Multicast
        assert!(!is_valid_ipv6_str("2001:db8::1", false)); // Documentation
        assert!(!is_valid_ipv6_str("2001:0db8::1", false)); // Documentation

        // Invalid formats
        assert!(!is_valid_ipv6_str("192.168.1.1", false)); // IPv4
        assert!(!is_valid_ipv6_str("invalid", false));
        assert!(!is_valid_ipv6_str("", false));
        assert!(!is_valid_ipv6_str("2001:db8::g", false));
    }

    #[test]
    fn test_is_valid_ipv6_allow_loopback() {
        assert!(is_valid_ipv6_str("::1", true));
        assert!(!is_valid_ipv6_str("::", true));
    }

    // Additional edge case tests for IPv6 validation

    #[test]
    fn test_ipv6_compression_variants() {
        // Valid compressed addresses (not in documentation range)
        assert!(is_valid_ipv6_str("2001:4860::8888", false));
        assert!(is_valid_ipv6_str("2001:4860:0:0:0:0:0:8888", false));
        assert!(is_valid_ipv6_str("2001::", false));
    }

    #[test]
    fn test_ipv6_with_port() {
        // IPv6 addresses with port notation should be rejected
        assert!(!is_valid_ipv6_str("[2001:db8::1]:8080", false));
    }

    #[test]
    fn test_ipv6_zone_id() {
        // IPv6 addresses with zone ID should be rejected
        assert!(!is_valid_ipv6_str("fe80::1%eth0", false));
    }

    #[test]
    fn test_ipv6_max_compression() {
        assert!(!is_valid_ipv6_str("::", false)); // Fully compressed - unspecified, rejected
        assert!(is_valid_ipv6_str("2001::", false)); // Trailing zeroes
        assert!(!is_valid_ipv6_str("::1", false)); // Leading zeroes - loopback, rejected
    }

    #[test]
    fn test_ipv6_boundary_values() {
        // Minimum valid IPv6 (all zeros)
        assert!(!is_valid_ipv6_str("::", false)); // Unspecified, rejected
        // Maximum valid IPv6 (all F's)
        assert!(!is_valid_ipv6_str(
            "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            false
        )); // Multicast-ish
    }

    #[test]
    fn test_ipv6_partial_compression() {
        assert!(is_valid_ipv6_str("2001:4860:0:0:1:0:0:1", false));
        assert!(is_valid_ipv6_str("2001:4860::1:0:0:1", false));
        assert!(is_valid_ipv6_str("2001:4860:0:0:1::1", false));
    }

    #[test]
    fn test_ipv6_multiple_double_colon() {
        // Only one :: is allowed
        assert!(!is_valid_ipv6_str("2001::db8::1", false));
    }

    #[test]
    fn test_ipv6_leading_trailing_colons() {
        assert!(!is_valid_ipv6_str(":2001:db8::1", false));
        assert!(!is_valid_ipv6_str("2001:db8::1:", false));
    }

    #[test]
    fn test_ipv6_invalid_characters() {
        assert!(!is_valid_ipv6_str("2001:db8::g", false));
        assert!(!is_valid_ipv6_str("2001:db8::1.2.3.4", false));
        assert!(!is_valid_ipv6_str("2001:db8::12345", false)); // Too many digits
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
        assert!(validate_record_name("**.example.com").is_err()); // Double wildcard
        assert!(validate_record_name("a*.example.com").is_err()); // Partial wildcard
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

    #[test]
    fn test_redact_secrets_borrowed_fast_path() {
        let msg = "IPv6 change detected: 2001:db8::1";
        match redact_secrets(msg, "tok123", "zone9") {
            std::borrow::Cow::Borrowed(s) => assert_eq!(s, msg),
            _ => panic!("expected borrowed"),
        }
    }

    #[test]
    fn test_redact_secrets_rewrites() {
        assert_eq!(
            redact_secrets("token=abcd1234", "abcd1234", "zone9").as_ref(),
            "token=***REDACTED***"
        );
        assert_eq!(
            redact_secrets("z=abc123 t=xyz789", "xyz789", "abc123").as_ref(),
            "z=***REDACTED*** t=***REDACTED***"
        );
    }

    #[test]
    fn test_redact_secrets_empty_secret_is_noop() {
        let msg = "nothing to see";
        match redact_secrets(msg, "", "zone9") {
            std::borrow::Cow::Borrowed(s) => assert_eq!(s, msg),
            _ => panic!("expected borrowed"),
        }
    }

    /// Manual CPU benchmarks. Run with:
    /// `cargo test --release -p textops -- --ignored bench --nocapture`
    mod cpu_bench {
        use super::*;
        use std::hint::black_box;
        use std::net::Ipv6Addr;
        use std::time::Instant;

        fn measure<F: FnMut()>(
            mut f: F,
            iters: u32,
            passes: u32,
        ) -> (std::time::Duration, std::time::Duration) {
            // Warmup
            for _ in 0..iters {
                f();
            }
            let mut mins = Vec::new();
            for _ in 0..passes {
                let start = Instant::now();
                for _ in 0..iters {
                    f();
                }
                mins.push(start.elapsed() / iters);
            }
            mins.sort();
            (mins[0], mins[mins.len() / 2])
        }

        #[test]
        #[ignore = "manual benchmark"]
        fn bench_is_valid_ipv6() {
            let global: Ipv6Addr = "2001:db8:1:2:3:4:5:6".parse().unwrap();
            let linklocal: Ipv6Addr = "fe80::1".parse().unwrap();
            let (min, med) = measure(
                || {
                    black_box(is_valid_ipv6(black_box(global), false));
                    black_box(is_valid_ipv6(black_box(linklocal), false));
                },
                100_000,
                7,
            );
            println!("bench_is_valid_ipv6: min {min:?}/pair median {med:?}");
        }

        #[test]
        #[ignore = "manual benchmark"]
        fn bench_validate_record_name() {
            let (min, med) = measure(
                || {
                    black_box(validate_record_name(black_box("home.example.com")).is_ok());
                },
                50_000,
                7,
            );
            println!("bench_validate_record_name: min {min:?}/call median {med:?}");
        }

        #[test]
        #[ignore = "manual benchmark"]
        fn bench_redact_secrets() {
            let plain = "IPv6 change detected: 2001:db8::1";
            let secret = "token=abcd1234efgh5678 zone=abc123";
            let (min, med) = measure(
                || {
                    black_box(redact_secrets(
                        black_box(plain),
                        black_box("tok123"),
                        black_box("zone9"),
                    ));
                },
                50_000,
                7,
            );
            println!("bench_redact_no_match: min {min:?} median {med:?}");

            let (min, med) = measure(
                || {
                    black_box(redact_secrets(
                        black_box(secret),
                        black_box("abcd1234efgh5678"),
                        black_box("abc123"),
                    ));
                },
                50_000,
                7,
            );
            println!("bench_redact_rewrite: min {min:?} median {med:?}");
        }
    }
}
