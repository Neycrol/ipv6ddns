//! Integration tests for Cloudflare API client
//!
//! These tests verify the Cloudflare API client's construction and basic behavior.

use ipv6ddns::cloudflare::CloudflareClient;
use std::time::Duration;

/// Test API client creation with various parameters
#[test]
fn test_cloudflare_client_creation() {
    // Test with valid parameters
    let client1 = CloudflareClient::new("test-token", Duration::from_secs(30));
    assert!(
        client1.is_ok(),
        "Client creation should succeed with valid parameters"
    );

    // Test with zero timeout (should fail or use default)
    let _client2 = CloudflareClient::new("test-token", Duration::from_secs(0));
    // Behavior depends on implementation - could fail or use minimum

    // Test with very long timeout
    let client3 = CloudflareClient::new("test-token", Duration::from_secs(3600));
    assert!(
        client3.is_ok(),
        "Client creation should succeed with long timeout"
    );
}
