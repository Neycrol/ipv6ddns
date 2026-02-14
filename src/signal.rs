//! Signal handling for graceful shutdown and resync triggers
//!
//! This module provides a SignalHandler for managing OS signals
//! such as SIGTERM (graceful shutdown) and SIGHUP (force resync).

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
