//! DNS provider abstraction layer
//!
//! This module defines a trait for DNS provider implementations, allowing
//! ipv6ddns to support multiple DNS providers beyond Cloudflare.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

//==============================================================================
// Types
//==============================================================================

/// Represents a DNS record from any provider
///
/// This struct contains the essential fields for a DNS record, including
/// its ID, type, name, content, proxy status, and TTL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecord {
    /// The unique identifier for this DNS record
    pub id: String,
    /// The type of DNS record (e.g., "AAAA" for IPv6)
    #[serde(rename = "type")]
    pub record_type: String,
    /// The domain name for this record
    pub name: String,
    /// The IP address or other content of the record
    pub content: String,
    /// Whether proxy is enabled for this record (provider-specific)
    pub proxied: bool,
    /// Time-to-live value in seconds
    pub ttl: u64,
}

/// Policy for handling multiple records with the same name
///
/// When multiple records exist for a given record name, this enum
/// defines how the provider should handle the update operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiRecordPolicy {
    /// Refuse to update if multiple records exist (default)
    ///
    /// This is the safest option as it prevents accidental updates to
    /// unintended records. The operation will fail with an error.
    Error,
    /// Update only the first record found
    ///
    /// This option is useful when you want to update a single record
    /// but don't care which one is updated.
    UpdateFirst,
    /// Update all matching records
    ///
    /// This option will update all records with the given name.
    /// Be careful as this may affect multiple records.
    UpdateAll,
}

//==============================================================================
// Trait
//==============================================================================

/// DNS provider trait for managing DNS records
///
/// This trait defines the interface that all DNS provider implementations
/// must support. It allows ipv6ddns to work with multiple DNS providers
/// through a common API.
#[async_trait]
pub trait DnsProvider: Send + Sync {
    /// Creates or updates an AAAA record with the given IPv6 address
    ///
    /// This method implements an upsert operation: it will create a new record
    /// if none exists, or update existing records according to the specified policy.
    ///
    /// # Arguments
    ///
    /// * `zone_id` - The zone ID for the domain (provider-specific)
    /// * `record_name` - The DNS record name
    /// * `ipv6_addr` - The IPv6 address to set
    /// * `policy` - The policy for handling multiple records
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the created or updated `DnsRecord` or an error
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Multiple records exist and policy is `Error`
    /// - The HTTP request fails
    /// - The API returns an error response
    /// - Rate limit is exceeded
    /// - Server error occurs
    async fn upsert_aaaa_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: &str,
        policy: MultiRecordPolicy,
    ) -> anyhow::Result<DnsRecord>;

    // Future providers can add lookup APIs as needed; keep the trait minimal.
}

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_record_equality() {
        let record1 = DnsRecord {
            id: "abc123".to_string(),
            record_type: "AAAA".to_string(),
            name: "test.example.com".to_string(),
            content: "2001:db8::1".to_string(),
            proxied: false,
            ttl: 1,
        };

        let record2 = DnsRecord {
            id: "abc123".to_string(),
            record_type: "AAAA".to_string(),
            name: "test.example.com".to_string(),
            content: "2001:db8::1".to_string(),
            proxied: false,
            ttl: 1,
        };

        assert_eq!(record1, record2);
    }

    #[test]
    fn test_dns_record_inequality() {
        let record1 = DnsRecord {
            id: "abc123".to_string(),
            record_type: "AAAA".to_string(),
            name: "test.example.com".to_string(),
            content: "2001:db8::1".to_string(),
            proxied: false,
            ttl: 1,
        };

        let record2 = DnsRecord {
            id: "def456".to_string(),
            record_type: "AAAA".to_string(),
            name: "test.example.com".to_string(),
            content: "2001:db8::1".to_string(),
            proxied: false,
            ttl: 1,
        };

        assert_ne!(record1, record2);
    }

    #[test]
    fn test_multi_record_policy_variants() {
        let policies = [
            MultiRecordPolicy::Error,
            MultiRecordPolicy::UpdateFirst,
            MultiRecordPolicy::UpdateAll,
        ];

        assert_eq!(policies.len(), 3);
        assert!(policies.contains(&MultiRecordPolicy::Error));
        assert!(policies.contains(&MultiRecordPolicy::UpdateFirst));
        assert!(policies.contains(&MultiRecordPolicy::UpdateAll));
    }
}
