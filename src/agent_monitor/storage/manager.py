"""Async SQLite storage manager for session data."""

import json
import logging
from datetime import datetime, date
from pathlib import Path
from typing import Any, Optional
import uuid

import aiosqlite

from agent_monitor.models import (
    AgentType,
    SessionStatus,
    UnifiedSession,
    SessionEvent,
    AgentMetrics,
    EventType,
)

logger = logging.getLogger(__name__)


class StorageManager:
    """Async SQLite storage for sessions and events."""

    SCHEMA_VERSION = 1

    def __init__(self, db_path: Path | str):
        self.db_path = Path(db_path).expanduser()
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._db: Optional[aiosqlite.Connection] = None

    async def initialize(self) -> None:
        """Initialize database and run migrations."""
        self._db = await aiosqlite.connect(self.db_path)
        self._db.row_factory = aiosqlite.Row

        # Enable foreign keys
        await self._db.execute("PRAGMA foreign_keys = ON")

        # Check if we need to create schema
        await self._run_migrations()
        logger.info(f"Storage initialized at {self.db_path}")

    async def close(self) -> None:
        """Close database connection."""
        if self._db:
            await self._db.close()
            self._db = None

    async def _run_migrations(self) -> None:
        """Run database migrations."""
        # Check if schema_version table exists
        async with self._db.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='schema_version'"
        ) as cursor:
            if not await cursor.fetchone():
                await self._create_schema()
                return

        # Check current version and run migrations
        async with self._db.execute(
            "SELECT MAX(version) FROM schema_version"
        ) as cursor:
            row = await cursor.fetchone()
            current_version = row[0] if row and row[0] else 0

        if current_version < self.SCHEMA_VERSION:
            await self._run_migration_scripts(current_version)

    async def _create_schema(self) -> None:
        """Create initial database schema."""
        schema_path = Path(__file__).parent / "schema.sql"
        with open(schema_path) as f:
            schema_sql = f.read()

        await self._db.executescript(schema_sql)
        await self._db.commit()
        logger.info("Database schema created")

    async def _run_migration_scripts(self, from_version: int) -> None:
        """Run migration scripts from version to current."""
        # Add migration logic here as needed
        pass

    # =========================================================================
    # Session Operations
    # =========================================================================

    async def upsert_session(self, session: UnifiedSession) -> None:
        """Insert or update a session."""
        await self._db.execute(
            """
            INSERT INTO sessions (
                id, agent_type, external_id, project_path, status,
                started_at, last_activity_at, ended_at, duration_seconds,
                message_count, tool_call_count, file_operations,
                tokens_input, tokens_output, estimated_cost,
                model_id, model_version, pid, parent_pid,
                current_task, progress, tasks_completed, tasks_total,
                metadata_json, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
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
                model_id = excluded.model_id,
                pid = excluded.pid,
                current_task = excluded.current_task,
                progress = excluded.progress,
                tasks_completed = excluded.tasks_completed,
                tasks_total = excluded.tasks_total,
                metadata_json = excluded.metadata_json,
                updated_at = CURRENT_TIMESTAMP
            """,
            (
                session.id,
                session.agent_type.value,
                session.external_id,
                session.project_path,
                session.status.value,
                session.started_at.isoformat(),
                session.last_activity_at.isoformat(),
                session.ended_at.isoformat() if session.ended_at else None,
                session.duration_seconds,
                session.message_count,
                session.tool_call_count,
                session.file_operations,
                session.tokens_input,
                session.tokens_output,
                session.estimated_cost,
                session.model_id,
                session.model_version,
                session.pid,
                session.parent_pid,
                session.current_task,
                session.progress,
                session.tasks_completed,
                session.tasks_total,
                json.dumps(session.metadata),
            ),
        )
        await self._db.commit()

    async def get_session(self, session_id: str) -> Optional[UnifiedSession]:
        """Get a session by ID."""
        async with self._db.execute(
            "SELECT * FROM sessions WHERE id = ?", (session_id,)
        ) as cursor:
            row = await cursor.fetchone()
            if row:
                return self._row_to_session(row)
        return None

    async def get_active_sessions(
        self,
        agent_types: Optional[list[AgentType]] = None,
        limit: int = 100,
    ) -> list[UnifiedSession]:
        """Get currently active sessions."""
        query = "SELECT * FROM sessions WHERE status = ?"
        params: list[Any] = [SessionStatus.ACTIVE.value]

        if agent_types:
            placeholders = ",".join("?" * len(agent_types))
            query += f" AND agent_type IN ({placeholders})"
            params.extend([t.value for t in agent_types])

        query += " ORDER BY last_activity_at DESC LIMIT ?"
        params.append(limit)

        async with self._db.execute(query, params) as cursor:
            rows = await cursor.fetchall()
            return [self._row_to_session(row) for row in rows]

    async def get_sessions_by_project(
        self,
        project_path: str,
        limit: int = 50,
    ) -> list[UnifiedSession]:
        """Get sessions for a specific project."""
        async with self._db.execute(
            """
            SELECT * FROM sessions
            WHERE project_path = ?
            ORDER BY started_at DESC LIMIT ?
            """,
            (project_path, limit),
        ) as cursor:
            rows = await cursor.fetchall()
            return [self._row_to_session(row) for row in rows]

    async def get_recent_sessions(
        self,
        hours: int = 24,
        limit: int = 100,
    ) -> list[UnifiedSession]:
        """Get sessions from the last N hours."""
        async with self._db.execute(
            """
            SELECT * FROM sessions
            WHERE started_at > datetime('now', ? || ' hours')
            ORDER BY started_at DESC LIMIT ?
            """,
            (f"-{hours}", limit),
        ) as cursor:
            rows = await cursor.fetchall()
            return [self._row_to_session(row) for row in rows]

    async def get_session_by_external_id(
        self,
        agent_type: AgentType,
        external_id: str,
    ) -> Optional[UnifiedSession]:
        """Get a session by its external ID (tool-specific identifier)."""
        async with self._db.execute(
            """
            SELECT * FROM sessions
            WHERE agent_type = ? AND external_id = ?
            ORDER BY last_activity_at DESC
            LIMIT 1
            """,
            (agent_type.value, external_id),
        ) as cursor:
            row = await cursor.fetchone()
            if row:
                return self._row_to_session(row)
        return None

    async def find_sessions_by_pid(
        self,
        pid: int,
        agent_type: Optional[AgentType] = None,
    ) -> list[UnifiedSession]:
        """Find sessions associated with a process ID."""
        query = "SELECT * FROM sessions WHERE pid = ?"
        params: list[Any] = [pid]

        if agent_type:
            query += " AND agent_type = ?"
            params.append(agent_type.value)

        query += " ORDER BY last_activity_at DESC"

        async with self._db.execute(query, params) as cursor:
            rows = await cursor.fetchall()
            return [self._row_to_session(row) for row in rows]

    async def deduplicate_sessions(self) -> dict[str, int]:
        """
        Remove duplicate sessions, keeping the one with most activity.

        Returns dict with counts of duplicates found and removed.
        """
        stats = {"duplicates_found": 0, "duplicates_removed": 0, "events_migrated": 0}

        # Find duplicate external_ids
        async with self._db.execute(
            """
            SELECT agent_type, external_id, COUNT(*) as cnt
            FROM sessions
            GROUP BY agent_type, external_id
            HAVING cnt > 1
            """
        ) as cursor:
            duplicates = await cursor.fetchall()

        for dup in duplicates:
            agent_type = dup["agent_type"]
            external_id = dup["external_id"]
            stats["duplicates_found"] += dup["cnt"] - 1

            # Get all sessions with this external_id, ordered by activity
            async with self._db.execute(
                """
                SELECT id, message_count, tool_call_count, tokens_input, tokens_output,
                       last_activity_at
                FROM sessions
                WHERE agent_type = ? AND external_id = ?
                ORDER BY
                    (message_count + tool_call_count) DESC,
                    tokens_input DESC,
                    last_activity_at DESC
                """,
                (agent_type, external_id),
            ) as cursor:
                sessions = await cursor.fetchall()

            if len(sessions) < 2:
                continue

            # Keep the first (most active) session
            keep_id = sessions[0]["id"]
            remove_ids = [s["id"] for s in sessions[1:]]

            # Migrate events from duplicate sessions to the kept session
            for remove_id in remove_ids:
                await self._db.execute(
                    "UPDATE session_events SET session_id = ? WHERE session_id = ?",
                    (keep_id, remove_id),
                )
                # Count migrated events
                async with self._db.execute(
                    "SELECT changes()"
                ) as cursor:
                    row = await cursor.fetchone()
                    if row:
                        stats["events_migrated"] += row[0]

            # Aggregate metrics from duplicates into the kept session
            async with self._db.execute(
                """
                SELECT
                    SUM(message_count) as total_messages,
                    SUM(tool_call_count) as total_tools,
                    SUM(file_operations) as total_files,
                    SUM(tokens_input) as total_input,
                    SUM(tokens_output) as total_output,
                    SUM(estimated_cost) as total_cost,
                    MAX(last_activity_at) as latest_activity
                FROM sessions
                WHERE id IN ({})
                """.format(",".join("?" * len(remove_ids))),
                remove_ids,
            ) as cursor:
                totals = await cursor.fetchone()

            if totals and totals["total_messages"]:
                await self._db.execute(
                    """
                    UPDATE sessions SET
                        message_count = message_count + ?,
                        tool_call_count = tool_call_count + ?,
                        file_operations = file_operations + ?,
                        tokens_input = tokens_input + ?,
                        tokens_output = tokens_output + ?,
                        estimated_cost = estimated_cost + ?,
                        last_activity_at = MAX(last_activity_at, ?),
                        updated_at = CURRENT_TIMESTAMP
                    WHERE id = ?
                    """,
                    (
                        totals["total_messages"] or 0,
                        totals["total_tools"] or 0,
                        totals["total_files"] or 0,
                        totals["total_input"] or 0,
                        totals["total_output"] or 0,
                        totals["total_cost"] or 0.0,
                        totals["latest_activity"],
                        keep_id,
                    ),
                )

            # Delete duplicate sessions
            placeholders = ",".join("?" * len(remove_ids))
            await self._db.execute(
                f"DELETE FROM sessions WHERE id IN ({placeholders})",
                remove_ids,
            )
            stats["duplicates_removed"] += len(remove_ids)

        await self._db.commit()
        logger.info(
            f"Deduplication complete: found {stats['duplicates_found']} duplicates, "
            f"removed {stats['duplicates_removed']}, migrated {stats['events_migrated']} events"
        )
        return stats

    async def cleanup_stale_sessions(
        self,
        inactive_hours: int = 24,
        mark_completed: bool = True,
    ) -> int:
        """
        Clean up sessions that have been inactive for too long.

        Args:
            inactive_hours: Hours of inactivity before marking stale
            mark_completed: If True, mark as completed; if False, delete

        Returns:
            Number of sessions affected
        """
        if mark_completed:
            result = await self._db.execute(
                """
                UPDATE sessions
                SET status = 'completed',
                    ended_at = last_activity_at,
                    updated_at = CURRENT_TIMESTAMP
                WHERE status = 'active'
                    AND last_activity_at < datetime('now', ? || ' hours')
                """,
                (f"-{inactive_hours}",),
            )
        else:
            result = await self._db.execute(
                """
                DELETE FROM sessions
                WHERE status = 'active'
                    AND last_activity_at < datetime('now', ? || ' hours')
                """,
                (f"-{inactive_hours}",),
            )

        await self._db.commit()
        return result.rowcount

    def _row_to_session(self, row: aiosqlite.Row) -> UnifiedSession:
        """Convert database row to UnifiedSession."""
        return UnifiedSession(
            id=row["id"],
            agent_type=AgentType(row["agent_type"]),
            external_id=row["external_id"],
            project_path=row["project_path"],
            status=SessionStatus(row["status"]),
            started_at=datetime.fromisoformat(row["started_at"]),
            last_activity_at=datetime.fromisoformat(row["last_activity_at"]),
            ended_at=datetime.fromisoformat(row["ended_at"]) if row["ended_at"] else None,
            duration_seconds=row["duration_seconds"],
            message_count=row["message_count"],
            tool_call_count=row["tool_call_count"],
            file_operations=row["file_operations"],
            tokens_input=row["tokens_input"],
            tokens_output=row["tokens_output"],
            estimated_cost=row["estimated_cost"],
            model_id=row["model_id"],
            model_version=row["model_version"],
            pid=row["pid"],
            parent_pid=row["parent_pid"],
            current_task=row["current_task"],
            progress=row["progress"],
            tasks_completed=row["tasks_completed"],
            tasks_total=row["tasks_total"],
            metadata=json.loads(row["metadata_json"]) if row["metadata_json"] else {},
        )

    # =========================================================================
    # Event Operations
    # =========================================================================

    async def insert_event(self, event: SessionEvent) -> None:
        """Insert a session event."""
        await self._db.execute(
            """
            INSERT INTO session_events (
                id, session_id, event_type, timestamp, agent_type,
                content, metadata_json, working_directory, project_name,
                tool_name, tool_input_json, tool_output_json, tool_duration_ms, tool_success,
                file_path, file_operation,
                tokens_input, tokens_output, estimated_cost, model_used,
                parent_session_id, subagent_task,
                error_type, error_message,
                raw_data_json, confidence
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                event.id,
                event.session_id,
                event.event_type.value,
                event.timestamp.isoformat(),
                event.agent_type.value,
                event.content,
                json.dumps(event.metadata) if event.metadata else "{}",
                event.working_directory,
                event.project_name,
                event.tool_name,
                json.dumps(event.tool_input) if event.tool_input else None,
                json.dumps(event.tool_output) if event.tool_output else None,
                event.tool_duration_ms,
                1 if event.tool_success else 0 if event.tool_success is False else None,
                event.file_path,
                event.file_operation,
                event.tokens_input,
                event.tokens_output,
                event.estimated_cost,
                event.model_used,
                event.parent_session_id,
                event.subagent_task,
                event.error_type,
                event.error_message,
                json.dumps(event.raw_data) if event.raw_data else None,
                event.confidence,
            ),
        )
        await self._db.commit()

    async def get_session_events(
        self,
        session_id: str,
        event_types: Optional[list[EventType]] = None,
        limit: int = 1000,
    ) -> list[SessionEvent]:
        """Get events for a session."""
        query = "SELECT * FROM session_events WHERE session_id = ?"
        params: list[Any] = [session_id]

        if event_types:
            placeholders = ",".join("?" * len(event_types))
            query += f" AND event_type IN ({placeholders})"
            params.extend([t.value for t in event_types])

        query += " ORDER BY timestamp DESC LIMIT ?"
        params.append(limit)

        async with self._db.execute(query, params) as cursor:
            rows = await cursor.fetchall()
            return [self._row_to_event(row) for row in rows]

    async def get_recent_events(
        self,
        minutes: int = 60,
        event_types: Optional[list[EventType]] = None,
        limit: int = 500,
    ) -> list[SessionEvent]:
        """Get recent events across all sessions."""
        query = "SELECT * FROM session_events WHERE timestamp > datetime('now', ? || ' minutes')"
        params: list[Any] = [f"-{minutes}"]

        if event_types:
            placeholders = ",".join("?" * len(event_types))
            query += f" AND event_type IN ({placeholders})"
            params.extend([t.value for t in event_types])

        query += " ORDER BY timestamp DESC LIMIT ?"
        params.append(limit)

        async with self._db.execute(query, params) as cursor:
            rows = await cursor.fetchall()
            return [self._row_to_event(row) for row in rows]

    def _row_to_event(self, row: aiosqlite.Row) -> SessionEvent:
        """Convert database row to SessionEvent."""
        return SessionEvent(
            id=row["id"],
            session_id=row["session_id"],
            event_type=EventType(row["event_type"]),
            timestamp=datetime.fromisoformat(row["timestamp"]),
            agent_type=AgentType(row["agent_type"]),
            content=row["content"],
            metadata=json.loads(row["metadata_json"]) if row["metadata_json"] else {},
            working_directory=row["working_directory"],
            project_name=row["project_name"],
            tool_name=row["tool_name"],
            tool_input=json.loads(row["tool_input_json"]) if row["tool_input_json"] else None,
            tool_output=json.loads(row["tool_output_json"]) if row["tool_output_json"] else None,
            tool_duration_ms=row["tool_duration_ms"],
            tool_success=bool(row["tool_success"]) if row["tool_success"] is not None else None,
            file_path=row["file_path"],
            file_operation=row["file_operation"],
            tokens_input=row["tokens_input"],
            tokens_output=row["tokens_output"],
            estimated_cost=row["estimated_cost"],
            model_used=row["model_used"],
            parent_session_id=row["parent_session_id"],
            subagent_task=row["subagent_task"],
            error_type=row["error_type"],
            error_message=row["error_message"],
            raw_data=json.loads(row["raw_data_json"]) if row["raw_data_json"] else None,
            confidence=row["confidence"],
        )

    # =========================================================================
    # Metrics Operations
    # =========================================================================

    async def get_summary_metrics(
        self,
        agent_type: Optional[AgentType] = None,
        hours: int = 24,
    ) -> dict[str, Any]:
        """Get summary metrics for dashboard."""
        type_filter = "AND agent_type = ?" if agent_type else ""
        params = [f"-{hours}"]
        if agent_type:
            params.append(agent_type.value)

        # Session counts
        async with self._db.execute(
            f"""
            SELECT
                COUNT(*) as total_sessions,
                SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END) as active_sessions,
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed_sessions,
                SUM(CASE WHEN status = 'crashed' THEN 1 ELSE 0 END) as crashed_sessions,
                SUM(message_count) as total_messages,
                SUM(tool_call_count) as total_tool_calls,
                SUM(tokens_input) as total_tokens_input,
                SUM(tokens_output) as total_tokens_output,
                SUM(estimated_cost) as total_cost,
                AVG(duration_seconds) as avg_duration
            FROM sessions
            WHERE started_at > datetime('now', ? || ' hours') {type_filter}
            """,
            params,
        ) as cursor:
            row = await cursor.fetchone()

        # Model distribution
        async with self._db.execute(
            f"""
            SELECT model_id, COUNT(*) as count
            FROM sessions
            WHERE started_at > datetime('now', ? || ' hours')
                AND model_id IS NOT NULL {type_filter}
            GROUP BY model_id
            ORDER BY count DESC
            """,
            params,
        ) as cursor:
            model_rows = await cursor.fetchall()
            model_usage = {r["model_id"]: r["count"] for r in model_rows}

        # Hourly distribution
        async with self._db.execute(
            f"""
            SELECT strftime('%H', started_at) as hour, COUNT(*) as count
            FROM sessions
            WHERE started_at > datetime('now', ? || ' hours') {type_filter}
            GROUP BY hour
            ORDER BY hour
            """,
            params,
        ) as cursor:
            hour_rows = await cursor.fetchall()
            hourly = {int(r["hour"]): r["count"] for r in hour_rows}

        return {
            "total_sessions": row["total_sessions"] or 0,
            "active_sessions": row["active_sessions"] or 0,
            "completed_sessions": row["completed_sessions"] or 0,
            "crashed_sessions": row["crashed_sessions"] or 0,
            "total_messages": row["total_messages"] or 0,
            "total_tool_calls": row["total_tool_calls"] or 0,
            "total_tokens_input": row["total_tokens_input"] or 0,
            "total_tokens_output": row["total_tokens_output"] or 0,
            "total_cost": row["total_cost"] or 0.0,
            "avg_session_duration": row["avg_duration"] or 0.0,
            "model_usage": model_usage,
            "hourly_distribution": hourly,
        }

    async def update_hourly_metrics(
        self,
        agent_type: AgentType,
        hour_start: datetime,
    ) -> None:
        """Update aggregated hourly metrics."""
        hour_end = hour_start.replace(minute=59, second=59)

        async with self._db.execute(
            """
            SELECT
                COUNT(*) as session_count,
                SUM(message_count) as message_count,
                SUM(tool_call_count) as tool_call_count,
                SUM(file_operations) as file_operations,
                SUM(tokens_input) as tokens_input,
                SUM(tokens_output) as tokens_output,
                SUM(estimated_cost) as estimated_cost
            FROM sessions
            WHERE agent_type = ?
                AND started_at >= ?
                AND started_at <= ?
            """,
            (agent_type.value, hour_start.isoformat(), hour_end.isoformat()),
        ) as cursor:
            row = await cursor.fetchone()

        # Get model usage for this hour
        async with self._db.execute(
            """
            SELECT model_id, COUNT(*) as count
            FROM sessions
            WHERE agent_type = ?
                AND started_at >= ?
                AND started_at <= ?
                AND model_id IS NOT NULL
            GROUP BY model_id
            """,
            (agent_type.value, hour_start.isoformat(), hour_end.isoformat()),
        ) as cursor:
            model_rows = await cursor.fetchall()
            model_usage = {r["model_id"]: r["count"] for r in model_rows}

        await self._db.execute(
            """
            INSERT INTO hourly_metrics (
                id, agent_type, hour_start,
                session_count, message_count, tool_call_count, file_operations,
                tokens_input, tokens_output, estimated_cost, model_usage_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(agent_type, hour_start) DO UPDATE SET
                session_count = excluded.session_count,
                message_count = excluded.message_count,
                tool_call_count = excluded.tool_call_count,
                file_operations = excluded.file_operations,
                tokens_input = excluded.tokens_input,
                tokens_output = excluded.tokens_output,
                estimated_cost = excluded.estimated_cost,
                model_usage_json = excluded.model_usage_json
            """,
            (
                str(uuid.uuid4()),
                agent_type.value,
                hour_start.isoformat(),
                row["session_count"] or 0,
                row["message_count"] or 0,
                row["tool_call_count"] or 0,
                row["file_operations"] or 0,
                row["tokens_input"] or 0,
                row["tokens_output"] or 0,
                row["estimated_cost"] or 0.0,
                json.dumps(model_usage),
            ),
        )
        await self._db.commit()
