"""Claude Code adapter with native hook integration and file monitoring."""

import asyncio
import json
import logging
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
from agent_monitor.config import DaemonConfig, calculate_cost

logger = logging.getLogger(__name__)


class ClaudeCodeAdapter(BaseAdapter):
    """
    First-class adapter for Claude Code with native hook integration.

    Data sources:
    1. Hooks (real-time) - SessionStart, PreToolUse, PostToolUse, SubagentStop
    2. Files (near real-time) - history.jsonl, debug logs, transcripts
    3. Process (fallback) - psutil detection
    """

    def __init__(
        self,
        config: DaemonConfig,
        event_bus: "EventBus",
        storage: "StorageManager",
    ):
        super().__init__(
            name="claude_code",
            agent_type=AgentType.CLAUDE_CODE,
            data_sources=[DataSource.HOOKS, DataSource.FILES, DataSource.PROCESS],
            config=config,
            event_bus=event_bus,
            storage=storage,
        )

        self.claude_home = config.claude_home
        self.history_file = self.claude_home / "history.jsonl"
        self.debug_dir = self.claude_home / "debug"
        self.projects_dir = self.claude_home / "projects"
        self.stats_cache = self.claude_home / "stats-cache.json"

        self._watcher_task: Optional[asyncio.Task] = None
        self._stats_task: Optional[asyncio.Task] = None
        self._last_history_pos: int = 0
        self._project_sessions: dict[str, str] = {}  # project_path -> session_id
        self._last_stats_mtime: float = 0  # Track stats file modification time

    def get_capabilities(self) -> dict[str, bool]:
        """Claude Code has the richest capability set."""
        return {
            "real_time_events": True,
            "historical_data": True,
            "send_commands": True,
            "token_tracking": True,
            "cost_tracking": True,
            "file_change_tracking": True,
            "subagent_tracking": True,
            "hook_integration": True,
            "transcript_access": True,
        }

    async def start(self) -> None:
        """Start the Claude Code adapter."""
        self._running = True
        self.status = AdapterStatus.DISCOVERING

        # Load existing active sessions from database into memory
        await self.load_active_sessions()

        # Initial session discovery
        await self.discover_sessions()

        # Load initial token stats
        await self._load_token_stats()

        # Start file watcher
        self._watcher_task = asyncio.create_task(self._watch_files())

        # Start periodic stats updater
        self._stats_task = asyncio.create_task(self._periodic_stats_update())

        self.status = AdapterStatus.CONNECTED
        logger.info("Claude Code adapter started")

    async def stop(self) -> None:
        """Stop the adapter gracefully."""
        self._running = False

        if self._watcher_task:
            self._watcher_task.cancel()
            try:
                await self._watcher_task
            except asyncio.CancelledError:
                pass

        if self._stats_task:
            self._stats_task.cancel()
            try:
                await self._stats_task
            except asyncio.CancelledError:
                pass

        self.status = AdapterStatus.INACTIVE
        logger.info("Claude Code adapter stopped")

    async def _load_token_stats(self) -> None:
        """Load token usage from stats-cache.json."""
        if not self.stats_cache.exists():
            return

        try:
            with open(self.stats_cache) as f:
                stats = json.load(f)

            # Update the file modification time
            self._last_stats_mtime = self.stats_cache.stat().st_mtime

            updated_count = 0

            # Stats are organized by project path
            for project_path, project_stats in stats.items():
                if not isinstance(project_stats, dict):
                    continue

                # Skip empty or invalid project paths
                if not project_path or project_path == "/":
                    continue

                # Find session by exact project path match
                session = self._find_session_for_project(project_path)

                if not session:
                    # Create a session for this project from stats
                    # Use the last session ID from stats if available
                    last_session_id = project_stats.get("lastSessionId", "")
                    external_id = last_session_id or f"stats_{project_path}"

                    session, _ = await self.find_or_create_session(
                        external_id=external_id,
                        create_func=lambda pp=project_path, ps=project_stats: UnifiedSession.create(
                            agent_type=AgentType.CLAUDE_CODE,
                            project_path=pp,
                            external_id=ps.get("lastSessionId", f"stats_{pp}"),
                            metadata={"source": "stats_cache"},
                        ),
                    )

                # Apply token stats to the session
                self._apply_stats_to_session(session, project_stats)
                await self.update_session(session)
                updated_count += 1

                # Track project -> session mapping
                self._project_sessions[project_path] = session.id

            logger.debug(f"Loaded token stats for {updated_count} projects")

        except Exception as e:
            logger.error(f"Error loading token stats: {e}")

    def _find_session_for_project(self, project_path: str) -> Optional[UnifiedSession]:
        """Find a session matching the given project path."""
        # Try exact match first
        for session in self._sessions.values():
            if session.project_path == project_path:
                return session

        # Try partial match (project path might be a subdirectory)
        for session in self._sessions.values():
            if session.project_path and session.project_path != "/":
                if project_path.startswith(session.project_path) or session.project_path.startswith(project_path):
                    return session

        # Check if we have a cached project -> session mapping
        if project_path in self._project_sessions:
            session_id = self._project_sessions[project_path]
            return self._sessions.get(session_id)

        return None

    def _apply_stats_to_session(
        self,
        session: UnifiedSession,
        project_stats: dict[str, Any],
    ) -> None:
        """Apply stats from stats-cache.json to a session."""
        # Extract token counts
        if "totalTokens" in project_stats:
            tokens = project_stats["totalTokens"]
            if isinstance(tokens, dict):
                session.tokens_input = tokens.get("input", 0)
                session.tokens_output = tokens.get("output", 0)
            elif isinstance(tokens, int):
                # Estimate split if only total is available
                session.tokens_input = int(tokens * 0.3)
                session.tokens_output = int(tokens * 0.7)

        # Extract cost
        if "totalCost" in project_stats:
            session.estimated_cost = float(project_stats.get("totalCost", 0))
        elif session.tokens_input or session.tokens_output:
            # Calculate cost from tokens if not provided
            session.estimated_cost = calculate_cost(
                session.tokens_input,
                session.tokens_output,
                session.model_id or "claude-sonnet-4-20250514"
            )

        # Extract model info
        if "lastModel" in project_stats:
            session.model_id = project_stats.get("lastModel")

        # Extract message count if available
        if "messageCount" in project_stats:
            session.message_count = max(session.message_count, project_stats.get("messageCount", 0))

    async def _periodic_stats_update(self) -> None:
        """Periodically check for stats updates."""
        while self._running:
            try:
                await asyncio.sleep(30)  # Check every 30 seconds

                if not self.stats_cache.exists():
                    continue

                # Check if file has been modified
                current_mtime = self.stats_cache.stat().st_mtime
                if current_mtime > self._last_stats_mtime:
                    await self._load_token_stats()

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Error in stats update: {e}")

    async def discover_sessions(self) -> list[UnifiedSession]:
        """Discover Claude Code sessions from multiple sources."""
        discovered_sessions = []

        # Method 1: Process detection (highest priority for active sessions)
        proc_sessions = await self._discover_from_processes()

        # Method 2: Parse history file for recent sessions
        history_sessions = await self._discover_from_history()

        # Process sessions from both sources, deduplicating by external_id
        all_candidates = proc_sessions + history_sessions
        seen_external_ids: set[str] = set()

        for candidate in all_candidates:
            external_id = candidate.external_id

            # Skip if we've already processed this external_id in this discovery run
            if external_id in seen_external_ids:
                continue
            seen_external_ids.add(external_id)

            # Use find_or_create to prevent duplicates
            session, is_new = await self.find_or_create_session(
                external_id=external_id,
                create_func=lambda c=candidate: c,
            )

            if not is_new:
                # Update existing session with new info
                session.update_activity()
                if candidate.pid and not session.pid:
                    session.pid = candidate.pid
                # Update project path if we have a real one and current is generic
                if candidate.project_path and candidate.project_path != "/" and session.project_path in ("/", ""):
                    session.project_path = candidate.project_path
                # Merge metadata
                session.metadata.update(candidate.metadata)
                await self.update_session(session)

            discovered_sessions.append(session)

        return discovered_sessions

    async def _discover_from_processes(self) -> list[UnifiedSession]:
        """Find running Claude Code processes."""
        sessions = []

        for proc in psutil.process_iter(["pid", "name", "cmdline", "cwd", "create_time"]):
            try:
                info = proc.info
                name = (info.get("name") or "").lower()
                cmdline_list = info.get("cmdline") or []
                cmdline = " ".join(cmdline_list) if cmdline_list else ""

                # Check if this is a Claude Code process
                if "claude" in name or "@anthropic-ai/claude-code" in cmdline:
                    cwd = info.get("cwd") or ""
                    if cwd:
                        session = UnifiedSession.create(
                            agent_type=AgentType.CLAUDE_CODE,
                            project_path=cwd,
                            external_id=f"proc_{info['pid']}",
                            pid=info["pid"],
                            metadata={
                                "source": "process",
                                "cmdline": cmdline_list,
                            },
                        )
                        if info.get("create_time"):
                            session.started_at = datetime.fromtimestamp(info["create_time"])
                        sessions.append(session)

            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue

        return sessions

    async def _discover_from_history(self) -> list[UnifiedSession]:
        """Parse history.jsonl for recent sessions."""
        sessions = []

        if not self.history_file.exists():
            return sessions

        # Read last 1000 lines for recent sessions
        try:
            with open(self.history_file) as f:
                lines = f.readlines()[-1000:]

            # Group by project path
            project_entries: dict[str, list[dict]] = {}
            for line in lines:
                try:
                    entry = json.loads(line)
                    project = entry.get("project", "")
                    if project:
                        if project not in project_entries:
                            project_entries[project] = []
                        project_entries[project].append(entry)
                except json.JSONDecodeError:
                    continue

            # Create sessions from grouped entries
            for project, entries in project_entries.items():
                if not entries:
                    continue

                # Get first and last entry timestamps
                first_entry = entries[0]
                last_entry = entries[-1]

                first_ts = first_entry.get("timestamp", 0)
                last_ts = last_entry.get("timestamp", 0)

                session = UnifiedSession.create(
                    agent_type=AgentType.CLAUDE_CODE,
                    project_path=project,
                    external_id=first_entry.get("sessionId", str(uuid.uuid4())),
                    message_count=len(entries),
                    metadata={
                        "source": "history",
                        "first_message": first_entry.get("display", "")[:100],
                    },
                )

                if first_ts:
                    session.started_at = datetime.fromtimestamp(first_ts / 1000)
                if last_ts:
                    session.last_activity_at = datetime.fromtimestamp(last_ts / 1000)

                # Check if still active (activity in last 30 minutes)
                if last_ts and (datetime.now().timestamp() * 1000 - last_ts) < 30 * 60 * 1000:
                    session.status = SessionStatus.ACTIVE
                else:
                    session.status = SessionStatus.COMPLETED

                sessions.append(session)

        except Exception as e:
            logger.error(f"Error parsing history file: {e}")

        return sessions

    async def _watch_files(self) -> None:
        """Watch Claude Code files for changes."""
        watch_paths = [
            self.history_file,
            self.debug_dir,
        ]

        # Only watch paths that exist
        existing_paths = [p for p in watch_paths if p.exists()]
        if not existing_paths:
            logger.warning("No Claude Code paths to watch")
            return

        try:
            async for changes in awatch(*existing_paths, debounce=500, step=200):
                if not self._running:
                    break

                for change_type, path in changes:
                    try:
                        await self._handle_file_change(change_type, Path(path))
                    except Exception as e:
                        logger.error(f"Error handling file change {path}: {e}")

        except asyncio.CancelledError:
            pass
        except Exception as e:
            logger.error(f"File watcher error: {e}")

    async def _handle_file_change(self, change_type: Change, path: Path) -> None:
        """Handle a file change event."""
        if path.name == "history.jsonl":
            await self._process_history_update()
        elif path.parent.name == "debug" and path.suffix == ".txt":
            await self._process_debug_update(path)
        elif path.suffix == ".jsonl" and "projects" in str(path):
            await self._process_conversation_update(path)

    async def _process_history_update(self) -> None:
        """Process new entries in history.jsonl."""
        if not self.history_file.exists():
            return

        try:
            with open(self.history_file) as f:
                # Seek to last position
                f.seek(self._last_history_pos)
                new_lines = f.readlines()
                self._last_history_pos = f.tell()

            for line in new_lines:
                try:
                    entry = json.loads(line)
                    await self._process_history_entry(entry)
                except json.JSONDecodeError:
                    continue

        except Exception as e:
            logger.error(f"Error processing history update: {e}")

    async def _process_history_entry(self, entry: dict[str, Any]) -> None:
        """Process a single history entry."""
        project = entry.get("project", "")
        session_id = entry.get("sessionId", "")

        if not project:
            return

        # Find or create session
        session = None
        if session_id:
            session = self._sessions.get(session_id)
        if not session and project in self._project_sessions:
            session = self._sessions.get(self._project_sessions[project])

        if not session:
            session = UnifiedSession.create(
                agent_type=AgentType.CLAUDE_CODE,
                project_path=project,
                external_id=session_id or str(uuid.uuid4()),
            )
            await self.register_session(session)
            if session_id:
                self._project_sessions[project] = session.id

        # Update session
        session.update_activity()
        session.message_count += 1

        # Emit event
        event = SessionEvent.create(
            session_id=session.id,
            event_type=EventType.PROMPT_RECEIVED,
            agent_type=AgentType.CLAUDE_CODE,
            content=entry.get("display", "")[:500],
            working_directory=project,
            raw_data=entry,
        )
        await self.emit_event(event)
        await self.update_session(session)

    async def _process_debug_update(self, debug_path: Path) -> None:
        """Process updates to a debug log file."""
        session_id = debug_path.stem

        # Read last few lines for new events
        try:
            with open(debug_path) as f:
                lines = f.readlines()[-50:]

            for line in lines:
                # Parse debug log format: timestamp [LEVEL] message
                if "[" not in line:
                    continue

                try:
                    parts = line.strip().split(" ", 3)
                    if len(parts) >= 3:
                        timestamp_str = parts[0]
                        level = parts[1].strip("[]")
                        message = parts[2] if len(parts) > 2 else ""

                        # Look for tool execution patterns
                        if "Tool" in message or "Bash" in message or "Read" in message:
                            session = self._sessions.get(session_id)
                            if session:
                                session.tool_call_count += 1
                                await self.update_session(session)

                except Exception:
                    continue

        except Exception as e:
            logger.debug(f"Error processing debug file {debug_path}: {e}")

    async def _process_conversation_update(self, conv_path: Path) -> None:
        """Process updates to a conversation file to extract Claude's responses and tool calls."""
        session_id = conv_path.stem

        try:
            # Read last few entries from the conversation file
            with open(conv_path) as f:
                lines = f.readlines()[-20:]

            for line in lines:
                try:
                    entry = json.loads(line)
                    await self._process_conversation_entry(entry, session_id)
                except json.JSONDecodeError:
                    continue

        except Exception as e:
            logger.debug(f"Error processing conversation file {conv_path}: {e}")

    async def _process_conversation_entry(self, entry: dict[str, Any], session_id: str) -> None:
        """Process a single conversation entry (Claude's response or tool call)."""
        entry_type = entry.get("type", "")
        message = entry.get("message", {})
        project = entry.get("cwd", "")

        # Find or create session
        session = self._sessions.get(session_id)
        if not session:
            # Try to find by project path
            for s in self._sessions.values():
                if s.project_path == project:
                    session = s
                    break

        if not session:
            return

        # Extract token usage from the entry if present
        usage = entry.get("usage", {}) or message.get("usage", {})
        if usage:
            input_tokens = usage.get("input_tokens", 0)
            output_tokens = usage.get("output_tokens", 0)
            if input_tokens:
                session.tokens_input += input_tokens
            if output_tokens:
                session.tokens_output += output_tokens

            # Update cost estimate
            if input_tokens or output_tokens:
                model_id = entry.get("model") or session.model_id or "claude-sonnet-4-20250514"
                session.estimated_cost = calculate_cost(
                    session.tokens_input,
                    session.tokens_output,
                    model_id
                )
                if entry.get("model"):
                    session.model_id = entry.get("model")

        content_blocks = message.get("content", [])
        if not isinstance(content_blocks, list):
            return

        for block in content_blocks:
            if not isinstance(block, dict):
                continue

            block_type = block.get("type", "")

            if block_type == "text" and entry_type == "assistant":
                # Claude's text response
                text = block.get("text", "")[:500]
                event = SessionEvent.create(
                    session_id=session.id,
                    event_type=EventType.RESPONSE_GENERATED,
                    agent_type=AgentType.CLAUDE_CODE,
                    content=text,
                    working_directory=project,
                    raw_data={"type": "text", "text": text},
                )
                await self.emit_event(event)

            elif block_type == "tool_use":
                # Claude calling a tool
                tool_name = block.get("name", "unknown")
                tool_input = block.get("input", {})

                session.tool_call_count += 1

                # Determine event type based on tool
                if tool_name in ("Read", "Glob", "Grep"):
                    event_type = EventType.FILE_READ
                elif tool_name in ("Write", "Edit"):
                    event_type = EventType.FILE_MODIFIED
                    session.file_operations += 1
                elif tool_name == "Bash":
                    event_type = EventType.TOOL_EXECUTED
                else:
                    event_type = EventType.TOOL_EXECUTED

                event = SessionEvent.create(
                    session_id=session.id,
                    event_type=event_type,
                    agent_type=AgentType.CLAUDE_CODE,
                    content=f"{tool_name}: {str(tool_input)[:200]}",
                    working_directory=project,
                    tool_name=tool_name,
                    raw_data={"tool": tool_name, "input": tool_input},
                )
                await self.emit_event(event)

            elif block_type == "thinking":
                # Claude's thinking (extended thinking)
                thinking = block.get("thinking", "")[:300]
                event = SessionEvent.create(
                    session_id=session.id,
                    event_type=EventType.THINKING,
                    agent_type=AgentType.CLAUDE_CODE,
                    content=thinking,
                    working_directory=project,
                    raw_data={"type": "thinking"},
                )
                await self.emit_event(event)

        # Update session
        session.update_activity()
        await self.update_session(session)

    async def parse_conversation(self, session_id: str) -> list[dict]:
        """Parse a full conversation file for a session."""
        events = []

        # Find the conversation file
        for project_dir in self.projects_dir.iterdir():
            conv_file = project_dir / f"{session_id}.jsonl"
            if conv_file.exists():
                try:
                    with open(conv_file) as f:
                        for line in f:
                            try:
                                entry = json.loads(line)
                                events.append(self._format_conversation_entry(entry))
                            except json.JSONDecodeError:
                                continue
                except Exception as e:
                    logger.error(f"Error parsing conversation {session_id}: {e}")
                break

        return events

    def _format_conversation_entry(self, entry: dict) -> dict:
        """Format a conversation entry for display."""
        entry_type = entry.get("type", "")
        message = entry.get("message", {})
        timestamp = entry.get("timestamp", 0)

        result = {
            "type": entry_type,
            "timestamp": timestamp,
            "content": [],
        }

        content_blocks = message.get("content", [])
        if isinstance(content_blocks, list):
            for block in content_blocks:
                if isinstance(block, dict):
                    block_type = block.get("type", "")
                    if block_type == "text":
                        result["content"].append({
                            "type": "text",
                            "text": block.get("text", ""),
                        })
                    elif block_type == "tool_use":
                        result["content"].append({
                            "type": "tool",
                            "name": block.get("name", ""),
                            "input": block.get("input", {}),
                        })
                    elif block_type == "thinking":
                        result["content"].append({
                            "type": "thinking",
                            "text": block.get("thinking", ""),
                        })

        return result

    async def parse_history(self) -> list[SessionEvent]:
        """Parse full history file for past events."""
        events = []

        if not self.history_file.exists():
            return events

        try:
            with open(self.history_file) as f:
                for line in f:
                    try:
                        entry = json.loads(line)
                        event = self._history_entry_to_event(entry)
                        if event:
                            events.append(event)
                    except json.JSONDecodeError:
                        continue

        except Exception as e:
            logger.error(f"Error parsing full history: {e}")

        return events

    def _history_entry_to_event(self, entry: dict[str, Any]) -> Optional[SessionEvent]:
        """Convert a history entry to a SessionEvent."""
        project = entry.get("project", "")
        if not project:
            return None

        timestamp = entry.get("timestamp", 0)
        if not timestamp:
            return None

        return SessionEvent(
            id=str(uuid.uuid4()),
            session_id=entry.get("sessionId", "unknown"),
            event_type=EventType.PROMPT_RECEIVED,
            timestamp=datetime.fromtimestamp(timestamp / 1000),
            agent_type=AgentType.CLAUDE_CODE,
            content=entry.get("display", ""),
            working_directory=project,
            raw_data=entry,
        )


# =========================================================================
# Hook Installation
# =========================================================================

HOOK_CONFIG = {
    "SessionStart": [
        {
            "hooks": [
                {
                    "type": "command",
                    "command": "agent-monitor hook session_start",
                    "timeout": 5,
                }
            ]
        }
    ],
    "SessionEnd": [
        {
            "hooks": [
                {
                    "type": "command",
                    "command": "agent-monitor hook session_end",
                    "timeout": 5,
                }
            ]
        }
    ],
    "PreToolUse": [
        {
            "matcher": ".*",
            "hooks": [
                {
                    "type": "command",
                    "command": "agent-monitor hook tool_start",
                    "timeout": 5,
                }
            ],
        }
    ],
    "PostToolUse": [
        {
            "matcher": ".*",
            "hooks": [
                {
                    "type": "command",
                    "command": "agent-monitor hook tool_complete",
                    "timeout": 5,
                }
            ],
        }
    ],
    "SubagentStop": [
        {
            "hooks": [
                {
                    "type": "command",
                    "command": "agent-monitor hook subagent_stop",
                    "timeout": 5,
                }
            ]
        }
    ],
}


async def install_hooks(claude_home: Path) -> None:
    """
    Install monitoring hooks into Claude Code settings.

    Merges our hooks with existing hooks in settings.json.
    """
    settings_path = claude_home / "settings.json"

    # Load existing settings
    existing: dict[str, Any] = {}
    if settings_path.exists():
        try:
            with open(settings_path) as f:
                existing = json.load(f)
        except json.JSONDecodeError:
            logger.warning("Could not parse existing settings.json")

    # Merge hooks
    if "hooks" not in existing:
        existing["hooks"] = {}

    hooks_modified = False
    for event_type, config in HOOK_CONFIG.items():
        if event_type not in existing["hooks"]:
            existing["hooks"][event_type] = []

        # Check if our hook is already installed
        our_command = f"agent-monitor hook"
        already_installed = any(
            our_command in str(hook)
            for hook in existing["hooks"][event_type]
        )

        if not already_installed:
            existing["hooks"][event_type].extend(config)
            hooks_modified = True

    # Write back if modified
    if hooks_modified:
        settings_path.parent.mkdir(parents=True, exist_ok=True)
        with open(settings_path, "w") as f:
            json.dump(existing, f, indent=2)
        logger.info(f"Installed monitoring hooks in {settings_path}")
    else:
        logger.debug("Monitoring hooks already installed")
