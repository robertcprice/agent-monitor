"""Cursor AI adapter with process detection and log parsing."""

import asyncio
import json
import logging
import re
from datetime import datetime
from pathlib import Path
from typing import Optional, Any
import uuid

import psutil

from agent_monitor.adapters.base import BaseAdapter
from agent_monitor.models import (
    UnifiedSession,
    SessionEvent,
    AgentType,
    SessionStatus,
    EventType,
    DataSource,
    AdapterStatus,
)
from agent_monitor.config import DaemonConfig

logger = logging.getLogger(__name__)


class CursorAdapter(BaseAdapter):
    """
    Adapter for Cursor AI code editor.

    Data sources:
    1. Process detection - Find running Cursor instances
    2. IDE state - Recently viewed files, project info
    3. Logs - Activity from log files
    """

    def __init__(
        self,
        config: DaemonConfig,
        event_bus: "EventBus",
        storage: "StorageManager",
    ):
        super().__init__(
            name="cursor",
            agent_type=AgentType.CURSOR,
            data_sources=[DataSource.PROCESS, DataSource.FILES],
            config=config,
            event_bus=event_bus,
            storage=storage,
        )

        # Cursor paths
        self.cursor_support = Path.home() / "Library/Application Support/Cursor"
        self.cursor_config = Path.home() / ".cursor"
        self.ide_state_file = self.cursor_config / "ide_state.json"
        self.logs_dir = self.cursor_support / "logs"

        self._poll_task: Optional[asyncio.Task] = None
        self._last_log_check: dict[str, int] = {}  # log_path -> last_pos

    def get_capabilities(self) -> dict[str, bool]:
        """Cursor has limited capabilities compared to Claude Code."""
        return {
            "real_time_events": False,  # No hooks
            "historical_data": True,
            "send_commands": False,
            "token_tracking": False,
            "cost_tracking": False,
            "file_change_tracking": True,
            "subagent_tracking": False,
            "hook_integration": False,
            "transcript_access": False,
        }

    async def start(self) -> None:
        """Start the Cursor adapter."""
        self._running = True
        self.status = AdapterStatus.DISCOVERING

        # Initial discovery
        await self.discover_sessions()

        # Start polling for changes
        self._poll_task = asyncio.create_task(self._poll_loop())

        self.status = AdapterStatus.CONNECTED
        logger.info("Cursor adapter started")

    async def stop(self) -> None:
        """Stop the adapter."""
        self._running = False

        if self._poll_task:
            self._poll_task.cancel()
            try:
                await self._poll_task
            except asyncio.CancelledError:
                pass

        self.status = AdapterStatus.INACTIVE
        logger.info("Cursor adapter stopped")

    async def discover_sessions(self) -> list[UnifiedSession]:
        """Discover Cursor sessions from processes and state files."""
        sessions = []

        # Method 1: Process detection
        proc_sessions = await self._discover_from_processes()
        sessions.extend(proc_sessions)

        # Method 2: IDE state (recent projects)
        state_sessions = await self._discover_from_ide_state()
        sessions.extend(state_sessions)

        # Deduplicate
        seen = set()
        unique_sessions = []
        for session in sessions:
            key = session.project_path
            if key not in seen:
                seen.add(key)
                unique_sessions.append(session)
                await self.register_session(session)

        return unique_sessions

    async def _discover_from_processes(self) -> list[UnifiedSession]:
        """Find running Cursor processes."""
        sessions = []

        for proc in psutil.process_iter(["pid", "name", "cmdline", "cwd", "create_time"]):
            try:
                info = proc.info
                name = (info.get("name") or "").lower()
                cmdline_list = info.get("cmdline") or []
                cmdline = " ".join(cmdline_list) if cmdline_list else ""

                # Check if this is a Cursor process (main Electron app)
                if "cursor" in name and "helper" not in name.lower():
                    # Try to get the workspace from cmdline
                    workspace = self._extract_workspace(cmdline_list)
                    if not workspace:
                        workspace = info.get("cwd") or ""

                    if workspace:
                        session = UnifiedSession.create(
                            agent_type=AgentType.CURSOR,
                            project_path=workspace,
                            external_id=f"cursor_proc_{info['pid']}",
                            pid=info["pid"],
                            metadata={
                                "source": "process",
                                "process_name": name,
                            },
                        )
                        if info.get("create_time"):
                            session.started_at = datetime.fromtimestamp(info["create_time"])
                        sessions.append(session)

            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        return sessions

    def _extract_workspace(self, cmdline: list[str]) -> Optional[str]:
        """Extract workspace path from Cursor command line."""
        for i, arg in enumerate(cmdline):
            # Look for folder arguments
            if arg == "--folder-uri" and i + 1 < len(cmdline):
                uri = cmdline[i + 1]
                if uri.startswith("file://"):
                    return uri[7:]  # Remove file:// prefix
            # Look for direct paths
            if arg.startswith("/") and Path(arg).is_dir():
                return arg

        return None

    async def _discover_from_ide_state(self) -> list[UnifiedSession]:
        """Parse IDE state for recent files/projects."""
        sessions = []

        if not self.ide_state_file.exists():
            return sessions

        try:
            with open(self.ide_state_file) as f:
                state = json.load(f)

            recent_files = state.get("recentlyViewedFiles", [])

            # Group by project directory
            projects: dict[str, list[str]] = {}
            for file_info in recent_files:
                path = file_info.get("absolutePath", "")
                if not path:
                    continue

                # Find project root (look for common markers)
                project = self._find_project_root(Path(path))
                if project:
                    if project not in projects:
                        projects[project] = []
                    projects[project].append(path)

            # Create sessions for each project
            for project_path, files in projects.items():
                session = UnifiedSession.create(
                    agent_type=AgentType.CURSOR,
                    project_path=project_path,
                    external_id=f"cursor_state_{hash(project_path) % 100000}",
                    file_operations=len(files),
                    metadata={
                        "source": "ide_state",
                        "recent_files": files[:5],  # Keep first 5
                    },
                )
                session.status = SessionStatus.IDLE
                sessions.append(session)

        except Exception as e:
            logger.error(f"Error parsing Cursor IDE state: {e}")

        return sessions

    def _find_project_root(self, file_path: Path) -> Optional[str]:
        """Find the project root for a file."""
        markers = [".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod"]

        current = file_path.parent
        for _ in range(10):  # Max 10 levels up
            for marker in markers:
                if (current / marker).exists():
                    return str(current)
            parent = current.parent
            if parent == current:
                break
            current = parent

        return None

    async def _poll_loop(self) -> None:
        """Poll for changes periodically."""
        while self._running:
            try:
                # Check for new processes
                await self._check_processes()

                # Check log files for activity
                await self._check_logs()

                # Wait before next poll
                await asyncio.sleep(30)  # Poll every 30 seconds

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Cursor poll error: {e}")
                await asyncio.sleep(60)

    async def _check_processes(self) -> None:
        """Check for new or ended Cursor processes."""
        current_pids = set()

        for proc in psutil.process_iter(["pid", "name"]):
            try:
                name = (proc.info.get("name") or "").lower()
                if "cursor" in name and "helper" not in name:
                    current_pids.add(proc.info["pid"])
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        # Check for ended sessions
        for session in list(self._sessions.values()):
            if session.pid and session.pid not in current_pids:
                session.status = SessionStatus.COMPLETED
                session.end()
                await self.update_session(session)

    async def _check_logs(self) -> None:
        """Check Cursor logs for new activity."""
        if not self.logs_dir.exists():
            return

        # Find latest log directory
        log_dirs = sorted(self.logs_dir.iterdir(), reverse=True)
        if not log_dirs:
            return

        latest_log_dir = log_dirs[0]

        # Check main.log for activity
        main_log = latest_log_dir / "main.log"
        if main_log.exists():
            await self._parse_main_log(main_log)

    async def _parse_main_log(self, log_path: Path) -> None:
        """Parse Cursor main.log for events."""
        try:
            last_pos = self._last_log_check.get(str(log_path), 0)

            with open(log_path) as f:
                f.seek(last_pos)
                new_lines = f.readlines()
                self._last_log_check[str(log_path)] = f.tell()

            for line in new_lines:
                # Look for file open events
                if "openTextDocument" in line or "didOpen" in line:
                    # Extract file path if present
                    match = re.search(r'file://([^\s\]"]+)', line)
                    if match:
                        file_path = match.group(1)
                        await self._handle_file_event(file_path, "opened")

        except Exception as e:
            logger.debug(f"Error parsing Cursor log: {e}")

    async def _handle_file_event(self, file_path: str, action: str) -> None:
        """Handle a file event from Cursor."""
        project_root = self._find_project_root(Path(file_path))
        if not project_root:
            return

        # Find or create session
        session = None
        for s in self._sessions.values():
            if s.project_path == project_root:
                session = s
                break

        if not session:
            session = UnifiedSession.create(
                agent_type=AgentType.CURSOR,
                project_path=project_root,
                external_id=f"cursor_{hash(project_root) % 100000}",
            )
            await self.register_session(session)

        # Update session
        session.update_activity()
        session.file_operations += 1

        # Emit event
        event = SessionEvent.create(
            session_id=session.id,
            event_type=EventType.FILE_READ,
            agent_type=AgentType.CURSOR,
            content=f"File {action}: {Path(file_path).name}",
            working_directory=project_root,
            raw_data={"file": file_path, "action": action},
        )
        await self.emit_event(event)
        await self.update_session(session)
