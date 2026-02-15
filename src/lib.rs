//! IPv6 DDNS library
//!
//! This library provides IPv6 Dynamic DNS functionality for Cloudflare.

pub mod cloudflare;
pub mod config;
pub mod constants;
pub mod daemon;
pub mod dns_provider;
pub mod health;
pub mod netlink;
pub mod validation;

// Re-export commonly used items for convenience
pub use config::Config;
pub use daemon::{AppState, Daemon, RecordState};
pub use dns_provider::DnsProvider;
pub use netlink::{detect_global_ipv6, NetlinkEvent, NetlinkSocket};
