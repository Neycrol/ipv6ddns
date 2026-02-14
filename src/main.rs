//! ipv6ddns - IPv6 DDNS client for Cloudflare
//!
//! Architecture:
//! - Netlink socket for real-time IPv6 address change events (zero CPU when idle)
//! - Automatic fallback to polling on systems without netlink support
//! - Minimal state machine for record tracking
//! - Uses a compact HTTPS client for Cloudflare API calls

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use tracing::Level;

mod cloudflare;
mod config;
mod constants;
mod daemon;
mod dns_provider;
mod health;
mod netlink;
mod signal;
mod validation;

use cloudflare::CloudflareClient;
use config::Config;
use daemon::Daemon;
use netlink::NetlinkSocket;

/// Application version
const VERSION: &str = env!("CARGO_PKG_VERSION");

//==============================================================================
// Main
//==============================================================================

#[derive(Debug, Parser)]
#[command(name = "ipv6ddns")]
#[command(version = VERSION)]
struct Args {
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config).context("Config load failed")?;

    tracing_subscriber::fmt()
        .with_max_level(resolve_log_level(config.verbose))
        .init();

    let cf_client = CloudflareClient::new(config.api_token.as_str(), config.timeout)
        .context("Cloudflare client failed")?;

    let netlink = NetlinkSocket::new(Some(config.poll_interval), config.allow_loopback)
        .context("Netlink socket failed")?;

    let mut daemon = Daemon::new(config, std::sync::Arc::new(cf_client), netlink);
    daemon.run().await?;

    Ok(())
}

fn resolve_log_level(verbose: bool) -> Level {
    if let Ok(raw) = std::env::var("RUST_LOG") {
        for token in raw.split(',') {
            let value = token.rsplit('=').next().unwrap_or(token).trim();
            let level = match value.to_ascii_lowercase().as_str() {
                "trace" => Some(Level::TRACE),
                "debug" => Some(Level::DEBUG),
                "info" => Some(Level::INFO),
                "warn" | "warning" => Some(Level::WARN),
                "error" => Some(Level::ERROR),
                _ => None,
            };
            if let Some(level) = level {
                return level;
            }
        }
    }
    if verbose {
        Level::DEBUG
    } else {
        Level::INFO
    }
}
