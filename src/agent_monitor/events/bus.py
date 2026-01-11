"""Async event bus for internal event distribution."""

import asyncio
import logging
from collections import defaultdict
from typing import Any, Callable, Awaitable, Optional

from agent_monitor.models import SessionEvent, EventType

logger = logging.getLogger(__name__)

# Type for event handlers
EventHandler = Callable[[SessionEvent], Awaitable[None]]


class EventBus:
    """
    Async event bus for distributing session events.

    Supports:
    - Type-specific subscriptions
    - Wildcard (*) subscriptions for all events
    - Async event processing
    - Graceful shutdown
    """

    def __init__(self, max_queue_size: int = 10000):
        self._handlers: dict[str, list[EventHandler]] = defaultdict(list)
        self._queue: asyncio.Queue[SessionEvent] = asyncio.Queue(maxsize=max_queue_size)
        self._running = False
        self._process_task: Optional[asyncio.Task[None]] = None

    def subscribe(self, event_type: str | EventType, handler: EventHandler) -> None:
        """
        Subscribe to events of a specific type.

        Args:
            event_type: Event type to subscribe to, or "*" for all events
            handler: Async callback function
        """
        type_key = event_type.value if isinstance(event_type, EventType) else event_type
        self._handlers[type_key].append(handler)
        logger.debug(f"Handler subscribed to {type_key}")

    def unsubscribe(self, event_type: str | EventType, handler: EventHandler) -> None:
        """Unsubscribe a handler from an event type."""
        type_key = event_type.value if isinstance(event_type, EventType) else event_type
        if type_key in self._handlers:
            try:
                self._handlers[type_key].remove(handler)
            except ValueError:
                pass

    async def publish(self, event: SessionEvent) -> None:
        """
        Publish an event to the queue.

        Args:
            event: The event to publish
        """
        try:
            self._queue.put_nowait(event)
        except asyncio.QueueFull:
            logger.warning("Event queue full, dropping oldest event")
            try:
                self._queue.get_nowait()
                self._queue.put_nowait(event)
            except (asyncio.QueueEmpty, asyncio.QueueFull):
                pass

    async def start(self) -> None:
        """Start the event processing loop."""
        if self._running:
            return

        self._running = True
        self._process_task = asyncio.create_task(self._process_loop())
        logger.info("Event bus started")

    async def stop(self) -> None:
        """Stop the event processing loop gracefully."""
        self._running = False

        if self._process_task:
            # Process remaining events
            await self._flush()
            self._process_task.cancel()
            try:
                await self._process_task
            except asyncio.CancelledError:
                pass

        logger.info("Event bus stopped")

    async def _process_loop(self) -> None:
        """Main event processing loop."""
        while self._running:
            try:
                # Wait for event with timeout to allow shutdown checks
                event = await asyncio.wait_for(
                    self._queue.get(),
                    timeout=1.0
                )
                await self._dispatch(event)
            except asyncio.TimeoutError:
                continue
            except Exception as e:
                logger.error(f"Error in event processing loop: {e}")

    async def _dispatch(self, event: SessionEvent) -> None:
        """Dispatch event to all subscribed handlers."""
        # Get handlers for this specific event type
        type_handlers = self._handlers.get(event.event_type.value, [])
        # Get wildcard handlers
        wildcard_handlers = self._handlers.get("*", [])

        all_handlers = type_handlers + wildcard_handlers

        if not all_handlers:
            return

        # Run all handlers concurrently
        results = await asyncio.gather(
            *[self._safe_call(h, event) for h in all_handlers],
            return_exceptions=True
        )

        # Log any errors
        for result in results:
            if isinstance(result, Exception):
                logger.error(f"Handler error: {result}")

    async def _safe_call(self, handler: EventHandler, event: SessionEvent) -> None:
        """Safely call a handler, catching any exceptions."""
        try:
            await handler(event)
        except Exception as e:
            logger.error(f"Error in event handler: {e}")
            raise

    async def _flush(self) -> None:
        """Process all remaining events in the queue."""
        while not self._queue.empty():
            try:
                event = self._queue.get_nowait()
                await self._dispatch(event)
            except asyncio.QueueEmpty:
                break

    @property
    def pending_count(self) -> int:
        """Number of events pending in the queue."""
        return self._queue.qsize()

    @property
    def is_running(self) -> bool:
        """Whether the event bus is currently running."""
        return self._running


class EventFilter:
    """
    Filter for selecting which events to receive.

    Used by IPC clients to subscribe to specific events.
    """

    def __init__(
        self,
        agent_types: Optional[list[str]] = None,
        event_types: Optional[list[str]] = None,
        project_paths: Optional[list[str]] = None,
        session_ids: Optional[list[str]] = None,
    ):
        self.agent_types = set(agent_types) if agent_types else None
        self.event_types = set(event_types) if event_types else None
        self.project_paths = set(project_paths) if project_paths else None
        self.session_ids = set(session_ids) if session_ids else None

    def matches(self, event: SessionEvent) -> bool:
        """Check if an event matches this filter."""
        if self.agent_types and event.agent_type.value not in self.agent_types:
            return False

        if self.event_types and event.event_type.value not in self.event_types:
            return False

        if self.session_ids and event.session_id not in self.session_ids:
            return False

        if self.project_paths and event.working_directory:
            if not any(event.working_directory.startswith(p) for p in self.project_paths):
                return False

        return True

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for serialization."""
        return {
            "agent_types": list(self.agent_types) if self.agent_types else None,
            "event_types": list(self.event_types) if self.event_types else None,
            "project_paths": list(self.project_paths) if self.project_paths else None,
            "session_ids": list(self.session_ids) if self.session_ids else None,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "EventFilter":
        """Create from dictionary."""
        return cls(
            agent_types=data.get("agent_types"),
            event_types=data.get("event_types"),
            project_paths=data.get("project_paths"),
            session_ids=data.get("session_ids"),
        )
