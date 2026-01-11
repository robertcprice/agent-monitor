"""Aider AI adapter with process detection and chat history parsing."""

import asyncio
import json
import logging
import re
from datetime import datetime
from pathlib import Path
from typing import Optional, Any
import uuid

import psutil
from watchfiles import awatch, Change

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


class AiderAdapter(BaseAdapter):
    """
    Adapter for Aider AI pair programming tool.

    Data sources:
    1. Process detection - Find running aider processes
    2. Chat history - Parse .aider.chat.history.md files
    3. Input history - Parse .aider.input.history
    """

    def __init__(
        self,
        config: DaemonConfig,
        event_bus: "EventBus",
        storage: "StorageManager",
    ):
        super().__init__(
            name="aider",
            agent_type=AgentType.AIDER,
            data_sources=[DataSource.PROCESS, DataSource.FILES],
            config=config,
            event_bus=event_bus,
            storage=storage,
        )

        # Common paths where aider creates files
        self.home_dir = Path.home()
        self.aider_config = self.home_dir / ".aider.conf.yml"

        self._poll_task: Optional[asyncio.Task] = None
        self._watcher_task: Optional[asyncio.Task] = None
        self._watched_projects: set[str] = set()
        self._last_history_pos: dict[str, int] = {}  # history_file -> last_pos

    def get_capabilities(self) -> dict[str, bool]:
        """Aider capabilities based on available data."""
        return {
            "real_time_events": False,  # No hooks, but can watch files
            "historical_data": True,
            "send_commands": False,
            "token_tracking": True,  # Aider shows token usage
            "cost_tracking": True,  # Aider shows costs
            "file_change_tracking": True,
            "subagent_tracking": False,
            "hook_integration": False,
            "transcript_access": True,  # Chat history files
        }

    async def start(self) -> None:
        """Start the Aider adapter."""
        self._running = True
        self.status = AdapterStatus.DISCOVERING

        # Initial discovery
        await self.discover_sessions()

        # Start polling for processes
        self._poll_task = asyncio.create_task(self._poll_loop())

        self.status = AdapterStatus.CONNECTED
        logger.info("Aider adapter started")

    async def stop(self) -> None:
        """Stop the adapter."""
        self._running = False

        if self._poll_task:
            self._poll_task.cancel()
            try:
                await self._poll_task
            except asyncio.CancelledError:
                pass

        if self._watcher_task:
            self._watcher_task.cancel()
            try:
                await self._watcher_task
            except asyncio.CancelledError:
                pass

        self.status = AdapterStatus.INACTIVE
        logger.info("Aider adapter stopped")

    async def discover_sessions(self) -> list[UnifiedSession]:
        """Discover Aider sessions from processes and history files."""
        sessions = []

        # Method 1: Process detection
        proc_sessions = await self._discover_from_processes()
        sessions.extend(proc_sessions)

        # Method 2: Find .aider files in common project locations
        history_sessions = await self._discover_from_history_files()
        sessions.extend(history_sessions)

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
        """Find running aider processes."""
        sessions = []

        for proc in psutil.process_iter(["pid", "name", "cmdline", "cwd", "create_time"]):
            try:
                info = proc.info
                cmdline_list = info.get("cmdline") or []
                cmdline = " ".join(cmdline_list).lower() if cmdline_list else ""

                # Check if this is an aider process
                if "aider" in cmdline or (info.get("name") or "").lower() == "aider":
                    cwd = info.get("cwd") or ""
                    if cwd:
                        session = UnifiedSession.create(
                            agent_type=AgentType.AIDER,
                            project_path=cwd,
                            external_id=f"aider_proc_{info['pid']}",
                            pid=info["pid"],
                            status=SessionStatus.ACTIVE,
                            metadata={
                                "source": "process",
                                "cmdline": cmdline_list,
                            },
                        )
                        if info.get("create_time"):
                            session.started_at = datetime.fromtimestamp(info["create_time"])

                        # Try to get model from cmdline
                        model = self._extract_model(cmdline_list)
                        if model:
                            session.model_id = model

                        sessions.append(session)
                        self._watched_projects.add(cwd)

            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        return sessions

    def _extract_model(self, cmdline: list[str]) -> Optional[str]:
        """Extract model name from aider command line."""
        for i, arg in enumerate(cmdline):
            if arg in ("--model", "-m") and i + 1 < len(cmdline):
                return cmdline[i + 1]
            if arg.startswith("--model="):
                return arg.split("=", 1)[1]
        return None

    async def _discover_from_history_files(self) -> list[UnifiedSession]:
        """Find projects with .aider history files."""
        sessions = []

        # Search in common project locations
        search_paths = [
            self.home_dir / "projects",
            self.home_dir / "code",
            self.home_dir / "dev",
            self.home_dir / "src",
            self.home_dir / "Documents",
        ]

        for search_path in search_paths:
            if not search_path.exists():
                continue

            # Find .aider.chat.history.md files
            try:
                for history_file in search_path.rglob(".aider.chat.history.md"):
                    project_path = str(history_file.parent)

                    # Skip if already found via process
                    if project_path in self._watched_projects:
                        continue

                    # Parse history for session info
                    session_info = await self._parse_chat_history(history_file)

                    session = UnifiedSession.create(
                        agent_type=AgentType.AIDER,
                        project_path=project_path,
                        external_id=f"aider_history_{hash(project_path) % 100000}",
                        message_count=session_info.get("message_count", 0),
                        tokens_input=session_info.get("tokens_input", 0),
                        tokens_output=session_info.get("tokens_output", 0),
                        estimated_cost=session_info.get("cost", 0.0),
                        status=SessionStatus.COMPLETED,
                        metadata={
                            "source": "history_file",
                            "model": session_info.get("model"),
                            "last_message": session_info.get("last_message", "")[:100],
                        },
                    )

                    if session_info.get("last_timestamp"):
                        session.last_activity_at = session_info["last_timestamp"]

                    sessions.append(session)
                    self._watched_projects.add(project_path)

            except Exception as e:
                logger.debug(f"Error searching {search_path}: {e}")

        return sessions

    async def _parse_chat_history(self, history_file: Path) -> dict[str, Any]:
        """Parse an aider chat history file."""
        result = {
            "message_count": 0,
            "tokens_input": 0,
            "tokens_output": 0,
            "cost": 0.0,
            "model": None,
            "last_message": "",
            "last_timestamp": None,
        }

        try:
            with open(history_file) as f:
                content = f.read()

            # Count user messages (lines starting with #### USER)
            user_messages = content.count("#### USER")
            result["message_count"] = user_messages

            # Extract token usage if present
            # Aider format: "Tokens: 1,234 input, 567 output"
            token_matches = re.findall(
                r"Tokens:\s*([\d,]+)\s*input,\s*([\d,]+)\s*output", content
            )
            for match in token_matches:
                result["tokens_input"] += int(match[0].replace(",", ""))
                result["tokens_output"] += int(match[1].replace(",", ""))

            # Extract cost if present
            # Aider format: "Cost: $0.12"
            cost_matches = re.findall(r"Cost:\s*\$?([\d.]+)", content)
            for match in cost_matches:
                result["cost"] += float(match)

            # Get last message
            lines = content.strip().split("\n")
            for line in reversed(lines):
                if line.strip() and not line.startswith("#"):
                    result["last_message"] = line.strip()
                    break

            # Get file modification time as last activity
            stat = history_file.stat()
            result["last_timestamp"] = datetime.fromtimestamp(stat.st_mtime)

        except Exception as e:
            logger.debug(f"Error parsing aider history {history_file}: {e}")

        return result

    async def _poll_loop(self) -> None:
        """Poll for changes periodically."""
        while self._running:
            try:
                # Check for new/ended processes
                await self._check_processes()

                # Check history files for updates
                await self._check_history_files()

                await asyncio.sleep(30)

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Aider poll error: {e}")
                await asyncio.sleep(60)

    async def _check_processes(self) -> None:
        """Check for new or ended aider processes."""
        current_pids = set()

        for proc in psutil.process_iter(["pid", "cmdline"]):
            try:
                cmdline = " ".join(proc.info.get("cmdline") or []).lower()
                if "aider" in cmdline:
                    current_pids.add(proc.info["pid"])
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        # Check for ended sessions
        for session in list(self._sessions.values()):
            if session.pid and session.pid not in current_pids:
                if session.status == SessionStatus.ACTIVE:
                    session.status = SessionStatus.COMPLETED
                    session.end()
                    await self.update_session(session)

    async def _check_history_files(self) -> None:
        """Check history files for new content."""
        for project_path in self._watched_projects:
            history_file = Path(project_path) / ".aider.chat.history.md"
            if not history_file.exists():
                continue

            try:
                last_pos = self._last_history_pos.get(str(history_file), 0)

                with open(history_file) as f:
                    f.seek(0, 2)  # Seek to end
                    current_size = f.tell()

                    if current_size > last_pos:
                        f.seek(last_pos)
                        new_content = f.read()
                        self._last_history_pos[str(history_file)] = f.tell()

                        # Find session and emit events
                        await self._process_new_content(project_path, new_content)

            except Exception as e:
                logger.debug(f"Error checking aider history: {e}")

    async def _process_new_content(self, project_path: str, content: str) -> None:
        """Process new content from history file."""
        # Find session
        session = None
        for s in self._sessions.values():
            if s.project_path == project_path:
                session = s
                break

        if not session:
            return

        # Count new messages
        new_user_messages = content.count("#### USER")
        new_assistant_messages = content.count("#### ASSISTANT")

        if new_user_messages > 0:
            session.message_count += new_user_messages
            session.update_activity()

            # Emit event for new user message
            event = SessionEvent.create(
                session_id=session.id,
                event_type=EventType.PROMPT_RECEIVED,
                agent_type=AgentType.AIDER,
                content=f"User message in aider session",
                working_directory=project_path,
            )
            await self.emit_event(event)

        if new_assistant_messages > 0:
            # Emit event for assistant response
            event = SessionEvent.create(
                session_id=session.id,
                event_type=EventType.RESPONSE_GENERATED,
                agent_type=AgentType.AIDER,
                content=f"Aider response",
                working_directory=project_path,
            )
            await self.emit_event(event)

        await self.update_session(session)

    async def parse_full_history(self, project_path: str) -> list[dict]:
        """Parse full chat history for a project."""
        history_file = Path(project_path) / ".aider.chat.history.md"
        if not history_file.exists():
            return []

        messages = []
        try:
            with open(history_file) as f:
                content = f.read()

            # Split by message markers
            parts = re.split(r"#### (USER|ASSISTANT)", content)

            current_role = None
            for part in parts:
                part = part.strip()
                if part == "USER":
                    current_role = "user"
                elif part == "ASSISTANT":
                    current_role = "assistant"
                elif current_role and part:
                    messages.append({
                        "role": current_role,
                        "content": part[:500],  # Truncate
                    })

        except Exception as e:
            logger.error(f"Error parsing aider history: {e}")

        return messages
