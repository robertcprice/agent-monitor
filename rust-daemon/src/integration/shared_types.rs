//! Shared types for integration between agent-monitor and terminit.
//!
//! These types provide a common interface that both systems can use
//! for interoperability.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::models::{EventType, Session, SessionEvent};

/// Unified agent event that both systems understand.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_kind")]
pub enum UnifiedAgentEvent {
    /// Session lifecycle events
    SessionStarted {
        session_id: String,
        agent_type: String,
        project_path: String,
        timestamp: DateTime<Utc>,
    },
    SessionEnded {
        session_id: String,
        reason: String,
        timestamp: DateTime<Utc>,
    },

    /// Prompt and response events
    PromptReceived {
        session_id: String,
        content_preview: String,
        timestamp: DateTime<Utc>,
    },
    ResponseGenerated {
        session_id: String,
        content_preview: String,
        tokens: Option<TokenUsage>,
        timestamp: DateTime<Utc>,
    },

    /// Thinking and reasoning events
    Thinking {
        session_id: String,
        content_preview: String,
        timestamp: DateTime<Utc>,
    },

    /// Tool usage events
    ToolStarted {
        session_id: String,
        tool_name: String,
        tool_input_preview: String,
        timestamp: DateTime<Utc>,
    },
    ToolCompleted {
        session_id: String,
        tool_name: String,
        success: bool,
        duration_ms: Option<u64>,
        timestamp: DateTime<Utc>,
    },

    /// File operations
    FileRead {
        session_id: String,
        file_path: String,
        timestamp: DateTime<Utc>,
    },
    FileWritten {
        session_id: String,
        file_path: String,
        lines_changed: Option<u32>,
        timestamp: DateTime<Utc>,
    },

    /// Error events
    Error {
        session_id: String,
        error_type: String,
        message: String,
        timestamp: DateTime<Utc>,
    },

    /// Raw/custom events
    Custom {
        session_id: String,
        event_type: String,
        data: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
}

/// Token usage information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
}

/// Unified session state that both systems can use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSessionState {
    pub id: String,
    pub agent_type: String,
    pub project_path: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub tokens: TokenUsage,
    pub estimated_cost: f64,
    pub model_id: Option<String>,
    pub terminal_id: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Convert agent-monitor Session to unified format.
impl From<&Session> for UnifiedSessionState {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            agent_type: session.agent_type.to_string(),
            project_path: session.project_path.clone(),
            status: session.status.to_string(),
            started_at: session.started_at,
            last_activity: session.last_activity_at,
            message_count: session.message_count,
            tool_call_count: session.tool_call_count,
            tokens: TokenUsage {
                input_tokens: session.tokens_input,
                output_tokens: session.tokens_output,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            estimated_cost: session.estimated_cost,
            model_id: session.model_id.clone(),
            terminal_id: session.external_id.clone().into(),
            metadata: session.metadata.clone(),
        }
    }
}

/// Convert agent-monitor SessionEvent to unified format.
impl From<&SessionEvent> for UnifiedAgentEvent {
    fn from(event: &SessionEvent) -> Self {
        let session_id = event.session_id.clone();
        let timestamp = event.timestamp;

        match event.event_type {
            EventType::SessionStart => UnifiedAgentEvent::SessionStarted {
                session_id,
                agent_type: event.agent_type.to_string(),
                project_path: event.working_directory.clone().unwrap_or_default(),
                timestamp,
            },
            EventType::SessionEnd => UnifiedAgentEvent::SessionEnded {
                session_id,
                reason: "completed".to_string(),
                timestamp,
            },
            EventType::PromptReceived => UnifiedAgentEvent::PromptReceived {
                session_id,
                content_preview: event.content.clone().unwrap_or_default(),
                timestamp,
            },
            EventType::ResponseGenerated => UnifiedAgentEvent::ResponseGenerated {
                session_id,
                content_preview: event.content.clone().unwrap_or_default(),
                tokens: event.tokens_input.map(|input| TokenUsage {
                    input_tokens: input,
                    output_tokens: event.tokens_output.unwrap_or(0),
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
                timestamp,
            },
            EventType::Thinking => UnifiedAgentEvent::Thinking {
                session_id,
                content_preview: event.content.clone().unwrap_or_default(),
                timestamp,
            },
            EventType::ToolStart => UnifiedAgentEvent::ToolStarted {
                session_id,
                tool_name: event.tool_name.clone().unwrap_or_default(),
                tool_input_preview: event.content.clone().unwrap_or_default(),
                timestamp,
            },
            EventType::ToolComplete | EventType::ToolExecuted => UnifiedAgentEvent::ToolCompleted {
                session_id,
                tool_name: event.tool_name.clone().unwrap_or_default(),
                success: true,
                duration_ms: None,
                timestamp,
            },
            EventType::FileRead => UnifiedAgentEvent::FileRead {
                session_id,
                file_path: event.file_path.clone().unwrap_or_default(),
                timestamp,
            },
            EventType::FileModified => UnifiedAgentEvent::FileWritten {
                session_id,
                file_path: event.file_path.clone().unwrap_or_default(),
                lines_changed: None,
                timestamp,
            },
            EventType::Error => UnifiedAgentEvent::Error {
                session_id,
                error_type: "unknown".to_string(),
                message: event.error_message.clone().unwrap_or_default(),
                timestamp,
            },
            EventType::Custom => UnifiedAgentEvent::Custom {
                session_id,
                event_type: "custom".to_string(),
                data: event.raw_data.clone().unwrap_or(serde_json::json!({})),
                timestamp,
            },
        }
    }
}

/// Message format for IPC between agent-monitor and terminit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "message_type")]
pub enum BridgeMessage {
    /// Session update from agent-monitor to terminit
    SessionUpdate { session: UnifiedSessionState },

    /// Event notification
    EventNotification { event: UnifiedAgentEvent },

    /// Request for current sessions
    GetSessions,

    /// Response with current sessions
    SessionsList { sessions: Vec<UnifiedSessionState> },

    /// Subscribe to events for a session
    Subscribe { session_id: Option<String> },

    /// Unsubscribe from events
    Unsubscribe { session_id: Option<String> },

    /// Ping/pong for connection health
    Ping,
    Pong,

    /// Error response
    Error { code: String, message: String },
}

/// Configuration for the bridge connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    /// Path to terminit socket (if using Unix socket)
    pub terminit_socket: Option<String>,

    /// Port for TCP connection (if using TCP)
    pub terminit_port: Option<u16>,

    /// Whether to auto-connect on startup
    pub auto_connect: bool,

    /// Reconnection interval in seconds
    pub reconnect_interval: u64,

    /// Event buffer size
    pub event_buffer_size: usize,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            terminit_socket: Some("/tmp/terminit.sock".to_string()),
            terminit_port: Some(9876),
            auto_connect: true,
            reconnect_interval: 5,
            event_buffer_size: 1000,
        }
    }
}
