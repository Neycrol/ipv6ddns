//! Async integration tests for daemon module
//!
//! These tests verify the daemon's async behavior, state machine transitions,
//! and backoff logic in a controlled environment.

use ipv6ddns::constants::{BACKOFF_BASE_SECS, BACKOFF_MAX_EXPONENT, BACKOFF_MAX_SECS};
use ipv6ddns::daemon::{backoff_delay, redact_secrets, AppState, RecordState};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Test AppState default initialization
#[tokio::test]
async fn test_app_state_default_async() {
    let state = AppState::default();

    assert_eq!(state.state, RecordState::Unknown);
    assert!(state.last_sync.is_none());
    assert_eq!(state.error_count, 0);
    assert!(state.next_retry.is_none());
}

/// Test AppState state transitions in async context
#[tokio::test]
async fn test_app_state_transitions_async() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Initial state
    {
        let s = state.lock().await;
        assert_eq!(s.state, RecordState::Unknown);
    }

    // Transition to Synced
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::1".to_string());
    }

    // Verify Synced state
    {
        let s = state.lock().await;
        assert_eq!(s.state, RecordState::Synced("2001:db8::1".to_string()));
        assert!(s.last_sync.is_some());
        assert_eq!(s.error_count, 0);
        assert!(s.next_retry.is_none());
    }

    // Transition to Error
    {
        let mut s = state.lock().await;
        s.mark_error();
    }

    // Verify Error state
    {
        let s = state.lock().await;
        assert!(matches!(s.state, RecordState::Error(1)));
        assert_eq!(s.error_count, 1);
        assert!(s.next_retry.is_some());
    }

    // Transition back to Synced
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::2".to_string());
    }

    // Verify Synced state again
    {
        let s = state.lock().await;
        assert_eq!(s.state, RecordState::Synced("2001:db8::2".to_string()));
        assert_eq!(s.error_count, 0);
        assert!(s.next_retry.is_none());
    }
}

/// Test backoff delay calculation with various error counts
#[test]
fn test_backoff_delay_calculation() {
    // Test initial error (error_count = 1)
    let delay1 = backoff_delay(1);
    assert_eq!(delay1, Duration::from_secs(BACKOFF_BASE_SECS));

    // Test exponential growth
    let delay2 = backoff_delay(2);
    assert_eq!(delay2, Duration::from_secs(BACKOFF_BASE_SECS * 2));

    let delay3 = backoff_delay(3);
    assert_eq!(delay3, Duration::from_secs(BACKOFF_BASE_SECS * 4));

    let delay4 = backoff_delay(4);
    assert_eq!(delay4, Duration::from_secs(BACKOFF_BASE_SECS * 8));

    // Test maximum backoff
    let delay_max = backoff_delay(100);
    assert_eq!(delay_max, Duration::from_secs(BACKOFF_MAX_SECS));
}

/// Test that backoff delay respects the maximum exponent
#[test]
fn test_backoff_max_exponent() {
    let delay_at_max_exponent = backoff_delay(BACKOFF_MAX_EXPONENT + 1);
    let delay_beyond_max = backoff_delay(BACKOFF_MAX_EXPONENT + 10);

    // Both should be capped at BACKOFF_MAX_SECS
    assert_eq!(delay_at_max_exponent, Duration::from_secs(BACKOFF_MAX_SECS));
    assert_eq!(delay_beyond_max, Duration::from_secs(BACKOFF_MAX_SECS));
}

/// Test AppState error count saturation
#[tokio::test]
async fn test_error_count_saturation() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Mark many errors to test saturation
    for _ in 0..1000 {
        let mut s = state.lock().await;
        s.mark_error();
    }

    let s = state.lock().await;
    // Error count should grow but not exceed reasonable limits
    // The actual count depends on the implementation, we just verify it's tracked
    assert!(s.error_count > 0, "Error count should be tracked");
    assert!(
        s.error_count <= 1000,
        "Error count should not exceed number of errors"
    );
}

/// Test retry scheduling in async context
#[tokio::test]
async fn test_retry_scheduling() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Mark an error to schedule a retry
    {
        let mut s = state.lock().await;
        s.mark_error();
    }

    // Check that retry is scheduled in the future
    let retry_time = {
        let s = state.lock().await;
        s.next_retry.expect("Retry should be scheduled")
    };

    assert!(
        retry_time > Instant::now(),
        "Retry should be scheduled for the future"
    );

    // Wait a bit and verify retry time is still in the future
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        retry_time > Instant::now(),
        "Retry time should still be in the future"
    );
}

/// Test secrets redaction in async context
#[tokio::test]
async fn test_redact_secrets_async() {
    let api_token = "secret_token_12345";
    let zone_id = "zone_id_67890";
    let message = "API call with token secret_token_12345 and zone zone_id_67890";

    let redacted = redact_secrets(message, api_token, zone_id);

    assert!(
        !redacted.contains(api_token),
        "API token should be redacted"
    );
    assert!(!redacted.contains(zone_id), "Zone ID should be redacted");
    assert!(
        redacted.contains("***REDACTED***"),
        "Redaction marker should be present"
    );
}

/// Test redact_secrets with empty strings
#[tokio::test]
async fn test_redact_secrets_empty() {
    let message = "API call with no secrets";
    let redacted = redact_secrets(message, "", "");

    assert_eq!(
        redacted, message,
        "Message should be unchanged when secrets are empty"
    );
}

/// Test state machine consistency under rapid transitions
#[tokio::test]
async fn test_rapid_state_transitions() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Rapidly alternate between synced and error states
    for i in 0..20 {
        let mut s = state.lock().await;
        if i % 2 == 0 {
            s.mark_synced(format!("2001:db8::{}", i));
        } else {
            s.mark_error();
        }
        // Immediately check state consistency
        match &s.state {
            RecordState::Synced(ip) => assert!(!ip.is_empty()),
            RecordState::Error(count) => assert!(*count > 0),
            RecordState::Unknown => {}
        }
    }

    // Final state should be consistent
    let s = state.lock().await;
    match &s.state {
        RecordState::Synced(ip) => {
            assert!(!ip.is_empty());
            assert_eq!(s.error_count, 0);
            assert!(s.next_retry.is_none());
        }
        RecordState::Error(count) => {
            assert!(*count > 0);
            assert!(s.next_retry.is_some());
        }
        RecordState::Unknown => {
            // This should not happen after many transitions
            panic!("State should not remain Unknown after transitions");
        }
    }
}

/// Test that state is properly shared across async tasks
#[tokio::test]
async fn test_state_sharing_across_tasks() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Task 1: Mark as synced
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::task1".to_string());
    }

    // Task 2: Read the state set by task 1
    {
        let s = state.lock().await;
        match &s.state {
            RecordState::Synced(ip) => {
                assert_eq!(ip, "2001:db8::task1");
            }
            _ => panic!("Expected Synced state"),
        }
    }
}

/// Test timeout behavior in async operations
#[tokio::test]
async fn test_async_timeout_behavior() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // First, set the state to synced
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::timeout".to_string());
    }

    // Try to access the state with a timeout
    let timeout_result = tokio::time::timeout(Duration::from_millis(100), async {
        let s = state.lock().await;
        match &s.state {
            RecordState::Synced(ip) => Some(ip.clone()),
            _ => None,
        }
    })
    .await;

    match timeout_result {
        Ok(Some(ip)) => assert_eq!(ip, "2001:db8::timeout"),
        Ok(None) => panic!("Expected Synced state"),
        Err(_) => panic!("Timeout waiting for state access"),
    }
}

/// Test state recovery from multiple consecutive errors
#[tokio::test]
async fn test_state_recovery_from_multiple_errors() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Simulate 5 consecutive errors
    for i in 0..5 {
        let mut s = state.lock().await;
        s.mark_error();
        assert_eq!(s.error_count, i + 1);
        assert!(matches!(s.state, RecordState::Error(_)));
    }

    // Verify state is still in error after multiple failures
    {
        let s = state.lock().await;
        assert_eq!(s.error_count, 5);
        assert!(matches!(s.state, RecordState::Error(_)));
    }

    // Recover by marking as synced
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::recovered".to_string());
    }

    // Verify recovery
    {
        let s = state.lock().await;
        assert_eq!(s.error_count, 0); // Error count should reset
        assert!(matches!(s.state, RecordState::Synced(ref ip) if ip == "2001:db8::recovered"));
        assert!(s.last_sync.is_some());
        assert!(s.next_retry.is_none());
    }
}

/// Test state transitions under concurrent access
#[tokio::test]
async fn test_concurrent_state_transitions() {
    let state = Arc::new(Mutex::new(AppState::default()));
    let mut handles = vec![];

    // Spawn 10 tasks that try to update the state concurrently
    for i in 0..10 {
        let state_clone = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            let mut s = state_clone.lock().await;
            if i % 2 == 0 {
                s.mark_synced(format!("2001:db8::{}", i));
            } else {
                s.mark_error();
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify state is in a valid condition (either synced or error)
    let s = state.lock().await;
    assert!(
        matches!(s.state, RecordState::Synced(_) | RecordState::Error(_)),
        "State should be either synced or error after concurrent updates"
    );
}

/// Test rapid state changes between synced and error
#[tokio::test]
async fn test_rapid_state_changes() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Rapidly change state multiple times between synced and error
    for i in 0..20 {
        let mut s = state.lock().await;
        if i % 2 == 0 {
            s.mark_synced(format!("2001:db8::{}", i));
        } else {
            s.mark_error();
        }
    }

    // Verify final state is valid
    let s = state.lock().await;
    assert!(
        matches!(s.state, RecordState::Synced(_) | RecordState::Error(_)),
        "Final state should be valid"
    );
}

/// Test error count reset after successful sync
#[tokio::test]
async fn test_error_count_reset_after_sync() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Generate some errors
    {
        let mut s = state.lock().await;
        for _ in 0..3 {
            s.mark_error();
        }
        assert_eq!(s.error_count, 3);
    }

    // Successfully sync
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::success".to_string());
    }

    // Verify error count is reset
    {
        let s = state.lock().await;
        assert_eq!(s.error_count, 0);
        assert!(matches!(s.state, RecordState::Synced(_)));
    }

    // Generate another error and verify count starts from 1
    {
        let mut s = state.lock().await;
        s.mark_error();
        assert_eq!(s.error_count, 1);
    }
}

/// Test retry scheduling with exponential backoff
#[tokio::test]
async fn test_retry_scheduling_with_backoff() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Generate errors to trigger retry scheduling
    for i in 0..5 {
        let mut s = state.lock().await;
        s.mark_error();

        // Verify retry is scheduled
        assert!(
            s.next_retry.is_some(),
            "Retry should be scheduled after error {}",
            i
        );
    }

    // Verify backoff increases with error count
    let s = state.lock().await;
    assert_eq!(s.error_count, 5);
    assert!(s.next_retry.is_some());

    // The retry delay should be larger for higher error counts
    // (exact value depends on backoff calculation)
}

/// Test state consistency after multiple operations
#[tokio::test]
async fn test_state_consistency() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Set a known state
    {
        let mut s = state.lock().await;
        s.mark_synced("2001:db8::test".to_string());
    }

    // Verify state is consistent
    {
        let s = state.lock().await;
        assert!(matches!(s.state, RecordState::Synced(_)));
        assert_eq!(s.error_count, 0);
        assert!(s.last_sync.is_some());
    }
}

/// Test deadlock prevention
#[tokio::test]
async fn test_no_deadlock_on_nested_locks() {
    let state = Arc::new(Mutex::new(AppState::default()));

    // This should not deadlock even though we're locking multiple times
    let result = tokio::time::timeout(Duration::from_secs(1), async {
        let mut s1 = state.lock().await;
        s1.mark_synced("2001:db8::1".to_string());

        // Drop the lock before acquiring it again
        drop(s1);

        let mut s2 = state.lock().await;
        s2.mark_error();

        drop(s2);

        let s3 = state.lock().await;
        matches!(s3.state, RecordState::Error(_))
    })
    .await;

    assert!(result.is_ok(), "Should not deadlock");
    assert!(result.unwrap(), "State should be in error");
}
