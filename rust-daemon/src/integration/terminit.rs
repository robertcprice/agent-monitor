//! Terminit bridge for bidirectional communication.
//!
//! This module provides a bridge between agent-monitor and terminit,
//! enabling real-time session and event synchronization.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info};

use crate::events::EventBus;
use crate::models::Session;
use crate::storage::Storage;

use super::shared_types::{BridgeConfig, BridgeMessage, UnifiedAgentEvent, UnifiedSessionState};

/// Bridge for communication with terminit.
pub struct TerminitBridge {
    config: BridgeConfig,
    storage: Storage,
    event_bus: EventBus,
    /// Sender for outgoing messages to terminit
    outgoing_tx: broadcast::Sender<BridgeMessage>,
    /// Track connected terminit instances
    connected_clients: Arc<RwLock<Vec<mpsc::Sender<BridgeMessage>>>>,
    /// Running state
    running: Arc<RwLock<bool>>,
}

impl TerminitBridge {
    /// Create a new terminit bridge.
    pub fn new(config: BridgeConfig, storage: Storage, event_bus: EventBus) -> Self {
        let (outgoing_tx, _) = broadcast::channel(config.event_buffer_size);

        Self {
            config,
            storage,
            event_bus,
            outgoing_tx,
            connected_clients: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the bridge server (listens for terminit connections).
    pub async fn start_server(&self) -> Result<()> {
        *self.running.write().await = true;

        // Start Unix socket server
        if let Some(socket_path) = &self.config.terminit_socket {
            let path = PathBuf::from(socket_path);
            self.start_socket_server(path).await?;
        }

        Ok(())
    }

    /// Start the Unix socket server.
    async fn start_socket_server(&self, socket_path: PathBuf) -> Result<()> {
        // Remove existing socket
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)?;
        info!("Terminit bridge listening at {:?}", socket_path);

        let storage = self.storage.clone();
        let event_bus = self.event_bus.clone();
        let outgoing_tx = self.outgoing_tx.clone();
        let connected_clients = self.connected_clients.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            loop {
                if !*running.read().await {
                    break;
                }

                match listener.accept().await {
                    Ok((stream, _)) => {
                        info!("Terminit client connected");

                        let storage = storage.clone();
                        let outgoing_rx = outgoing_tx.subscribe();
                        let clients = connected_clients.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_terminit_client(stream, storage, outgoing_rx, clients).await
                            {
                                error!("Terminit client error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Accept error: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the bridge.
    pub async fn stop(&self) {
        *self.running.write().await = false;
        info!("Terminit bridge stopped");
    }

    /// Broadcast an event to all connected terminit clients.
    pub fn broadcast_event(&self, event: UnifiedAgentEvent) {
        let message = BridgeMessage::EventNotification { event };
        let _ = self.outgoing_tx.send(message);
    }

    /// Broadcast a session update to all connected terminit clients.
    pub fn broadcast_session_update(&self, session: &Session) {
        let unified = UnifiedSessionState::from(session);
        let message = BridgeMessage::SessionUpdate { session: unified };
        let _ = self.outgoing_tx.send(message);
    }

    /// Get the number of connected terminit clients.
    pub async fn connected_count(&self) -> usize {
        self.connected_clients.read().await.len()
    }
}

/// Handle a connected terminit client.
async fn handle_terminit_client(
    stream: UnixStream,
    storage: Storage,
    mut outgoing_rx: broadcast::Receiver<BridgeMessage>,
    _clients: Arc<RwLock<Vec<mpsc::Sender<BridgeMessage>>>>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Send initial session list
    if let Ok(sessions) = storage.get_active_sessions(100).await {
        let unified: Vec<UnifiedSessionState> = sessions.iter().map(|s| s.into()).collect();
        let message = BridgeMessage::SessionsList { sessions: unified };
        let json = serde_json::to_string(&message)? + "\n";
        writer.write_all(json.as_bytes()).await?;
    }

    loop {
        tokio::select! {
            // Handle incoming messages from terminit
            result = reader.read_line(&mut line) => {
                match result {
                    Ok(0) => break, // Connection closed
                    Ok(_) => {
                        if let Ok(message) = serde_json::from_str::<BridgeMessage>(&line) {
                            let response = handle_message(message, &storage).await;
                            if let Some(resp) = response {
                                let json = serde_json::to_string(&resp)? + "\n";
                                writer.write_all(json.as_bytes()).await?;
                            }
                        }
                        line.clear();
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }

            // Forward outgoing messages to terminit
            result = outgoing_rx.recv() => {
                match result {
                    Ok(message) => {
                        let json = serde_json::to_string(&message)? + "\n";
                        if writer.write_all(json.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    debug!("Terminit client disconnected");
    Ok(())
}

/// Handle an incoming message from terminit.
async fn handle_message(message: BridgeMessage, storage: &Storage) -> Option<BridgeMessage> {
    match message {
        BridgeMessage::Ping => Some(BridgeMessage::Pong),

        BridgeMessage::GetSessions => {
            match storage.get_active_sessions(100).await {
                Ok(sessions) => {
                    let unified: Vec<UnifiedSessionState> = sessions.iter().map(|s| s.into()).collect();
                    Some(BridgeMessage::SessionsList { sessions: unified })
                }
                Err(e) => Some(BridgeMessage::Error {
                    code: "storage_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        BridgeMessage::Subscribe { session_id } => {
            debug!("Client subscribed to session: {:?}", session_id);
            None // Subscription is handled implicitly via broadcast
        }

        BridgeMessage::Unsubscribe { session_id } => {
            debug!("Client unsubscribed from session: {:?}", session_id);
            None
        }

        // Ignore other message types (they're outgoing)
        _ => None,
    }
}

/// Helper to create a bridge with default configuration.
pub fn create_default_bridge(storage: Storage, event_bus: EventBus) -> TerminitBridge {
    TerminitBridge::new(BridgeConfig::default(), storage, event_bus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_config_default() {
        let config = BridgeConfig::default();
        assert!(config.auto_connect);
        assert_eq!(config.reconnect_interval, 5);
        assert_eq!(config.event_buffer_size, 1000);
    }
}
