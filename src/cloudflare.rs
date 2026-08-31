//! Cloudflare API client for DNS operations
//!
//! This module provides a client for interacting with the Cloudflare API to manage
//! DNS records, specifically AAAA records for IPv6 addresses. It uses reqwest
//! with rustls for async HTTP requests.
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

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::constants::{
    CLOUDFLARE_API_BASE, CLOUDFLARE_USER_AGENT, DNS_RECORD_TYPE_AAAA, DNS_TTL_AUTO,
    HTTP_POOL_IDLE_TIMEOUT_SECS, HTTP_POOL_MAX_IDLE_PER_HOST, HTTP_STATUS_FORBIDDEN,
    HTTP_STATUS_SERVER_ERROR_MAX, HTTP_STATUS_SERVER_ERROR_MIN, HTTP_STATUS_TOO_MANY_REQUESTS,
    HTTP_STATUS_UNAUTHORIZED,
};
use crate::dns_provider::{DnsProvider, DnsRecord, MultiRecordPolicy};

//==============================================================================
// Types
//==============================================================================

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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
pub struct CloudflareClient {
    /// Cloudflare API token with DNS edit permissions
    api_token: zeroize::Zeroizing<String>,
    /// HTTP client for making requests
    client: reqwest::Client,
    /// API root (overridable for testing against a local mock)
    api_base: String,
    /// Known record ID from the last successful sync.
    ///
    /// Architecture note: this eliminates the entire discovery GET stage.
    /// Previously every sync paid two HTTPS round trips — one listing request
    /// just to learn the record ID, then the actual update. The ID is stable,
    /// so after the first discovery we PATCH directly and only re-discover if
    /// Cloudflare answers 404 (record deleted externally).
    cached_record: std::sync::Mutex<Option<CachedRecordId>>,
}

/// Identity of the AAAA record managed by this daemon.
struct CachedRecordId {
    zone_id: String,
    name: String,
    id: String,
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
    fn build_aaaa_payload(record_name: &str, ipv6_addr: std::net::Ipv6Addr) -> Result<String> {
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
        Self::with_api_base(CLOUDFLARE_API_BASE.to_string(), api_token, timeout)
    }

    /// Same as [`new`] but against an explicit API base URL (test hook).
    fn with_api_base(api_base: String, api_token: &str, timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(timeout)
            .timeout(timeout)
            .user_agent(CLOUDFLARE_USER_AGENT)
            .pool_max_idle_per_host(HTTP_POOL_MAX_IDLE_PER_HOST)
            // Keep-alive window sized to IPv6-change bursts, not to reqwest's
            // default. Measured on a live instance: an idle pooled TLS
            // connection (hyper buffers + rustls session state + parked
            // connection task) pins ~2.6 MB of resident memory. Sync events
            // are typically hours apart, so holding a connection for the
            // default 90 s buys almost nothing while paying that cost every
            // time; a short window still lets a burst of address changes
            // (SLAAC/DAD flurries) share one handshake.
            .pool_idle_timeout(Duration::from_secs(HTTP_POOL_IDLE_TIMEOUT_SECS))
            .build()
            .context("build reqwest client")?;

        Ok(Self {
            api_token: zeroize::Zeroizing::new(api_token.to_string()),
            client,
            api_base,
            cached_record: std::sync::Mutex::new(None),
        })
    }

    async fn send_json_request<T>(
        &self,
        method: Method,
        url: &str,
        payload: Option<String>,
        context: &str,
    ) -> Result<(StatusCode, ApiResponse<T>)>
    where
        T: DeserializeOwned,
    {
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(self.api_token.as_str())
            .header("Accept", "application/json");

        if let Some(payload) = payload {
            request = request
                .header("Content-Type", "application/json")
                .body(payload);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("HTTP request failed for {}", context))?;

        let status = response.status();
        let response_body = response
            .bytes()
            .await
            .with_context(|| format!("Failed to read HTTP response for {}", context))?;

        let body: ApiResponse<T> = serde_json::from_slice(&response_body)
            .with_context(|| format!("Failed to parse JSON response for {}", context))?;

        Ok((status, body))
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
        status: StatusCode,
        body: &ApiResponse<T>,
        context: &str,
    ) -> Result<()> {
        if !body.success {
            let status_code = status.as_u16();
            match status_code {
                HTTP_STATUS_UNAUTHORIZED => {
                    bail!(
                        "API error: Authentication failed (401): {}. \
                         Please verify your API token has 'Zone - DNS - Edit' permissions at \
                         https://dash.cloudflare.com/profile/api-tokens",
                        context
                    );
                }
                HTTP_STATUS_FORBIDDEN => {
                    bail!(
                        "API error: Permission denied (403): {}. \
                         Please verify your API token has 'Zone - DNS - Edit' permissions. \
                         Details: {}",
                        context,
                        body.errors
                            .iter()
                            .map(|e| e.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
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
                code if (HTTP_STATUS_SERVER_ERROR_MIN..=HTTP_STATUS_SERVER_ERROR_MAX)
                    .contains(&code) =>
                {
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

    /// Create a new AAAA record
    async fn create_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: std::net::Ipv6Addr,
    ) -> Result<DnsRecord> {
        let url = format!("{}/zones/{}/dns_records", self.api_base, zone_id);
        let payload = Self::build_aaaa_payload(record_name, ipv6_addr)?;

        debug!("POST {} (record: {}, ip: {})", url, record_name, ipv6_addr);
        let context = format!("create record '{}' in zone '{}'", record_name, zone_id);
        let (status, body): (StatusCode, ApiResponse<DnsRecord>) = self
            .send_json_request(Method::POST, &url, Some(payload), &context)
            .await
            .with_context(|| {
                format!(
                    "POST request failed to create record '{}' in zone '{}'",
                    record_name, zone_id
                )
            })?;

        let ctx = format!("Create record '{}' in zone '{}'", record_name, zone_id);
        Self::handle_api_response(status, &body, &ctx)?;

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
        ipv6_addr: std::net::Ipv6Addr,
    ) -> Result<DnsRecord> {
        let url = format!(
            "{}/zones/{}/dns_records/{}",
            self.api_base, zone_id, record_id
        );
        let payload = Self::build_aaaa_payload(record_name, ipv6_addr)?;

        debug!(
            "PUT {} (record: {}, id: {}, ip: {})",
            url, record_name, record_id, ipv6_addr
        );
        let context = format!(
            "update record '{}' (ID: {}) in zone '{}'",
            record_name, record_id, zone_id
        );
        let (status, body): (StatusCode, ApiResponse<DnsRecord>) = self
            .send_json_request(Method::PUT, &url, Some(payload), &context)
            .await
            .with_context(|| {
                format!(
                    "PUT request failed to update record '{}' (ID: {}) in zone '{}'",
                    record_name, record_id, zone_id
                )
            })?;

        let ctx = format!(
            "Update record '{}' (ID: {}) in zone '{}'",
            record_name, record_id, zone_id
        );
        Self::handle_api_response(status, &body, &ctx)?;

        body.result.with_context(|| {
            format!(
                "API returned success but no result for record '{}' (ID: {})",
                record_name, record_id
            )
        })
    }
}

//==============================================================================
// DnsProvider Implementation
//==============================================================================

impl DnsProvider for CloudflareClient {
    async fn upsert_aaaa_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: std::net::Ipv6Addr,
        policy: MultiRecordPolicy,
    ) -> Result<crate::dns_provider::DnsRecord> {
        self.upsert_aaaa_record_impl(zone_id, record_name, ipv6_addr, policy)
            .await
    }

    // get_records is intentionally omitted from the trait; Cloudflare keeps
    // an internal implementation for upsert logic.
}

impl CloudflareClient {
    async fn upsert_single_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: std::net::Ipv6Addr,
        record: Option<DnsRecord>,
    ) -> Result<DnsRecord> {
        if let Some(record) = record {
            // Compare as native addresses (normalizes formatting differences
            // from the API, e.g. "2001:db8::1" vs "2001:0db8::0001").
            if record
                .content
                .parse::<std::net::Ipv6Addr>()
                .is_ok_and(|existing| existing == ipv6_addr)
            {
                debug!("Record already matches {}", ipv6_addr);
                return Ok(record);
            }
            self.update_record(zone_id, &record.id, record_name, ipv6_addr)
                .await
        } else {
            self.create_record(zone_id, record_name, ipv6_addr).await
        }
    }

    /// Looks up the cached record ID for this exact zone+name, if any.
    fn cached_record_id(&self, zone_id: &str, record_name: &str) -> Option<String> {
        let guard = self
            .cached_record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .as_ref()
            .filter(|c| c.zone_id == zone_id && c.name == record_name)
            .map(|c| c.id.clone())
    }

    fn store_cached_record_id(&self, zone_id: &str, record_name: &str, id: &str) {
        let mut guard = self
            .cached_record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(CachedRecordId {
            zone_id: zone_id.to_string(),
            name: record_name.to_string(),
            id: id.to_string(),
        });
    }

    /// Drops the cache entry if it belongs to this zone+name.
    fn clear_cached_record_id(&self, zone_id: &str, record_name: &str) {
        let mut guard = self
            .cached_record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard
            .as_ref()
            .is_some_and(|c| c.zone_id == zone_id && c.name == record_name)
        {
            *guard = None;
        }
    }

    /// Update via a known record ID. Returns `Ok(None)` when Cloudflare
    /// reports the record does not exist (HTTP 404), i.e. the cached ID is
    /// stale and the caller should fall back to discovery.
    async fn update_record_if_exists(
        &self,
        zone_id: &str,
        record_id: &str,
        record_name: &str,
        ipv6_addr: std::net::Ipv6Addr,
    ) -> Result<Option<DnsRecord>> {
        let url = format!(
            "{}/zones/{}/dns_records/{}",
            self.api_base, zone_id, record_id
        );
        let payload = Self::build_aaaa_payload(record_name, ipv6_addr)?;

        debug!(
            "PUT {} (record: {}, id: {}, ip: {})",
            url, record_name, record_id, ipv6_addr
        );
        let context = format!(
            "update record '{}' (ID: {}) in zone '{}'",
            record_name, record_id, zone_id
        );
        let (status, body): (StatusCode, ApiResponse<DnsRecord>) = self
            .send_json_request(reqwest::Method::PUT, &url, Some(payload), &context)
            .await?;

        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Self::handle_api_response(status, &body, &context)?;

        body.result.map(Some).with_context(|| {
            format!(
                "API returned success but no result for record '{}' (ID: {})",
                record_name, record_id
            )
        })
    }

    /// Internal implementation of upsert_aaaa_record
    async fn upsert_aaaa_record_impl(
        &self,
        zone_id: &str,
        record_name: &str,
        ipv6_addr: std::net::Ipv6Addr,
        policy: MultiRecordPolicy,
    ) -> Result<DnsRecord> {
        // Cached fast path, ONLY for `UpdateFirst`: its semantics ("update
        // some one record") tolerate external topology changes between syncs.
        // `Error` policy deliberately does NOT use the cache — its guarantee
        // ("refuse when duplicates exist") requires seeing the live record
        // set on every sync, so it always pays the discovery GET. Correctness
        // outranks the saved round trip.
        if matches!(policy, MultiRecordPolicy::UpdateFirst) {
            if let Some(id) = self.cached_record_id(zone_id, record_name) {
                match self
                    .update_record_if_exists(zone_id, &id, record_name, ipv6_addr)
                    .await
                {
                    Ok(Some(record)) => return Ok(record),
                    Ok(None) => {
                        warn!(
                            "Cached record ID for '{}' no longer exists; rediscovering",
                            record_name
                        );
                        self.clear_cached_record_id(zone_id, record_name);
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        let records = self.get_records_impl(zone_id, record_name).await?;
        match policy {
            MultiRecordPolicy::Error => {
                if records.len() > 1 {
                    warn!("Multiple AAAA records found for {}", record_name);
                    bail!(
                        "Multiple AAAA records found for {}. Refusing to update.",
                        record_name
                    );
                }
                self.upsert_single_record(
                    zone_id,
                    record_name,
                    ipv6_addr,
                    records.into_iter().next(),
                )
                .await
            }
            MultiRecordPolicy::UpdateFirst => {
                let result = self
                    .upsert_single_record(
                        zone_id,
                        record_name,
                        ipv6_addr,
                        records.into_iter().next(),
                    )
                    .await?;
                self.store_cached_record_id(zone_id, record_name, &result.id);
                Ok(result)
            }
            MultiRecordPolicy::UpdateAll => {
                if records.is_empty() {
                    return self.create_record(zone_id, record_name, ipv6_addr).await;
                }
                let mut first = None;
                for record in records {
                    if record
                        .content
                        .parse::<std::net::Ipv6Addr>()
                        .is_ok_and(|existing| existing == ipv6_addr)
                    {
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
                first.with_context(|| {
                    format!(
                        "No records remained after update-all for '{}' in zone '{}'",
                        record_name, zone_id
                    )
                })
            }
        }
    }

    /// Internal implementation of get_records
    async fn get_records_impl(&self, zone_id: &str, record_name: &str) -> Result<Vec<DnsRecord>> {
        // Record names are validated by `validate_record_name` (letters, digits,
        // '-', '_', '*' wildcard labels, '@' apex), so they are always safe to
        // interpolate into a URL query without additional percent-encoding.
        let url = format!(
            "{}/zones/{}/dns_records?name={}&type=AAAA",
            self.api_base, zone_id, record_name
        );

        debug!("GET {} (record: {})", url, record_name);
        let context = format!("get record '{}' in zone '{}'", record_name, zone_id);
        let (status, body): (StatusCode, ApiResponse<Vec<DnsRecord>>) = self
            .send_json_request(Method::GET, &url, None, &context)
            .await
            .with_context(|| {
                format!(
                    "GET request failed for record '{}' in zone '{}'",
                    record_name, zone_id
                )
            })?;

        let ctx = format!("GET record '{}' in zone '{}'", record_name, zone_id);
        Self::handle_api_response(status, &body, &ctx)?;

        Ok(body.result.unwrap_or_default())
    }
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
            name: "example.com".to_string(),
            content: "2001:db8::1".to_string(),
            proxied: false,
            ttl: 1,
        };

        let s = format!("{}", record);
        assert!(s.contains("example.com"));
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
                "name": "example.com",
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
                "name": "example.com",
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
                    "name": "example.com",
                    "content": "2001:db8::1",
                    "proxied": false,
                    "ttl": 1
                },
                {
                    "id": "def456",
                    "type": "AAAA",
                    "name": "example.com",
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
            "name": "example.com",
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
            "name": "example.com",
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

    // Additional edge case tests for API response parsing

    #[test]
    fn test_dns_record_full_ipv6() {
        let json = r#"{
            "id": "abc123",
            "type": "AAAA",
            "name": "example.com",
            "content": "2001:0db8:0000:0000:0000:0000:0000:0001",
            "proxied": false,
            "ttl": 1
        }"#;

        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.content, "2001:0db8:0000:0000:0000:0000:0000:0001");
    }

    #[test]
    fn test_dns_record_compressed_ipv6() {
        let json = r#"{
            "id": "abc123",
            "type": "AAAA",
            "name": "example.com",
            "content": "2001:db8::1",
            "proxied": false,
            "ttl": 1
        }"#;

        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.content, "2001:db8::1");
    }

    #[test]
    fn test_dns_record_min_ttl() {
        let json = r#"{
            "id": "abc123",
            "type": "AAAA",
            "name": "example.com",
            "content": "2001:db8::1",
            "proxied": false,
            "ttl": 1
        }"#;

        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.ttl, 1);
    }

    #[test]
    fn test_dns_record_max_ttl() {
        let json = r#"{
            "id": "abc123",
            "type": "AAAA",
            "name": "example.com",
            "content": "2001:db8::1",
            "proxied": false,
            "ttl": 86400
        }"#;

        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.ttl, 86400);
    }

    #[test]
    fn test_api_response_with_large_error_code() {
        let json = r#"{
            "success": false,
            "errors": [
                {
                    "code": 9999,
                    "message": "Unknown error code"
                }
            ],
            "messages": [],
            "result": null
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.errors[0].code, 9999);
    }

    #[test]
    fn test_api_response_with_empty_error_message() {
        let json = r#"{
            "success": false,
            "errors": [
                {
                    "code": 1000,
                    "message": ""
                }
            ],
            "messages": [],
            "result": null
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.errors[0].message, "");
    }

    #[test]
    fn test_api_response_with_special_characters_in_message() {
        let json = r#"{
            "success": false,
            "errors": [
                {
                    "code": 1000,
                    "message": "Error: \"Invalid token\" - please check your API key"
                }
            ],
            "messages": [],
            "result": null
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert!(resp.errors[0].message.contains("\""));
    }

    #[test]
    fn test_api_response_with_unicode_in_message() {
        let json = r#"{
            "success": false,
            "errors": [
                {
                    "code": 1000,
                    "message": "错误: 无效的令牌"
                }
            ],
            "messages": [],
            "result": null
        }"#;

        let resp: ApiResponse<DnsRecord> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert!(resp.errors[0].message.contains("错误"));
    }

    #[test]
    fn test_api_error_with_large_code() {
        let err = ApiError {
            code: 999999,
            message: "Very large error code".to_string(),
        };
        assert_eq!(format!("{}", err), "[999999] Very large error code");
    }

    #[test]
    fn test_api_error_with_large_code_deserialization() {
        let json = r#"{
            "code": 9999,
            "message": "Large error code"
        }"#;

        let err: ApiError = serde_json::from_str(json).unwrap();
        assert_eq!(err.code, 9999);
    }

    /// End-to-end verification of the record-ID cache architecture against a
    /// local mock of the Cloudflare API. Asserts the actual HTTP request
    /// sequence, proving the discovery GET stage is eliminated after the
    /// first sync and recovered via 404 fallback when the record vanishes.
    mod mock_e2e {
        use super::*;
        use std::sync::Mutex as StdMutex;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        struct MockCf {
            records: StdMutex<Vec<serde_json::Value>>,
            requests: StdMutex<Vec<String>>,
            next_id: StdMutex<u64>,
        }

        const ZONE: &str = "zoneid123";
        const NAME: &str = "test.example.com";

        async fn spawn_mock(state: std::sync::Arc<MockCf>) -> std::net::SocketAddr {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((sock, _)) = listener.accept().await else {
                        return;
                    };
                    let st = state.clone();
                    tokio::spawn(async move {
                        let _ = handle_conn(sock, st).await;
                    });
                }
            });
            addr
        }

        async fn handle_conn(
            mut sock: tokio::net::TcpStream,
            st: std::sync::Arc<MockCf>,
        ) -> std::io::Result<()> {
            let (method, path, body) = read_request(&mut sock).await?;
            st.requests.lock().unwrap().push(method.clone());

            let prefix = format!("/zones/{ZONE}/dns_records");
            let (status, payload): (&str, serde_json::Value) = if method == "GET"
                && path.starts_with(&format!("{prefix}?"))
            {
                let records = st.records.lock().unwrap().clone();
                (
                    "200 OK",
                    serde_json::json!({"success": true, "errors": [], "messages": [], "result": records}),
                )
            } else if method == "POST" && path == prefix {
                let mut rec: serde_json::Value = serde_json::from_str(&body).unwrap();
                let mut nid = st.next_id.lock().unwrap();
                *nid += 1;
                rec["id"] = serde_json::json!(format!("rec-{nid}"));
                st.records.lock().unwrap().push(rec.clone());
                (
                    "200 OK",
                    serde_json::json!({"success": true, "errors": [], "messages": [], "result": rec}),
                )
            } else if (method == "PUT" || method == "PATCH")
                && path.starts_with(&format!("{prefix}/"))
            {
                let id = path.rsplit('/').next().unwrap_or("");
                let mut records = st.records.lock().unwrap();
                match records.iter_mut().find(|r| r["id"] == id) {
                    Some(rec) => {
                        let upd: serde_json::Value = serde_json::from_str(&body).unwrap();
                        if let (Some(obj), Some(new_content)) =
                            (rec.as_object_mut(), upd.get("content"))
                        {
                            obj.insert("content".into(), new_content.clone());
                            if let Some(name) = upd.get("name") {
                                obj.insert("name".into(), name.clone());
                            }
                        }
                        (
                            "200 OK",
                            serde_json::json!({"success": true, "errors": [], "messages": [], "result": rec.clone()}),
                        )
                    }
                    None => (
                        "404 Not Found",
                        serde_json::json!({"success": false, "errors": [{"code": 81044, "message": "record does not exist"}], "messages": [], "result": null}),
                    ),
                }
            } else {
                (
                    "404 Not Found",
                    serde_json::json!({"success": false, "errors": [], "messages": [], "result": null}),
                )
            };
            let body_bytes = payload.to_string();
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_bytes.len(),
                body_bytes
            );
            sock.write_all(resp.as_bytes()).await?;
            sock.shutdown().await
        }

        async fn read_request(
            sock: &mut tokio::net::TcpStream,
        ) -> std::io::Result<(String, String, String)> {
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                    let clen = head
                        .lines()
                        .find_map(|l| {
                            if l.to_ascii_lowercase().starts_with("content-length:") {
                                l.split(':').nth(1)?.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    while buf.len() < pos + 4 + clen {
                        let n = sock.read(&mut tmp).await?;
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let mut parts = head.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("").to_string();
                    let body = String::from_utf8_lossy(&buf[pos + 4..]).to_string();
                    return Ok((method, path, body));
                }
                let n = sock.read(&mut tmp).await?;
                if n == 0 {
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                buf.extend_from_slice(&tmp[..n]);
            }
        }

        fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack.windows(needle.len()).position(|w| w == needle)
        }

        async fn make_client(
            state: &std::sync::Arc<MockCf>,
        ) -> (CloudflareClient, std::net::SocketAddr) {
            let addr = spawn_mock(state.clone()).await;
            let client = CloudflareClient::with_api_base(
                format!("http://{addr}"),
                "test-token",
                Duration::from_secs(3),
            )
            .unwrap();
            (client, addr)
        }

        fn record_json(id: &str, content: &str) -> serde_json::Value {
            serde_json::json!({
                "id": id,
                "type": "AAAA",
                "name": NAME,
                "content": content,
                "proxied": false,
                "ttl": 1
            })
        }

        async fn upsert(client: &CloudflareClient, ip: &str) {
            client
                .upsert_aaaa_record(
                    ZONE,
                    NAME,
                    ip.parse().unwrap(),
                    MultiRecordPolicy::UpdateFirst,
                )
                .await
                .unwrap();
        }

        /// THE architectural assertion: sync #1 discovers (GET+PUT); every
        /// later sync PATCHes directly with NO GET stage; external deletion
        /// is recovered via one extra round trip (PUT-404, GET, PUT).
        #[tokio::test]
        async fn test_record_id_cache_cuts_discovery_stage() {
            let state = std::sync::Arc::new(MockCf {
                records: StdMutex::new(vec![record_json("rec-1", "2001:db8::1")]),
                requests: StdMutex::new(Vec::new()),
                next_id: StdMutex::new(1),
            });
            let (client, _addr) = make_client(&state).await;
            let ip2 = "2001:db8::2";
            let ip3 = "2001:db8::3";

            // Sync #1: discovery then update.
            upsert(&client, ip2).await;
            assert_eq!(*state.requests.lock().unwrap(), vec!["GET", "PUT"]);

            // Sync #2: cached ID — the GET stage is gone.
            *state.requests.lock().unwrap() = Vec::new();
            upsert(&client, ip3).await;
            assert_eq!(*state.requests.lock().unwrap(), vec!["PUT"]);

            // Sync #3: record deleted externally. Cached PUT hits 404, we
            // rediscover (GET -> empty), CREATE a fresh record.
            state.records.lock().unwrap().clear();
            *state.requests.lock().unwrap() = Vec::new();
            upsert(&client, ip2).await; // 404 on stale id
            assert_eq!(*state.requests.lock().unwrap(), vec!["PUT", "GET", "POST"]);

            // Sync #4: cache re-populated by recovery — GET-free again.
            *state.requests.lock().unwrap() = Vec::new();
            upsert(&client, ip3).await;
            assert_eq!(*state.requests.lock().unwrap(), vec!["PUT"]);
        }

        /// `Error` policy must NOT use the cache: its duplicate-refusal
        /// guarantee requires a live discovery on every sync.
        #[tokio::test]
        async fn test_error_policy_always_discovers_and_refuses_duplicates() {
            let state = std::sync::Arc::new(MockCf {
                records: StdMutex::new(vec![record_json("rec-1", "2001:db8::1")]),
                requests: StdMutex::new(Vec::new()),
                next_id: StdMutex::new(1),
            });
            let addr = spawn_mock(state.clone()).await;
            let client = CloudflareClient::with_api_base(
                format!("http://{addr}"),
                "test-token",
                Duration::from_secs(3),
            )
            .unwrap(); // Sync #1 and #2 BOTH pay discovery: no caching under Error policy.
            for ip in ["2001:db8::2", "2001:db8::3"] {
                client
                    .upsert_aaaa_record(ZONE, NAME, ip.parse().unwrap(), MultiRecordPolicy::Error)
                    .await
                    .unwrap();
            }
            assert_eq!(
                *state.requests.lock().unwrap(),
                vec!["GET", "PUT", "GET", "PUT"]
            );

            // Duplicate appears → Error policy refuses, even though it has
            // seen this record before.
            state
                .records
                .lock()
                .unwrap()
                .push(record_json("rec-dup", "2001:db8::9"));
            *state.requests.lock().unwrap() = Vec::new();
            let err = client
                .upsert_aaaa_record(
                    ZONE,
                    NAME,
                    "2001:db8::2".parse().unwrap(),
                    MultiRecordPolicy::Error,
                )
                .await
                .expect_err("duplicate must be refused");
            assert!(err.to_string().contains("Multiple AAAA records"));
            assert_eq!(*state.requests.lock().unwrap(), vec!["GET"]); // refused before any write
        }
    }
}
