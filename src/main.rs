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

const ENV_API_TOKEN: &str = "CLOUDFLARE_API_TOKEN";
const ENV_ZONE_ID: &str = "CLOUDFLARE_ZONE_ID";
const ENV_RECORD_NAME: &str = "CLOUDFLARE_RECORD_NAME";
const ENV_MULTI_RECORD: &str = "CLOUDFLARE_MULTI_RECORD";

/// Redacts sensitive data (API tokens and zone IDs) from log messages.
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

#[derive(Debug, Clone)]
pub struct Config {
    pub api_token: String,
    pub zone_id: String,
    pub record: String,
    pub timeout: Duration,
    pub poll_interval: Duration,
    pub verbose: bool,
    pub multi_record: MultiRecordPolicy,
}

impl Config {
    pub fn load(config_path: Option<PathBuf>) -> Result<Self> {
        let mut config = Self::load_from_file(config_path)?;
        Self::override_with_env(&mut config)?;
        Self::validate(&config)?;
        Ok(config)
    }

    fn load_from_file(config_path: Option<PathBuf>) -> Result<Self> {
        let mut api_token = String::new();
        let mut zone_id = String::new();
        let mut record = String::new();
        let mut timeout = 30u64;
        let mut poll_interval = 60u64;
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
                timeout = toml_config.timeout.unwrap_or(30);
                poll_interval = toml_config.poll_interval.unwrap_or(60);
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordState {
    Unknown,
    Synced(String),
    Error(u64),
}

struct AppState {
    state: RecordState,
    last_sync: Option<DateTime<Utc>>,
    error_count: u64,
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
    fn mark_synced(&mut self, ip: String) {
        self.state = RecordState::Synced(ip);
        self.last_sync = Some(Utc::now());
        self.error_count = 0;
        self.next_retry = None;
    }

    fn mark_error(&mut self) {
        self.error_count = self.error_count.saturating_add(1);
        self.state = RecordState::Error(self.error_count);
        self.next_retry = Some(Instant::now() + backoff_delay(self.error_count));
    }
}

const BACKOFF_BASE: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(600);

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

struct Daemon {
    config: Arc<Config>,
    state: Arc<tokio::sync::Mutex<AppState>>,
    cf_client: Arc<CloudflareClient>,
    netlink: NetlinkSocket,
}

impl Daemon {
    fn new(config: Config, cf_client: CloudflareClient, netlink: NetlinkSocket) -> Self {
        Self {
            config: Arc::new(config),
            state: Arc::new(tokio::sync::Mutex::new(AppState::default())),
            cf_client: Arc::new(cf_client),
            netlink,
        }
    }

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
#[command(version = "1.0.0")]
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
            msg.starts_with("Missing ")
                || msg.contains("Missing required")
                || msg.contains("missing required")
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

    // Integration tests for daemon lifecycle

    #[test]
    fn test_daemon_config_initialization() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "test_token"
zone_id = "test_zone"
record_name = "test.example.com"
timeout = 30
poll_interval = 60
verbose = false
multi_record = "error"
"#,
        );

        let config = Config::load(Some(path)).expect("config load");
        assert_eq!(config.api_token, "test_token");
        assert_eq!(config.zone_id, "test_zone");
        assert_eq!(config.record, "test.example.com");
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.poll_interval, Duration::from_secs(60));
        assert!(!config.verbose);
        assert!(matches!(config.multi_record, MultiRecordPolicy::Error));
    }

    #[test]
    fn test_daemon_config_validation_with_invalid_record() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "test_token"
zone_id = "test_zone"
record_name = "invalid record name with spaces"
timeout = 30
poll_interval = 60
verbose = false
multi_record = "error"
"#,
        );

        let result = Config::load(Some(path));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("spaces") || err.contains("invalid"));
    }

    #[test]
    fn test_daemon_config_timeout_limits() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "test_token"
zone_id = "test_zone"
record_name = "test.example.com"
timeout = 1
poll_interval = 300
verbose = false
multi_record = "error"
"#,
        );

        let config = Config::load(Some(path)).expect("config load");
        assert_eq!(config.timeout, Duration::from_secs(1));
        assert_eq!(config.poll_interval, Duration::from_secs(300));
    }

    // State machine transition tests

    #[test]
    fn test_state_machine_unknown_to_synced() {
        let mut state = AppState::default();
        assert_eq!(state.state, RecordState::Unknown);

        state.mark_synced("2001:db8::1".to_string());
        assert_eq!(state.state, RecordState::Synced("2001:db8::1".to_string()));
        assert!(state.last_sync.is_some());
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_state_machine_synced_to_error() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".to_string());

        state.mark_error();
        assert!(matches!(state.state, RecordState::Error(1)));
        assert_eq!(state.error_count, 1);
        assert!(state.next_retry.is_some());
    }

    #[test]
    fn test_state_machine_error_to_synced() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".to_string());
        state.mark_error();

        state.mark_synced("2001:db8::2".to_string());
        assert_eq!(state.state, RecordState::Synced("2001:db8::2".to_string()));
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_state_machine_multiple_errors_increases_backoff() {
        let mut state = AppState::default();

        state.mark_error();
        let retry1 = state.next_retry.unwrap();
        assert_eq!(state.error_count, 1);

        state.mark_error();
        let retry2 = state.next_retry.unwrap();
        assert_eq!(state.error_count, 2);

        state.mark_error();
        let retry3 = state.next_retry.unwrap();
        assert_eq!(state.error_count, 3);

        // Verify backoff increases exponentially
        assert!(retry2 > retry1);
        assert!(retry3 > retry2);

        // Verify backoff delay calculation
        let delay1 = retry1.duration_since(Instant::now());
        let delay2 = retry2.duration_since(Instant::now());
        let delay3 = retry3.duration_since(Instant::now());

        // delay2 should be approximately 2x delay1
        assert!(delay2.as_secs() >= delay1.as_secs() * 2 - 1);
        // delay3 should be approximately 2x delay2
        assert!(delay3.as_secs() >= delay2.as_secs() * 2 - 1);
    }

    #[test]
    fn test_state_machine_backoff_max_limit() {
        let mut state = AppState::default();

        // Simulate many errors to hit max backoff
        for _ in 0..20 {
            state.mark_error();
        }

        let retry_time = state.next_retry.unwrap();
        let delay = retry_time.duration_since(Instant::now());

        // Verify backoff is capped at BACKOFF_MAX
        assert!(delay.as_secs() <= BACKOFF_MAX.as_secs());
        assert!(delay.as_secs() >= BACKOFF_MAX.as_secs() - 1);
    }

    #[test]
    fn test_state_machine_sync_with_same_ip_no_change() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".to_string());

        // Simulate sync with same IP (should be idempotent)
        state.mark_synced("2001:db8::1".to_string());
        assert_eq!(state.state, RecordState::Synced("2001:db8::1".to_string()));
        assert_eq!(state.error_count, 0);
    }

    #[test]
    fn test_state_machine_sync_with_different_ip_updates() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".to_string());

        // Sync with different IP
        state.mark_synced("2001:db8::2".to_string());
        assert_eq!(state.state, RecordState::Synced("2001:db8::2".to_string()));
        assert_eq!(state.error_count, 0);
    }

    // Netlink event simulation tests

    #[test]
    fn test_netlink_event_ipv6_added() {
        let event = NetlinkEvent::Ipv6Added("2001:db8::1".to_string());
        assert!(matches!(event, NetlinkEvent::Ipv6Added(_)));

        if let NetlinkEvent::Ipv6Added(ip) = event {
            assert_eq!(ip, "2001:db8::1".to_string());
        }
    }

    #[test]
    fn test_netlink_event_ipv6_removed() {
        let event = NetlinkEvent::Ipv6Removed;
        assert!(matches!(event, NetlinkEvent::Ipv6Removed));
    }

    #[test]
    fn test_netlink_event_unknown() {
        let event = NetlinkEvent::Unknown;
        assert!(matches!(event, NetlinkEvent::Unknown));
    }

    #[test]
    fn test_netlink_event_sequence() {
        let events = vec![
            NetlinkEvent::Ipv6Added("2001:db8::1".to_string()),
            NetlinkEvent::Ipv6Added("2001:db8::2".to_string()),
            NetlinkEvent::Ipv6Removed,
            NetlinkEvent::Unknown,
        ];

        assert!(matches!(events[0], NetlinkEvent::Ipv6Added(_)));
        assert!(matches!(events[1], NetlinkEvent::Ipv6Added(_)));
        assert!(matches!(events[2], NetlinkEvent::Ipv6Removed));
        assert!(matches!(events[3], NetlinkEvent::Unknown));
    }

    #[test]
    fn test_ipv6_address_validation_for_events() {
        let valid_ips = vec![
            "2001:db8::1",
            "::1",
            "fe80::1",
            "2001:0db8:0000:0000:0000:0000:0000:0001",
        ];

        for ip in valid_ips {
            let event = NetlinkEvent::Ipv6Added(ip.to_string());
            assert!(matches!(event, NetlinkEvent::Ipv6Added(_)));
            assert!(ip.parse::<std::net::Ipv6Addr>().is_ok());
        }
    }

    #[test]
    fn test_ipv6_address_validation_rejects_invalid() {
        let invalid_ips = vec!["192.168.1.1", "invalid", "", "2001:db8::g"];

        for ip in invalid_ips {
            assert!(ip.parse::<std::net::Ipv6Addr>().is_err());
        }
    }
}
