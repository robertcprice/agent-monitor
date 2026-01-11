"""Plugin system for custom agent adapters."""

import asyncio
import json
import logging
import re
from datetime import datetime
from pathlib import Path
from typing import Optional, Any
import uuid

import psutil
import yaml
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


class PluginManifest:
    """Configuration for a custom agent plugin."""

    def __init__(self, data: dict[str, Any]):
        self.name = data.get("name", "unknown")
        self.display_name = data.get("display_name", self.name)
        self.description = data.get("description", "")
        self.version = data.get("version", "1.0.0")

        # Process detection
        self.process_pattern = data.get("process_pattern", "")
        self.process_name = data.get("process_name", "")

        # Log/data paths
        self.log_path = data.get("log_path", "")
        self.history_path = data.get("history_path", "")
        self.data_dir = data.get("data_dir", "")

        # Log parsing rules
        self.log_format = data.get("log_format", "plain")  # plain, json, csv
        self.message_pattern = data.get("message_pattern", "")
        self.event_patterns = data.get("event_patterns", {})

        # Capabilities
        capabilities = data.get("capabilities", {})
        self.capabilities = {
            "real_time_events": capabilities.get("real_time_events", False),
            "historical_data": capabilities.get("historical_data", True),
            "token_tracking": capabilities.get("token_tracking", False),
            "cost_tracking": capabilities.get("cost_tracking", False),
            "file_change_tracking": capabilities.get("file_change_tracking", False),
            "transcript_access": capabilities.get("transcript_access", False),
        }

        # Poll interval in seconds
        self.poll_interval = data.get("poll_interval", 30)

    @classmethod
    def from_file(cls, path: Path) -> "PluginManifest":
        """Load manifest from a YAML file."""
        with open(path) as f:
            if path.suffix in (".yaml", ".yml"):
                data = yaml.safe_load(f)
            else:
                data = json.load(f)
        return cls(data)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary."""
        return {
            "name": self.name,
            "display_name": self.display_name,
            "description": self.description,
            "version": self.version,
            "process_pattern": self.process_pattern,
            "process_name": self.process_name,
            "log_path": self.log_path,
            "history_path": self.history_path,
            "capabilities": self.capabilities,
        }


class PluginAdapter(BaseAdapter):
    """
    Generic adapter for custom agents defined via manifest files.

    Supports:
    - Process detection via name or cmdline pattern
    - Log file parsing with configurable patterns
    - Periodic polling for updates
    """

    def __init__(
        self,
        manifest: PluginManifest,
        config: DaemonConfig,
        event_bus: "EventBus",
        storage: "StorageManager",
    ):
        super().__init__(
            name=manifest.name,
            agent_type=AgentType.CUSTOM,
            data_sources=[DataSource.PROCESS, DataSource.FILES],
            config=config,
            event_bus=event_bus,
            storage=storage,
        )

        self.manifest = manifest
        self._poll_task: Optional[asyncio.Task] = None
        self._last_log_pos: dict[str, int] = {}

    def get_capabilities(self) -> dict[str, bool]:
        """Return capabilities from manifest."""
        return self.manifest.capabilities

    async def start(self) -> None:
        """Start the plugin adapter."""
        self._running = True
        self.status = AdapterStatus.DISCOVERING

        await self.discover_sessions()

        self._poll_task = asyncio.create_task(self._poll_loop())

        self.status = AdapterStatus.CONNECTED
        logger.info(f"Plugin adapter '{self.manifest.name}' started")

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
        logger.info(f"Plugin adapter '{self.manifest.name}' stopped")

    async def discover_sessions(self) -> list[UnifiedSession]:
        """Discover sessions based on manifest configuration."""
        sessions = []

        # Process detection
        if self.manifest.process_pattern or self.manifest.process_name:
            proc_sessions = await self._discover_from_processes()
            sessions.extend(proc_sessions)

        # Log/data file discovery
        if self.manifest.log_path or self.manifest.data_dir:
            file_sessions = await self._discover_from_files()
            sessions.extend(file_sessions)

        # Deduplicate and register
        seen = set()
        unique = []
        for session in sessions:
            if session.project_path not in seen:
                seen.add(session.project_path)
                unique.append(session)
                await self.register_session(session)

        return unique

    async def _discover_from_processes(self) -> list[UnifiedSession]:
        """Find processes matching the manifest pattern."""
        sessions = []
        pattern = self.manifest.process_pattern
        name_match = self.manifest.process_name.lower()

        for proc in psutil.process_iter(["pid", "name", "cmdline", "cwd", "create_time"]):
            try:
                info = proc.info
                proc_name = (info.get("name") or "").lower()
                cmdline_list = info.get("cmdline") or []
                cmdline = " ".join(cmdline_list) if cmdline_list else ""

                # Check name match
                name_matches = name_match and name_match in proc_name

                # Check pattern match
                pattern_matches = pattern and re.search(pattern, cmdline, re.IGNORECASE)

                if name_matches or pattern_matches:
                    cwd = info.get("cwd") or ""
                    session = UnifiedSession.create(
                        agent_type=AgentType.CUSTOM,
                        project_path=cwd or f"/proc/{info['pid']}",
                        external_id=f"{self.manifest.name}_proc_{info['pid']}",
                        pid=info["pid"],
                        status=SessionStatus.ACTIVE,
                        metadata={
                            "source": "process",
                            "plugin": self.manifest.name,
                            "process_name": proc_name,
                        },
                    )
                    if info.get("create_time"):
                        session.started_at = datetime.fromtimestamp(info["create_time"])
                    sessions.append(session)

            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        return sessions

    async def _discover_from_files(self) -> list[UnifiedSession]:
        """Discover sessions from log/data files."""
        sessions = []

        # Expand path patterns
        log_path = Path(self.manifest.log_path).expanduser() if self.manifest.log_path else None
        data_dir = Path(self.manifest.data_dir).expanduser() if self.manifest.data_dir else None

        if log_path and log_path.exists():
            if log_path.is_file():
                session = await self._session_from_log(log_path)
                if session:
                    sessions.append(session)
            elif log_path.is_dir():
                for log_file in log_path.glob("*.log"):
                    session = await self._session_from_log(log_file)
                    if session:
                        sessions.append(session)

        if data_dir and data_dir.is_dir():
            # Look for session markers
            for marker in data_dir.iterdir():
                if marker.is_dir():
                    session = UnifiedSession.create(
                        agent_type=AgentType.CUSTOM,
                        project_path=str(marker),
                        external_id=f"{self.manifest.name}_{marker.name}",
                        status=SessionStatus.COMPLETED,
                        metadata={
                            "source": "data_dir",
                            "plugin": self.manifest.name,
                        },
                    )
                    sessions.append(session)

        return sessions

    async def _session_from_log(self, log_path: Path) -> Optional[UnifiedSession]:
        """Create a session from a log file."""
        try:
            stat = log_path.stat()

            # Count lines/messages
            with open(log_path) as f:
                lines = f.readlines()

            message_count = len(lines)
            if self.manifest.message_pattern:
                message_count = sum(
                    1 for line in lines
                    if re.search(self.manifest.message_pattern, line)
                )

            session = UnifiedSession.create(
                agent_type=AgentType.CUSTOM,
                project_path=str(log_path.parent),
                external_id=f"{self.manifest.name}_{log_path.stem}",
                message_count=message_count,
                status=SessionStatus.COMPLETED,
                metadata={
                    "source": "log_file",
                    "plugin": self.manifest.name,
                    "log_file": str(log_path),
                },
            )
            session.last_activity_at = datetime.fromtimestamp(stat.st_mtime)

            return session

        except Exception as e:
            logger.debug(f"Error parsing log {log_path}: {e}")
            return None

    async def _poll_loop(self) -> None:
        """Poll for changes based on manifest interval."""
        while self._running:
            try:
                await self._check_processes()
                await self._check_logs()
                await asyncio.sleep(self.manifest.poll_interval)

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Plugin '{self.manifest.name}' poll error: {e}")
                await asyncio.sleep(60)

    async def _check_processes(self) -> None:
        """Check for process changes."""
        current_pids = set()

        pattern = self.manifest.process_pattern
        name_match = self.manifest.process_name.lower()

        for proc in psutil.process_iter(["pid", "name", "cmdline"]):
            try:
                proc_name = (proc.info.get("name") or "").lower()
                cmdline = " ".join(proc.info.get("cmdline") or [])

                if (name_match and name_match in proc_name) or \
                   (pattern and re.search(pattern, cmdline, re.IGNORECASE)):
                    current_pids.add(proc.info["pid"])

            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        # Mark ended sessions
        for session in list(self._sessions.values()):
            if session.pid and session.pid not in current_pids:
                if session.status == SessionStatus.ACTIVE:
                    session.status = SessionStatus.COMPLETED
                    session.end()
                    await self.update_session(session)

    async def _check_logs(self) -> None:
        """Check log files for new content."""
        log_path = Path(self.manifest.log_path).expanduser() if self.manifest.log_path else None
        if not log_path or not log_path.exists():
            return

        log_files = [log_path] if log_path.is_file() else list(log_path.glob("*.log"))

        for log_file in log_files:
            try:
                last_pos = self._last_log_pos.get(str(log_file), 0)

                with open(log_file) as f:
                    f.seek(0, 2)
                    current_size = f.tell()

                    if current_size > last_pos:
                        f.seek(last_pos)
                        new_content = f.read()
                        self._last_log_pos[str(log_file)] = f.tell()

                        await self._process_log_content(log_file, new_content)

            except Exception as e:
                logger.debug(f"Error checking log {log_file}: {e}")

    async def _process_log_content(self, log_file: Path, content: str) -> None:
        """Process new log content and emit events."""
        # Find matching session
        session = None
        for s in self._sessions.values():
            if s.metadata.get("log_file") == str(log_file):
                session = s
                break

        if not session:
            return

        # Parse based on format
        if self.manifest.log_format == "json":
            await self._parse_json_log(session, content)
        else:
            await self._parse_plain_log(session, content)

    async def _parse_plain_log(self, session: UnifiedSession, content: str) -> None:
        """Parse plain text log content."""
        lines = content.strip().split("\n")

        for line in lines:
            if not line.strip():
                continue

            # Check event patterns
            for event_name, pattern in self.manifest.event_patterns.items():
                if re.search(pattern, line):
                    event = SessionEvent.create(
                        session_id=session.id,
                        event_type=EventType.CUSTOM,
                        agent_type=AgentType.CUSTOM,
                        content=line[:500],
                        working_directory=session.project_path,
                        raw_data={"event": event_name, "line": line},
                    )
                    await self.emit_event(event)

            # Count as message if matches message pattern
            if self.manifest.message_pattern:
                if re.search(self.manifest.message_pattern, line):
                    session.message_count += 1

        session.update_activity()
        await self.update_session(session)

    async def _parse_json_log(self, session: UnifiedSession, content: str) -> None:
        """Parse JSON log content (one JSON object per line)."""
        for line in content.strip().split("\n"):
            if not line.strip():
                continue

            try:
                entry = json.loads(line)

                event = SessionEvent.create(
                    session_id=session.id,
                    event_type=EventType.CUSTOM,
                    agent_type=AgentType.CUSTOM,
                    content=str(entry)[:500],
                    working_directory=session.project_path,
                    raw_data=entry,
                )
                await self.emit_event(event)

                session.message_count += 1

            except json.JSONDecodeError:
                continue

        session.update_activity()
        await self.update_session(session)


class PluginDiscovery:
    """Discovers and loads plugin manifests."""

    def __init__(self, config: DaemonConfig):
        self.config = config
        self.plugins_dir = config.config_dir / "adapters"
        self.manifests: dict[str, PluginManifest] = {}

    def discover(self) -> list[PluginManifest]:
        """Discover all plugin manifests."""
        manifests = []

        if not self.plugins_dir.exists():
            return manifests

        # Load YAML and JSON manifests
        for path in self.plugins_dir.iterdir():
            if path.suffix in (".yaml", ".yml", ".json"):
                try:
                    manifest = PluginManifest.from_file(path)
                    self.manifests[manifest.name] = manifest
                    manifests.append(manifest)
                    logger.info(f"Loaded plugin manifest: {manifest.name}")
                except Exception as e:
                    logger.error(f"Error loading plugin {path}: {e}")

        return manifests

    def get_manifest(self, name: str) -> Optional[PluginManifest]:
        """Get a manifest by name."""
        return self.manifests.get(name)

    def create_adapter(
        self,
        manifest: PluginManifest,
        event_bus: "EventBus",
        storage: "StorageManager",
    ) -> PluginAdapter:
        """Create an adapter from a manifest."""
        return PluginAdapter(manifest, self.config, event_bus, storage)


# Example manifest for reference
EXAMPLE_MANIFEST = """
# Example custom agent manifest
# Save as ~/.config/agent-monitor/adapters/my_agent.yaml

name: my_custom_agent
display_name: My Custom Agent
description: A custom AI agent for specific tasks
version: 1.0.0

# Process detection (optional)
process_pattern: "python.*my_agent"
process_name: my_agent

# Data paths (optional, supports ~ expansion)
log_path: ~/.my_agent/logs/
history_path: ~/.my_agent/history.jsonl
data_dir: ~/.my_agent/sessions/

# Log parsing
log_format: plain  # plain, json
message_pattern: "\\[USER\\]|\\[AGENT\\]"
event_patterns:
  tool_use: "\\[TOOL\\]"
  error: "\\[ERROR\\]"

# Capabilities
capabilities:
  historical_data: true
  token_tracking: false
  file_change_tracking: true

# Poll interval in seconds
poll_interval: 30
"""
