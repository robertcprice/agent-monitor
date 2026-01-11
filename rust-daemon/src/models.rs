//! Data models for agent monitoring.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Types of AI agents that can be monitored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    ClaudeCode,
    Cursor,
    Aider,
    GeminiCli,
    OpenaiCodex,
    Custom,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentType::ClaudeCode => write!(f, "claude_code"),
            AgentType::Cursor => write!(f, "cursor"),
            AgentType::Aider => write!(f, "aider"),
            AgentType::GeminiCli => write!(f, "gemini_cli"),
            AgentType::OpenaiCodex => write!(f, "openai_codex"),
            AgentType::Custom => write!(f, "custom"),
        }
    }
}

/// Status of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Idle,
    Completed,
    Crashed,
    Unknown,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Idle => write!(f, "idle"),
            SessionStatus::Completed => write!(f, "completed"),
            SessionStatus::Crashed => write!(f, "crashed"),
            SessionStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Types of events in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStart,
    SessionEnd,
    PromptReceived,
    ResponseGenerated,
    Thinking,
    ToolStart,
    ToolComplete,
    ToolExecuted,
    FileRead,
    FileModified,
    Error,
    Custom,
}

/// A unified session across all agent types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent_type: AgentType,
    pub external_id: String,
    pub project_path: String,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: f64,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub file_operations: i64,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub estimated_cost: f64,
    pub model_id: Option<String>,
    pub pid: Option<i32>,
    pub current_task: Option<String>,
    pub progress: f64,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Session {
    /// Create a new session.
    pub fn new(agent_type: AgentType, project_path: &str, external_id: &str) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            agent_type,
            external_id: external_id.to_string(),
            project_path: project_path.to_string(),
            status: SessionStatus::Active,
            started_at: now,
            last_activity_at: now,
            ended_at: None,
            duration_seconds: 0.0,
            message_count: 0,
            tool_call_count: 0,
            file_operations: 0,
            tokens_input: 0,
            tokens_output: 0,
            estimated_cost: 0.0,
            model_id: None,
            pid: None,
            current_task: None,
            progress: 0.0,
            metadata: HashMap::new(),
        }
    }

    /// Update the last activity timestamp.
    pub fn update_activity(&mut self) {
        self.last_activity_at = Utc::now();
        self.duration_seconds = (self.last_activity_at - self.started_at).num_seconds() as f64;
    }

    /// End the session.
    pub fn end(&mut self) {
        let now = Utc::now();
        self.ended_at = Some(now);
        self.last_activity_at = now;
        self.duration_seconds = (now - self.started_at).num_seconds() as f64;
    }
}

/// An event within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    pub agent_type: AgentType,
    pub content: Option<String>,
    pub working_directory: Option<String>,
    pub tool_name: Option<String>,
    pub file_path: Option<String>,
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    pub error_message: Option<String>,
    pub raw_data: Option<serde_json::Value>,
}

impl SessionEvent {
    /// Create a new event with random ID.
    pub fn new(session_id: &str, event_type: EventType, agent_type: AgentType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            event_type,
            timestamp: Utc::now(),
            agent_type,
            content: None,
            working_directory: None,
            tool_name: None,
            file_path: None,
            tokens_input: None,
            tokens_output: None,
            error_message: None,
            raw_data: None,
        }
    }

    /// Create a new event with a deterministic ID based on content hash.
    /// This prevents duplicate events when re-parsing log files.
    pub fn new_with_stable_id(
        session_id: &str,
        event_type: EventType,
        agent_type: AgentType,
        timestamp: DateTime<Utc>,
        content: Option<&str>,
    ) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Create stable ID from session + timestamp + type + FULL content hash
        let mut hasher = DefaultHasher::new();
        session_id.hash(&mut hasher);
        timestamp.timestamp_millis().hash(&mut hasher);
        format!("{:?}", event_type).hash(&mut hasher);
        if let Some(c) = content {
            // Hash the FULL content to differentiate similar messages at same timestamp
            c.hash(&mut hasher);
        }
        let hash = hasher.finish();
        let stable_id = format!("evt_{:016x}", hash);

        Self {
            id: stable_id,
            session_id: session_id.to_string(),
            event_type,
            timestamp,
            agent_type,
            content: content.map(|s| s.to_string()),
            working_directory: None,
            tool_name: None,
            file_path: None,
            tokens_input: None,
            tokens_output: None,
            error_message: None,
            raw_data: None,
        }
    }
}

/// Summary metrics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SummaryMetrics {
    pub total_sessions: i64,
    pub active_sessions: i64,
    pub total_messages: i64,
    pub total_tools: i64,
    pub total_cost: f64,
    pub today_messages: i64,
}
