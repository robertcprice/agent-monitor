//! SQLite storage layer for session and event persistence.

use anyhow::Result;
use sqlx::{sqlite::SqlitePool, Row};
use std::path::Path;
use std::sync::Arc;

use crate::models::{Session, SessionEvent, SessionStatus, AgentType, SummaryMetrics};

/// Storage manager for session data.
#[derive(Clone)]
pub struct Storage {
    pool: Arc<SqlitePool>,
}

impl Storage {
    /// Create a new storage instance.
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;

        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// Initialize the database schema.
    pub async fn initialize(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                agent_type TEXT NOT NULL,
                external_id TEXT NOT NULL,
                project_path TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'unknown',
                started_at TEXT NOT NULL,
                last_activity_at TEXT NOT NULL,
                ended_at TEXT,
                duration_seconds REAL DEFAULT 0,
                message_count INTEGER DEFAULT 0,
                tool_call_count INTEGER DEFAULT 0,
                file_operations INTEGER DEFAULT 0,
                tokens_input INTEGER DEFAULT 0,
                tokens_output INTEGER DEFAULT 0,
                estimated_cost REAL DEFAULT 0,
                model_id TEXT,
                pid INTEGER,
                current_task TEXT,
                progress REAL DEFAULT 0,
                metadata_json TEXT DEFAULT '{}',
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&*self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                agent_type TEXT NOT NULL,
                content TEXT,
                working_directory TEXT,
                tool_name TEXT,
                file_path TEXT,
                tokens_input INTEGER,
                tokens_output INTEGER,
                error_message TEXT,
                raw_data_json TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&*self.pool)
        .await?;

        // Create indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status)")
            .execute(&*self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_agent_type ON sessions(agent_type)")
            .execute(&*self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_session_id ON session_events(session_id)")
            .execute(&*self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_timestamp ON session_events(timestamp)")
            .execute(&*self.pool)
            .await?;

        Ok(())
    }

    /// Insert or update a session.
    pub async fn upsert_session(&self, session: &Session) -> Result<()> {
        let metadata_json = serde_json::to_string(&session.metadata)?;

        sqlx::query(
            r#"
            INSERT INTO sessions (
                id, agent_type, external_id, project_path, status,
                started_at, last_activity_at, ended_at, duration_seconds,
                message_count, tool_call_count, file_operations,
                tokens_input, tokens_output, estimated_cost,
                model_id, pid, current_task, progress, metadata_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                last_activity_at = excluded.last_activity_at,
                ended_at = excluded.ended_at,
                duration_seconds = excluded.duration_seconds,
                message_count = excluded.message_count,
                tool_call_count = excluded.tool_call_count,
                file_operations = excluded.file_operations,
                tokens_input = excluded.tokens_input,
                tokens_output = excluded.tokens_output,
                estimated_cost = excluded.estimated_cost,
                current_task = excluded.current_task,
                progress = excluded.progress,
                metadata_json = excluded.metadata_json,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&session.id)
        .bind(session.agent_type.to_string())
        .bind(&session.external_id)
        .bind(&session.project_path)
        .bind(session.status.to_string())
        .bind(session.started_at.to_rfc3339())
        .bind(session.last_activity_at.to_rfc3339())
        .bind(session.ended_at.map(|t| t.to_rfc3339()))
        .bind(session.duration_seconds)
        .bind(session.message_count)
        .bind(session.tool_call_count)
        .bind(session.file_operations)
        .bind(session.tokens_input)
        .bind(session.tokens_output)
        .bind(session.estimated_cost)
        .bind(&session.model_id)
        .bind(session.pid)
        .bind(&session.current_task)
        .bind(session.progress)
        .bind(&metadata_json)
        .execute(&*self.pool)
        .await?;

        Ok(())
    }

    /// Get active sessions.
    pub async fn get_active_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM sessions
            WHERE status = 'active'
            ORDER BY last_activity_at DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&*self.pool)
        .await?;

        let sessions = rows
            .iter()
            .filter_map(|row| self.row_to_session(row).ok())
            .collect();

        Ok(sessions)
    }

    /// Get a single session by ID.
    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let row = sqlx::query(
            r#"
            SELECT * FROM sessions WHERE id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&*self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(self.row_to_session(&r)?)),
            None => Ok(None),
        }
    }

    /// Get recent sessions.
    pub async fn get_recent_sessions(&self, hours: i64, limit: usize) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM sessions
            WHERE datetime(last_activity_at) > datetime('now', ? || ' hours')
            ORDER BY last_activity_at DESC
            LIMIT ?
            "#,
        )
        .bind(-hours)
        .bind(limit as i64)
        .fetch_all(&*self.pool)
        .await?;

        let sessions = rows
            .iter()
            .filter_map(|row| self.row_to_session(row).ok())
            .collect();

        Ok(sessions)
    }

    /// Get summary metrics.
    pub async fn get_summary_metrics(&self, hours: i64) -> Result<SummaryMetrics> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) as total_sessions,
                SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END) as active_sessions,
                SUM(message_count) as total_messages,
                SUM(tool_call_count) as total_tools,
                SUM(estimated_cost) as total_cost
            FROM sessions
            WHERE datetime(last_activity_at) > datetime('now', ? || ' hours')
            "#,
        )
        .bind(-hours)
        .fetch_one(&*self.pool)
        .await?;

        Ok(SummaryMetrics {
            total_sessions: row.get::<i64, _>("total_sessions"),
            active_sessions: row.get::<i64, _>("active_sessions"),
            total_messages: row.get::<Option<i64>, _>("total_messages").unwrap_or(0),
            total_tools: row.get::<Option<i64>, _>("total_tools").unwrap_or(0),
            total_cost: row.get::<Option<f64>, _>("total_cost").unwrap_or(0.0),
            today_messages: 0, // TODO: Calculate from today
        })
    }

    /// Insert an event (ignores duplicates based on ID).
    pub async fn insert_event(&self, event: &SessionEvent) -> Result<()> {
        let raw_data_json = event
            .raw_data
            .as_ref()
            .map(|d| serde_json::to_string(d).unwrap_or_default());

        sqlx::query(
            r#"
            INSERT OR IGNORE INTO session_events (
                id, session_id, event_type, timestamp, agent_type,
                content, working_directory, tool_name, file_path,
                tokens_input, tokens_output, error_message, raw_data_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&event.id)
        .bind(&event.session_id)
        .bind(format!("{:?}", event.event_type).to_lowercase())
        .bind(event.timestamp.to_rfc3339())
        .bind(event.agent_type.to_string())
        .bind(&event.content)
        .bind(&event.working_directory)
        .bind(&event.tool_name)
        .bind(&event.file_path)
        .bind(event.tokens_input)
        .bind(event.tokens_output)
        .bind(&event.error_message)
        .bind(&raw_data_json)
        .execute(&*self.pool)
        .await?;

        Ok(())
    }

    /// Get recent events.
    pub async fn get_recent_events(&self, limit: usize) -> Result<Vec<SessionEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM session_events
            ORDER BY timestamp DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&*self.pool)
        .await?;

        let events = rows
            .iter()
            .filter_map(|row| self.row_to_event(row).ok())
            .collect();

        Ok(events)
    }

    /// Get events for a specific session (newest first).
    pub async fn get_session_events(&self, session_id: &str, limit: usize) -> Result<Vec<SessionEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM session_events
            WHERE session_id = ?
            ORDER BY timestamp DESC
            LIMIT ?
            "#,
        )
        .bind(session_id)
        .bind(limit as i64)
        .fetch_all(&*self.pool)
        .await?;

        let events = rows
            .iter()
            .filter_map(|row| self.row_to_event(row).ok())
            .collect();

        Ok(events)
    }

    /// Delete all sessions by agent type.
    pub async fn delete_sessions_by_type(&self, agent_type: &str) -> Result<i64> {
        // First delete related events
        sqlx::query(
            r#"
            DELETE FROM session_events
            WHERE session_id IN (SELECT id FROM sessions WHERE agent_type = ?)
            "#,
        )
        .bind(agent_type)
        .execute(&*self.pool)
        .await?;

        // Then delete sessions
        let result = sqlx::query(
            r#"
            DELETE FROM sessions WHERE agent_type = ?
            "#,
        )
        .bind(agent_type)
        .execute(&*self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }

    /// Clear all sessions and events.
    pub async fn clear_all(&self) -> Result<()> {
        sqlx::query("DELETE FROM session_events")
            .execute(&*self.pool)
            .await?;
        sqlx::query("DELETE FROM sessions")
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    fn row_to_session(&self, row: &sqlx::sqlite::SqliteRow) -> Result<Session> {
        use chrono::DateTime;

        let metadata_json: String = row.get("metadata_json");
        let metadata = serde_json::from_str(&metadata_json).unwrap_or_default();

        let status_str: String = row.get("status");
        let status = match status_str.as_str() {
            "active" => SessionStatus::Active,
            "idle" => SessionStatus::Idle,
            "completed" => SessionStatus::Completed,
            "crashed" => SessionStatus::Crashed,
            _ => SessionStatus::Unknown,
        };

        let agent_type_str: String = row.get("agent_type");
        let agent_type = match agent_type_str.as_str() {
            "claude_code" => AgentType::ClaudeCode,
            "cursor" => AgentType::Cursor,
            "aider" => AgentType::Aider,
            _ => AgentType::Custom,
        };

        let started_at_str: String = row.get("started_at");
        let last_activity_str: String = row.get("last_activity_at");
        let ended_at_str: Option<String> = row.get("ended_at");

        Ok(Session {
            id: row.get("id"),
            agent_type,
            external_id: row.get("external_id"),
            project_path: row.get("project_path"),
            status,
            started_at: DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&chrono::Utc),
            last_activity_at: DateTime::parse_from_rfc3339(&last_activity_str)?
                .with_timezone(&chrono::Utc),
            ended_at: ended_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&chrono::Utc)),
            duration_seconds: row.get("duration_seconds"),
            message_count: row.get("message_count"),
            tool_call_count: row.get("tool_call_count"),
            file_operations: row.get("file_operations"),
            tokens_input: row.get("tokens_input"),
            tokens_output: row.get("tokens_output"),
            estimated_cost: row.get("estimated_cost"),
            model_id: row.get("model_id"),
            pid: row.get("pid"),
            current_task: row.get("current_task"),
            progress: row.get("progress"),
            metadata,
        })
    }

    fn row_to_event(&self, row: &sqlx::sqlite::SqliteRow) -> Result<SessionEvent> {
        use crate::models::EventType;
        use chrono::DateTime;

        let event_type_str: String = row.get("event_type");
        let event_type = match event_type_str.as_str() {
            "sessionstart" | "session_start" => EventType::SessionStart,
            "sessionend" | "session_end" => EventType::SessionEnd,
            "promptreceived" | "prompt_received" => EventType::PromptReceived,
            "responsegenerated" | "response_generated" => EventType::ResponseGenerated,
            "thinking" => EventType::Thinking,
            "toolstart" | "tool_start" => EventType::ToolStart,
            "toolcomplete" | "tool_complete" => EventType::ToolComplete,
            "toolexecuted" | "tool_executed" => EventType::ToolExecuted,
            "fileread" | "file_read" => EventType::FileRead,
            "filemodified" | "file_modified" => EventType::FileModified,
            "error" => EventType::Error,
            _ => EventType::Custom,
        };

        let agent_type_str: String = row.get("agent_type");
        let agent_type = match agent_type_str.as_str() {
            "claude_code" => AgentType::ClaudeCode,
            "cursor" => AgentType::Cursor,
            "aider" => AgentType::Aider,
            _ => AgentType::Custom,
        };

        let timestamp_str: String = row.get("timestamp");
        let raw_data_json: Option<String> = row.get("raw_data_json");
        let raw_data = raw_data_json.and_then(|s| serde_json::from_str(&s).ok());

        Ok(SessionEvent {
            id: row.get("id"),
            session_id: row.get("session_id"),
            event_type,
            timestamp: DateTime::parse_from_rfc3339(&timestamp_str)?.with_timezone(&chrono::Utc),
            agent_type,
            content: row.get("content"),
            working_directory: row.get("working_directory"),
            tool_name: row.get("tool_name"),
            file_path: row.get("file_path"),
            tokens_input: row.get("tokens_input"),
            tokens_output: row.get("tokens_output"),
            error_message: row.get("error_message"),
            raw_data,
        })
    }
}
