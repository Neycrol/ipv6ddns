//! Daemon module for ipv6ddns
//!
//! This module contains the main daemon implementation for IPv6 DDNS synchronization.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::constants::{
    BACKOFF_BASE_SECS, BACKOFF_MAX_EXPONENT, BACKOFF_MAX_SECS, CONFIG_WATCH_DEBOUNCE_MS,
};
use crate::dns_provider::DnsProvider;
use crate::health::HealthServer;
use crate::netlink::{detect_global_ipv6, NetlinkEvent, NetlinkSocket};

//==============================================================================
// State Machine
//==============================================================================

/// Represents the current state of DNS record synchronization
///
/// This enum tracks the synchronization status of the DNS record with Cloudflare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordState {
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
pub struct AppState {
    /// Current synchronization state
    pub state: RecordState,
    /// Timestamp of the last successful sync (UTC)
    pub last_sync: Option<DateTime<Utc>>,
    /// Number of consecutive errors
    pub error_count: u64,
    /// Next time to retry after an error (if in backoff period)
    pub next_retry: Option<Instant>,
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
    pub fn mark_synced(&mut self, ip: String) {
        self.state = RecordState::Synced(ip);
        self.last_sync = Some(Utc::now());
        self.error_count = 0;
        self.next_retry = None;
    }

    /// Marks the record as having a sync error
    ///
    /// This method increments the error count, updates the state to `Error`,
    /// and schedules a retry using exponential backoff.
    pub fn mark_error(&mut self) {
        self.error_count = self.error_count.saturating_add(1);
        self.state = RecordState::Error(self.error_count);
        self.next_retry = Some(Instant::now() + backoff_delay(self.error_count));
    }
}

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
/// # use ipv6ddns::daemon::backoff_delay;
/// # use std::time::Duration;
/// let delay = backoff_delay(1);
/// assert_eq!(delay, Duration::from_secs(5));
///
/// let delay = backoff_delay(2);
/// assert_eq!(delay, Duration::from_secs(10));
/// ```
pub fn backoff_delay(error_count: u64) -> Duration {
    let exp = error_count.saturating_sub(1).min(BACKOFF_MAX_EXPONENT);
    let secs = BACKOFF_BASE_SECS
        .saturating_mul(1u64 << exp)
        .min(BACKOFF_MAX_SECS);
    Duration::from_secs(secs)
}

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
#[must_use]
pub fn redact_secrets(message: &str, api_token: &str, zone_id: &str) -> String {
    let mut sanitized = message.to_string();

    if !api_token.is_empty() {
        sanitized = sanitized.replace(api_token, "***REDACTED***");
    }
    if !zone_id.is_empty() {
        sanitized = sanitized.replace(zone_id, "***REDACTED***");
    }

    sanitized
}

//==============================================================================
// Daemon
//==============================================================================

/// Main daemon for IPv6 DDNS synchronization
///
/// The daemon monitors IPv6 address changes and updates DNS records
/// accordingly. It supports both event-driven (netlink) and polling-based monitoring.
pub struct Daemon {
    /// Shared configuration (protected by RwLock for hot-reloading)
    config: Arc<tokio::sync::RwLock<Config>>,
    /// Shared application state (protected by mutex)
    state: Arc<tokio::sync::Mutex<AppState>>,
    /// DNS provider client (trait object)
    dns_provider: Arc<dyn DnsProvider>,
    /// Netlink socket for IPv6 address monitoring
    netlink: NetlinkSocket,
}

impl Daemon {
    /// Creates a new daemon instance
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the daemon
    /// * `dns_provider` - DNS provider client (trait object)
    /// * `netlink` - Netlink socket for IPv6 monitoring
    pub fn new(config: Config, dns_provider: Arc<dyn DnsProvider>, netlink: NetlinkSocket) -> Self {
        Self {
            config: Arc::new(tokio::sync::RwLock::new(config)),
            state: Arc::new(tokio::sync::Mutex::new(AppState::default())),
            dns_provider,
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
    ///    - Config file changes (if config_path is provided)
    ///
    /// # Arguments
    ///
    /// * `config_path` - Optional path to config file for watching
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on graceful shutdown or an error if the daemon fails.
    pub async fn run(&mut self, config_path: Option<std::path::PathBuf>) -> Result<()> {
        info!("Starting ipv6ddns daemon");

        // Atomically read all initial configuration values to avoid race conditions
        let (
            record,
            mode,
            multi_record,
            zone_id,
            api_token,
            health_port,
            allow_loopback,
            has_config_path,
            config_path_from_config,
        ) = {
            let config = self.config.read().await;
            (
                config.record.clone(),
                if self.netlink.is_event_driven() {
                    "event-driven (netlink)"
                } else {
                    "polling"
                },
                config.multi_record,
                config.zone_id.clone(),
                config.api_token.clone(),
                config.health_port,
                config.allow_loopback,
                config.config_path.is_some(),
                config.config_path.clone(),
            )
        };

        info!("Record: {}", record);
        info!("Mode: {}", mode);
        info!("Multi-record policy: {:?}", multi_record);
        debug!(
            "Zone ID: {}",
            redact_secrets(zone_id.as_str(), api_token.as_str(), zone_id.as_str())
        );

        let mut health_server = if health_port > 0 {
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], health_port));
            match HealthServer::start(addr, Arc::clone(&self.state)).await {
                Ok(server) => Some(server),
                Err(e) => {
                    error!("Health server failed to start: {:#}", e);
                    None
                }
            }
        } else {
            None
        };

        if let Some(ip) = detect_global_ipv6(allow_loopback) {
            info!("Initial IPv6: {}", ip);
            _ = self.sync_record(&ip).await;
        } else {
            warn!("No IPv6 on startup");
        }

        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sighup = signal(SignalKind::hangup())?;

        // Set up config file watching if path is provided
        let (config_tx, mut config_rx) = tokio::sync::mpsc::channel::<()>(10);
        let mut _watcher: Option<RecommendedWatcher> = None;
        let mut debounce_timer = None;

        // Use config_path parameter if provided, otherwise use config_path from config
        let watch_path = config_path.or(config_path_from_config);

        if let Some(ref path) = watch_path {
            let path_clone = path.clone();
            let tx = config_tx.clone();

            // Create file watcher
            let mut watcher = RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    match res {
                        Ok(event) => {
                            // Check if the event affects our config file
                            let is_relevant = event.paths.iter().any(|p| {
                                p == &path_clone || p.file_name() == path_clone.file_name()
                            });

                            // Only trigger on modify events (not on initial watch)
                            if is_relevant && event.kind.is_modify() {
                                debug!("Config file changed: {:?}", event);
                                // Use try_send to avoid blocking if the channel is full
                                if let Err(e) = tx.try_send(()) {
                                    warn!("Failed to send config change notification: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Config file watch error: {}", e);
                        }
                    }
                },
                NotifyConfig::default().with_poll_interval(Duration::from_secs(1)),
            )?;

            watcher.watch(path.as_path(), RecursiveMode::NonRecursive)?;
            info!("Watching config file for changes: {}", path.display());
            _watcher = Some(watcher);
        } else if has_config_path {
            warn!("Config loaded from environment variables only, file watching disabled");
        } else {
            info!("No config file specified, file watching disabled");
        }

        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("SIGTERM received");
                    break;
                }
                _ = sighup.recv() => {
                    info!("SIGHUP received: reloading configuration and forcing resync");

                    // Try to reload configuration
                    let reload_result = {
                        let config = self.config.read().await;
                        config.reload()
                    };

                    match reload_result {
                        Ok(new_config) => {
                            info!("Configuration reloaded successfully");
                            // Update the config
                            *self.config.write().await = new_config;
                        }
                        Err(e) => {
                            error!("Configuration reload failed: {:#}. Keeping old configuration.", e);
                            // Continue with old config
                        }
                    }

                    // Force resync regardless of reload success
                    let config = self.config.read().await;
                    if let Some(ip) = detect_global_ipv6(config.allow_loopback) {
                        if let Err(e) = self.sync_record(&ip).await {
                            error!("Sync failed: {:#}", e);
                        }
                    } else {
                        warn!("No IPv6 on SIGHUP");
                    }
                }
                // Handle config file changes with debouncing
                Some(()) = async {
                    if _watcher.is_some() {
                        config_rx.recv().await
                    } else {
                        None
                    }
                } => {
                    // Start or reset debounce timer
                    debounce_timer = Some(tokio::time::Instant::now() + Duration::from_millis(CONFIG_WATCH_DEBOUNCE_MS));
                }
                // Check debounce timer
                _ = async {
                    if let Some(timer) = debounce_timer {
                        tokio::time::sleep_until(timer).await;
                        Result::<(), ()>::Ok(())
                    } else {
                        std::future::pending().await
                    }
                } => {
                    // Debounce period elapsed, handle the config change
                    debounce_timer = None;
                    self.handle_config_change().await;
                }
                event = self.netlink.recv() => {
                    self.handle_event(event).await;
                }
            }
        }

        info!("Daemon stopped");
        if let Some(server) = health_server.as_mut() {
            server.stop().await;
        }

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

    /// Handles configuration file changes
    ///
    /// This method is called when the configuration file is modified.
    /// It attempts to reload the configuration and applies the new settings.
    /// If the reload fails, the old configuration is preserved.
    ///
    /// # Behavior
    ///
    /// - Debounces rapid file changes (500ms cooldown)
    /// - Validates new configuration before applying
    /// - Logs success or failure
    /// - Preserves old config if reload fails
    pub async fn handle_config_change(&self) {
        info!("Configuration file changed, reloading...");

        let reload_result = {
            let config = self.config.read().await;
            config.reload()
        };

        match reload_result {
            Ok(new_config) => {
                info!("Configuration reloaded successfully from file change");
                // Update the config
                *self.config.write().await = new_config;

                // Force a resync with new config
                let config = self.config.read().await;
                if let Some(ip) = detect_global_ipv6(config.allow_loopback) {
                    if let Err(e) = self.sync_record(&ip).await {
                        error!("Sync failed after config reload: {:#}", e);
                    }
                }
            }
            Err(e) => {
                error!(
                    "Configuration reload failed: {:#}. Keeping old configuration.",
                    e
                );
            }
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

        // Read config for this sync operation
        let config = self.config.read().await;
        let redacted_zone = redact_secrets(
            config.zone_id.as_str(),
            config.api_token.as_str(),
            config.zone_id.as_str(),
        );
        info!(
            "Syncing {} -> {} (zone: {})",
            config.record, ip, redacted_zone
        );
        let zone_id = config.zone_id.clone();
        let record = config.record.clone();
        let multi_record = config.multi_record;
        drop(config); // Release read lock

        let result = self
            .dns_provider
            .upsert_aaaa_record(zone_id.as_str(), &record, ip, multi_record)
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
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::BACKOFF_MAX_SECS;

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
        assert_eq!(delay, Duration::from_secs(BACKOFF_MAX_SECS));

        let delay = backoff_delay(100);
        assert_eq!(delay, Duration::from_secs(BACKOFF_MAX_SECS));
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

        // Verify backoff is capped at BACKOFF_MAX_SECS
        assert!(delay.as_secs() <= BACKOFF_MAX_SECS);
        assert!(delay.as_secs() >= BACKOFF_MAX_SECS - 1);
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
        let events = [
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
