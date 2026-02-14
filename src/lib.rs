//! ipv6ddns - IPv6 DDNS client for Cloudflare
//!
//! This library provides an event-driven IPv6 DDNS client for Cloudflare.
//! It monitors IPv6 address changes and automatically updates DNS records.
//!
//! # Features
//!
//! - Event-driven IPv6 address monitoring via netlink (zero CPU when idle)
//! - Automatic fallback to polling on systems without netlink
//! - Cloudflare API integration with comprehensive error handling
//! - Structured logging with tracing
//! - Configurable multi-record policies
//! - Health check server
//! - Android companion app
//!
//! # Example
//!
//! ```no_run
//! use ipv6ddns::{Config, Daemon, CloudflareClient, NetlinkSocket};
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = Config::load(None)?;
//!     let client = CloudflareClient::new(&config.api_token, config.timeout)?;
//!     let netlink = NetlinkSocket::new(Some(config.poll_interval), config.allow_loopback)?;
//!     
//!     let mut daemon = Daemon::new(config, Arc::new(client), netlink);
//!     daemon.run().await?;
//!     
//!     Ok(())
//! }
//! ```

pub mod cloudflare;
pub mod config;
pub mod constants;
pub mod daemon;
pub mod dns_provider;
pub mod health;
pub mod netlink;
pub mod signal;
pub mod validation;

// Re-export main types for convenience
pub use cloudflare::CloudflareClient;
pub use config::Config;
pub use daemon::Daemon;
pub use dns_provider::{DnsProvider, DnsRecord, MultiRecordPolicy};
pub use netlink::NetlinkSocket;

// Re-export error types
pub use anyhow::{Error, Result};
