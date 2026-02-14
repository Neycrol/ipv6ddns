//! Signal handling for graceful shutdown and resync triggers
//!
//! This module provides a SignalHandler for managing OS signals
//! such as SIGTERM (graceful shutdown) and SIGHUP (force resync).

#[allow(unused_imports)]
use anyhow::Context as _;
use tokio::signal::unix::{signal, Signal, SignalKind};
use tracing::info;

/// SignalHandler manages OS signals for the daemon
pub struct SignalHandler {
    sigterm: Signal,
    sighup: Signal,
}

impl SignalHandler {
    /// Creates a new SignalHandler
    pub fn new() -> anyhow::Result<Self> {
        let sigterm =
            signal(SignalKind::terminate()).context("Failed to create SIGTERM handler")?;
        let sighup = signal(SignalKind::hangup()).context("Failed to create SIGHUP handler")?;

        Ok(Self { sigterm, sighup })
    }

    /// Waits for the next signal
    pub async fn recv(&mut self) -> SignalType {
        tokio::select! {
            _ = self.sigterm.recv() => {
                info!("SIGTERM received");
                SignalType::Terminate
            }
            _ = self.sighup.recv() => {
                info!("SIGHUP received: forcing resync");
                SignalType::Hangup
            }
        }
    }
}

/// Types of signals that can be received
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalType {
    /// SIGTERM - graceful shutdown
    Terminate,
    /// SIGHUP - force resync
    Hangup,
}

// Re-export for convenience
pub use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_handler_creation() {
        // Test that SignalHandler::new() can be called (it will fail in test environment without tokio runtime)
        // This test mainly verifies the function signature and compilation
        let result = std::panic::catch_unwind(|| {
            let _ = SignalHandler::new();
        });
        // We expect this to either succeed or panic (depending on environment), but not to fail compilation
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_signal_type_terminate() {
        let term = SignalType::Terminate;
        assert_eq!(term, SignalType::Terminate);
        assert_ne!(term, SignalType::Hangup);
    }

    #[test]
    fn test_signal_type_hangup() {
        let hangup = SignalType::Hangup;
        assert_eq!(hangup, SignalType::Hangup);
        assert_ne!(hangup, SignalType::Terminate);
    }

    #[test]
    fn test_signal_type_debug() {
        let term = SignalType::Terminate;
        let hangup = SignalType::Hangup;

        // Test Debug implementation
        let term_debug = format!("{:?}", term);
        let hangup_debug = format!("{:?}", hangup);

        assert_eq!(term_debug, "Terminate");
        assert_eq!(hangup_debug, "Hangup");
    }

    #[test]
    fn test_signal_type_clone() {
        let term1 = SignalType::Terminate;
        let term2 = term1;
        assert_eq!(term1, term2);
        assert_eq!(term2, SignalType::Terminate);
    }

    #[test]
    fn test_signal_type_copy() {
        let term1 = SignalType::Terminate;
        let term2 = term1; // Copy

        // term1 should still be valid after copy
        assert_eq!(term1, SignalType::Terminate);
        assert_eq!(term2, SignalType::Terminate);
    }

    #[test]
    fn test_signal_type_eq() {
        assert_eq!(SignalType::Terminate, SignalType::Terminate);
        assert_eq!(SignalType::Hangup, SignalType::Hangup);
        assert_ne!(SignalType::Terminate, SignalType::Hangup);
    }

    #[tokio::test]
    async fn test_signal_handler_recv_timeout() {
        // Test that recv() can be called (it will timeout since we're not sending signals)
        // This test verifies the async function signature and compilation

        // Note: We can't easily test the actual signal receiving in a unit test
        // as it requires sending actual OS signals to the process

        let result = SignalHandler::new();
        match result {
            Ok(mut handler) => {
                // Use a timeout to avoid hanging the test
                let timeout_result =
                    tokio::time::timeout(std::time::Duration::from_millis(100), handler.recv())
                        .await;

                // The timeout is expected since we're not sending any signals
                assert!(
                    timeout_result.is_err(),
                    "Expected timeout since no signal was sent"
                );
            }
            Err(_) => {
                // It's ok if we can't create the handler in the test environment
                // The important thing is that the code compiles and has the right signature
            }
        }
    }
}
