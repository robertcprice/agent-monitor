//! Agent adapters for monitoring different AI tools.

use anyhow::Result;
use async_trait::async_trait;
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use sysinfo::System;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::events::EventBus;
use crate::models::{AgentType, EventType, Session, SessionEvent, SessionStatus};
use crate::storage::Storage;

/// Trait for agent adapters.
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Get adapter name.
    fn name(&self) -> &str;

    /// Get agent type.
    fn agent_type(&self) -> AgentType;

    /// Start the adapter.
    async fn start(&mut self) -> Result<()>;

    /// Stop the adapter.
    async fn stop(&mut self) -> Result<()>;

    /// Discover existing sessions.
    async fn discover_sessions(&self) -> Result<Vec<Session>>;

    /// Get adapter capabilities.
    fn capabilities(&self) -> HashMap<String, bool>;
}

/// Registry of all adapters.
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn Adapter>>,
    config: Config,
    event_bus: EventBus,
    storage: Storage,
}

impl AdapterRegistry {
    /// Create a new adapter registry.
    pub fn new(config: &Config, event_bus: EventBus, storage: Storage) -> Self {
        Self {
            adapters: Vec::new(),
            config: config.clone(),
            event_bus,
            storage,
        }
    }

    /// Register the Claude Code adapter.
    pub async fn register_claude_code(&mut self) -> Result<()> {
        let adapter = ClaudeCodeAdapter::new(
            &self.config,
            self.event_bus.clone(),
            self.storage.clone(),
        );
        self.adapters.push(Box::new(adapter));
        Ok(())
    }

    /// Register the Cursor adapter.
    pub async fn register_cursor(&mut self) -> Result<()> {
        let adapter = CursorAdapter::new(
            &self.config,
            self.event_bus.clone(),
            self.storage.clone(),
        );
        self.adapters.push(Box::new(adapter));
        Ok(())
    }

    /// Register the Aider adapter.
    pub async fn register_aider(&mut self) -> Result<()> {
        let adapter = AiderAdapter::new(
            &self.config,
            self.event_bus.clone(),
            self.storage.clone(),
        );
        self.adapters.push(Box::new(adapter));
        Ok(())
    }

    /// Register all available adapters.
    pub async fn register_all(&mut self) -> Result<()> {
        self.register_claude_code().await?;
        // Cursor adapter disabled - causes false positives detecting any Cursor process as AI session
        // TODO: Re-enable when Cursor adapter properly detects actual AI agent sessions
        // self.register_cursor().await?;
        self.register_aider().await?;
        Ok(())
    }

    /// Start all adapters.
    pub async fn start_all(&mut self) -> Result<()> {
        for adapter in &mut self.adapters {
            info!("Starting adapter: {}", adapter.name());
            adapter.start().await?;
        }
        Ok(())
    }

    /// Stop all adapters.
    pub async fn stop_all(&mut self) -> Result<()> {
        for adapter in &mut self.adapters {
            info!("Stopping adapter: {}", adapter.name());
            adapter.stop().await?;
        }
        Ok(())
    }
}

/// Claude Code adapter with file watching and process detection.
pub struct ClaudeCodeAdapter {
    claude_home: PathBuf,
    history_file: PathBuf,
    projects_dir: PathBuf,
    event_bus: EventBus,
    storage: Storage,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    running: Arc<RwLock<bool>>,
    /// Track the last read position in history file
    last_history_pos: Arc<RwLock<u64>>,
    /// Sender to stop file watcher
    watcher_stop_tx: Option<mpsc::Sender<()>>,
}

impl ClaudeCodeAdapter {
    /// Create a new Claude Code adapter.
    pub fn new(config: &Config, event_bus: EventBus, storage: Storage) -> Self {
        Self {
            claude_home: config.claude_home.clone(),
            history_file: config.claude_home.join("history.jsonl"),
            projects_dir: config.claude_home.join("projects"),
            event_bus,
            storage,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            last_history_pos: Arc::new(RwLock::new(0)),
            watcher_stop_tx: None,
        }
    }

    /// Start the real-time file watcher for Claude Code directories.
    fn start_file_watcher(
        claude_home: PathBuf,
        history_file: PathBuf,
        projects_dir: PathBuf,
        storage: Storage,
        event_bus: EventBus,
        sessions: Arc<RwLock<HashMap<String, Session>>>,
        last_history_pos: Arc<RwLock<u64>>,
        mut stop_rx: mpsc::Receiver<()>,
    ) {
        tokio::spawn(async move {
            // Channel for file events
            let (tx, mut rx) = mpsc::channel::<Event>(100);

            // Create the watcher
            let watcher_result: Result<RecommendedWatcher, notify::Error> = {
                let tx = tx.clone();
                Watcher::new(
                    move |res: Result<Event, notify::Error>| {
                        if let Ok(event) = res {
                            let _ = tx.blocking_send(event);
                        }
                    },
                    NotifyConfig::default(),
                )
            };

            let mut watcher = match watcher_result {
                Ok(w) => w,
                Err(e) => {
                    error!("Failed to create file watcher: {}", e);
                    return;
                }
            };

            // Watch the Claude home directory
            if let Err(e) = watcher.watch(&claude_home, RecursiveMode::Recursive) {
                warn!("Failed to watch Claude home directory: {}", e);
            } else {
                info!("📁 Watching: {:?}", claude_home);
            }

            // Also watch projects directory if it exists
            if projects_dir.exists() {
                if let Err(e) = watcher.watch(&projects_dir, RecursiveMode::Recursive) {
                    warn!("Failed to watch projects directory: {}", e);
                } else {
                    info!("📁 Watching: {:?}", projects_dir);
                }
            }

            // Initialize history position to end of file
            if history_file.exists() {
                if let Ok(metadata) = std::fs::metadata(&history_file) {
                    *last_history_pos.write().await = metadata.len();
                }
            }

            info!("✦ File watcher started");

            loop {
                tokio::select! {
                    // Check for stop signal
                    _ = stop_rx.recv() => {
                        info!("File watcher stopping...");
                        break;
                    }
                    // Handle file events
                    Some(event) = rx.recv() => {
                        Self::handle_file_event(
                            event,
                            &history_file,
                            &storage,
                            &event_bus,
                            &sessions,
                            &last_history_pos,
                        ).await;
                    }
                }
            }
        });
    }

    /// Handle a file system event.
    async fn handle_file_event(
        event: Event,
        history_file: &PathBuf,
        storage: &Storage,
        event_bus: &EventBus,
        sessions: &Arc<RwLock<HashMap<String, Session>>>,
        last_history_pos: &Arc<RwLock<u64>>,
    ) {
        use notify::EventKind;

        // Check if this is a modify event
        if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
            return;
        }

        for path in &event.paths {
            // Process history.jsonl
            if path == history_file {
                debug!("History file changed, reading new entries...");
                if let Err(e) = Self::process_file_changes(
                    path,
                    storage,
                    event_bus,
                    sessions,
                    last_history_pos,
                ).await {
                    warn!("Error processing history changes: {}", e);
                }
            }
            // Process project session JSONL files
            else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                // Only process if it's in a projects directory
                if path.to_string_lossy().contains("/projects/") {
                    debug!("Project session file changed: {:?}", path);
                    if let Err(e) = Self::process_file_changes(
                        path,
                        storage,
                        event_bus,
                        sessions,
                        last_history_pos,
                    ).await {
                        warn!("Error processing project session: {}", e);
                    }
                }
            }
        }
    }

    /// Process changes from any JSONL file (history or project session).
    async fn process_file_changes(
        file_path: &PathBuf,
        storage: &Storage,
        event_bus: &EventBus,
        sessions: &Arc<RwLock<HashMap<String, Session>>>,
        _last_history_pos: &Arc<RwLock<u64>>,
    ) -> Result<()> {
        use std::io::{BufRead, BufReader};

        if !file_path.exists() {
            return Ok(());
        }

        let file = std::fs::File::open(file_path)?;
        let reader = BufReader::new(file);

        // Read last 50 lines for incremental updates
        let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
        let start = lines.len().saturating_sub(50);

        for line in &lines[start..] {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(entry) = serde_json::from_str::<Value>(line) {
                Self::process_entry(&entry, storage, event_bus, sessions).await;
            }
        }

        Ok(())
    }

    /// Process a single JSON entry from any source.
    async fn process_entry(
        entry: &Value,
        storage: &Storage,
        event_bus: &EventBus,
        sessions: &Arc<RwLock<HashMap<String, Session>>>,
    ) {
        // Support both history.jsonl format (project) and session file format (cwd)
        let project = entry.get("cwd")
            .or_else(|| entry.get("project"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let session_id = entry.get("sessionId").and_then(|v| v.as_str()).unwrap_or("");
        let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Skip file-history-snapshot entries
        if msg_type == "file-history-snapshot" {
            return;
        }

        if project.is_empty() {
            return;
        }

        let mut sessions_guard = sessions.write().await;

        let session = sessions_guard
            .entry(project.to_string())
            .or_insert_with(|| {
                let mut s = Session::new(AgentType::ClaudeCode, project, session_id);
                s.metadata.insert(
                    "source".to_string(),
                    serde_json::Value::String("file_watch".to_string()),
                );
                s
            });

        session.message_count += 1;
        session.update_activity();
        session.status = SessionStatus::Active;

        // Extract token information from message.usage (new format)
        if let Some(message) = entry.get("message") {
            if let Some(usage) = message.get("usage") {
                if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_i64()) {
                    session.tokens_input += input;
                }
                if let Some(output) = usage.get("output_tokens").and_then(|v| v.as_i64()) {
                    session.tokens_output += output;
                }
            }
            // Extract model ID
            if session.model_id.is_none() {
                if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
                    session.model_id = Some(model.to_string());
                }
            }
        }

        // Calculate cost
        let input_cost = session.tokens_input as f64 * 3.0 / 1_000_000.0;
        let output_cost = session.tokens_output as f64 * 15.0 / 1_000_000.0;
        session.estimated_cost = input_cost + output_cost;

        // Count tool calls
        if msg_type == "assistant" {
            if let Some(message) = entry.get("message") {
                if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            session.tool_call_count += 1;
                        }
                    }
                }
            }
        }

        // Upsert to storage
        if let Err(e) = storage.upsert_session(session).await {
            warn!("Failed to upsert session: {}", e);
        }

        // Create and store event with stable ID to prevent duplicates
        let role = entry.get("message")
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            .unwrap_or(msg_type);

        let event_type = match role {
            "user" => EventType::PromptReceived,
            "assistant" => EventType::ResponseGenerated,
            _ => EventType::Custom,
        };

        // Extract timestamp from entry (format: "2026-01-05T18:56:29.954Z")
        let event_timestamp = entry.get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(|ts_str| chrono::DateTime::parse_from_rfc3339(ts_str).ok())
            .map(|ts| ts.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        // Build full content FIRST so we can use it for stable ID
        let mut full_content: Option<String> = None;
        let mut tool_name: Option<String> = None;

        if let Some(message) = entry.get("message") {
            // First check if content is a plain string (user messages often)
            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                full_content = Some(content.to_string());
            }
            // Content is an array - extract from different block types
            else if let Some(content_array) = message.get("content").and_then(|c| c.as_array()) {
                let mut text_parts: Vec<String> = Vec::new();
                for block in content_array {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match block_type {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                text_parts.push(text.to_string());
                            }
                        }
                        "thinking" => {
                            if let Some(thinking) = block.get("thinking").and_then(|t| t.as_str()) {
                                text_parts.push(format!("[THINKING]\n{}", thinking));
                            }
                        }
                        "tool_use" => {
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                            let input = block.get("input")
                                .map(|i| serde_json::to_string_pretty(i).unwrap_or_default())
                                .unwrap_or_default();
                            text_parts.push(format!("[TOOL: {}]\n{}", name, input));
                            tool_name = Some(name.to_string());
                        }
                        "tool_result" => {
                            if let Some(content) = block.get("content").and_then(|c| c.as_str()) {
                                text_parts.push(format!("[RESULT]\n{}", content));
                            }
                        }
                        _ => {}
                    }
                }
                if !text_parts.is_empty() {
                    full_content = Some(text_parts.join("\n\n"));
                }
            }
        }

        // Fallback for history.jsonl format
        if full_content.is_none() {
            if let Some(display) = entry.get("display").and_then(|v| v.as_str()) {
                full_content = Some(display.to_string());
            }
        }

        // Create event with stable ID based on session + timestamp + FULL content
        let mut event = SessionEvent::new_with_stable_id(
            &session.id,
            event_type,
            AgentType::ClaudeCode,
            event_timestamp,
            full_content.as_deref(),
        );
        event.working_directory = Some(project.to_string());
        event.tool_name = tool_name;

        // Extract token info
        if let Some(message) = entry.get("message") {
            if let Some(usage) = message.get("usage") {
                event.tokens_input = usage.get("input_tokens").and_then(|v| v.as_i64());
                event.tokens_output = usage.get("output_tokens").and_then(|v| v.as_i64());
            }
        }

        // Store and publish event
        if let Err(e) = storage.insert_event(&event).await {
            warn!("Failed to insert event: {}", e);
        }
        event_bus.publish(event);
    }

    /// Parse history.jsonl file.
    async fn parse_history(&self) -> Result<Vec<Session>> {
        use std::io::{BufRead, BufReader};

        let mut sessions: HashMap<String, Session> = HashMap::new();

        if !self.history_file.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(&self.history_file)?;
        let reader = BufReader::new(file);

        // Read last 1000 lines
        let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
        let start = if lines.len() > 1000 {
            lines.len() - 1000
        } else {
            0
        };

        for line in &lines[start..] {
            if let Ok(entry) = serde_json::from_str::<Value>(line) {
                let project = entry.get("project").and_then(|v| v.as_str()).unwrap_or("");
                let session_id = entry
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Parse timestamp - support both RFC3339 string and Unix milliseconds
                let timestamp_ms: i64 = if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
                    // RFC3339 format: "2026-01-05T18:56:29.954Z"
                    chrono::DateTime::parse_from_rfc3339(ts_str)
                        .map(|dt| dt.timestamp_millis())
                        .unwrap_or(0)
                } else {
                    // Fallback to i64 (Unix milliseconds)
                    entry.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0)
                };

                if !project.is_empty() {
                    let session = sessions.entry(project.to_string()).or_insert_with(|| {
                        let mut s = Session::new(AgentType::ClaudeCode, project, session_id);
                        s.metadata.insert(
                            "source".to_string(),
                            serde_json::Value::String("history".to_string()),
                        );
                        s
                    });

                    session.message_count += 1;
                    session.update_activity();

                    // Check if still active (activity in last 30 minutes)
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    if timestamp_ms > 0 && (now_ms - timestamp_ms) < 30 * 60 * 1000 {
                        session.status = SessionStatus::Active;
                    } else {
                        session.status = SessionStatus::Completed;
                    }
                }
            }
        }

        Ok(sessions.into_values().collect())
    }

    /// Find running Claude Code processes.
    async fn find_processes(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        let system = System::new_all();

        for (pid, process) in system.processes() {
            // sysinfo 0.30+ returns OsStr for name and OsString for cmd
            let name = format!("{:?}", process.name()).to_lowercase();
            let cmd: String = process.cmd()
                .iter()
                .map(|s| format!("{:?}", s))
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();

            if name.contains("claude") || cmd.contains("@anthropic-ai/claude-code") {
                let cwd = process.cwd()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                if !cwd.is_empty() {
                    let mut session =
                        Session::new(AgentType::ClaudeCode, &cwd, &format!("proc_{}", pid));
                    session.pid = Some(pid.as_u32() as i32);
                    session.metadata.insert(
                        "source".to_string(),
                        serde_json::Value::String("process".to_string()),
                    );
                    sessions.push(session);
                }
            }
        }

        Ok(sessions)
    }
}

#[async_trait]
impl Adapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "claude_code"
    }

    fn agent_type(&self) -> AgentType {
        AgentType::ClaudeCode
    }

    async fn start(&mut self) -> Result<()> {
        *self.running.write().await = true;

        // Initial discovery
        let sessions = self.discover_sessions().await?;
        for session in sessions {
            self.storage.upsert_session(&session).await?;
            self.sessions
                .write()
                .await
                .insert(session.id.clone(), session);
        }

        // Create stop channel for file watcher
        let (stop_tx, stop_rx) = mpsc::channel::<()>(1);
        self.watcher_stop_tx = Some(stop_tx);

        // Start the real file watcher
        Self::start_file_watcher(
            self.claude_home.clone(),
            self.history_file.clone(),
            self.projects_dir.clone(),
            self.storage.clone(),
            self.event_bus.clone(),
            self.sessions.clone(),
            self.last_history_pos.clone(),
            stop_rx,
        );

        // Also start a periodic process scanner (every 60 seconds)
        let storage = self.storage.clone();
        let sessions = self.sessions.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(60));

            while *running.read().await {
                interval.tick().await;

                // Scan for new processes
                let system = System::new_all();
                for (pid, process) in system.processes() {
                    let name = format!("{:?}", process.name()).to_lowercase();
                    let cmd: String = process.cmd()
                        .iter()
                        .map(|s| format!("{:?}", s))
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase();

                    if name.contains("claude") || cmd.contains("@anthropic-ai/claude-code") {
                        let cwd = process.cwd()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();

                        if !cwd.is_empty() {
                            let mut sessions_guard = sessions.write().await;
                            if !sessions_guard.contains_key(&cwd) {
                                let mut session = Session::new(
                                    AgentType::ClaudeCode,
                                    &cwd,
                                    &format!("proc_{}", pid),
                                );
                                session.pid = Some(pid.as_u32() as i32);
                                session.metadata.insert(
                                    "source".to_string(),
                                    serde_json::Value::String("process_scan".to_string()),
                                );

                                if let Err(e) = storage.upsert_session(&session).await {
                                    warn!("Failed to save process-detected session: {}", e);
                                }

                                sessions_guard.insert(cwd, session);
                            }
                        }
                    }
                }

                debug!("Process scan complete");
            }
        });

        info!("Claude Code adapter started with file watching");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.write().await = false;

        // Signal file watcher to stop
        if let Some(tx) = self.watcher_stop_tx.take() {
            let _ = tx.send(()).await;
        }

        info!("Claude Code adapter stopped");
        Ok(())
    }

    async fn discover_sessions(&self) -> Result<Vec<Session>> {
        let mut all_sessions = Vec::new();

        // From processes
        let proc_sessions = self.find_processes().await?;
        all_sessions.extend(proc_sessions);

        // From history
        let history_sessions = self.parse_history().await?;
        all_sessions.extend(history_sessions);

        // Deduplicate by project path
        let mut seen = std::collections::HashSet::new();
        all_sessions.retain(|s| seen.insert(s.project_path.clone()));

        Ok(all_sessions)
    }

    fn capabilities(&self) -> HashMap<String, bool> {
        let mut caps = HashMap::new();
        caps.insert("real_time_events".to_string(), true);
        caps.insert("historical_data".to_string(), true);
        caps.insert("token_tracking".to_string(), true);
        caps.insert("cost_tracking".to_string(), true);
        caps.insert("file_change_tracking".to_string(), true);
        caps.insert("transcript_access".to_string(), true);
        caps
    }
}

// ============================================================================
// Cursor Adapter
// ============================================================================

/// Cursor IDE adapter for monitoring AI-assisted coding sessions.
pub struct CursorAdapter {
    cursor_home: PathBuf,
    storage_dir: PathBuf,
    event_bus: EventBus,
    storage: Storage,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    running: Arc<RwLock<bool>>,
    watcher_stop_tx: Option<mpsc::Sender<()>>,
}

impl CursorAdapter {
    /// Create a new Cursor adapter.
    pub fn new(_config: &Config, event_bus: EventBus, storage: Storage) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        // Cursor stores data in different locations per platform
        #[cfg(target_os = "macos")]
        let cursor_home = home.join("Library/Application Support/Cursor");
        #[cfg(target_os = "linux")]
        let cursor_home = home.join(".config/Cursor");
        #[cfg(target_os = "windows")]
        let cursor_home = home.join("AppData/Roaming/Cursor");
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let cursor_home = home.join(".cursor");

        Self {
            storage_dir: cursor_home.join("User/globalStorage"),
            cursor_home,
            event_bus,
            storage,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            watcher_stop_tx: None,
        }
    }

    /// Find running Cursor processes.
    async fn find_processes(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        let system = System::new_all();

        for (pid, process) in system.processes() {
            let name = format!("{:?}", process.name()).to_lowercase();

            if name.contains("cursor") && !name.contains("cursorless") {
                let cwd = process.cwd()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                if !cwd.is_empty() && !cwd.contains("Application Support") {
                    let mut session = Session::new(
                        AgentType::Cursor,
                        &cwd,
                        &format!("cursor_{}", pid),
                    );
                    session.pid = Some(pid.as_u32() as i32);
                    session.metadata.insert(
                        "source".to_string(),
                        serde_json::Value::String("process".to_string()),
                    );
                    sessions.push(session);
                }
            }
        }

        Ok(sessions)
    }

    /// Parse Cursor workspace state files.
    async fn parse_workspace_state(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();

        // Cursor stores workspace state in SQLite databases
        let state_db = self.storage_dir.join("state.vscdb");
        if state_db.exists() {
            debug!("Found Cursor state database: {:?}", state_db);
            // Would need to query SQLite for recent workspaces
            // For now, we rely on process detection
        }

        // Also check for workspace storage
        let workspace_storage = self.storage_dir.join("workspaceStorage");
        if workspace_storage.exists() && workspace_storage.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&workspace_storage) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() {
                        // Each folder represents a workspace
                        let workspace_json = path.join("workspace.json");
                        if workspace_json.exists() {
                            if let Ok(content) = std::fs::read_to_string(&workspace_json) {
                                if let Ok(data) = serde_json::from_str::<Value>(&content) {
                                    if let Some(folder) = data.get("folder").and_then(|v| v.as_str()) {
                                        // Decode the folder path (it's URL encoded)
                                        let folder = folder.replace("file://", "");
                                        let folder = percent_encoding::percent_decode_str(&folder)
                                            .decode_utf8_lossy()
                                            .to_string();

                                        if !folder.is_empty() {
                                            let session = Session::new(
                                                AgentType::Cursor,
                                                &folder,
                                                &format!("workspace_{}", entry.file_name().to_string_lossy()),
                                            );
                                            sessions.push(session);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(sessions)
    }
}

#[async_trait]
impl Adapter for CursorAdapter {
    fn name(&self) -> &str {
        "cursor"
    }

    fn agent_type(&self) -> AgentType {
        AgentType::Cursor
    }

    async fn start(&mut self) -> Result<()> {
        *self.running.write().await = true;

        // Initial discovery
        let sessions = self.discover_sessions().await?;
        for session in sessions {
            self.storage.upsert_session(&session).await?;
            self.sessions.write().await.insert(session.id.clone(), session);
        }

        // Start periodic process scanner
        let storage = self.storage.clone();
        let sessions = self.sessions.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(30));

            while *running.read().await {
                interval.tick().await;

                let system = System::new_all();
                for (pid, process) in system.processes() {
                    let name = format!("{:?}", process.name()).to_lowercase();

                    if name.contains("cursor") && !name.contains("cursorless") {
                        let cwd = process.cwd()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();

                        if !cwd.is_empty() && !cwd.contains("Application Support") {
                            let mut sessions_guard = sessions.write().await;
                            if !sessions_guard.contains_key(&cwd) {
                                let mut session = Session::new(
                                    AgentType::Cursor,
                                    &cwd,
                                    &format!("cursor_{}", pid),
                                );
                                session.pid = Some(pid.as_u32() as i32);

                                if let Err(e) = storage.upsert_session(&session).await {
                                    warn!("Failed to save Cursor session: {}", e);
                                }

                                sessions_guard.insert(cwd, session);
                            }
                        }
                    }
                }
            }
        });

        info!("Cursor adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.write().await = false;
        if let Some(tx) = self.watcher_stop_tx.take() {
            let _ = tx.send(()).await;
        }
        info!("Cursor adapter stopped");
        Ok(())
    }

    async fn discover_sessions(&self) -> Result<Vec<Session>> {
        let mut all_sessions = Vec::new();

        // From processes
        let proc_sessions = self.find_processes().await?;
        all_sessions.extend(proc_sessions);

        // From workspace state
        let workspace_sessions = self.parse_workspace_state().await?;
        all_sessions.extend(workspace_sessions);

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        all_sessions.retain(|s| seen.insert(s.project_path.clone()));

        Ok(all_sessions)
    }

    fn capabilities(&self) -> HashMap<String, bool> {
        let mut caps = HashMap::new();
        caps.insert("real_time_events".to_string(), false); // Limited event access
        caps.insert("historical_data".to_string(), true);
        caps.insert("token_tracking".to_string(), false); // Cursor doesn't expose tokens
        caps.insert("cost_tracking".to_string(), false);
        caps.insert("file_change_tracking".to_string(), true);
        caps.insert("transcript_access".to_string(), false);
        caps
    }
}

// ============================================================================
// Aider Adapter
// ============================================================================

/// Aider CLI adapter for monitoring AI-assisted coding sessions.
pub struct AiderAdapter {
    aider_home: PathBuf,
    history_file: PathBuf,
    event_bus: EventBus,
    storage: Storage,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    running: Arc<RwLock<bool>>,
    last_history_pos: Arc<RwLock<u64>>,
    watcher_stop_tx: Option<mpsc::Sender<()>>,
}

impl AiderAdapter {
    /// Create a new Aider adapter.
    pub fn new(_config: &Config, event_bus: EventBus, storage: Storage) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let aider_home = home.join(".aider");

        Self {
            history_file: aider_home.join("history.md"),
            aider_home,
            event_bus,
            storage,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            last_history_pos: Arc::new(RwLock::new(0)),
            watcher_stop_tx: None,
        }
    }

    /// Find running Aider processes.
    async fn find_processes(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        let system = System::new_all();

        for (pid, process) in system.processes() {
            let cmd: String = process.cmd()
                .iter()
                .map(|s| format!("{:?}", s))
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();

            if cmd.contains("aider") && !cmd.contains("aider-") {
                let cwd = process.cwd()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                if !cwd.is_empty() {
                    let mut session = Session::new(
                        AgentType::Aider,
                        &cwd,
                        &format!("aider_{}", pid),
                    );
                    session.pid = Some(pid.as_u32() as i32);
                    session.metadata.insert(
                        "source".to_string(),
                        serde_json::Value::String("process".to_string()),
                    );

                    // Try to detect model from command line
                    if cmd.contains("--model") {
                        if let Some(model_pos) = cmd.find("--model") {
                            let after = &cmd[model_pos + 7..];
                            let model: String = after.split_whitespace().next().unwrap_or("").to_string();
                            if !model.is_empty() {
                                session.model_id = Some(model);
                            }
                        }
                    }

                    sessions.push(session);
                }
            }
        }

        Ok(sessions)
    }

    /// Parse Aider history file (.aider.chat.history.md files in project dirs).
    async fn scan_project_histories(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();

        // Scan home directory for .aider folders in projects
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        // Common development directories to scan
        let scan_dirs = vec![
            home.join("projects"),
            home.join("dev"),
            home.join("code"),
            home.join("workspace"),
            home.clone(),
        ];

        for scan_dir in scan_dirs {
            if scan_dir.exists() && scan_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&scan_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.is_dir() {
                            // Look for .aider.chat.history.md
                            let history = path.join(".aider.chat.history.md");
                            if history.exists() {
                                if let Ok(metadata) = std::fs::metadata(&history) {
                                    // Only include if modified in last 7 days
                                    if let Ok(modified) = metadata.modified() {
                                        let age = modified.elapsed().unwrap_or_default();
                                        if age.as_secs() < 7 * 24 * 60 * 60 {
                                            let mut session = Session::new(
                                                AgentType::Aider,
                                                &path.to_string_lossy(),
                                                &format!("aider_history_{}", entry.file_name().to_string_lossy()),
                                            );
                                            session.metadata.insert(
                                                "source".to_string(),
                                                serde_json::Value::String("history".to_string()),
                                            );
                                            session.status = SessionStatus::Completed;
                                            sessions.push(session);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(sessions)
    }
}

#[async_trait]
impl Adapter for AiderAdapter {
    fn name(&self) -> &str {
        "aider"
    }

    fn agent_type(&self) -> AgentType {
        AgentType::Aider
    }

    async fn start(&mut self) -> Result<()> {
        *self.running.write().await = true;

        // Initial discovery
        let sessions = self.discover_sessions().await?;
        for session in sessions {
            self.storage.upsert_session(&session).await?;
            self.sessions.write().await.insert(session.id.clone(), session);
        }

        // Start periodic process scanner
        let storage = self.storage.clone();
        let sessions = self.sessions.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(30));

            while *running.read().await {
                interval.tick().await;

                let system = System::new_all();
                for (pid, process) in system.processes() {
                    let cmd: String = process.cmd()
                        .iter()
                        .map(|s| format!("{:?}", s))
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase();

                    if cmd.contains("aider") && !cmd.contains("aider-") {
                        let cwd = process.cwd()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();

                        if !cwd.is_empty() {
                            let mut sessions_guard = sessions.write().await;
                            if !sessions_guard.contains_key(&cwd) {
                                let mut session = Session::new(
                                    AgentType::Aider,
                                    &cwd,
                                    &format!("aider_{}", pid),
                                );
                                session.pid = Some(pid.as_u32() as i32);

                                if let Err(e) = storage.upsert_session(&session).await {
                                    warn!("Failed to save Aider session: {}", e);
                                }

                                sessions_guard.insert(cwd, session);
                            }
                        }
                    }
                }
            }
        });

        info!("Aider adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.write().await = false;
        if let Some(tx) = self.watcher_stop_tx.take() {
            let _ = tx.send(()).await;
        }
        info!("Aider adapter stopped");
        Ok(())
    }

    async fn discover_sessions(&self) -> Result<Vec<Session>> {
        let mut all_sessions = Vec::new();

        // From processes
        let proc_sessions = self.find_processes().await?;
        all_sessions.extend(proc_sessions);

        // From project histories
        let history_sessions = self.scan_project_histories().await?;
        all_sessions.extend(history_sessions);

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        all_sessions.retain(|s| seen.insert(s.project_path.clone()));

        Ok(all_sessions)
    }

    fn capabilities(&self) -> HashMap<String, bool> {
        let mut caps = HashMap::new();
        caps.insert("real_time_events".to_string(), false);
        caps.insert("historical_data".to_string(), true);
        caps.insert("token_tracking".to_string(), true); // Aider tracks tokens
        caps.insert("cost_tracking".to_string(), true);  // Aider tracks cost
        caps.insert("file_change_tracking".to_string(), true);
        caps.insert("transcript_access".to_string(), true);
        caps
    }
}
