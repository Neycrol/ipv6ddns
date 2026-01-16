//! ipv6ddns - IPv6 DDNS client for Cloudflare
//!
//! Architecture:
//! - Netlink socket for real-time IPv6 address change events (zero CPU when idle)
//! - Automatic fallback to polling on systems without netlink support
//! - Minimal state machine for record tracking
//! - Uses reqwest for HTTP (rustls)

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

mod cloudflare;
mod netlink;
mod validation;

use cloudflare::{CloudflareClient, MultiRecordPolicy};
use netlink::{detect_global_ipv6, NetlinkEvent, NetlinkSocket};
use validation::validate_record_name;

//==============================================================================
// Config
//==============================================================================

/// Environment variable name for Cloudflare API token
const ENV_API_TOKEN: &str = "CLOUDFLARE_API_TOKEN";
/// Environment variable name for Cloudflare zone ID
const ENV_ZONE_ID: &str = "CLOUDFLARE_ZONE_ID";
/// Environment variable name for DNS record name
const ENV_RECORD_NAME: &str = "CLOUDFLARE_RECORD_NAME";
/// Environment variable name for multi-record policy
const ENV_MULTI_RECORD: &str = "CLOUDFLARE_MULTI_RECORD";

/// Default HTTP request timeout in seconds
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Default polling interval in seconds (when netlink is unavailable)
const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
/// Application version
const VERSION: &str = "1.0.0";

/// Redacts sensitive data (API tokens and zone IDs) from log messages
///
/// This function replaces occurrences of the API token and zone ID with
/// `***REDACTED***` to prevent sensitive data from appearing in logs.
///
/// # Arguments
///
/// * `message` - The message to sanitize
/// * `api_token` - The API token to redact
/// * `zone_id` - The zone ID to redact
///
/// # Returns
///
/// Returns the sanitized message with sensitive data redacted
///
/// # Examples
///
/// ```text
/// let message = "API call with token secret123 and zone zone456";
/// let redacted = redact_secrets(message, "secret123", "zone456");
/// assert!(!redacted.contains("secret123"));
/// assert!(!redacted.contains("zone456"));
/// assert!(redacted.contains("***REDACTED***"));
/// ```
fn redact_secrets(message: &str, api_token: &str, zone_id: &str) -> String {
    let mut sanitized = message.to_string();

    if !api_token.is_empty() {
        sanitized = sanitized.replace(api_token, "***REDACTED***");
    }
    if !zone_id.is_empty() {
        sanitized = sanitized.replace(zone_id, "***REDACTED***");
    }

    sanitized
}

/// Configuration for the ipv6ddns daemon
///
/// This struct holds all configuration parameters needed to run the daemon,
/// including Cloudflare API credentials, DNS record settings, and runtime options.
///
/// # Fields
///
/// - `api_token`: Cloudflare API token with DNS edit permissions
/// - `zone_id`: Cloudflare zone ID for the domain
/// - `record`: DNS record name to update (e.g., "home.example.com")
/// - `timeout`: HTTP request timeout in seconds
/// - `poll_interval`: Polling interval in seconds (fallback when netlink unavailable)
/// - `verbose`: Enable verbose logging
/// - `multi_record`: Policy for handling multiple AAAA records
///
/// # Configuration Loading Priority
///
/// Configuration is loaded from multiple sources in order of precedence:
/// 1. Environment variables (highest priority)
/// 2. Config file (`/etc/ipv6ddns/config.toml` or custom path)
/// 3. Defaults (lowest priority)
///
/// # Example
///
/// ```text
/// use ipv6ddns::Config;
/// use std::path::PathBuf;
///
/// let config = Config::load(Some(PathBuf::from("/etc/ipv6ddns/config.toml")))
///     .expect("Failed to load config");
/// println!("Monitoring record: {}", config.record);
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    /// Cloudflare API token with DNS edit permissions
    ///
    /// This token should have the `Zone:DNS:Edit` permission.
    /// It can be set via the `CLOUDFLARE_API_TOKEN` environment variable.
    pub api_token: String,
    /// Cloudflare zone ID for the domain
    ///
    /// The zone ID can be found in the Cloudflare dashboard under your domain's DNS settings.
    /// It can be set via the `CLOUDFLARE_ZONE_ID` environment variable.
    pub zone_id: String,
    /// DNS record name to update (e.g., "home.example.com")
    ///
    /// This is the full DNS record name including subdomain if applicable.
    /// It can be set via the `CLOUDFLARE_RECORD_NAME` environment variable.
    pub record: String,
    /// HTTP request timeout in seconds
    ///
    /// Default: 30 seconds
    pub timeout: Duration,
    /// Polling interval in seconds (fallback when netlink unavailable)
    ///
    /// Default: 60 seconds
    /// This is only used when netlink socket creation fails.
    pub poll_interval: Duration,
    /// Enable verbose logging
    ///
    /// Default: false
    pub verbose: bool,
    /// Policy for handling multiple AAAA records
    ///
    /// Default: `MultiRecordPolicy::Error`
    /// Can be set via the `CLOUDFLARE_MULTI_RECORD` environment variable.
    pub multi_record: MultiRecordPolicy,
}

impl Config {
    /// Loads configuration from file and environment variables
    ///
    /// This method loads configuration in the following order:
    /// 1. Loads from the specified config file (if provided and exists)
    /// 2. Overrides with environment variables (if set)
    /// 3. Validates the final configuration
    ///
    /// # Arguments
    ///
    /// * `config_path` - Optional path to a TOML config file
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the loaded `Config` or an error if:
    /// - The config file cannot be read or parsed
    /// - Required fields are missing after loading
    /// - The record name is invalid
    ///
    /// # Environment Variables
    ///
    /// The following environment variables can override config file values:
    /// - `CLOUDFLARE_API_TOKEN` - Cloudflare API token
    /// - `CLOUDFLARE_ZONE_ID` - Cloudflare zone ID
    /// - `CLOUDFLARE_RECORD_NAME` - DNS record name
    /// - `CLOUDFLARE_MULTI_RECORD` - Multi-record policy (error|first|all)
    ///
    /// # Example
    ///
    /// ```text
    /// use ipv6ddns::Config;
    /// use std::path::PathBuf;
    ///
    /// let config = Config::load(Some(PathBuf::from("/etc/ipv6ddns/config.toml")))
    ///     .expect("Failed to load config");
    /// ```
    pub fn load(config_path: Option<PathBuf>) -> Result<Self> {
        let mut config = Self::load_from_file(config_path)?;
        Self::override_with_env(&mut config)?;
        Self::validate(&config)?;
        Ok(config)
    }

    /// Loads configuration from a TOML file
    ///
    /// # Arguments
    ///
    /// * `config_path` - Optional path to a TOML config file
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the loaded `Config` with default values
    /// for any missing fields.
    fn load_from_file(config_path: Option<PathBuf>) -> Result<Self> {
        let mut api_token = String::new();
        let mut zone_id = String::new();
        let mut record = String::new();
        let mut timeout = DEFAULT_TIMEOUT_SECS;
        let mut poll_interval = DEFAULT_POLL_INTERVAL_SECS;
        let mut verbose = false;
        let mut multi_record = MultiRecordPolicy::Error;

        if let Some(path) = config_path {
            if path.exists() {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config: {}", path.display()))?;
                let toml_config: TomlConfig =
                    toml::from_str(&content).with_context(|| "Failed to parse config file")?;

                api_token = toml_config.api_token.unwrap_or_default();
                zone_id = toml_config.zone_id.unwrap_or_default();
                record = toml_config.record_name.unwrap_or_default();
                timeout = toml_config.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
                poll_interval = toml_config.poll_interval.unwrap_or(DEFAULT_POLL_INTERVAL_SECS);
                verbose = toml_config.verbose.unwrap_or(false);
                if let Some(v) = toml_config.multi_record.as_deref() {
                    multi_record = parse_multi_record(v)?;
                }
            }
        }

        Ok(Self {
            api_token,
            zone_id,
            record,
            timeout: Duration::from_secs(timeout),
            poll_interval: Duration::from_secs(poll_interval),
            verbose,
            multi_record,
        })
    }

    /// Overrides configuration values with environment variables
    ///
    /// This method checks for environment variables and updates the config
    /// if they are set and non-empty.
    ///
    /// # Arguments
    ///
    /// * `config` - Mutable reference to the config to update
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` or an error if the multi-record policy is invalid.
    fn override_with_env(config: &mut Self) -> Result<()> {
        if let Ok(v) = env::var(ENV_API_TOKEN) {
            if !v.is_empty() {
                config.api_token = v;
            }
        }
        if let Ok(v) = env::var(ENV_ZONE_ID) {
            if !v.is_empty() {
                config.zone_id = v;
            }
        }
        if let Ok(v) = env::var(ENV_RECORD_NAME) {
            if !v.is_empty() {
                config.record = v;
            }
        }
        if let Ok(v) = env::var(ENV_MULTI_RECORD) {
            if !v.is_empty() {
                config.multi_record = parse_multi_record(&v)?;
            }
        }
        Ok(())
    }

    /// Validates the configuration
    ///
    /// Ensures that all required fields are present and valid.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` or an error if:
    /// - API token is missing
    /// - Zone ID is missing
    /// - Record name is missing
    /// - Record name is invalid
    fn validate(&self) -> Result<()> {
        if self.api_token.is_empty() {
            return Err(anyhow::anyhow!("Missing {}", ENV_API_TOKEN));
        }
        if self.zone_id.is_empty() {
            return Err(anyhow::anyhow!("Missing {}", ENV_ZONE_ID));
        }
        if self.record.is_empty() {
            return Err(anyhow::anyhow!("Missing {}", ENV_RECORD_NAME));
        }
        validate_record_name(&self.record)?;
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct TomlConfig {
    api_token: Option<String>,
    zone_id: Option<String>,
    #[serde(rename = "record_name")]
    record_name: Option<String>,
    timeout: Option<u64>,
    #[serde(rename = "poll_interval")]
    poll_interval: Option<u64>,
    verbose: Option<bool>,
    multi_record: Option<String>,
}

/// Parses a multi-record policy string into a `MultiRecordPolicy` enum
///
/// This function accepts multiple aliases for each policy type:
/// - `Error`: "error", "fail", "reject"
/// - `UpdateFirst`: "first", "update_first", "updatefirst"
/// - `UpdateAll`: "all", "update_all", "updateall"
///
/// # Arguments
///
/// * `value` - The policy string to parse
///
/// # Returns
///
/// Returns a `Result` containing the parsed `MultiRecordPolicy` or an error
/// if the value is invalid.
///
/// # Examples
///
/// ```text
/// # use ipv6ddns::main::parse_multi_record;
/// # use ipv6ddns::cloudflare::MultiRecordPolicy;
/// assert!(matches!(
///     parse_multi_record("first").unwrap(),
///     MultiRecordPolicy::UpdateFirst
/// ));
/// assert!(parse_multi_record("bogus").is_err());
/// ```
fn parse_multi_record(value: &str) -> Result<MultiRecordPolicy> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "error" | "fail" | "reject" => Ok(MultiRecordPolicy::Error),
        "first" | "update_first" | "updatefirst" => Ok(MultiRecordPolicy::UpdateFirst),
        "all" | "update_all" | "updateall" => Ok(MultiRecordPolicy::UpdateAll),
        _ => Err(anyhow::anyhow!(
            "Invalid multi_record policy: '{}'. Use: error|first|all",
            value
        )),
    }
}

//==============================================================================
// State Machine
//==============================================================================

/// Represents the current state of DNS record synchronization
///
/// This enum tracks the synchronization status of the DNS record with Cloudflare.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordState {
    /// Initial state, no record has been synced yet
    Unknown,
    /// Record successfully synced with Cloudflare, contains the current IP
    Synced(String),
    /// Last sync attempt failed, contains the error count
    Error(u64),
}

/// Application state for tracking DNS record synchronization
///
/// This struct maintains the state of the DNS record synchronization process,
/// including the current sync status, last sync time, error count, and next retry time.
struct AppState {
    /// Current synchronization state
    state: RecordState,
    /// Timestamp of the last successful sync (UTC)
    last_sync: Option<DateTime<Utc>>,
    /// Number of consecutive errors
    error_count: u64,
    /// Next time to retry after an error (if in backoff period)
    next_retry: Option<Instant>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            state: RecordState::Unknown,
            last_sync: None,
            error_count: 0,
            next_retry: None,
        }
    }
}

impl AppState {
    /// Marks the record as successfully synced
    ///
    /// This method updates the state to `Synced`, records the sync time,
    /// resets the error count, and clears any pending retry.
    ///
    /// # Arguments
    ///
    /// * `ip` - The IPv6 address that was synced
    fn mark_synced(&mut self, ip: String) {
        self.state = RecordState::Synced(ip);
        self.last_sync = Some(Utc::now());
        self.error_count = 0;
        self.next_retry = None;
    }

    /// Marks the record as having a sync error
    ///
    /// This method increments the error count, updates the state to `Error`,
    /// and schedules a retry using exponential backoff.
    fn mark_error(&mut self) {
        self.error_count = self.error_count.saturating_add(1);
        self.state = RecordState::Error(self.error_count);
        self.next_retry = Some(Instant::now() + backoff_delay(self.error_count));
    }
}

/// Base delay for exponential backoff (5 seconds)
const BACKOFF_BASE: Duration = Duration::from_secs(5);
/// Maximum delay for exponential backoff (10 minutes)
const BACKOFF_MAX: Duration = Duration::from_secs(600);

/// Calculates the backoff delay based on the error count
///
/// This function implements exponential backoff with a maximum delay.
/// The delay formula is: `min(5 * 2^(error_count - 1), 600)` seconds
///
/// # Arguments
///
/// * `error_count` - Number of consecutive errors
///
/// # Returns
///
/// Returns the backoff duration
///
/// # Examples
///
/// ```text
/// # use ipv6ddns::main::backoff_delay;
/// # use std::time::Duration;
/// let delay = backoff_delay(1);
/// assert_eq!(delay, Duration::from_secs(5));
///
/// let delay = backoff_delay(2);
/// assert_eq!(delay, Duration::from_secs(10));
/// ```
fn backoff_delay(error_count: u64) -> Duration {
    let exp = error_count.saturating_sub(1).min(10);
    let secs = BACKOFF_BASE
        .as_secs()
        .saturating_mul(1u64 << exp)
        .min(BACKOFF_MAX.as_secs());
    Duration::from_secs(secs)
}

//==============================================================================
// Daemon
//==============================================================================

/// Main daemon for IPv6 DDNS synchronization
///
/// The daemon monitors IPv6 address changes and updates Cloudflare DNS records
/// accordingly. It supports both event-driven (netlink) and polling-based monitoring.
struct Daemon {
    /// Shared configuration
    config: Arc<Config>,
    /// Shared application state (protected by mutex)
    state: Arc<tokio::sync::Mutex<AppState>>,
    /// Cloudflare API client
    cf_client: Arc<CloudflareClient>,
    /// Netlink socket for IPv6 address monitoring
    netlink: NetlinkSocket,
}

impl Daemon {
    /// Creates a new daemon instance
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the daemon
    /// * `cf_client` - Cloudflare API client
    /// * `netlink` - Netlink socket for IPv6 monitoring
    fn new(config: Config, cf_client: CloudflareClient, netlink: NetlinkSocket) -> Self {
        Self {
            config: Arc::new(config),
            state: Arc::new(tokio::sync::Mutex::new(AppState::default())),
            cf_client: Arc::new(cf_client),
            netlink,
        }
    }

    /// Runs the daemon main loop
    ///
    /// This method:
    /// 1. Logs daemon startup information
    /// 2. Performs initial sync if IPv6 is available
    /// 3. Enters the main event loop, handling:
    ///    - SIGTERM: Graceful shutdown
    ///    - SIGHUP: Force resync
    ///    - Netlink events: IPv6 address changes
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on graceful shutdown or an error if the daemon fails.
    async fn run(&mut self) -> Result<()> {
        info!("Starting ipv6ddns daemon");
        info!("Record: {}", self.config.record);
        info!(
            "Mode: {}",
            if self.netlink.is_event_driven() {
                "event-driven (netlink)"
            } else {
                "polling"
            }
        );
        info!("Multi-record policy: {:?}", self.config.multi_record);
        debug!(
            "Zone ID: {}",
            redact_secrets(
                &self.config.zone_id,
                &self.config.api_token,
                &self.config.zone_id
            )
        );

        if let Some(ip) = detect_global_ipv6() {
            info!("Initial IPv6: {}", ip);
            _ = self.sync_record(&ip).await;
        } else {
            warn!("No IPv6 on startup");
        }

        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sighup = signal(SignalKind::hangup())?;

        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("SIGTERM received");
                    break;
                }
                _ = sighup.recv() => {
                    info!("SIGHUP received: forcing resync");
                    if let Some(ip) = detect_global_ipv6() {
                        if let Err(e) = self.sync_record(&ip).await {
                            error!("Sync failed: {:#}", e);
                        }
                    } else {
                        warn!("No IPv6 on SIGHUP");
                    }
                }
                event = self.netlink.recv() => {
                    self.handle_event(event).await;
                }
            }
        }

        info!("Daemon stopped");
        Ok(())
    }

    /// Handles a netlink event
    ///
    /// # Arguments
    ///
    /// * `event` - The netlink event to handle
    async fn handle_event(&self, event: Result<NetlinkEvent>) {
        match event {
            Ok(NetlinkEvent::Ipv6Added(ip)) => {
                info!("IPv6 change detected: {}", ip);
                if let Err(e) = self.sync_record(&ip).await {
                    error!("Sync failed: {:#}", e);
                }
            }
            Ok(NetlinkEvent::Ipv6Removed) => {
                warn!("IPv6 address removed");
            }
            Ok(NetlinkEvent::Unknown) => {}
            Err(e) => debug!("Netlink error: {:#}", e),
        }
    }

    /// Synchronizes the DNS record with the current IPv6 address
    ///
    /// This method:
    /// 1. Validates the IPv6 address format
    /// 2. Checks if the IP has changed (skips if same)
    /// 3. Checks if backoff is active (skips if in backoff period)
    /// 4. Calls Cloudflare API to update or create the record
    /// 5. Updates the application state on success or failure
    ///
    /// # Arguments
    ///
    /// * `ip` - The IPv6 address to sync
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful sync or an error if sync fails.
    async fn sync_record(&self, ip: &str) -> Result<()> {
        // Validate IPv6 address format before making API calls
        if ip.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(anyhow::anyhow!("Invalid IPv6 address format: {}", ip));
        }

        {
            let state = self.state.lock().await;
            if let RecordState::Synced(current) = &state.state {
                if current == ip {
                    debug!("No change: {}", ip);
                    return Ok(());
                }
            }
            if let Some(next_retry) = state.next_retry {
                if next_retry > Instant::now() {
                    debug!("Backoff active; skipping sync until {:?}", next_retry);
                    return Ok(());
                }
            }
        }

        let redacted_zone = redact_secrets(
            &self.config.zone_id,
            &self.config.api_token,
            &self.config.zone_id,
        );
        info!(
            "Syncing {} -> {} (zone: {})",
            self.config.record, ip, redacted_zone
        );

        let result = self
            .cf_client
            .upsert_aaaa_record(
                &self.config.zone_id,
                &self.config.record,
                ip,
                self.config.multi_record,
            )
            .await;

        match result {
            Ok(record) => {
                let mut state = self.state.lock().await;
                state.mark_synced(ip.to_string());
                info!("Synced (ID: {})", record.id);
                Ok(())
            }
            Err(e) => {
                let mut state = self.state.lock().await;
                state.mark_error();
                error!("Sync failed: {:#}", e);
                Err(e)
            }
        }
    }
}

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

    let cf_client = CloudflareClient::new(&config.api_token, config.timeout)
        .context("Cloudflare client failed")?;

    let netlink =
        NetlinkSocket::new(Some(config.poll_interval)).context("Netlink socket failed")?;

    let mut daemon = Daemon::new(config, cf_client, netlink);
    daemon.run().await?;

    Ok(())
}

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let keys = [
                ENV_API_TOKEN,
                ENV_ZONE_ID,
                ENV_RECORD_NAME,
                ENV_MULTI_RECORD,
            ];
            let mut saved = Vec::with_capacity(keys.len());
            for key in keys {
                saved.push((key, std::env::var(key).ok()));
                std::env::remove_var(key);
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                if let Some(val) = value {
                    std::env::set_var(key, val);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }

    fn write_config(contents: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).expect("write config");
        (dir, path)
    }

    #[test]
    #[serial]
    fn config_load_from_file() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "file_token"
zone_id = "file_zone"
record_name = "file.example.com"
timeout = 45
poll_interval = 90
verbose = true
multi_record = "all"
"#,
        );

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.api_token, "file_token");
        assert_eq!(cfg.zone_id, "file_zone");
        assert_eq!(cfg.record, "file.example.com");
        assert_eq!(cfg.timeout, Duration::from_secs(45));
        assert_eq!(cfg.poll_interval, Duration::from_secs(90));
        assert!(cfg.verbose);
        assert!(matches!(cfg.multi_record, MultiRecordPolicy::UpdateAll));
    }

    #[test]
    #[serial]
    fn config_env_overrides_file() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "file_token"
zone_id = "file_zone"
record_name = "file.example.com"
"#,
        );

        std::env::set_var(ENV_API_TOKEN, "env_token");
        std::env::set_var(ENV_ZONE_ID, "env_zone");
        std::env::set_var(ENV_RECORD_NAME, "env.example.com");

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.api_token, "env_token");
        assert_eq!(cfg.zone_id, "env_zone");
        assert_eq!(cfg.record, "env.example.com");
    }

    #[test]
    #[serial]
    fn config_missing_required_fields() {
        let _env = EnvGuard::new();
        let err = Config::load(None).expect_err("missing required");
        let msg = format!("{err}");
        assert!(
            msg.contains(ENV_API_TOKEN)
                || msg.contains(ENV_ZONE_ID)
                || msg.contains(ENV_RECORD_NAME)
        );
    }

    #[test]
    fn parse_multi_record_valid_and_invalid() {
        assert!(matches!(
            parse_multi_record("first").unwrap(),
            MultiRecordPolicy::UpdateFirst
        ));
        assert!(parse_multi_record("bogus").is_err());
    }

    // Integration tests for daemon lifecycle

    #[test]
    fn validate_record_name_valid_cases() {
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
    fn validate_record_name_invalid_cases() {
        assert!(validate_record_name("").is_err());
        assert!(validate_record_name(" ").is_err());
        assert!(validate_record_name("example com").is_err());
        assert!(validate_record_name(".example.com").is_err());
        assert!(validate_record_name("example..com").is_err());
        assert!(validate_record_name("-example.com").is_err());
        assert!(validate_record_name("example-.com").is_err());
        assert!(validate_record_name("ex@mple.com").is_err());
        // Note: "example.com." is valid (FQDN with trailing dot)
        assert!(validate_record_name(&"a.".repeat(254)).is_err());
    }

    #[test]
    fn test_backoff_delay_calculation() {
        let delay = backoff_delay(1);
        assert_eq!(delay, Duration::from_secs(5));

        let delay = backoff_delay(2);
        assert_eq!(delay, Duration::from_secs(10));

        let delay = backoff_delay(3);
        assert_eq!(delay, Duration::from_secs(20));

        let delay = backoff_delay(5);
        assert_eq!(delay, Duration::from_secs(80));

        let delay = backoff_delay(10);
        assert_eq!(delay, BACKOFF_MAX);

        let delay = backoff_delay(100);
        assert_eq!(delay, BACKOFF_MAX);
    }

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.state, RecordState::Unknown);
        assert!(state.last_sync.is_none());
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_app_state_mark_synced() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".to_string());

        assert_eq!(state.state, RecordState::Synced("2001:db8::1".to_string()));
        assert!(state.last_sync.is_some());
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_app_state_mark_error() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".to_string());
        state.mark_error();

        assert!(matches!(state.state, RecordState::Error(_)));
        assert_eq!(state.error_count, 1);
        assert!(state.next_retry.is_some());
    }

    #[test]
    fn test_app_state_error_backoff_increases() {
        let mut state = AppState::default();

        state.mark_error();
        let retry1 = state.next_retry.unwrap();
        state.mark_error();
        let retry2 = state.next_retry.unwrap();

        assert!(retry2 > retry1);
    }

    #[test]
    fn test_app_state_sync_resets_error() {
        let mut state = AppState::default();
        state.mark_error();
        state.mark_synced("2001:db8::1".to_string());

        assert_eq!(state.state, RecordState::Synced("2001:db8::1".to_string()));
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_redact_secrets() {
        let api_token = "secret_token_123";
        let zone_id = "zone_id_456";
        let message = "API call with token secret_token_123 and zone zone_id_456";

        let redacted = redact_secrets(message, api_token, zone_id);
        assert!(!redacted.contains(api_token));
        assert!(!redacted.contains(zone_id));
        assert!(redacted.contains("***REDACTED***"));
    }

    #[test]
    fn test_redact_secrets_empty() {
        let message = "API call with no secrets";
        let redacted = redact_secrets(message, "", "");
        assert_eq!(redacted, message);
    }
}
