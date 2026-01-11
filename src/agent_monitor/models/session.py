"""Session and event data models."""

from dataclasses import dataclass, field, asdict
from datetime import datetime
from pathlib import Path
from typing import Any, Optional
import uuid

from agent_monitor.models.enums import AgentType, SessionStatus, EventType


@dataclass
class UnifiedSession:
    """Unified session representation across all agent types."""

    id: str
    agent_type: AgentType
    external_id: str  # Tool-specific session ID
    project_path: str
    status: SessionStatus

    # Timing
    started_at: datetime
    last_activity_at: datetime
    ended_at: Optional[datetime] = None
    duration_seconds: float = 0.0

    # Metrics
    message_count: int = 0
    tool_call_count: int = 0
    file_operations: int = 0
    tokens_input: int = 0
    tokens_output: int = 0

    # Cost tracking (USD)
    estimated_cost: float = 0.0

    # Model info
    model_id: Optional[str] = None
    model_version: Optional[str] = None

    # Process info
    pid: Optional[int] = None
    parent_pid: Optional[int] = None

    # Current task/progress
    current_task: Optional[str] = None
    progress: float = 0.0  # 0.0 to 1.0
    tasks_completed: int = 0
    tasks_total: int = 0

    # Rich context
    metadata: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def create(
        cls,
        agent_type: AgentType,
        project_path: str,
        external_id: str = "",
        **kwargs: Any,
    ) -> "UnifiedSession":
        """Create a new session with generated ID and timestamps."""
        now = datetime.now()
        return cls(
            id=str(uuid.uuid4()),
            agent_type=agent_type,
            external_id=external_id or str(uuid.uuid4()),
            project_path=project_path,
            status=SessionStatus.ACTIVE,
            started_at=now,
            last_activity_at=now,
            **kwargs,
        )

    def update_activity(self) -> None:
        """Update last activity timestamp and duration."""
        now = datetime.now()
        self.last_activity_at = now
        self.duration_seconds = (now - self.started_at).total_seconds()

    def end(self, status: SessionStatus = SessionStatus.COMPLETED) -> None:
        """Mark session as ended."""
        now = datetime.now()
        self.ended_at = now
        self.status = status
        self.duration_seconds = (now - self.started_at).total_seconds()

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for serialization."""
        data = asdict(self)
        # Convert enums to strings
        data["agent_type"] = self.agent_type.value
        data["status"] = self.status.value
        # Convert datetimes to ISO format
        data["started_at"] = self.started_at.isoformat()
        data["last_activity_at"] = self.last_activity_at.isoformat()
        if self.ended_at:
            data["ended_at"] = self.ended_at.isoformat()
        return data

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "UnifiedSession":
        """Create from dictionary."""
        # Convert strings to enums
        data["agent_type"] = AgentType(data["agent_type"])
        data["status"] = SessionStatus(data["status"])
        # Convert ISO strings to datetimes
        data["started_at"] = datetime.fromisoformat(data["started_at"])
        data["last_activity_at"] = datetime.fromisoformat(data["last_activity_at"])
        if data.get("ended_at"):
            data["ended_at"] = datetime.fromisoformat(data["ended_at"])
        return cls(**data)


@dataclass
class SessionEvent:
    """Event within a session."""

    id: str
    session_id: str
    event_type: EventType
    timestamp: datetime
    agent_type: AgentType

    # Content
    content: Optional[str] = None
    metadata: dict[str, Any] = field(default_factory=dict)

    # Context
    working_directory: Optional[str] = None
    project_name: Optional[str] = None

    # For tool calls
    tool_name: Optional[str] = None
    tool_input: Optional[dict[str, Any]] = None
    tool_output: Optional[dict[str, Any]] = None
    tool_duration_ms: Optional[int] = None
    tool_success: Optional[bool] = None

    # For file operations
    file_path: Optional[str] = None
    file_operation: Optional[str] = None

    # Token/cost tracking
    tokens_input: Optional[int] = None
    tokens_output: Optional[int] = None
    estimated_cost: Optional[float] = None
    model_used: Optional[str] = None

    # Subagent info
    parent_session_id: Optional[str] = None
    subagent_task: Optional[str] = None

    # Error info
    error_type: Optional[str] = None
    error_message: Optional[str] = None

    # Raw data from source
    raw_data: Optional[dict[str, Any]] = None
    confidence: float = 1.0

    @classmethod
    def create(
        cls,
        session_id: str,
        event_type: EventType,
        agent_type: AgentType,
        **kwargs: Any,
    ) -> "SessionEvent":
        """Create a new event with generated ID and timestamp."""
        return cls(
            id=str(uuid.uuid4()),
            session_id=session_id,
            event_type=event_type,
            agent_type=agent_type,
            timestamp=datetime.now(),
            **kwargs,
        )

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for serialization."""
        data = asdict(self)
        data["event_type"] = self.event_type.value
        data["agent_type"] = self.agent_type.value
        data["timestamp"] = self.timestamp.isoformat()
        return data

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "SessionEvent":
        """Create from dictionary."""
        data["event_type"] = EventType(data["event_type"])
        data["agent_type"] = AgentType(data["agent_type"])
        data["timestamp"] = datetime.fromisoformat(data["timestamp"])
        return cls(**data)


@dataclass
class AgentMetrics:
    """Aggregated metrics for an agent type over a time period."""

    agent_type: AgentType
    period_start: datetime
    period_end: datetime

    # Session counts
    total_sessions: int = 0
    active_sessions: int = 0
    completed_sessions: int = 0
    crashed_sessions: int = 0

    # Activity
    total_messages: int = 0
    total_tool_calls: int = 0
    total_file_operations: int = 0

    # Tokens
    total_tokens_input: int = 0
    total_tokens_output: int = 0

    # Cost tracking
    estimated_cost_usd: float = 0.0

    # Performance
    avg_session_duration_seconds: float = 0.0
    avg_messages_per_session: float = 0.0

    # Model distribution
    model_usage: dict[str, int] = field(default_factory=dict)

    # Peak hours (hour -> session count)
    hourly_distribution: dict[int, int] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for serialization."""
        data = asdict(self)
        data["agent_type"] = self.agent_type.value
        data["period_start"] = self.period_start.isoformat()
        data["period_end"] = self.period_end.isoformat()
        return data
