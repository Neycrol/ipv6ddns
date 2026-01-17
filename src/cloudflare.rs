//! Cloudflare API client for DNS operations
//!
//! This module provides a client for interacting with the Cloudflare API to manage
//! DNS records, specifically AAAA records for IPv6 addresses. It uses reqwest with
//! rustls for HTTP requests.
//!
//! # Features
//!
//! - Returns detailed errors on rate limiting (backoff is handled by the daemon)
//! - Support for multiple AAAA records with configurable policies
//! - Automatic record creation (upsert operation)
//! - Comprehensive error handling with detailed context
//!
//! # Usage
//!
//! ```text
//! use ipv6ddns::cloudflare::{CloudflareClient, MultiRecordPolicy};
//! use std::time::Duration;
//!
//! let client = CloudflareClient::new("your-api-token", Duration::from_secs(30))?;
//! let record = client.upsert_aaaa_record(
//!     "zone-id",
//!     "example.com",
//!     "2001:db8::1",
//!     MultiRecordPolicy::Error
//! ).await?;
//! ```
//!
//! # Error Handling
//!
//! The client returns detailed errors for:
//! - Authentication failures (401 errors)
//! - Rate limiting (429 errors)
//! - Server errors (5xx errors)
//! - Invalid input or malformed requests
//!
//! # Rate Limiting
//!
//! Cloudflare has rate limits on API requests. This client reports rate-limit
//! errors; exponential backoff is handled by the daemon.

use std::fmt;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use urlencoding::encode;
use zeroize::ZeroizeOnDrop;

/// Cloudflare API base URL
const API_BASE: &str = "https://api.cloudflare.com/client/v4";
/// User agent string for API requests
const USER_AGENT: &str = "ipv6ddns/1.0";
/// DNS record type for IPv6 addresses
const DNS_RECORD_TYPE_AAAA: &str = "AAAA";
/// TTL value for automatic TTL (1 second)
const DNS_TTL_AUTO: u64 = 1;
/// HTTP status code for rate limiting
const HTTP_STATUS_TOO_MANY_REQUESTS: u16 = 429;

//==============================================================================
// Types
//==============================================================================

/// Represents a DNS record from Cloudflare API
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
    /// Whether Cloudflare proxy is enabled for this record
    pub proxied: bool,
    /// Time-to-live value in seconds (1 = automatic)
    pub ttl: u64,
}

impl fmt::Display for DnsRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DNS {} {} -> {} (TTL: {}, Proxied: {})",
            self.record_type, self.name, self.content, self.ttl, self.proxied
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    errors: Vec<ApiError>,
    messages: Vec<String>,
    result: Option<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiError {
    code: u64,
    message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

//==============================================================================
// Client
//==============================================================================

/// Cloudflare API client for DNS operations
///
/// This client provides methods to interact with the Cloudflare API for
/// managing DNS records, specifically AAAA records for IPv6 addresses.
/// It uses reqwest with rustls for HTTP requests. The API token is wrapped
/// in `Zeroizing` to ensure it is securely cleared from memory when dropped.
#[derive(ZeroizeOnDrop)]
pub struct CloudflareClient {
    /// Cloudflare API token with DNS edit permissions
    #[zeroize(skip)]
    api_token: zeroize::Zeroizing<String>,
    /// HTTP client for making requests
    #[zeroize(skip)]
    client: reqwest::Client,
}

impl CloudflareClient {
    /// Builds the JSON payload for an AAAA record
    ///
    /// # Arguments
    ///
    /// * `record_name` - The DNS record name
    /// * `ipv6_addr` - The IPv6 address
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the serialized JSON payload or an error
    fn build_aaaa_payload(record_name: &str, ipv6_addr: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Payload {
            #[serde(rename = "type")]
            rt: &'static str,
            name: String,
            content: String,
            ttl: u64,
            proxied: bool,
        }

        serde_json::to_string(&Payload {
            rt: DNS_RECORD_TYPE_AAAA,
            name: record_name.to_string(),
            content: ipv6_addr.to_string(),
            ttl: DNS_TTL_AUTO,
            proxied: false,
        })
        .context("Failed to serialize AAAA payload")
    }

    /// Creates a new Cloudflare API client
    ///
    /// # Arguments
    ///
    /// * `api_token` - Cloudflare API token with DNS edit permissions
    /// * `timeout` - HTTP request timeout duration
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the client or an error if client creation fails
    pub fn new(api_token: &str, timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .user_agent(USER_AGENT)
            .build()
            .context("build reqwest client")?;

        Ok(Self {
            api_token: zeroize::Zeroizing::new(api_token.to_string()),
            client,
        })
    }

    /// Helper function to handle API response errors
    ///
    /// # Arguments
    ///
    /// * `status` - The HTTP status code
    /// * `body` - The API response body
    /// * `context` - Context message for the error
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the response was successful, otherwise returns an error
    fn handle_api_response<T>(
        &self,
        status: StatusCode,
        body: &ApiResponse<T>,
        context: &str,
    ) -> Result<()> {
        if !body.success {
            let status_code = status.as_u16();
            match status_code {
                401 => {
                    bail!(
                        "API error: Authentication failed (401): {}. \
                         Please verify your API token has 'Zone - DNS - Edit' permissions at \
                         https://dash.cloudflare.com/profile/api-tokens",
                        context
                    );
                }
                403 => {
                    bail!(
                        "API error: Permission denied (403): {}. \
                         Please verify your API token has 'Zone - DNS - Edit' permissions for zone '{}'",
                        context,
                        body.errors
                            .first()
                            .map(|e| e.message.clone())
                            .unwrap_or_else(|| "unknown".to_string())
                    );
                }
                HTTP_STATUS_TOO_MANY_REQUESTS => {
                    bail!(
                        "Rate limited by Cloudflare (429): {}. \
                         The daemon will automatically retry with exponential backoff. \
                         Please wait before retrying manually.",
                        context
                    );
                }
                code if (500..600).contains(&code) => {
                    bail!(
                        "Cloudflare server error ({}): {}. \
                         This is a temporary issue on Cloudflare's side. \
                         The daemon will automatically retry with exponential backoff.",
                        code,
                        context
                    );
                }
                _ => {
                    bail!(
                        "API error ({}): {}: {}. \
                         For more information, see https://developers.cloudflare.com/api/troubleshooting/",
                        status_code,
                        context,
                        body.errors
                            .iter()
                            .map(|e| e.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }
        Ok(())
    }

    /// Retrieves all AAAA records for a given record name in a zone
    ///
    /// # Arguments
    ///
    /// * `zone_id` - The Cloudflare zone ID
    /// * `record_name` - The DNS record name to query
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing a vector of `DnsRecord` objects or an error
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The HTTP request fails
    /// - The API returns an error response
    /// - Rate limit is exceeded (429 status)
    /// - Server error occurs (5xx status)
    pub async fn get_records(&self, zone_id: &str, record_name: &str) -> Result<Vec<DnsRecord>> {
        let record_name = encode(record_name);
        let url = format!(
            "{}/zones/{}/dns_records?name={}&type=AAAA",
            API_BASE, zone_id, record_name
        );

        debug!("GET {} (record: {})", url, record_name);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(self.api_token.as_str())
            .send()
            .await
            .with_context(|| {
                format!(
                    "GET request failed for record '{}' in zone '{}'",
                    record_name, zone_id
                )
            })?;
        let status = resp.status();
        let body: ApiResponse<Vec<DnsRecord>> = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse response for record '{}'", record_name))?;

        let ctx = format!("GET record '{}' in zone '{}'", record_name, zone_id);
        self.handle_api_response(status, &body, &ctx)?;

        Ok(body.result.unwrap_or_default())
    }

    /// Creates or updates an AAAA record with the given IPv6 address
    ///
    /// This method implements an upsert operation: it will create a new record
    /// if none exists, or update existing records according to the specified policy.
    ///
    /// # Arguments
    ///
    /// * `zone_id` - The Cloudflare zone ID
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
    /// - Rate limit is exceeded (429 status)
    /// - Server error occurs (5xx status)
    pub async fn upsert_aaaa_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: &str,
        policy: MultiRecordPolicy,
    ) -> Result<DnsRecord> {
        let records = self.get_records(zone_id, record_name).await?;
        match policy {
            MultiRecordPolicy::Error => {
                if records.len() > 1 {
                    warn!("Multiple AAAA records found for {}", record_name);
                    bail!(
                        "Multiple AAAA records found for {}. Refusing to update.",
                        record_name
                    );
                }
                if let Some(record) = records.into_iter().next() {
                    if record.content == ipv6_addr {
                        debug!("Record already matches {}", ipv6_addr);
                        return Ok(record);
                    }
                    self.update_record(zone_id, &record.id, record_name, ipv6_addr)
                        .await
                } else {
                    self.create_record(zone_id, record_name, ipv6_addr).await
                }
            }
            MultiRecordPolicy::UpdateFirst => {
                if let Some(record) = records.into_iter().next() {
                    if record.content == ipv6_addr {
                        debug!("Record already matches {}", ipv6_addr);
                        return Ok(record);
                    }
                    self.update_record(zone_id, &record.id, record_name, ipv6_addr)
                        .await
                } else {
                    self.create_record(zone_id, record_name, ipv6_addr).await
                }
            }
            MultiRecordPolicy::UpdateAll => {
                if records.is_empty() {
                    return self.create_record(zone_id, record_name, ipv6_addr).await;
                }
                let mut first = None;
                for record in records {
                    if record.content == ipv6_addr {
                        if first.is_none() {
                            first = Some(record);
                        }
                        continue;
                    }
                    let updated = self
                        .update_record(zone_id, &record.id, record_name, ipv6_addr)
                        .await?;
                    if first.is_none() {
                        first = Some(updated);
                    }
                }
                Ok(first.unwrap())
            }
        }
    }

    /// Create a new AAAA record
    async fn create_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: &str,
    ) -> Result<DnsRecord> {
        let url = format!("{}/zones/{}/dns_records", API_BASE, zone_id);
        let payload = Self::build_aaaa_payload(record_name, ipv6_addr)?;

        debug!("POST {} (record: {}, ip: {})", url, record_name, ipv6_addr);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(self.api_token.as_str())
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .with_context(|| {
                format!(
                    "POST request failed to create record '{}' in zone '{}'",
                    record_name, zone_id
                )
            })?;
        let status = resp.status();
        let body: ApiResponse<DnsRecord> = resp.json().await.with_context(|| {
            format!(
                "Failed to parse create response for record '{}'",
                record_name
            )
        })?;

        let ctx = format!("Create record '{}' in zone '{}'", record_name, zone_id);
        self.handle_api_response(status, &body, &ctx)?;

        body.result.with_context(|| {
            format!(
                "API returned success but no result for record '{}'",
                record_name
            )
        })
    }

    /// Update an existing AAAA record
    async fn update_record(
        &self,
        zone_id: &str,
        record_id: &str,
        record_name: &str,
        ipv6_addr: &str,
    ) -> Result<DnsRecord> {
        let url = format!("{}/zones/{}/dns_records/{}", API_BASE, zone_id, record_id);
        let payload = Self::build_aaaa_payload(record_name, ipv6_addr)?;

        debug!(
            "PUT {} (record: {}, id: {}, ip: {})",
            url, record_name, record_id, ipv6_addr
        );
        let resp = self
            .client
            .put(&url)
            .bearer_auth(self.api_token.as_str())
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .with_context(|| {
                format!(
                    "PUT request failed to update record '{}' (ID: {}) in zone '{}'",
                    record_name, record_id, zone_id
                )
            })?;
        let status = resp.status();
        let body: ApiResponse<DnsRecord> = resp.json().await.with_context(|| {
            format!(
                "Failed to parse update response for record '{}' (ID: {})",
                record_name, record_id
            )
        })?;

        let ctx = format!(
            "Update record '{}' (ID: {}) in zone '{}'",
            record_name, record_id, zone_id
        );
        self.handle_api_response(status, &body, &ctx)?;

        body.result.with_context(|| {
            format!(
                "API returned success but no result for record '{}' (ID: {})",
                record_name, record_id
            )
        })
    }
}

/// Policy for handling multiple AAAA records with the same name
///
/// When multiple AAAA records exist for a given record name, this enum
/// defines how the client should handle the update operation.
#[derive(Debug, Clone, Copy)]
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
    /// Update all matching AAAA records
    ///
    /// This option will update all AAAA records with the given name.
    /// Be careful as this may affect multiple records.
    UpdateAll,
}

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_record_display() {
        let record = DnsRecord {
            id: "test123".to_string(),
            record_type: "AAAA".to_string(),
            name: "home.xishao.top".to_string(),
            content: "2001:db8::1".to_string(),
            proxied: false,
            ttl: 1,
        };

        let s = format!("{}", record);
        assert!(s.contains("home.xishao.top"));
        assert!(s.contains("2001:db8::1"));
    }

    #[test]
    fn test_api_response_parsing() {
        let json = r#"{
            "success": true,
            "errors": [],
            "messages": [],
            "result": {
                "id": "abc123",
                "type": "AAAA",
                "name": "test.example.com",
                "content": "::1",
                "proxied": false,
                "ttl": 1
            }
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.errors.is_empty());
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_api_error_display() {
        let err = ApiError {
            code: 6003,
            message: "Invalid request headers".to_string(),
        };
        assert_eq!(format!("{}", err), "[6003] Invalid request headers");
    }

    #[test]
    fn test_api_response_with_errors() {
        let json = r#"{
            "success": false,
            "errors": [
                {
                    "code": 1000,
                    "message": "Invalid API token"
                }
            ],
            "messages": [],
            "result": null
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.errors.len(), 1);
        assert_eq!(resp.errors[0].code, 1000);
        assert_eq!(resp.errors[0].message, "Invalid API token");
    }

    #[test]
    fn test_api_response_multiple_errors() {
        let json = r#"{
            "success": false,
            "errors": [
                {
                    "code": 1000,
                    "message": "Invalid API token"
                },
                {
                    "code": 1003,
                    "message": "Invalid or missing zone id"
                }
            ],
            "messages": [],
            "result": null
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.errors.len(), 2);
    }

    #[test]
    fn test_api_response_with_messages() {
        let json = r#"{
            "success": true,
            "errors": [],
            "messages": [
                "DNS record was successfully updated"
            ],
            "result": {
                "id": "abc123",
                "type": "AAAA",
                "name": "test.example.com",
                "content": "::1",
                "proxied": false,
                "ttl": 1
            }
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(resp.messages[0], "DNS record was successfully updated");
    }

    #[test]
    fn test_api_response_array_result() {
        let json = r#"{
            "success": true,
            "errors": [],
            "messages": [],
            "result": [
                {
                    "id": "abc123",
                    "type": "AAAA",
                    "name": "test.example.com",
                    "content": "2001:db8::1",
                    "proxied": false,
                    "ttl": 1
                },
                {
                    "id": "def456",
                    "type": "AAAA",
                    "name": "test.example.com",
                    "content": "2001:db8::2",
                    "proxied": false,
                    "ttl": 1
                }
            ]
        }"#;

        let resp: ApiResponse<Vec<DnsRecord>> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.result.is_some());
        assert_eq!(resp.result.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_dns_record_with_proxy() {
        let json = r#"{
            "id": "abc123",
            "type": "AAAA",
            "name": "test.example.com",
            "content": "2001:db8::1",
            "proxied": true,
            "ttl": 1
        }"#;

        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert!(record.proxied);
    }

    #[test]
    fn test_dns_record_with_custom_ttl() {
        let json = r#"{
            "id": "abc123",
            "type": "AAAA",
            "name": "test.example.com",
            "content": "2001:db8::1",
            "proxied": false,
            "ttl": 3600
        }"#;

        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.ttl, 3600);
    }

    #[test]
    fn test_api_error_zero_code() {
        let err = ApiError {
            code: 0,
            message: "Unknown error".to_string(),
        };
        assert_eq!(format!("{}", err), "[0] Unknown error");
    }

    #[test]
    fn test_api_response_empty_result() {
        let json = r#"{
            "success": true,
            "errors": [],
            "messages": [],
            "result": []
        }"#;

        let resp: ApiResponse<Vec<DnsRecord>> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.result.is_some());
        assert!(resp.result.unwrap().is_empty());
    }
}
