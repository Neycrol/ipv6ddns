//! Cloudflare API client for DNS operations
//!
//! Uses reqwest with rustls for HTTP requests.

use std::fmt;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use urlencoding::encode;

const API_BASE: &str = "https://api.cloudflare.com/client/v4";

//==============================================================================
// Types
//==============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub record_type: String,
    pub name: String,
    pub content: String,
    pub proxied: bool,
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

pub struct CloudflareClient {
    api_token: String,
    client: reqwest::Client,
}

impl CloudflareClient {
    pub fn new(api_token: &str, timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .user_agent("ipv6ddns/1.0")
            .build()
            .context("build reqwest client")?;

        Ok(Self {
            api_token: api_token.to_string(),
            client,
        })
    }

    /// Get a DNS record by name (AAAA only)
    pub async fn get_records(
        &self,
        zone_id: &str,
        record_name: &str,
    ) -> Result<Vec<DnsRecord>> {
        let record_name = encode(record_name);
        let url = format!(
            "{}/zones/{}/dns_records?name={}&type=AAAA",
            API_BASE, zone_id, record_name
        );

        debug!("GET {}", url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .context("GET request failed")?;
        let status = resp.status();
        let body: ApiResponse<Vec<DnsRecord>> =
            resp.json().await.context("Failed to parse response")?;

        if !body.success {
            if status == StatusCode::TOO_MANY_REQUESTS {
                bail!("Rate limited by Cloudflare");
            }
            if status.is_server_error() {
                bail!("Cloudflare server error: {}", status.as_u16());
            }
            bail!("API error: {}", body.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "));
        }

        Ok(body.result.unwrap_or_default())
    }

    /// Create or update an AAAA record
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
                    bail!("Multiple AAAA records found for {}. Refusing to update.", record_name);
                }
                if let Some(record) = records.into_iter().next() {
                    if record.content == ipv6_addr {
                        debug!("Record already matches {}", ipv6_addr);
                        return Ok(record);
                    }
                    self.update_record(zone_id, &record.id, record_name, ipv6_addr).await
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
                    self.update_record(zone_id, &record.id, record_name, ipv6_addr).await
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
                    let updated = self.update_record(zone_id, &record.id, record_name, ipv6_addr).await?;
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
        #[derive(Serialize)]
        struct Payload {
            #[serde(rename = "type")]
            rt: &'static str,
            name: String,
            content: String,
            ttl: u64,
            proxied: bool,
        }

        let url = format!("{}/zones/{}/dns_records", API_BASE, zone_id);
        let payload = serde_json::to_string(&Payload {
            rt: "AAAA",
            name: record_name.to_string(),
            content: ipv6_addr.to_string(),
            ttl: 1,
            proxied: false,
        })?;

        debug!("POST {}", url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_token)
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .context("POST request failed")?;
        let status = resp.status();
        let body: ApiResponse<DnsRecord> = resp.json().await.context("Failed to parse response")?;

        if !body.success {
            if status == StatusCode::TOO_MANY_REQUESTS {
                bail!("Rate limited by Cloudflare");
            }
            if status.is_server_error() {
                bail!("Cloudflare server error: {}", status.as_u16());
            }
            bail!("Create failed: {}", body.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "));
        }

        body.result.context("API returned success but no result")
    }

    /// Update an existing AAAA record
    async fn update_record(
        &self,
        zone_id: &str,
        record_id: &str,
        record_name: &str,
        ipv6_addr: &str,
    ) -> Result<DnsRecord> {
        #[derive(Serialize)]
        struct Payload {
            #[serde(rename = "type")]
            rt: &'static str,
            name: String,
            content: String,
            ttl: u64,
            proxied: bool,
        }

        let url = format!("{}/zones/{}/dns_records/{}", API_BASE, zone_id, record_id);
        let payload = serde_json::to_string(&Payload {
            rt: "AAAA",
            name: record_name.to_string(),
            content: ipv6_addr.to_string(),
            ttl: 1,
            proxied: false,
        })?;

        debug!("PUT {}", url);
        let resp = self
            .client
            .put(&url)
            .bearer_auth(&self.api_token)
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .context("PUT request failed")?;
        let status = resp.status();
        let body: ApiResponse<DnsRecord> = resp.json().await.context("Failed to parse response")?;

        if !body.success {
            if status == StatusCode::TOO_MANY_REQUESTS {
                bail!("Rate limited by Cloudflare");
            }
            if status.is_server_error() {
                bail!("Cloudflare server error: {}", status.as_u16());
            }
            bail!("Update failed: {}", body.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "));
        }

        body.result.context("API returned success but no result")
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MultiRecordPolicy {
    Error,
    UpdateFirst,
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
}
