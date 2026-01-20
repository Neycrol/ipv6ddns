//! Configuration module for ipv6ddns
//!
//! This module handles loading and validating configuration from files and environment variables.

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use zeroize::ZeroizeOnDrop;

use crate::constants::{
    DEFAULT_POLL_INTERVAL_SECS, DEFAULT_TIMEOUT_SECS, ENV_ALLOW_LOOPBACK, ENV_API_TOKEN,
    ENV_HEALTH_PORT, ENV_MULTI_RECORD, ENV_PROVIDER_TYPE, ENV_RECORD_NAME, ENV_ZONE_ID,
    MAX_POLL_INTERVAL_SECS, MAX_TIMEOUT_SECS, MAX_ZONE_ID_LENGTH, MIN_API_TOKEN_LENGTH,
    MIN_POLL_INTERVAL_SECS, MIN_TIMEOUT_SECS, MIN_ZONE_ID_LENGTH,
};
use crate::dns_provider::MultiRecordPolicy;
use crate::validation::validate_record_name;

//==============================================================================
// Config
//==============================================================================

/// Configuration for the ipv6ddns daemon
///
/// This struct holds all configuration parameters needed to run the daemon,
/// including Cloudflare API credentials, DNS record settings, and runtime options.
/// Sensitive fields (api_token and zone_id) are wrapped in `Zeroizing` to ensure
/// they are securely cleared from memory when dropped.
///
/// # Fields
///
/// - `api_token`: Cloudflare API token with DNS edit permissions
/// - `zone_id`: Cloudflare zone ID for the domain
/// - `record`: DNS record name to update (e.g., "example.com")
/// - `timeout`: HTTP request timeout in seconds
/// - `poll_interval`: Polling interval in seconds (fallback when netlink unavailable)
/// - `verbose`: Enable verbose logging
/// - `multi_record`: Policy for handling multiple AAAA records
/// - `allow_loopback`: Allow loopback IPv6 (::1) as a valid address
/// - `provider_type`: DNS provider type (default: "cloudflare")
/// - `health_port`: Port for health check endpoint (0 = disabled)
///
/// # Configuration Loading Priority
///
/// Configuration is loaded from multiple sources in order of precedence:
/// 1. Environment variables (highest priority)
/// 2. Config file (`/etc/ipv6ddns/config.toml` or custom path)
/// 3. Defaults (lowest priority)
#[derive(Debug, Clone, ZeroizeOnDrop)]
pub struct Config {
    /// Cloudflare API token with DNS edit permissions
    ///
    /// This token should have the `Zone:DNS:Edit` permission.
    /// It can be set via the `CLOUDFLARE_API_TOKEN` environment variable.
    #[zeroize(skip)]
    pub api_token: zeroize::Zeroizing<String>,
    /// Cloudflare zone ID for the domain
    ///
    /// The zone ID can be found in the Cloudflare dashboard under your domain's DNS settings.
    /// It can be set via the `CLOUDFLARE_ZONE_ID` environment variable.
    #[zeroize(skip)]
    pub zone_id: zeroize::Zeroizing<String>,
    /// DNS record name to update (e.g., "example.com")
    ///
    /// This is the full DNS record name including subdomain if applicable.
    /// It can be set via the `CLOUDFLARE_RECORD_NAME` environment variable.
    #[zeroize(skip)]
    pub record: String,
    /// HTTP request timeout in seconds
    ///
    /// Default: 30 seconds
    #[zeroize(skip)]
    pub timeout: Duration,
    /// Polling interval in seconds (fallback when netlink unavailable)
    ///
    /// Default: 60 seconds
    /// This is only used when netlink socket creation fails.
    #[zeroize(skip)]
    pub poll_interval: Duration,
    /// Enable verbose logging
    ///
    /// Default: false
    #[zeroize(skip)]
    pub verbose: bool,
    /// Policy for handling multiple AAAA records
    ///
    /// Default: `MultiRecordPolicy::Error`
    /// Can be set via the `CLOUDFLARE_MULTI_RECORD` environment variable.
    #[zeroize(skip)]
    pub multi_record: MultiRecordPolicy,
    /// Allow loopback IPv6 address (::1) to be used for DDNS updates
    ///
    /// Default: false
    /// Can be set via the `IPV6DDNS_ALLOW_LOOPBACK` environment variable.
    #[zeroize(skip)]
    pub allow_loopback: bool,
    /// DNS provider type
    ///
    /// Default: "cloudflare"
    /// Can be set via the `IPV6DDNS_PROVIDER_TYPE` environment variable.
    /// Currently supported: "cloudflare"
    #[zeroize(skip)]
    pub provider_type: String,
    /// Port for health check endpoint
    ///
    /// Default: 0 (disabled)
    /// Can be set via the `IPV6DDNS_HEALTH_PORT` environment variable.
    /// Set to 0 to disable the health check endpoint.
    #[zeroize(skip)]
    pub health_port: u16,
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
        let mut allow_loopback = false;
        let mut provider_type = "cloudflare".to_string();
        let mut health_port: u16 = 0;

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
                poll_interval = toml_config
                    .poll_interval
                    .unwrap_or(DEFAULT_POLL_INTERVAL_SECS);
                verbose = toml_config.verbose.unwrap_or(false);
                if let Some(v) = toml_config.multi_record.as_deref() {
                    multi_record = parse_multi_record(v)?;
                }
                if let Some(v) = toml_config.allow_loopback {
                    allow_loopback = v;
                }
                if let Some(v) = toml_config.provider_type {
                    provider_type = v;
                }
                if let Some(v) = toml_config.health_port {
                    health_port = v;
                }
            }
        }

        Ok(Self {
            api_token: zeroize::Zeroizing::new(api_token),
            zone_id: zeroize::Zeroizing::new(zone_id),
            record,
            timeout: Duration::from_secs(timeout),
            poll_interval: Duration::from_secs(poll_interval),
            verbose,
            multi_record,
            allow_loopback,
            provider_type,
            health_port,
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
                config.api_token = zeroize::Zeroizing::new(v);
            }
        }
        if let Ok(v) = env::var(ENV_ZONE_ID) {
            if !v.is_empty() {
                config.zone_id = zeroize::Zeroizing::new(v);
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
        if let Ok(v) = env::var(ENV_ALLOW_LOOPBACK) {
            if !v.is_empty() {
                config.allow_loopback =
                    parse_bool_env(&v).context("Invalid IPV6DDNS_ALLOW_LOOPBACK value")?;
            }
        }
        if let Ok(v) = env::var(ENV_PROVIDER_TYPE) {
            if !v.is_empty() {
                config.provider_type = v;
            }
        }
        if let Ok(v) = env::var(ENV_HEALTH_PORT) {
            if !v.is_empty() {
                config.health_port = v.parse().context("Invalid IPV6DDNS_HEALTH_PORT value")?;
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
    /// - API token is missing or too short
    /// - Zone ID is missing or invalid format
    /// - Record name is missing
    /// - Record name is invalid
    /// - Timeout is out of valid range
    /// - Poll interval is out of valid range
    fn validate(&self) -> Result<()> {
        if self.api_token.as_str().is_empty() {
            return Err(anyhow::anyhow!("Missing {}", ENV_API_TOKEN));
        }
        // Cloudflare API tokens are typically 40+ characters
        if self.api_token.as_str().len() < MIN_API_TOKEN_LENGTH {
            return Err(anyhow::anyhow!(
                "{} is too short ({} chars, minimum {})",
                ENV_API_TOKEN,
                self.api_token.as_str().len(),
                MIN_API_TOKEN_LENGTH
            ));
        }
        if self.zone_id.as_str().is_empty() {
            return Err(anyhow::anyhow!("Missing {}", ENV_ZONE_ID));
        }
        // Zone IDs are alphanumeric and typically 32 characters
        if !self.zone_id.as_str().chars().all(|c| c.is_alphanumeric()) {
            return Err(anyhow::anyhow!(
                "{} must be alphanumeric, got: {}",
                ENV_ZONE_ID,
                self.zone_id.as_str()
            ));
        }
        if self.zone_id.as_str().len() < MIN_ZONE_ID_LENGTH
            || self.zone_id.as_str().len() > MAX_ZONE_ID_LENGTH
        {
            return Err(anyhow::anyhow!(
                "{} has invalid length ({} chars, expected {}-{})",
                ENV_ZONE_ID,
                self.zone_id.as_str().len(),
                MIN_ZONE_ID_LENGTH,
                MAX_ZONE_ID_LENGTH
            ));
        }
        if self.record.is_empty() {
            return Err(anyhow::anyhow!("Missing {}", ENV_RECORD_NAME));
        }
        validate_record_name(&self.record)?;

        let provider = self.provider_type.trim().to_ascii_lowercase();
        if provider != "cloudflare" {
            return Err(anyhow::anyhow!(
                "{} must be \"cloudflare\" (only provider supported), got: {}",
                ENV_PROVIDER_TYPE,
                self.provider_type
            ));
        }

        let timeout_secs = self.timeout.as_secs();
        if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&timeout_secs) {
            return Err(anyhow::anyhow!(
                "timeout must be between {} and {} seconds, got {}",
                MIN_TIMEOUT_SECS,
                MAX_TIMEOUT_SECS,
                timeout_secs
            ));
        }

        let poll_interval_secs = self.poll_interval.as_secs();
        if !(MIN_POLL_INTERVAL_SECS..=MAX_POLL_INTERVAL_SECS).contains(&poll_interval_secs) {
            return Err(anyhow::anyhow!(
                "poll_interval must be between {} and {} seconds, got {}",
                MIN_POLL_INTERVAL_SECS,
                MAX_POLL_INTERVAL_SECS,
                poll_interval_secs
            ));
        }

        Ok(())
    }
}

/// Parses a boolean value from an environment variable
///
/// This function accepts multiple string representations of boolean values:
/// - `true`: "1", "true", "yes", "on"
/// - `false`: "0", "false", "no", "off"
///
/// # Arguments
///
/// * `value` - The string value to parse
///
/// # Returns
///
/// Returns a `Result` containing the parsed boolean or an error if invalid
fn parse_bool_env(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(anyhow::anyhow!(
            "expected boolean (true/false/1/0/yes/no/on/off)"
        )),
    }
}

/// TOML configuration file structure
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
    allow_loopback: Option<bool>,
    provider_type: Option<String>,
    health_port: Option<u16>,
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
pub fn parse_multi_record(value: &str) -> Result<MultiRecordPolicy> {
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
                ENV_ALLOW_LOOPBACK,
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
api_token = "file_token_123456789012345678901234567890"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
timeout = 45
poll_interval = 90
verbose = true
multi_record = "all"
allow_loopback = true
"#,
        );

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(
            cfg.api_token.as_str(),
            "file_token_123456789012345678901234567890"
        );
        assert_eq!(cfg.zone_id.as_str(), "0123456789abcdef0123456789abcdef");
        assert_eq!(cfg.record, "example.com");
        assert_eq!(cfg.timeout, Duration::from_secs(45));
        assert_eq!(cfg.poll_interval, Duration::from_secs(90));
        assert!(cfg.verbose);
        assert!(matches!(cfg.multi_record, MultiRecordPolicy::UpdateAll));
        assert!(cfg.allow_loopback);
    }

    #[test]
    #[serial]
    fn config_env_overrides_file() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "file_token_123456789012345678901234567890"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
allow_loopback = false
"#,
        );

        std::env::set_var(ENV_API_TOKEN, "env_token_123456789012345678901234567890");
        std::env::set_var(ENV_ZONE_ID, "envzone0123456789abcdef0123456789ab");
        std::env::set_var(ENV_RECORD_NAME, "example.com");
        std::env::set_var(ENV_ALLOW_LOOPBACK, "true");

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(
            cfg.api_token.as_str(),
            "env_token_123456789012345678901234567890"
        );
        assert_eq!(cfg.zone_id.as_str(), "envzone0123456789abcdef0123456789ab");
        assert_eq!(cfg.record, "example.com");
        assert!(cfg.allow_loopback);
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
    #[serial]
    fn config_api_token_too_short() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "short"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
"#,
        );
        let err = Config::load(Some(path)).expect_err("token too short");
        let msg = format!("{err}");
        assert!(msg.contains("too short"));
    }

    #[test]
    #[serial]
    fn config_zone_id_invalid_format() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "invalid-zone-id!"
record_name = "example.com"
"#,
        );
        let err = Config::load(Some(path)).expect_err("zone id invalid");
        let msg = format!("{err}");
        assert!(msg.contains("alphanumeric"));
    }

    #[test]
    #[serial]
    fn config_zone_id_invalid_length() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "short"
record_name = "example.com"
"#,
        );
        let err = Config::load(Some(path)).expect_err("zone id length");
        let msg = format!("{err}");
        assert!(msg.contains("invalid length"));
    }

    #[test]
    fn parse_multi_record_valid_and_invalid() {
        assert!(matches!(
            parse_multi_record("first").unwrap(),
            MultiRecordPolicy::UpdateFirst
        ));
        assert!(parse_multi_record("bogus").is_err());
    }

    // Additional edge case tests for config parsing

    #[test]
    #[serial]
    fn config_timeout_boundary_values() {
        let _env = EnvGuard::new();
        std::env::set_var(ENV_API_TOKEN, "0123456789012345678901234567890123456789");
        std::env::set_var(ENV_ZONE_ID, "0123456789abcdef0123456789abcdef");
        std::env::set_var(ENV_RECORD_NAME, "example.com");

        // Test minimum timeout via config file
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
timeout = 1
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.timeout, Duration::from_secs(1));

        // Test maximum timeout via config file
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
timeout = 300
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.timeout, Duration::from_secs(300));

        // Test timeout below minimum
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
timeout = 0
"#,
        );
        let err = Config::load(Some(path)).expect_err("timeout too low");
        assert!(format!("{err}").contains("timeout"));

        // Test timeout above maximum
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
timeout = 301
"#,
        );
        let err = Config::load(Some(path)).expect_err("timeout too high");
        assert!(format!("{err}").contains("timeout"));
    }

    #[test]
    #[serial]
    fn config_poll_interval_boundary_values() {
        let _env = EnvGuard::new();
        std::env::set_var(ENV_API_TOKEN, "0123456789012345678901234567890123456789");
        std::env::set_var(ENV_ZONE_ID, "0123456789abcdef0123456789abcdef");
        std::env::set_var(ENV_RECORD_NAME, "example.com");

        // Test minimum poll interval via config file
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
poll_interval = 10
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.poll_interval, Duration::from_secs(10));

        // Test maximum poll interval via config file
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
poll_interval = 3600
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.poll_interval, Duration::from_secs(3600));

        // Test poll interval below minimum
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
poll_interval = 9
"#,
        );
        let err = Config::load(Some(path)).expect_err("poll interval too low");
        assert!(format!("{err}").contains("poll_interval"));

        // Test poll interval above maximum
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
poll_interval = 3601
"#,
        );
        let err = Config::load(Some(path)).expect_err("poll interval too high");
        assert!(format!("{err}").contains("poll_interval"));
    }

    #[test]
    #[serial]
    fn config_api_token_exact_minimum_length() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "01234567890123456789012345678901"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.api_token.as_str().len(), 32);
    }

    #[test]
    #[serial]
    fn config_zone_id_exact_minimum_length() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef"
record_name = "example.com"
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.zone_id.as_str().len(), 16);
    }

    #[test]
    #[serial]
    fn config_zone_id_exact_maximum_length() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef01234567"
record_name = "example.com"
"#,
        );
        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.zone_id.as_str().len(), 40);
    }

    #[test]
    #[serial]
    fn config_multi_record_policy_variants() {
        let _env = EnvGuard::new();
        std::env::set_var(ENV_API_TOKEN, "0123456789012345678901234567890123456789");
        std::env::set_var(ENV_ZONE_ID, "0123456789abcdef0123456789abcdef");
        std::env::set_var(ENV_RECORD_NAME, "example.com");

        // Test error policy variants
        for policy in ["error", "fail", "reject"] {
            std::env::set_var(ENV_MULTI_RECORD, policy);
            let cfg = Config::load(None).expect("config load");
            assert!(matches!(cfg.multi_record, MultiRecordPolicy::Error));
        }

        // Test first policy variants
        for policy in ["first", "update_first", "updatefirst"] {
            std::env::set_var(ENV_MULTI_RECORD, policy);
            let cfg = Config::load(None).expect("config load");
            assert!(matches!(cfg.multi_record, MultiRecordPolicy::UpdateFirst));
        }

        // Test all policy variants
        for policy in ["all", "update_all", "updateall"] {
            std::env::set_var(ENV_MULTI_RECORD, policy);
            let cfg = Config::load(None).expect("config load");
            assert!(matches!(cfg.multi_record, MultiRecordPolicy::UpdateAll));
        }

        std::env::remove_var(ENV_MULTI_RECORD);
    }

    #[test]
    #[serial]
    fn config_allow_loopback_variants() {
        let _env = EnvGuard::new();
        std::env::set_var(ENV_API_TOKEN, "0123456789012345678901234567890123456789");
        std::env::set_var(ENV_ZONE_ID, "0123456789abcdef0123456789abcdef");
        std::env::set_var(ENV_RECORD_NAME, "example.com");

        // Test true variants
        for value in ["1", "true", "yes", "on"] {
            std::env::set_var(ENV_ALLOW_LOOPBACK, value);
            let cfg = Config::load(None).expect("config load");
            assert!(cfg.allow_loopback);
        }

        // Test false variants
        for value in ["0", "false", "no", "off"] {
            std::env::set_var(ENV_ALLOW_LOOPBACK, value);
            let cfg = Config::load(None).expect("config load");
            assert!(!cfg.allow_loopback);
        }

        std::env::remove_var(ENV_ALLOW_LOOPBACK);
    }

    #[test]
    #[serial]
    fn config_empty_env_values() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
"#,
        );

        // Empty env values should not override file values
        std::env::set_var(ENV_API_TOKEN, "");
        std::env::set_var(ENV_ZONE_ID, "");
        std::env::set_var(ENV_RECORD_NAME, "");

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(
            cfg.api_token.as_str(),
            "0123456789012345678901234567890123456789"
        );
        assert_eq!(cfg.zone_id.as_str(), "0123456789abcdef0123456789abcdef");
        assert_eq!(cfg.record, "example.com");
    }

    #[test]
    #[serial]
    fn config_whitespace_in_values() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
"#,
        );

        let cfg = Config::load(Some(path)).expect("config load");
        // Whitespace should not be present in zone_id (alphanumeric check)
        assert!(!cfg.zone_id.as_str().contains(" "));
    }

    #[test]
    #[serial]
    fn config_case_sensitivity_in_zone_id() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789"
zone_id = "ABCDEF0123456789abcdef0123456789"
record_name = "example.com"
"#,
        );

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(cfg.zone_id.as_str(), "ABCDEF0123456789abcdef0123456789");
    }

    #[test]
    #[serial]
    fn config_special_characters_in_api_token() {
        let _env = EnvGuard::new();
        let (_dir, path) = write_config(
            r#"
api_token = "0123456789012345678901234567890123456789!@#$%^&*()"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
"#,
        );

        let cfg = Config::load(Some(path)).expect("config load");
        assert_eq!(
            cfg.api_token.as_str(),
            "0123456789012345678901234567890123456789!@#$%^&*()"
        );
    }
}
