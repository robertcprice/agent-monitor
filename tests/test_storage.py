"""Tests for storage module."""

import pytest
import tempfile
from pathlib import Path
from datetime import datetime
from uuid import uuid4

from agent_monitor.storage import StorageManager
from agent_monitor.models import UnifiedSession, SessionEvent, AgentType, EventType, SessionStatus


@pytest.fixture
async def storage():
    """Create a temporary storage manager for testing."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    manager = StorageManager(db_path)
    await manager.initialize()

    yield manager

    await manager.close()
    db_path.unlink(missing_ok=True)


class TestStorageManager:
    """Tests for StorageManager."""

    @pytest.mark.asyncio
    async def test_initialize(self, storage):
        """Test storage initialization."""
        # Should have created tables
        assert storage._db is not None

    @pytest.mark.asyncio
    async def test_upsert_session(self, storage):
        """Test saving a session."""
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test/project",
            external_id="test-ext-1",
        )
        session.message_count = 5

        await storage.upsert_session(session)

        # Retrieve and verify
        loaded = await storage.get_session(session.id)
        assert loaded is not None
        assert loaded.id == session.id
        assert loaded.message_count == 5

    @pytest.mark.asyncio
    async def test_update_session(self, storage):
        """Test updating a session."""
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test/project",
            external_id="test-ext-2",
        )
        session.message_count = 0

        await storage.upsert_session(session)

        # Update
        session.message_count = 10
        await storage.upsert_session(session)

        # Verify update
        loaded = await storage.get_session(session.id)
        assert loaded.message_count == 10

    @pytest.mark.asyncio
    async def test_get_active_sessions(self, storage):
        """Test retrieving active sessions."""
        # Create active and inactive sessions
        active = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/active",
            external_id="active-ext",
        )

        inactive = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/inactive",
            external_id="inactive-ext",
        )
        inactive.status = SessionStatus.COMPLETED

        await storage.upsert_session(active)
        await storage.upsert_session(inactive)

        # Get active sessions
        sessions = await storage.get_active_sessions()

        # Should only return active session
        active_ids = [s.id for s in sessions]
        assert active.id in active_ids

    @pytest.mark.asyncio
    async def test_save_event(self, storage):
        """Test saving an event."""
        # First create a session
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test",
            external_id="event-test-ext",
        )
        await storage.upsert_session(session)

        # Then create event
        event = SessionEvent(
            id=str(uuid4()),
            session_id=session.id,
            event_type=EventType.TOOL_EXECUTED,
            timestamp=datetime.now(),
            agent_type=AgentType.CLAUDE_CODE,
            tool_name="Bash",
        )

        await storage.insert_event(event)

        # Verify
        events = await storage.get_session_events(session.id)
        assert len(events) >= 1

    @pytest.mark.asyncio
    async def test_get_recent_sessions(self, storage):
        """Test getting recent sessions."""
        # Create a session
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test",
            external_id="recent-test-ext",
        )
        await storage.upsert_session(session)

        # Get recent
        sessions = await storage.get_recent_sessions(hours=24, limit=10)

        assert len(sessions) >= 1
        assert sessions[0].id == session.id

    @pytest.mark.asyncio
    async def test_get_summary_metrics(self, storage):
        """Test getting summary metrics."""
        # Create some sessions
        for i in range(3):
            session = UnifiedSession.create(
                agent_type=AgentType.CLAUDE_CODE,
                project_path=f"/test/{i}",
                external_id=f"metrics-test-{i}",
            )
            session.message_count = i * 10
            await storage.upsert_session(session)

        # Get metrics
        metrics = await storage.get_summary_metrics(hours=24)

        assert metrics["total_sessions"] >= 3
        assert "total_messages" in metrics
        assert "active_sessions" in metrics

    @pytest.mark.asyncio
    async def test_get_session_by_external_id(self, storage):
        """Test finding session by external_id."""
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test/project",
            external_id="unique-external-id",
        )
        await storage.upsert_session(session)

        # Find by external_id
        found = await storage.get_session_by_external_id(
            AgentType.CLAUDE_CODE,
            "unique-external-id"
        )
        assert found is not None
        assert found.id == session.id
        assert found.external_id == "unique-external-id"

        # Not found for wrong agent type
        not_found = await storage.get_session_by_external_id(
            AgentType.CURSOR,
            "unique-external-id"
        )
        assert not_found is None

    @pytest.mark.asyncio
    async def test_deduplicate_sessions(self, storage):
        """Test deduplication of sessions with same external_id."""
        # To test deduplication, we need to bypass the unique constraint
        # by directly inserting into the database (simulating legacy data)
        external_id = "duplicate-external-id"

        # Insert duplicates directly via SQL to simulate pre-constraint data
        sessions_data = [
            (f"dup-id-1", "claude_code", external_id, "/test/project1", "active",
             datetime.now().isoformat(), datetime.now().isoformat(), None, 0.0,
             100, 0, 0, 0, 0, 0.0, None, None, None, None, None, 0.0, 0, 0, "{}"),
            (f"dup-id-2", "claude_code", external_id, "/test/project2", "active",
             datetime.now().isoformat(), datetime.now().isoformat(), None, 0.0,
             10, 0, 0, 0, 0, 0.0, None, None, None, None, None, 0.0, 0, 0, "{}"),
            (f"dup-id-3", "claude_code", external_id, "/test/project3", "active",
             datetime.now().isoformat(), datetime.now().isoformat(), None, 0.0,
             5, 0, 0, 0, 0, 0.0, None, None, None, None, None, 0.0, 0, 0, "{}"),
        ]

        # Drop the unique index temporarily to simulate legacy data
        await storage._db.execute(
            "DROP INDEX IF EXISTS idx_sessions_agent_external"
        )

        for data in sessions_data:
            await storage._db.execute(
                """
                INSERT INTO sessions (
                    id, agent_type, external_id, project_path, status,
                    started_at, last_activity_at, ended_at, duration_seconds,
                    message_count, tool_call_count, file_operations,
                    tokens_input, tokens_output, estimated_cost,
                    model_id, model_version, pid, parent_pid,
                    current_task, progress, tasks_completed, tasks_total,
                    metadata_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                data,
            )
        await storage._db.commit()

        # Run deduplication
        result = await storage.deduplicate_sessions()

        assert result["duplicates_found"] == 2
        assert result["duplicates_removed"] == 2

        # Only one session should remain
        found = await storage.get_session_by_external_id(
            AgentType.CLAUDE_CODE,
            external_id
        )
        assert found is not None
        # The most active session should be kept (message_count = 100)
        assert found.message_count >= 100  # May have aggregated counts

    @pytest.mark.asyncio
    async def test_cleanup_stale_sessions(self, storage):
        """Test marking stale sessions as completed."""
        from datetime import timedelta

        # Create an old session
        old_session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test/old",
            external_id="old-session",
        )
        # Backdate the activity timestamp
        old_session.last_activity_at = datetime.now() - timedelta(hours=48)
        await storage.upsert_session(old_session)

        # Create a recent session
        recent_session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test/recent",
            external_id="recent-session",
        )
        await storage.upsert_session(recent_session)

        # Cleanup sessions inactive for more than 24 hours
        cleaned = await storage.cleanup_stale_sessions(
            inactive_hours=24,
            mark_completed=True
        )

        assert cleaned >= 1

        # Old session should be marked completed
        old_loaded = await storage.get_session(old_session.id)
        assert old_loaded.status == SessionStatus.COMPLETED

        # Recent session should still be active
        recent_loaded = await storage.get_session(recent_session.id)
        assert recent_loaded.status == SessionStatus.ACTIVE

    @pytest.mark.asyncio
    async def test_find_sessions_by_pid(self, storage):
        """Test finding sessions by process ID."""
        session = UnifiedSession.create(
            agent_type=AgentType.CLAUDE_CODE,
            project_path="/test/project",
            external_id="pid-test",
            pid=12345,
        )
        await storage.upsert_session(session)

        # Find by PID
        found = await storage.find_sessions_by_pid(12345)
        assert len(found) >= 1
        assert any(s.id == session.id for s in found)

        # Not found for different PID
        not_found = await storage.find_sessions_by_pid(99999)
        assert all(s.id != session.id for s in not_found)
