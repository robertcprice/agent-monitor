//! Event bus for distributing events to subscribers.

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::models::SessionEvent;

/// Event bus for distributing session events.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<SessionEvent>,
    _receiver: Arc<RwLock<broadcast::Receiver<SessionEvent>>>,
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> Self {
        let (sender, receiver) = broadcast::channel(1000);
        Self {
            sender,
            _receiver: Arc::new(RwLock::new(receiver)),
        }
    }

    /// Publish an event to all subscribers.
    pub fn publish(&self, event: SessionEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
