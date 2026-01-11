"""Main daemon process for monitoring AI agent sessions."""

import asyncio
import logging
import signal
from datetime import datetime
from pathlib import Path
from typing import Optional

from agent_monitor.config import DaemonConfig
from agent_monitor.events import EventBus
from agent_monitor.models import DaemonState, SessionEvent, AgentType
from agent_monitor.storage import StorageManager

logger = logging.getLogger(__name__)


class AgentMonitorDaemon:
    """
    Main daemon process that orchestrates all agent monitoring.

    Responsibilities:
    - Manage adapter lifecycle
    - Route events between adapters, storage, and IPC
    - Handle graceful startup and shutdown
    - Coordinate periodic tasks (discovery, metrics aggregation)
    """

    def __init__(self, config: Optional[DaemonConfig] = None):
        self.config = config or DaemonConfig()
        self.state = DaemonState.INITIALIZING

        # Core components
        self.storage: Optional[StorageManager] = None
        self.event_bus = EventBus(max_queue_size=self.config.event_queue_size)
        self.adapters: dict[str, "BaseAdapter"] = {}  # Lazy import

        # Tasks
        self._discovery_task: Optional[asyncio.Task] = None
        self._metrics_task: Optional[asyncio.Task] = None
        self._shutdown_event = asyncio.Event()

    async def start(self) -> None:
        """Start the daemon and all subsystems."""
        logger.info("Starting Agent Monitor Daemon...")
        self.state = DaemonState.INITIALIZING

        try:
            # Ensure directories exist
            self.config.ensure_directories()

            # Initialize storage
            self.storage = StorageManager(self.config.db_path)
            await self.storage.initialize()

            # Start event bus
            await self.event_bus.start()

            # Subscribe to events for storage
            self.event_bus.subscribe("*", self._on_event)

            # Register adapters
            await self._register_adapters()

            # Install Claude Code hooks if enabled
            if self.config.auto_install_hooks:
                await self._install_hooks()

            # Start periodic tasks
            self._discovery_task = asyncio.create_task(self._discovery_loop())
            self._metrics_task = asyncio.create_task(self._metrics_loop())

            # Mark as running
            self.state = DaemonState.RUNNING
            logger.info("Agent Monitor Daemon started successfully")

            # Wait for shutdown signal
            await self._shutdown_event.wait()

        except Exception as e:
            logger.error(f"Error starting daemon: {e}")
            self.state = DaemonState.STOPPED
            raise

    async def stop(self) -> None:
        """Gracefully stop the daemon."""
        logger.info("Stopping Agent Monitor Daemon...")
        self.state = DaemonState.SHUTTING_DOWN

        # Cancel periodic tasks
        if self._discovery_task:
            self._discovery_task.cancel()
            try:
                await self._discovery_task
            except asyncio.CancelledError:
                pass

        if self._metrics_task:
            self._metrics_task.cancel()
            try:
                await self._metrics_task
            except asyncio.CancelledError:
                pass

        # Stop adapters
        for adapter in self.adapters.values():
            try:
                await adapter.stop()
            except Exception as e:
                logger.error(f"Error stopping adapter: {e}")

        # Stop event bus (flushes pending events)
        await self.event_bus.stop()

        # Close storage
        if self.storage:
            await self.storage.close()

        self.state = DaemonState.STOPPED
        logger.info("Agent Monitor Daemon stopped")

    def request_shutdown(self) -> None:
        """Request daemon shutdown (called from signal handlers)."""
        self._shutdown_event.set()

    async def _register_adapters(self) -> None:
        """Register and initialize all enabled adapters."""
        # Import adapters here to avoid circular imports
        from agent_monitor.adapters.claude_code import ClaudeCodeAdapter

        adapter_classes = {
            "claude_code": ClaudeCodeAdapter,
            # "cursor": CursorAdapter,  # TODO
            # "aider": AiderAdapter,    # TODO
        }

        for adapter_name in self.config.enabled_adapters:
            if adapter_name in adapter_classes:
                try:
                    adapter_class = adapter_classes[adapter_name]
                    adapter = adapter_class(
                        config=self.config,
                        event_bus=self.event_bus,
                        storage=self.storage,
                    )
                    await adapter.start()
                    self.adapters[adapter_name] = adapter
                    logger.info(f"Registered adapter: {adapter_name}")
                except Exception as e:
                    logger.error(f"Failed to register adapter {adapter_name}: {e}")

    async def _install_hooks(self) -> None:
        """Install Claude Code hooks for real-time monitoring."""
        from agent_monitor.adapters.claude_code import install_hooks

        try:
            await install_hooks(self.config.claude_home)
            logger.info("Claude Code hooks installed")
        except Exception as e:
            logger.warning(f"Failed to install Claude Code hooks: {e}")

    async def _discovery_loop(self) -> None:
        """Periodically discover new sessions."""
        while True:
            try:
                await asyncio.sleep(self.config.poll_interval)

                for adapter in self.adapters.values():
                    try:
                        await adapter.discover_sessions()
                    except Exception as e:
                        logger.error(f"Discovery error in {adapter.name}: {e}")

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Error in discovery loop: {e}")

    async def _metrics_loop(self) -> None:
        """Periodically aggregate metrics."""
        while True:
            try:
                # Run every hour
                await asyncio.sleep(3600)

                hour_start = datetime.now().replace(minute=0, second=0, microsecond=0)

                for agent_type_str in self.adapters.keys():
                    try:
                        agent_type = AgentType(agent_type_str)
                        await self.storage.update_hourly_metrics(agent_type, hour_start)
                    except Exception as e:
                        logger.error(f"Metrics aggregation error for {agent_type_str}: {e}")

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Error in metrics loop: {e}")

    async def _on_event(self, event: SessionEvent) -> None:
        """Handle incoming events from adapters."""
        try:
            # Store event
            await self.storage.insert_event(event)

            # Update session if needed
            session = await self.storage.get_session(event.session_id)
            if session:
                session.update_activity()
                session.message_count += 1 if event.event_type.value.startswith("prompt") else 0
                session.tool_call_count += 1 if event.event_type.value.startswith("tool") else 0
                await self.storage.upsert_session(session)

        except Exception as e:
            logger.error(f"Error handling event: {e}")

    async def get_status(self) -> dict:
        """Get daemon status for monitoring."""
        active_sessions = await self.storage.get_active_sessions() if self.storage else []

        return {
            "state": self.state.value,
            "adapters": list(self.adapters.keys()),
            "active_sessions": len(active_sessions),
            "pending_events": self.event_bus.pending_count,
        }


def setup_signal_handlers(daemon: AgentMonitorDaemon) -> None:
    """Set up signal handlers for graceful shutdown."""
    loop = asyncio.get_running_loop()

    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(
            sig,
            lambda: daemon.request_shutdown()
        )


async def run_daemon(config: Optional[DaemonConfig] = None) -> None:
    """Run the daemon with signal handling."""
    daemon = AgentMonitorDaemon(config)

    try:
        setup_signal_handlers(daemon)
        await daemon.start()
    finally:
        await daemon.stop()
