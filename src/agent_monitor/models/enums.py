"""Enumerations for agent monitoring."""

from enum import Enum


class AgentType(str, Enum):
    """Types of AI agents that can be monitored."""

    CLAUDE_CODE = "claude_code"
    CURSOR = "cursor"
    AIDER = "aider"
    GEMINI_CLI = "gemini_cli"
    OPENAI_CODEX = "openai_codex"
    CUSTOM = "custom"


class SessionStatus(str, Enum):
    """Status of an agent session."""

    ACTIVE = "active"
    IDLE = "idle"
    COMPLETED = "completed"
    CRASHED = "crashed"
    UNKNOWN = "unknown"


class EventType(str, Enum):
    """Types of events that can occur in a session."""

    # Session lifecycle
    SESSION_START = "session_start"
    SESSION_END = "session_end"
    SESSION_PAUSE = "session_pause"
    SESSION_RESUME = "session_resume"

    # Agent activity
    PROMPT_RECEIVED = "prompt_received"
    RESPONSE_STARTED = "response_started"
    RESPONSE_COMPLETED = "response_completed"
    RESPONSE_GENERATED = "response_generated"
    THINKING = "thinking"
    TOOL_EXECUTED = "tool_executed"

    # Tool execution
    TOOL_START = "tool_start"
    TOOL_COMPLETE = "tool_complete"
    TOOL_ERROR = "tool_error"
    TOOL_PERMISSION_REQUEST = "tool_permission_request"
    TOOL_PERMISSION_RESPONSE = "tool_permission_response"

    # File operations
    FILE_READ = "file_read"
    FILE_WRITE = "file_write"
    FILE_EDIT = "file_edit"
    FILE_DELETE = "file_delete"
    FILE_MODIFIED = "file_modified"

    # Subagent events
    SUBAGENT_START = "subagent_start"
    SUBAGENT_STOP = "subagent_stop"

    # Errors and warnings
    ERROR = "error"
    WARNING = "warning"

    # Metrics
    TOKEN_USAGE = "token_usage"
    COST_ESTIMATE = "cost_estimate"

    # Custom
    CUSTOM = "custom"


class DataSource(str, Enum):
    """Types of data sources an adapter can use."""

    HOOKS = "hooks"  # Native hook integration (Claude Code)
    FILES = "files"  # File watching (logs, history files)
    PROCESS = "process"  # Process monitoring (fallback)
    API = "api"  # API polling (if available)
    SOCKET = "socket"  # Socket/IPC communication


class AdapterStatus(str, Enum):
    """Status of an adapter."""

    INACTIVE = "inactive"
    DISCOVERING = "discovering"
    CONNECTED = "connected"
    ERROR = "error"
    DEGRADED = "degraded"


class DaemonState(str, Enum):
    """State of the daemon process."""

    INITIALIZING = "initializing"
    RUNNING = "running"
    PAUSED = "paused"
    DEGRADED = "degraded"
    SHUTTING_DOWN = "shutting_down"
    STOPPED = "stopped"
