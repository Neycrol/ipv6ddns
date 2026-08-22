//! Daemon module for ipv6ddns
//!
//! This module contains the main daemon implementation for IPv6 DDNS synchronization.

use std::borrow::Cow;
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::constants::{BACKOFF_BASE_SECS, BACKOFF_MAX_EXPONENT, BACKOFF_MAX_SECS};
use crate::dns_provider::DnsProvider;
use crate::health::HealthServer;
use crate::netlink::{NetlinkEvent, NetlinkSocket, detect_global_ipv6};
use crate::validation::is_valid_ipv6;

const EVENT_COALESCE_WINDOW: Duration = Duration::from_millis(80);
const EVENT_COALESCE_MAX_EVENTS: usize = 32;

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
    ///
    /// Stored as a native [`Ipv6Addr`] so change detection is a cheap 16-byte
    /// comparison instead of a heap-backed string compare.
    Synced(Ipv6Addr),
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
    /// Timestamp of the last successful sync
    pub last_sync: Option<SystemTime>,
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
    pub fn mark_synced(&mut self, ip: Ipv6Addr) {
        self.state = RecordState::Synced(ip);
        self.last_sync = Some(SystemTime::now());
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
/// CPU note: when neither secret appears in the message (the common case)
/// the original string is borrowed as-is — no allocation, no rewrite pass.
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
pub fn redact_secrets<'a>(message: &'a str, api_token: &str, zone_id: &str) -> Cow<'a, str> {
    let has_token = !api_token.is_empty() && message.contains(api_token);
    let has_zone = !zone_id.is_empty() && message.contains(zone_id);
    if !has_token && !has_zone {
        return Cow::Borrowed(message);
    }

    let mut sanitized = message.to_string();
    if has_token {
        sanitized = sanitized.replace(api_token, "***REDACTED***");
    }
    if has_zone {
        sanitized = sanitized.replace(zone_id, "***REDACTED***");
    }

    Cow::Owned(sanitized)
}

/// Determines whether an IPv6 address is eligible for DNS synchronization.
///
/// This wrapper provides an abstraction layer over basic validation, allowing
/// future filtering logic (e.g., prefix matching, blocklists) to be added
/// without changing call sites.
#[must_use]
fn is_syncable_ipv6(ip: Ipv6Addr, allow_loopback: bool) -> bool {
    is_valid_ipv6(ip, allow_loopback)
}

fn merge_burst_event(current: &mut NetlinkEvent, next: NetlinkEvent) {
    match next {
        NetlinkEvent::Ipv6Added(ip) => *current = NetlinkEvent::Ipv6Added(ip),
        NetlinkEvent::Ipv6Removed => *current = NetlinkEvent::Ipv6Removed,
        NetlinkEvent::Unknown => {}
    }
}

//==============================================================================
// Daemon
//==============================================================================

/// Main daemon for IPv6 DDNS synchronization
///
/// The daemon monitors IPv6 address changes and updates DNS records
/// accordingly. It supports both event-driven (netlink) and polling-based monitoring.
pub struct Daemon<P: DnsProvider> {
    /// Shared configuration
    config: Arc<Config>,
    /// Shared application state (protected by mutex)
    state: Arc<tokio::sync::Mutex<AppState>>,
    /// DNS provider client
    dns_provider: Arc<P>,
    /// Netlink socket for IPv6 address monitoring
    netlink: NetlinkSocket,
}

impl<P: DnsProvider> Daemon<P> {
    /// Creates a new daemon instance
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the daemon
    /// * `dns_provider` - DNS provider client
    /// * `netlink` - Netlink socket for IPv6 monitoring
    pub fn new(config: Config, dns_provider: Arc<P>, netlink: NetlinkSocket) -> Self {
        Self {
            config: Arc::new(config),
            state: Arc::new(tokio::sync::Mutex::new(AppState::default())),
            dns_provider,
            netlink,
        }
    }

    async fn sync_with_error_context(&self, ip: Ipv6Addr, error_context: &str) {
        if let Err(e) = self.sync_record(ip).await {
            error!("{}: {:#}", error_context, e);
        }
    }

    async fn detect_and_sync_with_context(
        &self,
        detected_info: &str,
        no_ipv6_warning: &str,
        sync_error_context: &str,
    ) {
        match detect_global_ipv6(self.config.allow_loopback) {
            Some(ip) => {
                info!("{}: {}", detected_info, ip);
                self.sync_with_error_context(ip, sync_error_context).await;
            }
            None => {
                warn!("{}", no_ipv6_warning);
            }
        }
    }

    async fn coalesce_burst_event(
        &mut self,
        first_event: Result<NetlinkEvent>,
    ) -> Result<NetlinkEvent> {
        let mut effective = first_event?;
        if !matches!(
            effective,
            NetlinkEvent::Ipv6Added(_) | NetlinkEvent::Ipv6Removed
        ) {
            return Ok(effective);
        }

        for _ in 0..EVENT_COALESCE_MAX_EVENTS {
            let next = match tokio::time::timeout(EVENT_COALESCE_WINDOW, self.netlink.recv()).await
            {
                Ok(next) => next,
                Err(_) => break,
            };

            match next {
                Ok(next_event) => merge_burst_event(&mut effective, next_event),
                Err(e) => return Err(e),
            }
        }

        Ok(effective)
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
    pub async fn run(&mut self) -> Result<()> {
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
                self.config.zone_id.as_str(),
                self.config.api_token.as_str(),
                self.config.zone_id.as_str()
            )
        );

        let mut health_server = if self.config.health_port > 0 {
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], self.config.health_port));
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

        self.detect_and_sync_with_context(
            "Initial IPv6",
            "No IPv6 on startup",
            "Initial sync failed",
        )
        .await;

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
                    self.detect_and_sync_with_context("Detected IPv6 on SIGHUP", "No IPv6 on SIGHUP", "Sync failed")
                        .await;
                }
                event = self.netlink.recv() => {
                    let coalesced = self.coalesce_burst_event(event).await;
                    self.handle_event(coalesced).await;
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
                if !is_syncable_ipv6(ip, self.config.allow_loopback) {
                    warn!(
                        "Ignoring non-routable IPv6 from netlink event: {} (will not sync)",
                        ip
                    );
                    if let Some(detected_ip) = detect_global_ipv6(self.config.allow_loopback) {
                        if detected_ip != ip {
                            info!(
                                "Using detected global IPv6 after filtering event: {}",
                                detected_ip
                            );
                            self.sync_with_error_context(detected_ip, "Sync failed")
                                .await;
                        }
                    }
                    return;
                }
                info!("IPv6 change detected: {}", ip);
                self.sync_with_error_context(ip, "Sync failed").await;
            }
            Ok(NetlinkEvent::Ipv6Removed) => {
                warn!("IPv6 address removed");
                self.detect_and_sync_with_context(
                    "Replacement IPv6 detected after removal",
                    "No global IPv6 available after removal; keeping DNS unchanged",
                    "Sync failed after IPv6 removal",
                )
                .await;
            }
            Ok(NetlinkEvent::Unknown) => {}
            Err(e) => debug!("Netlink error: {:#}", e),
        }
    }

    /// Synchronizes the DNS record with the current IPv6 address
    ///
    /// This method:
    /// 1. Validates the IPv6 address is syncable (format + routability rules)
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
    async fn sync_record(&self, ip: Ipv6Addr) -> Result<()> {
        // Validate IPv6 address routability before making API calls.
        // This is a hard safety gate against accidentally syncing link-local
        // addresses (e.g. fe80::/10) even if upstream metadata is inconsistent.
        if !is_syncable_ipv6(ip, self.config.allow_loopback) {
            return Err(anyhow::anyhow!(
                "Invalid or non-routable IPv6 address for sync: {}",
                ip
            ));
        }

        {
            let state = self.state.lock().await;
            if let RecordState::Synced(current) = &state.state {
                if *current == ip {
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
            self.config.zone_id.as_str(),
            self.config.api_token.as_str(),
            self.config.zone_id.as_str(),
        );
        info!(
            "Syncing {} -> {} (zone: {})",
            self.config.record, ip, redacted_zone
        );

        let result = self
            .dns_provider
            .upsert_aaaa_record(
                self.config.zone_id.as_str(),
                &self.config.record,
                ip,
                self.config.multi_record,
            )
            .await;

        match result {
            Ok(record) => {
                let mut state = self.state.lock().await;
                state.mark_synced(ip);
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

    /// Builds an `Ipv6Added` event from a textual address (test helper).
    fn ipv6_added(s: &str) -> NetlinkEvent {
        NetlinkEvent::Ipv6Added(s.parse().unwrap())
    }
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
        state.mark_synced("2001:db8::1".parse().unwrap());

        assert_eq!(
            state.state,
            RecordState::Synced("2001:db8::1".parse().unwrap())
        );
        assert!(state.last_sync.is_some());
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_app_state_mark_error() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".parse().unwrap());
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
        state.mark_synced("2001:db8::1".parse().unwrap());

        assert_eq!(
            state.state,
            RecordState::Synced("2001:db8::1".parse().unwrap())
        );
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

        state.mark_synced("2001:db8::1".parse().unwrap());
        assert_eq!(
            state.state,
            RecordState::Synced("2001:db8::1".parse().unwrap())
        );
        assert!(state.last_sync.is_some());
        assert_eq!(state.error_count, 0);
        assert!(state.next_retry.is_none());
    }

    #[test]
    fn test_state_machine_synced_to_error() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".parse().unwrap());

        state.mark_error();
        assert!(matches!(state.state, RecordState::Error(1)));
        assert_eq!(state.error_count, 1);
        assert!(state.next_retry.is_some());
    }

    #[test]
    fn test_state_machine_error_to_synced() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".parse().unwrap());
        state.mark_error();

        state.mark_synced("2001:db8::2".parse().unwrap());
        assert_eq!(
            state.state,
            RecordState::Synced("2001:db8::2".parse().unwrap())
        );
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
        state.mark_synced("2001:db8::1".parse().unwrap());

        // Simulate sync with same IP (should be idempotent)
        state.mark_synced("2001:db8::1".parse().unwrap());
        assert_eq!(
            state.state,
            RecordState::Synced("2001:db8::1".parse().unwrap())
        );
        assert_eq!(state.error_count, 0);
    }

    #[test]
    fn test_state_machine_sync_with_different_ip_updates() {
        let mut state = AppState::default();
        state.mark_synced("2001:db8::1".parse().unwrap());

        // Sync with different IP
        state.mark_synced("2001:db8::2".parse().unwrap());
        assert_eq!(
            state.state,
            RecordState::Synced("2001:db8::2".parse().unwrap())
        );
        assert_eq!(state.error_count, 0);
    }

    // Netlink event simulation tests

    #[test]
    fn test_netlink_event_ipv6_added() {
        let event = ipv6_added("2001:db8::1");
        assert!(matches!(event, NetlinkEvent::Ipv6Added(_)));

        if let NetlinkEvent::Ipv6Added(ip) = event {
            assert_eq!(ip, "2001:db8::1".parse::<std::net::Ipv6Addr>().unwrap());
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
            ipv6_added("2001:db8::1"),
            ipv6_added("2001:db8::2"),
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
            let event = NetlinkEvent::Ipv6Added(ip.parse().unwrap());
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

    #[test]
    fn test_is_syncable_ipv6_filters_link_local_even_if_syntactically_valid() {
        assert!(!is_syncable_ipv6("fe80::1".parse().unwrap(), false));
        assert!(!is_syncable_ipv6("fe80::dead:beef".parse().unwrap(), false));
    }

    #[test]
    fn test_is_syncable_ipv6_loopback_respects_flag() {
        assert!(!is_syncable_ipv6("::1".parse().unwrap(), false));
        assert!(is_syncable_ipv6("::1".parse().unwrap(), true));
    }

    #[test]
    fn test_merge_burst_event_prefers_latest_routable_state() {
        let mut current = ipv6_added("2001:db8::1");
        merge_burst_event(&mut current, ipv6_added("2001:db8::2"));
        assert_eq!(current, ipv6_added("2001:db8::2"));

        merge_burst_event(&mut current, NetlinkEvent::Ipv6Removed);
        assert_eq!(current, NetlinkEvent::Ipv6Removed);
    }

    #[test]
    fn test_merge_burst_event_ignores_unknown_event() {
        let mut current = ipv6_added("2001:db8::1");
        merge_burst_event(&mut current, NetlinkEvent::Unknown);
        assert_eq!(current, ipv6_added("2001:db8::1"));
    }
}
