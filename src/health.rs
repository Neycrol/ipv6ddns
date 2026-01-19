//! Health check endpoint for ipv6ddns
//!
//! This module provides a lightweight HTTP endpoint for health checks.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use tracing::{error, info};

use crate::daemon::{AppState, RecordState};

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
    /// Starts the health check server
    pub async fn start(addr: SocketAddr, state: Arc<Mutex<AppState>>) -> Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        info!("Health check server listening on {}", addr);

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept = listener.accept() => {
                        match accept {
                            Ok((mut socket, _peer)) => {
                                let state = Arc::clone(&state);
                                tokio::spawn(async move {
                                    let mut buf = [0u8; 1024];
                                    let _ = socket.read(&mut buf).await;

                                    let snapshot = state.lock().await;
                                    let response = build_response(&snapshot);
                                    let body = match serde_json::to_string(&response) {
                                        Ok(body) => body,
                                        Err(_) => "{\"status\":\"error\"}".to_string(),
                                    };

                                    let reply = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        body.len(),
                                        body
                                    );

                                    if let Err(e) = socket.write_all(reply.as_bytes()).await {
                                        error!("Health response write failed: {}", e);
                                    }
                                    let _ = socket.shutdown().await;
                                });
                            }
                            Err(e) => {
                                error!("Health listener accept error: {}", e);
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
        })
    }

    /// Stops the health check server
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

//==============================================================================
// Helpers
//==============================================================================

fn build_response(state: &AppState) -> HealthResponse {
    let (sync_state, healthy) = match &state.state {
        RecordState::Unknown => ("unknown".to_string(), false),
        RecordState::Synced(_) => ("synced".to_string(), true),
        RecordState::Error(_) => ("error".to_string(), false),
    };

    let last_sync_seconds_ago = state.last_sync.map(|ts| {
        let seconds = (Utc::now() - ts).num_seconds();
        seconds.max(0) as f64
    });

    HealthResponse {
        status: if healthy { "ok".to_string() } else { "degraded".to_string() },
        sync_state,
        last_sync_seconds_ago,
        error_count: state.error_count,
        healthy,
    }
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
}
