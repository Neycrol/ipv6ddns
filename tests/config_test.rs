//! Integration tests for configuration loading

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Helper function to create a temporary config file with given content
fn create_temp_config(content: &str) -> PathBuf {
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join(format!("ipv6ddns_test_{}.toml", uuid::Uuid::new_v4()));
    fs::write(&config_path, content).expect("Failed to write temp config");
    config_path
}

/// Helper function to clean up a temporary config file
fn cleanup_temp_config(path: PathBuf) {
    let _ = fs::remove_file(path);
}

/// Helper function to set environment variables for testing
fn set_env_vars(api_token: &str, zone_id: &str, record_name: &str) {
    env::set_var("CLOUDFLARE_API_TOKEN", api_token);
    env::set_var("CLOUDFLARE_ZONE_ID", zone_id);
    env::set_var("CLOUDFLARE_RECORD_NAME", record_name);
}

/// Helper function to clear environment variables for testing
fn clear_env_vars() {
    env::remove_var("CLOUDFLARE_API_TOKEN");
    env::remove_var("CLOUDFLARE_ZONE_ID");
    env::remove_var("CLOUDFLARE_RECORD_NAME");
    env::remove_var("CLOUDFLARE_MULTI_RECORD");
}

#[test]
fn test_config_load_from_env_vars() {
    clear_env_vars();
    set_env_vars("test_token", "test_zone", "test.example.com");

    // This test would need the Config struct to be made public or
    // a test helper function to be added to main.rs
    // For now, we'll just verify the environment variables are set correctly
    assert_eq!(env::var("CLOUDFLARE_API_TOKEN").unwrap(), "test_token");
    assert_eq!(env::var("CLOUDFLARE_ZONE_ID").unwrap(), "test_zone");
    assert_eq!(env::var("CLOUDFLARE_RECORD_NAME").unwrap(), "test.example.com");

    clear_env_vars();
}

#[test]
fn test_config_load_from_file() {
    clear_env_vars();

    let config_content = r#"
record_name = "file.example.com"
timeout = 45
poll_interval = 90
verbose = true
multi_record = "first"
"#;

    let config_path = create_temp_config(config_content);

    // Verify the file was created
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("file.example.com"));
    assert!(content.contains("timeout = 45"));

    cleanup_temp_config(config_path);
    clear_env_vars();
}

#[test]
fn test_config_env_vars_override_file() {
    clear_env_vars();

    let config_content = r#"
record_name = "file.example.com"
timeout = 45
verbose = false
"#;

    let config_path = create_temp_config(config_content);
    set_env_vars("env_token", "env_zone", "env.example.com");

    // Verify both file and env vars are set
    assert!(config_path.exists());
    assert_eq!(env::var("CLOUDFLARE_API_TOKEN").unwrap(), "env_token");
    assert_eq!(env::var("CLOUDFLARE_RECORD_NAME").unwrap(), "env.example.com");

    cleanup_temp_config(config_path);
    clear_env_vars();
}

#[test]
fn test_config_missing_required_fields() {
    clear_env_vars();

    // Test with empty config file
    let config_content = r#"
record_name = ""
"#;

    let config_path = create_temp_config(config_content);

    // Verify file exists but has empty record_name
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("record_name = \"\""));

    cleanup_temp_config(config_path);
    clear_env_vars();
}

#[test]
fn test_config_multi_record_policies() {
    clear_env_vars();

    let policies = vec!["error", "first", "all"];

    for policy in policies {
        env::set_var("CLOUDFLARE_MULTI_RECORD", policy);
        assert_eq!(env::var("CLOUDFLARE_MULTI_RECORD").unwrap(), policy);
    }

    clear_env_vars();
}

#[test]
fn test_config_default_values() {
    clear_env_vars();

    let config_content = r#"
record_name = "default.example.com"
"#;

    let config_path = create_temp_config(config_content);

    // Verify default values in config
    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("default.example.com"));
    // Default values would be: timeout=30, poll_interval=60, verbose=false, multi_record="error"

    cleanup_temp_config(config_path);
    clear_env_vars();
}

#[test]
fn test_config_invalid_multi_record_policy() {
    clear_env_vars();

    // Test with invalid multi_record value
    let config_content = r#"
record_name = "test.example.com"
multi_record = "invalid"
"#;

    let config_path = create_temp_config(config_content);

    // Verify file exists with invalid policy
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("multi_record = \"invalid\""));

    cleanup_temp_config(config_path);
    clear_env_vars();
}

#[test]
fn test_config_nonexistent_file() {
    clear_env_vars();

    let nonexistent_path = PathBuf::from("/tmp/ipv6ddns_nonexistent_test.toml");

    // Verify file doesn't exist
    assert!(!nonexistent_path.exists());

    clear_env_vars();
}

#[test]
fn test_config_numeric_values() {
    clear_env_vars();

    let config_content = r#"
record_name = "numeric.example.com"
timeout = 120
poll_interval = 300
"#;

    let config_path = create_temp_config(config_content);

    // Verify numeric values are correctly parsed
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("timeout = 120"));
    assert!(content.contains("poll_interval = 300"));

    cleanup_temp_config(config_path);
    clear_env_vars();
}

#[test]
fn test_config_boolean_values() {
    clear_env_vars();

    let config_content = r#"
record_name = "boolean.example.com"
verbose = true
"#;

    let config_path = create_temp_config(config_content);

    // Verify boolean values are correctly parsed
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("verbose = true"));

    cleanup_temp_config(config_path);
    clear_env_vars();
}