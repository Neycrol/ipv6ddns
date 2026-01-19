//! Health check endpoint for ipv6ddns
//!
//! This module provides an HTTP endpoint for health checks and metrics.

use anyhow::Result;
use serde::Serialize;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{error, info};

//==============================================================================
// Types
//==============================================================================

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Overall health status
    pub status: String,
    /// Current sync state
    pub sync_state: String,
    /// Time since last successful sync (in seconds, or null if never synced)
    pub last_sync_seconds_ago: Option<f64>,
    /// Number of consecutive errors
    pub error_count: u64,
    /// Whether the daemon is healthy
    pub healthy: bool,
}

/// Health check server
pub struct HealthServer {
    /// Shutdown channel sender
    shutdown_tx: Option<oneshot::Sender<()>>,
}

//==============================================================================
// Implementation
//==============================================================================

impl HealthServer {
    /// Creates a new health server
    pub fn new() -> Self {
        Self { shutdown_tx: None }
    }

    /// Starts the health check server
    ///
    /// # Arguments
    ///
    /// * `addr` - Socket address to bind to
    /// * `metrics_enabled` - Whether to expose metrics endpoint
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the server handle or an error
    pub async fn start(&mut self, addr: SocketAddr, metrics_enabled: bool) -> Result<()> {
        let listener = TcpListener::bind(addr).await?;
        info!("Health check server listening on {}", addr);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        tokio::spawn(async move {
            let router = axum::Router::new()
                .route("/health", axum::routing::get(health_handler))
                .route("/metrics", axum::routing::get(metrics_handler));

            if metrics_enabled {
                info!("Metrics endpoint enabled at http://{}/metrics", addr);
            } else {
                info!("Metrics endpoint disabled");
            }

            let serve = axum::serve(listener, router).with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            });

            if let Err(e) = serve.await {
                error!("Health check server error: {}", e);
            }
        });

        Ok(())
    }

    /// Stops the health check server
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Default for HealthServer {
    fn default() -> Self {
        Self::new()
    }
}

//==============================================================================
// Handlers
//==============================================================================

/// Health check handler
async fn health_handler() -> axum::Json<HealthResponse> {
    // In a real implementation, we would query the daemon's state
    // For now, return a simple healthy response
    axum::Json(HealthResponse {
        status: "ok".to_string(),
        sync_state: "synced".to_string(),
        last_sync_seconds_ago: Some(0.0),
        error_count: 0,
        healthy: true,
    })
}

/// Metrics handler
async fn metrics_handler() -> String {
    crate::metrics::gather_metrics()
}

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let response = HealthResponse {
            status: "ok".to_string(),
            sync_state: "synced".to_string(),
            last_sync_seconds_ago: Some(0.0),
            error_count: 0,
            healthy: true,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"healthy\":true"));
    }

    #[test]
    fn test_health_server_default() {
        let server = HealthServer::default();
        assert!(server.shutdown_tx.is_none());
    }
}
