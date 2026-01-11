"""Tests for data models."""

import pytest
from datetime import datetime
from uuid import uuid4

from agent_monitor.models import (
    UnifiedSession,
    SessionEvent,
    AgentType,
    SessionStatus,
    EventType,
)


class TestUnifiedSession:
    """Tests for UnifiedSession model."""

    def test_create_session(self):
        """Test creating a basic session using factory method."""
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/home/user/project",
            external_id="test-external",
        )
        assert session.agent_type == AgentType.CLAUDE_CODE
        assert session.project_path == "/home/user/project"
        assert session.status == SessionStatus.ACTIVE

    def test_session_to_dict(self):
        """Test session serialization."""
        now = datetime.now()
        session = UnifiedSession(
            id="test-id",
            agent_type=AgentType.CLAUDE_CODE,
            external_id="ext-id",
            project_path="/test/path",
            status=SessionStatus.ACTIVE,
            started_at=now,
            last_activity_at=now,
            message_count=5,
            tokens_input=1000,
            tokens_output=500,
        )
        data = session.to_dict()

        assert data["id"] == "test-id"
        assert data["agent_type"] == "claude_code"
        assert data["message_count"] == 5
        assert data["tokens_input"] == 1000

    def test_session_from_dict(self):
        """Test session deserialization."""
        now = datetime.now()
        data = {
            "id": "test-id",
            "agent_type": "claude_code",
            "external_id": "ext-id",
            "project_path": "/test/path",
            "started_at": now.isoformat(),
            "last_activity_at": now.isoformat(),
            "status": "active",
            "message_count": 10,
        }
        session = UnifiedSession.from_dict(data)

        assert session.id == "test-id"
        assert session.agent_type == AgentType.CLAUDE_CODE
        assert session.message_count == 10


class TestSessionEvent:
    """Tests for SessionEvent model."""

    def test_create_event(self):
        """Test creating a basic event."""
        event = SessionEvent(
            id=str(uuid4()),
            session_id="session-123",
            event_type=EventType.PROMPT_RECEIVED,
            timestamp=datetime.now(),
            agent_type=AgentType.CLAUDE_CODE,
        )
        assert event.event_type == EventType.PROMPT_RECEIVED
        assert event.session_id == "session-123"

    def test_event_to_dict(self):
        """Test event serialization."""
        event = SessionEvent(
            id="event-id",
            session_id="session-id",
            event_type=EventType.TOOL_EXECUTED,
            timestamp=datetime.now(),
            agent_type=AgentType.CLAUDE_CODE,
            tool_name="Bash",
            tool_duration_ms=150,
        )
        data = event.to_dict()

        assert data["id"] == "event-id"
        assert data["event_type"] == "tool_executed"
        assert data["tool_name"] == "Bash"
        assert data["tool_duration_ms"] == 150


class TestEnums:
    """Tests for enum values."""

    def test_agent_types(self):
        """Test all agent types exist."""
        assert AgentType.CLAUDE_CODE.value == "claude_code"
        assert AgentType.CURSOR.value == "cursor"
        assert AgentType.AIDER.value == "aider"

    def test_session_status(self):
        """Test session status values."""
        assert SessionStatus.ACTIVE.value == "active"
        assert SessionStatus.IDLE.value == "idle"
        assert SessionStatus.COMPLETED.value == "completed"
        assert SessionStatus.CRASHED.value == "crashed"

    def test_event_types(self):
        """Test key event types."""
        assert EventType.SESSION_START.value == "session_start"
        assert EventType.TOOL_EXECUTED.value == "tool_executed"
        assert EventType.PROMPT_RECEIVED.value == "prompt_received"
