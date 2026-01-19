//! ipv6ddns - IPv6 DDNS client for Cloudflare
//!
//! Architecture:
//! - Netlink socket for real-time IPv6 address change events (zero CPU when idle)
//! - Automatic fallback to polling on systems without netlink support
//! - Minimal state machine for record tracking
//! - Uses reqwest for HTTP (rustls)

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cloudflare;
mod config;
mod constants;
mod daemon;
mod dns_provider;
mod health;
mod metrics;
mod netlink;
mod validation;

use cloudflare::CloudflareClient;
use config::Config;
use daemon::Daemon;
use netlink::NetlinkSocket;

/// Application version
const VERSION: &str = "1.0.0";

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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config).context("Config load failed")?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(if config.verbose { "debug" } else { "info" }));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cf_client = CloudflareClient::new(config.api_token.as_str(), config.timeout)
        .context("Cloudflare client failed")?;

    let netlink = NetlinkSocket::new(Some(config.poll_interval), config.allow_loopback)
        .context("Netlink socket failed")?;

    let mut daemon = Daemon::new(config, std::sync::Arc::new(cf_client), netlink);
    daemon.run().await?;

    Ok(())
}
