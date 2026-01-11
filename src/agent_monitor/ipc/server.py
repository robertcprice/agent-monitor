"""Unix socket server for IPC communication."""

import asyncio
import json
import logging
from pathlib import Path
from typing import Any, Callable, Optional, Awaitable

from agent_monitor.models import SessionEvent
from agent_monitor.events.bus import EventFilter

logger = logging.getLogger(__name__)


class IPCServer:
    """
    Unix socket server for client communication.

    Supports:
    - JSON-based request/response protocol
    - Event streaming with filters
    - Multiple concurrent clients
    """

    def __init__(
        self,
        socket_path: Path,
        request_handler: Callable[[dict], Awaitable[dict]],
    ):
        self.socket_path = socket_path
        self.request_handler = request_handler
        self.server: Optional[asyncio.AbstractServer] = None
        self.clients: list[asyncio.StreamWriter] = []
        self._subscriptions: dict[asyncio.StreamWriter, EventFilter] = {}
        self._running = False

    async def start(self) -> None:
        """Start the Unix socket server."""
        # Remove stale socket
        if self.socket_path.exists():
            self.socket_path.unlink()

        self.server = await asyncio.start_unix_server(
            self._handle_client,
            path=str(self.socket_path),
        )

        # Set permissions (owner only)
        self.socket_path.chmod(0o600)

        self._running = True
        logger.info(f"IPC server started at {self.socket_path}")

    async def stop(self) -> None:
        """Stop the server."""
        self._running = False

        # Close all client connections
        for writer in self.clients[:]:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

        if self.server:
            self.server.close()
            await self.server.wait_closed()

        # Remove socket file
        if self.socket_path.exists():
            self.socket_path.unlink()

        logger.info("IPC server stopped")

    async def _handle_client(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        """Handle a client connection."""
        self.clients.append(writer)
        logger.debug("Client connected")

        try:
            while self._running:
                # Read a line (JSON message)
                data = await reader.readline()
                if not data:
                    break

                try:
                    request = json.loads(data.decode())
                    response = await self._process_request(request, writer)

                    # Send response
                    writer.write(json.dumps(response).encode() + b"\n")
                    await writer.drain()

                except json.JSONDecodeError as e:
                    error_response = {"error": f"Invalid JSON: {e}"}
                    writer.write(json.dumps(error_response).encode() + b"\n")
                    await writer.drain()

        except asyncio.CancelledError:
            pass
        except ConnectionResetError:
            pass
        except Exception as e:
            logger.error(f"Client error: {e}")
        finally:
            # Clean up
            if writer in self._subscriptions:
                del self._subscriptions[writer]
            if writer in self.clients:
                self.clients.remove(writer)

            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

            logger.debug("Client disconnected")

    async def _process_request(
        self,
        request: dict,
        writer: asyncio.StreamWriter,
    ) -> dict:
        """Process a client request."""
        action = request.get("action")

        # Handle subscription specially
        if action == "subscribe":
            return self._handle_subscribe(request, writer)

        if action == "unsubscribe":
            return self._handle_unsubscribe(writer)

        # Delegate to handler
        try:
            return await self.request_handler(request)
        except Exception as e:
            logger.error(f"Request handler error: {e}")
            return {"error": str(e)}

    def _handle_subscribe(
        self,
        request: dict,
        writer: asyncio.StreamWriter,
    ) -> dict:
        """Subscribe client to event stream."""
        filters = request.get("filters", {})
        event_filter = EventFilter.from_dict(filters)
        self._subscriptions[writer] = event_filter

        return {"status": "subscribed", "filters": filters}

    def _handle_unsubscribe(self, writer: asyncio.StreamWriter) -> dict:
        """Unsubscribe client from event stream."""
        if writer in self._subscriptions:
            del self._subscriptions[writer]

        return {"status": "unsubscribed"}

    async def broadcast_event(self, event: SessionEvent) -> None:
        """Broadcast event to subscribed clients."""
        message = json.dumps({
            "type": "event",
            "data": event.to_dict(),
        }).encode() + b"\n"

        for writer, event_filter in list(self._subscriptions.items()):
            if not event_filter.matches(event):
                continue

            try:
                writer.write(message)
                await writer.drain()
            except Exception:
                # Client disconnected
                if writer in self._subscriptions:
                    del self._subscriptions[writer]
                if writer in self.clients:
                    self.clients.remove(writer)

    @property
    def client_count(self) -> int:
        """Number of connected clients."""
        return len(self.clients)

    @property
    def subscriber_count(self) -> int:
        """Number of subscribed clients."""
        return len(self._subscriptions)


async def create_request_handler(daemon: "AgentMonitorDaemon") -> Callable:
    """Create a request handler for the IPC server."""

    async def handler(request: dict) -> dict:
        action = request.get("action")

        if action == "get_status":
            return await daemon.get_status()

        if action == "get_sessions":
            sessions = await daemon.storage.get_active_sessions()
            return {
                "sessions": [s.to_dict() for s in sessions],
            }

        if action == "get_session":
            session_id = request.get("session_id")
            if not session_id:
                return {"error": "session_id required"}

            session = await daemon.storage.get_session(session_id)
            if session:
                return {"session": session.to_dict()}
            return {"error": "Session not found"}

        if action == "get_metrics":
            hours = request.get("hours", 24)
            metrics = await daemon.storage.get_summary_metrics(hours=hours)
            return {"metrics": metrics}

        if action == "get_events":
            session_id = request.get("session_id")
            limit = request.get("limit", 100)

            if session_id:
                events = await daemon.storage.get_session_events(session_id, limit=limit)
            else:
                events = await daemon.storage.get_recent_events(limit=limit)

            return {"events": [e.to_dict() for e in events]}

        if action == "hook_event":
            # Handle hook event from Claude Code
            event_type = request.get("event_type")
            data = request.get("data", {})

            # TODO: Process hook event
            return {"status": "received"}

        return {"error": f"Unknown action: {action}"}

    return handler
