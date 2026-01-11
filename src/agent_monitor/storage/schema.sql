-- Agent Monitor Database Schema

-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    agent_type TEXT NOT NULL,
    external_id TEXT NOT NULL,
    project_path TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unknown',
    started_at TIMESTAMP NOT NULL,
    last_activity_at TIMESTAMP NOT NULL,
    ended_at TIMESTAMP,
    duration_seconds REAL DEFAULT 0,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    file_operations INTEGER DEFAULT 0,
    tokens_input INTEGER DEFAULT 0,
    tokens_output INTEGER DEFAULT 0,
    estimated_cost REAL DEFAULT 0,
    model_id TEXT,
    model_version TEXT,
    pid INTEGER,
    parent_pid INTEGER,
    current_task TEXT,
    progress REAL DEFAULT 0,
    tasks_completed INTEGER DEFAULT 0,
    tasks_total INTEGER DEFAULT 0,
    metadata_json TEXT DEFAULT '{}',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Session events table
CREATE TABLE IF NOT EXISTS session_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    timestamp TIMESTAMP NOT NULL,
    agent_type TEXT NOT NULL,
    content TEXT,
    metadata_json TEXT DEFAULT '{}',
    working_directory TEXT,
    project_name TEXT,
    tool_name TEXT,
    tool_input_json TEXT,
    tool_output_json TEXT,
    tool_duration_ms INTEGER,
    tool_success INTEGER,
    file_path TEXT,
    file_operation TEXT,
    tokens_input INTEGER,
    tokens_output INTEGER,
    estimated_cost REAL,
    model_used TEXT,
    parent_session_id TEXT,
    subagent_task TEXT,
    error_type TEXT,
    error_message TEXT,
    raw_data_json TEXT,
    confidence REAL DEFAULT 1.0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- Hourly aggregated metrics
CREATE TABLE IF NOT EXISTS hourly_metrics (
    id TEXT PRIMARY KEY,
    agent_type TEXT NOT NULL,
    hour_start TIMESTAMP NOT NULL,
    session_count INTEGER DEFAULT 0,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    file_operations INTEGER DEFAULT 0,
    tokens_input INTEGER DEFAULT 0,
    tokens_output INTEGER DEFAULT 0,
    estimated_cost REAL DEFAULT 0,
    model_usage_json TEXT DEFAULT '{}',
    UNIQUE(agent_type, hour_start)
);

-- Daily aggregated metrics
CREATE TABLE IF NOT EXISTS daily_metrics (
    id TEXT PRIMARY KEY,
    agent_type TEXT NOT NULL,
    date DATE NOT NULL,
    session_count INTEGER DEFAULT 0,
    completed_sessions INTEGER DEFAULT 0,
    crashed_sessions INTEGER DEFAULT 0,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    file_operations INTEGER DEFAULT 0,
    tokens_input INTEGER DEFAULT 0,
    tokens_output INTEGER DEFAULT 0,
    estimated_cost REAL DEFAULT 0,
    avg_session_duration REAL DEFAULT 0,
    peak_hour INTEGER,
    model_usage_json TEXT DEFAULT '{}',
    UNIQUE(agent_type, date)
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_sessions_agent_type ON sessions(agent_type);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at);
CREATE INDEX IF NOT EXISTS idx_sessions_project_path ON sessions(project_path);
CREATE INDEX IF NOT EXISTS idx_sessions_active ON sessions(status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_sessions_external_id ON sessions(external_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_agent_external ON sessions(agent_type, external_id);

CREATE INDEX IF NOT EXISTS idx_events_session_id ON session_events(session_id);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON session_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_type ON session_events(event_type);
CREATE INDEX IF NOT EXISTS idx_events_tool_name ON session_events(tool_name) WHERE tool_name IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_hourly_metrics_hour ON hourly_metrics(hour_start);
CREATE INDEX IF NOT EXISTS idx_daily_metrics_date ON daily_metrics(date);

-- Insert initial schema version
INSERT OR IGNORE INTO schema_version (version) VALUES (1);
