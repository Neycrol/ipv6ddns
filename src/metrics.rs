//! Prometheus metrics collection for ipv6ddns
//!
//! This module provides metrics collection for monitoring the daemon's behavior.

use lazy_static::lazy_static;
use prometheus::{
    register_counter_vec, register_gauge, register_histogram, register_histogram_vec,
    CounterVec, Gauge, Histogram, HistogramVec,
};

//==============================================================================
// Metrics
//==============================================================================

lazy_static! {
    /// Total number of successful DNS updates
    pub static ref DNS_UPDATES_TOTAL: CounterVec = register_counter_vec!(
        "ipv6ddns_dns_updates_total",
        "Total number of successful DNS updates",
        &["provider"]
    )
    .unwrap();

    /// Total number of DNS update errors
    pub static ref DNS_ERRORS_TOTAL: CounterVec = register_counter_vec!(
        "ipv6ddns_dns_errors_total",
        "Total number of DNS update errors",
        &["provider", "error_type"]
    )
    .unwrap();

    /// Current error count (consecutive errors)
    pub static ref ERROR_COUNT: Gauge = register_gauge!(
        "ipv6ddns_error_count",
        "Current number of consecutive errors"
    )
    .unwrap();

    /// Time since last successful sync in seconds
    pub static ref LAST_SYNC_SECONDS: Gauge = register_gauge!(
        "ipv6ddns_last_sync_seconds",
        "Time since last successful sync in seconds"
    )
    .unwrap();

    /// Current sync state (0=Unknown, 1=Synced, 2=Error)
    pub static ref SYNC_STATE: Gauge = register_gauge!(
        "ipv6ddns_sync_state",
        "Current sync state (0=Unknown, 1=Synced, 2=Error)"
    )
    .unwrap();

    /// DNS update duration histogram
    pub static ref DNS_UPDATE_DURATION_SECONDS: HistogramVec = register_histogram_vec!(
        "ipv6ddns_dns_update_duration_seconds",
        "DNS update duration in seconds",
        &["provider", "operation"]
    )
    .unwrap();

    /// IPv6 address change events histogram
    pub static ref IPV6_CHANGE_EVENTS: Histogram = register_histogram!(
        "ipv6ddns_ipv6_change_events",
        "IPv6 address change events",
        vec![0.1, 0.5, 1.0, 5.0]
    )
    .unwrap();
}

//==============================================================================
// Public Functions
//==============================================================================

/// Records a successful DNS update
///
/// # Arguments
///
/// * `provider` - DNS provider name (e.g., "cloudflare")
pub fn record_dns_update(provider: &str) {
    DNS_UPDATES_TOTAL.with_label_values(&[provider]).inc();
}

/// Records a DNS update error
///
/// # Arguments
///
/// * `provider` - DNS provider name (e.g., "cloudflare")
/// * `error_type` - Type of error (e.g., "rate_limit", "network")
pub fn record_dns_error(provider: &str, error_type: &str) {
    DNS_ERRORS_TOTAL
        .with_label_values(&[provider, error_type])
        .inc();
}

/// Sets the current error count
///
/// # Arguments
///
/// * `count` - Number of consecutive errors
pub fn set_error_count(count: u64) {
    ERROR_COUNT.set(count as f64);
}

/// Sets the time since last successful sync
///
/// # Arguments
///
/// * `seconds` - Seconds since last sync
pub fn set_last_sync(seconds: f64) {
    LAST_SYNC_SECONDS.set(seconds);
}

/// Sets the current sync state
///
/// # Arguments
///
/// * `state` - Sync state (0=Unknown, 1=Synced, 2=Error)
pub fn set_sync_state(state: u64) {
    SYNC_STATE.set(state as f64);
}

/// Starts a timer for DNS update duration
///
/// # Arguments
///
/// * `provider` - DNS provider name (e.g., "cloudflare")
/// * `operation` - Operation type (e.g., "upsert", "get")
///
/// # Returns
///
/// Returns a histogram timer
pub fn start_dns_update_timer(provider: &str, operation: &str) -> HistogramTimer {
    DNS_UPDATE_DURATION_SECONDS
        .with_label_values(&[provider, operation])
        .start_timer()
}

/// Starts a timer for IPv6 change event processing
///
/// # Returns
///
/// Returns a histogram timer
pub fn start_ipv6_change_timer() -> HistogramTimer {
    IPV6_CHANGE_EVENTS.start_timer()
}

/// Collects all metrics and returns them as text
///
/// # Returns
///
/// Returns the metrics in Prometheus text format
pub fn gather_metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .expect("Failed to encode metrics");
    String::from_utf8(buffer).expect("Metrics should be valid UTF-8")
}

//==============================================================================
// Types
//==============================================================================

/// Histogram timer for measuring duration
pub type HistogramTimer = prometheus::HistogramTimer;

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_dns_update() {
        record_dns_update("cloudflare");
        assert!(DNS_UPDATES_TOTAL
            .get_metric_with_label_values(&["cloudflare"])
            .is_ok());
    }

    #[test]
    fn test_record_dns_error() {
        record_dns_error("cloudflare", "rate_limit");
        assert!(DNS_ERRORS_TOTAL
            .get_metric_with_label_values(&["cloudflare", "rate_limit"])
            .is_ok());
    }

    #[test]
    fn test_set_error_count() {
        set_error_count(5);
        assert_eq!(ERROR_COUNT.get(), 5.0);
    }

    #[test]
    fn test_set_last_sync() {
        set_last_sync(123.45);
        assert_eq!(LAST_SYNC_SECONDS.get(), 123.45);
    }

    #[test]
    fn test_set_sync_state() {
        set_sync_state(1);
        assert_eq!(SYNC_STATE.get(), 1.0);
    }

    #[test]
    fn test_gather_metrics() {
        record_dns_update("cloudflare");
        set_error_count(0);
        set_sync_state(1);
        let metrics = gather_metrics();
        assert!(metrics.contains("ipv6ddns_dns_updates_total"));
        assert!(metrics.contains("ipv6ddns_error_count"));
        assert!(metrics.contains("ipv6ddns_sync_state"));
    }
}
