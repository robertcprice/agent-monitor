"""Base adapter interface for agent monitoring."""

from abc import ABC, abstractmethod
from typing import AsyncIterator, Optional, TYPE_CHECKING

from agent_monitor.models import (
    UnifiedSession,
    SessionEvent,
    AgentType,
    DataSource,
    AdapterStatus,
)

if TYPE_CHECKING:
    from agent_monitor.config import DaemonConfig
    from agent_monitor.events import EventBus
    from agent_monitor.storage import StorageManager


class BaseAdapter(ABC):
    """
    Abstract base class for all agent adapters.

    Adapters are responsible for:
    1. Detecting when their agent type is running
    2. Extracting session data from various sources
    3. Parsing agent-specific formats
    4. Emitting unified AgentEvent objects
    """

    def __init__(
        self,
        name: str,
        agent_type: AgentType,
        data_sources: list[DataSource],
        config: "DaemonConfig",
        event_bus: "EventBus",
        storage: "StorageManager",
    ):
        self.name = name
        self.agent_type = agent_type
        self.data_sources = data_sources
        self.config = config
        self.event_bus = event_bus
        self.storage = storage
        self.status = AdapterStatus.INACTIVE
        self._sessions: dict[str, UnifiedSession] = {}
        self._running = False

    # =========================================================================
    # Abstract Methods - Must be implemented by each adapter
    # =========================================================================

    @abstractmethod
    async def discover_sessions(self) -> list[UnifiedSession]:
        """
        Discover currently running agent sessions.

        Returns:
            List of active UnifiedSession objects
        """
        pass

    @abstractmethod
    async def start(self) -> None:
        """Start the adapter (initialize watchers, etc.)."""
        pass

    @abstractmethod
    async def stop(self) -> None:
        """Stop the adapter gracefully."""
        pass

    # =========================================================================
    # Optional Methods - Can be overridden by adapters
    # =========================================================================

    def get_capabilities(self) -> dict[str, bool]:
        """Get adapter capabilities."""
        return {
            "real_time_events": DataSource.HOOKS in self.data_sources,
            "historical_data": DataSource.FILES in self.data_sources,
            "send_commands": False,
            "token_tracking": False,
            "cost_tracking": False,
            "file_change_tracking": DataSource.FILES in self.data_sources,
            "subagent_tracking": False,
        }

    async def parse_history(self) -> list[SessionEvent]:
        """Parse historical session data."""
        return []

    # =========================================================================
    # Common Implementation
    # =========================================================================

    async def emit_event(self, event: SessionEvent) -> None:
        """Emit an event to the event bus."""
        await self.event_bus.publish(event)

    def get_session(self, session_id: str) -> Optional[UnifiedSession]:
        """Get a tracked session by ID."""
        return self._sessions.get(session_id)

    def get_session_by_external_id(self, external_id: str) -> Optional[UnifiedSession]:
        """Get a tracked session by its external ID."""
        for session in self._sessions.values():
            if session.external_id == external_id:
                return session
        return None

    def list_sessions(self) -> list[UnifiedSession]:
        """List all tracked sessions."""
        return list(self._sessions.values())

    async def find_or_create_session(
        self,
        external_id: str,
        create_func: callable,
    ) -> tuple[UnifiedSession, bool]:
        """
        Find existing session by external_id or create new one.

        Args:
            external_id: The tool-specific session identifier
            create_func: Callable that returns a new UnifiedSession if needed

        Returns:
            Tuple of (session, is_new) where is_new indicates if session was created
        """
        # Check in-memory cache first
        existing = self.get_session_by_external_id(external_id)
        if existing:
            return existing, False

        # Check database
        db_session = await self.storage.get_session_by_external_id(
            self.agent_type, external_id
        )
        if db_session:
            # Add to in-memory cache
            self._sessions[db_session.id] = db_session
            return db_session, False

        # Create new session
        new_session = create_func()
        await self.register_session(new_session)
        return new_session, True

    async def register_session(self, session: UnifiedSession) -> None:
        """
        Register a new session and persist to storage.

        Note: Prefer using find_or_create_session() to avoid duplicates.
        """
        # Double-check we're not creating a duplicate
        existing = self.get_session_by_external_id(session.external_id)
        if existing:
            # Update existing instead of creating duplicate
            existing.update_activity()
            if session.pid and not existing.pid:
                existing.pid = session.pid
            if session.project_path and existing.project_path in ("/", ""):
                existing.project_path = session.project_path
            existing.metadata.update(session.metadata)
            await self.storage.upsert_session(existing)
            return

        self._sessions[session.id] = session
        await self.storage.upsert_session(session)

    async def update_session(self, session: UnifiedSession) -> None:
        """Update a session and persist to storage."""
        session.update_activity()
        self._sessions[session.id] = session
        await self.storage.upsert_session(session)

    async def load_active_sessions(self) -> None:
        """Load active sessions from storage into memory cache."""
        sessions = await self.storage.get_active_sessions(
            agent_types=[self.agent_type],
            limit=500,
        )
        for session in sessions:
            self._sessions[session.id] = session
